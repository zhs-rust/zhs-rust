use zhs_entity::DataError;
use zhs_json::{Object, Value};

use crate::packet::{Reader, read_b_varbyte};
use crate::types::Column;

#[derive(Debug, Default)]
pub(crate) struct Response {
    pub(crate) results: Vec<Vec<Object>>,

    pub(crate) counts: Vec<u64>,

    pub(crate) login_ack: bool,

    pub(crate) transaction: Option<TransactionChange>,
}

#[derive(Debug)]
pub(crate) enum TransactionChange {
    Begin([u8; 8]),

    End,
}

impl Response {
    pub(crate) fn affected_rows(&self) -> u64 {
        self.counts.first().copied().unwrap_or(0)
    }

    pub(crate) fn last_insert_id(&self) -> Option<u64> {
        self.results.iter().find_map(|rows| {
            let value = rows.first()?.get("last_insert_id")?;
            let value = value.as_i64()?;
            u64::try_from(value).ok()
        })
    }
}

pub(crate) fn parse(payload: &[u8]) -> Result<Response, DataError> {
    let mut reader = Reader::new(payload);
    let mut response = Response::default();

    let mut columns: Option<Vec<Column>> = None;
    let mut rows: Vec<Object> = Vec::new();
    let mut error: Option<DataError> = None;

    while reader.remaining() > 0 {
        let token = reader.u8()?;

        match token {
            0x81 => {
                let count = reader.u16le()?;
                if count == 0xFFFF {
                    columns = None;
                    rows.clear();
                } else {
                    let mut parsed = Vec::with_capacity(count as usize);
                    for _ in 0..count {
                        parsed.push(Column::read(&mut reader, true)?);
                    }
                    columns = Some(parsed);
                    rows = Vec::new();
                }
            }

            0xD1 | 0xD2 => {
                let columns = columns.as_ref().ok_or_else(|| {
                    DataError::protocol("a row arrived before any column metadata")
                })?;

                let nulls = if token == 0xD2 {
                    Some(reader.take(columns.len().div_ceil(8))?.to_vec())
                } else {
                    None
                };

                let mut row = Object::with_capacity(columns.len());
                for (index, column) in columns.iter().enumerate() {
                    let is_null = nulls
                        .as_ref()
                        .is_some_and(|bits| bits[index / 8] & (1 << (index % 8)) != 0);
                    let value = if is_null {
                        Value::Null
                    } else {
                        column.read_value(&mut reader)?
                    };
                    row.insert(column.name.clone(), value);
                }
                rows.push(row);
            }

            0xFD..=0xFF => {
                let status = reader.u16le()?;
                let _current_command = reader.u16le()?;
                let count = reader.u64le()?;

                if status & 0x0010 != 0 {
                    response.counts.push(count);
                }

                if columns.is_some() {
                    response.results.push(std::mem::take(&mut rows));
                    columns = None;
                }
            }

            0xAA => {
                let token = ErrorToken::read(&mut reader)?;
                if token.class >= 11 && error.is_none() {
                    error = Some(DataError::Server {
                        code: token.number,
                        state: format!("{:02X}", token.state),
                        message: token.message,
                    });
                }
            }

            0xAB => {
                let _ = ErrorToken::read(&mut reader)?;
            }

            0xE3 => {
                let length = reader.u16le()? as usize;
                let body = reader.take(length)?;

                let mut inner = Reader::new(body);
                if let Ok(kind) = inner.u8() {
                    let new_value = read_b_varbyte(&mut inner).unwrap_or_default();
                    match kind {
                        8 => {
                            if let Ok(descriptor) = <[u8; 8]>::try_from(new_value.as_slice()) {
                                response.transaction = Some(TransactionChange::Begin(descriptor));
                            }
                        }
                        9 | 10 => response.transaction = Some(TransactionChange::End),
                        _ => {}
                    }
                }
            }

            0xAD => {
                response.login_ack = true;
                let length = reader.u16le()? as usize;
                reader.take(length)?;
            }

            0xA9 | 0xA4 | 0xA5 | 0xAE | 0xED => {
                let length = reader.u16le()? as usize;
                reader.take(length)?;
            }

            0x79 => {
                let _ = reader.u32le()?;
            }

            0xAC => {
                let _ordinal = reader.u16le()?;
                let _name = reader.b_varchar()?;
                let _status = reader.u8()?;
                let column = Column::read(&mut reader, false)?;
                let _value = column.read_value(&mut reader)?;
            }

            0xE4 => {
                let length = reader.u32le()? as usize;
                reader.take(length)?;
            }

            other => {
                return Err(DataError::protocol(format!(
                    "unknown TDS token {other:#04x}"
                )));
            }
        }
    }

    if columns.is_some() {
        response.results.push(rows);
    }

    match error {
        Some(error) => Err(error),
        None => Ok(response),
    }
}

struct ErrorToken {
    number: u32,
    state: u8,
    class: u8,
    message: String,
}

impl ErrorToken {
    fn read(reader: &mut Reader<'_>) -> Result<Self, DataError> {
        let _length = reader.u16le()?;
        let number = reader.u32le()?;
        let state = reader.u8()?;
        let class = reader.u8()?;
        let characters = reader.u16le()? as usize;
        let message = reader.utf16(characters)?;
        let _server = reader.b_varchar()?;
        let _procedure = reader.b_varchar()?;
        let _line = reader.u32le()?;

        Ok(Self {
            number,
            state,
            class,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{push_u8, push_u16le, push_u32le, push_u64le, push_utf16};

    fn int_column(name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_u32le(&mut bytes, 0);
        push_u16le(&mut bytes, 0x0001);
        push_u8(&mut bytes, 0x38);
        bytes.push(name.encode_utf16().count() as u8);
        push_utf16(&mut bytes, name);
        bytes
    }

    fn metadata(columns: &[&str]) -> Vec<u8> {
        let mut bytes = vec![0x81];
        push_u16le(&mut bytes, columns.len() as u16);
        for column in columns {
            bytes.extend_from_slice(&int_column(column));
        }
        bytes
    }

    fn done(count: u64, more: bool) -> Vec<u8> {
        let mut bytes = vec![0xFD];
        push_u16le(&mut bytes, 0x0010 | u16::from(more));
        push_u16le(&mut bytes, 0);
        push_u64le(&mut bytes, count);
        bytes
    }

    #[test]
    fn parses_rows_and_affected_counts() {
        let mut payload = metadata(&["id", "age"]);
        payload.push(0xD1);
        payload.extend_from_slice(&7i32.to_le_bytes());
        payload.extend_from_slice(&36i32.to_le_bytes());
        payload.extend_from_slice(&done(1, false));

        let response = parse(&payload).unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0][0].get("id"), Some(&Value::from(7i64)));
        assert_eq!(response.results[0][0].get("age"), Some(&Value::from(36i64)));
        assert_eq!(response.affected_rows(), 1);
    }

    #[test]
    fn parses_nbcrow_nulls() {
        let mut payload = metadata(&["id", "age"]);
        payload.push(0xD2);

        payload.push(0b0000_0010);
        payload.extend_from_slice(&7i32.to_le_bytes());
        payload.extend_from_slice(&done(1, false));

        let response = parse(&payload).unwrap();
        assert_eq!(response.results[0][0].get("id"), Some(&Value::from(7i64)));
        assert_eq!(response.results[0][0].get("age"), Some(&Value::Null));
    }

    #[test]
    fn parses_two_result_sets_in_order() {
        let mut payload = metadata(&["a"]);
        payload.push(0xD1);
        payload.extend_from_slice(&1i32.to_le_bytes());
        payload.extend_from_slice(&done(1, true));
        payload.extend_from_slice(&metadata(&["last_insert_id"]));
        payload.push(0xD1);
        payload.extend_from_slice(&9i32.to_le_bytes());
        payload.extend_from_slice(&done(1, false));

        let response = parse(&payload).unwrap();
        assert_eq!(response.results.len(), 2);
        assert_eq!(response.last_insert_id(), Some(9));
    }

    #[test]
    fn records_the_transaction_descriptor_from_envchange() {
        let mut payload = vec![0xE3];
        let mut body = vec![8u8, 8u8];
        body.extend_from_slice(&[0x0D, 0x00, 0x00, 0x00, 0x43, 0x00, 0x00, 0x00]);
        body.push(0);
        push_u16le(&mut payload, body.len() as u16);
        payload.extend_from_slice(&body);
        payload.extend_from_slice(&done(0, false));

        let response = parse(&payload).unwrap();
        match response.transaction {
            Some(TransactionChange::Begin(descriptor)) => {
                assert_eq!(descriptor, [0x0D, 0x00, 0x00, 0x00, 0x43, 0x00, 0x00, 0x00]);
            }
            other => panic!("expected a begin transaction change, got {other:?}"),
        }

        let mut payload = vec![0xE3];
        let mut body = vec![9u8, 8u8];
        body.extend_from_slice(&[0; 8]);
        body.push(8);
        body.extend_from_slice(&[0x0D, 0x00, 0x00, 0x00, 0x43, 0x00, 0x00, 0x00]);
        push_u16le(&mut payload, body.len() as u16);
        payload.extend_from_slice(&body);
        payload.extend_from_slice(&done(0, false));

        let response = parse(&payload).unwrap();
        assert!(matches!(response.transaction, Some(TransactionChange::End)));
    }

    #[test]
    fn surfaces_server_errors_with_code_and_state() {
        let mut payload = vec![0xAA];
        let mut body = Vec::new();
        push_u32le(&mut body, 2627);
        push_u8(&mut body, 1);
        push_u8(&mut body, 14);
        let message = "Violation of UNIQUE";
        push_u16le(&mut body, message.encode_utf16().count() as u16);
        push_utf16(&mut body, message);
        push_u8(&mut body, 1);
        push_utf16(&mut body, "s");
        push_u8(&mut body, 1);
        push_utf16(&mut body, "p");
        push_u32le(&mut body, 1);

        push_u16le(&mut payload, body.len() as u16);
        payload.extend_from_slice(&body);

        let error = parse(&payload).unwrap_err();
        assert_eq!(
            error,
            DataError::Server {
                code: 2627,
                state: "01".to_owned(),
                message: "Violation of UNIQUE".to_owned(),
            }
        );
    }
}
