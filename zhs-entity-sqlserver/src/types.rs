use zhs_entity::DataError;
use zhs_json::Value;

use crate::packet::{Reader, push_u8, push_u16le, push_utf16, read_plp};

const COLLATION_LATIN1: [u8; 5] = [0x09, 0x04, 0xD0, 0x00, 0x34];

const T_NULL: u8 = 0x1F;
const T_IMAGE: u8 = 0x22;
const T_TEXT: u8 = 0x23;
const T_GUID: u8 = 0x24;
const T_INTN: u8 = 0x26;
const T_DATE: u8 = 0x28;
const T_TIME: u8 = 0x29;
const T_DATETIME2: u8 = 0x2A;
const T_DATETIMEOFFSET: u8 = 0x2B;
const T_INT1: u8 = 0x30;
const T_BIT: u8 = 0x32;
const T_INT2: u8 = 0x34;
const T_INT4: u8 = 0x38;
const T_FLT4: u8 = 0x3B;
const T_MONEY: u8 = 0x3C;
const T_DATETIME: u8 = 0x3D;
const T_FLT8: u8 = 0x3E;
const T_SQLVARIANT: u8 = 0x62;
const T_NTEXT: u8 = 0x63;
const T_BITN: u8 = 0x68;
const T_DECIMALN: u8 = 0x6A;
const T_NUMERICN: u8 = 0x6C;
const T_FLTN: u8 = 0x6D;
const T_MONEYN: u8 = 0x6E;
const T_DATETIMN: u8 = 0x6F;
const T_MONEY4: u8 = 0x7A;
const T_INT8: u8 = 0x7F;
const T_VARBINARY: u8 = 0xA5;
const T_VARCHAR: u8 = 0xA7;
const T_BINARY: u8 = 0xAD;
const T_CHAR: u8 = 0xAF;
const T_NVARCHAR: u8 = 0xE7;
const T_NCHAR: u8 = 0xEF;
const T_XML: u8 = 0xF1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Column {
    pub(crate) name: String,

    pub(crate) type_id: u8,

    pub(crate) flags: u16,

    pub(crate) max_len: usize,

    pub(crate) scale: u8,

    pub(crate) precision: u8,
}

impl Column {
    pub(crate) fn read(reader: &mut Reader<'_>, with_name: bool) -> Result<Self, DataError> {
        let _user_type = reader.u32le()?;
        let flags = reader.u16le()?;
        let type_id = reader.u8()?;

        let mut max_len = 0;
        let mut scale = 0;
        let mut precision = 0;

        match type_id {
            T_NULL | T_INT1 | T_BIT | T_INT2 | T_INT4 | T_INT8 | T_FLT4 | T_FLT8 | T_MONEY
            | T_MONEY4 | T_DATETIME => {}
            T_GUID | T_INTN | T_FLTN | T_MONEYN | T_BITN | T_DATETIMN => {
                max_len = reader.u8()? as usize;
            }
            T_DECIMALN | T_NUMERICN => {
                max_len = reader.u8()? as usize;
                precision = reader.u8()?;
                scale = reader.u8()?;
                if scale > 38 {
                    return Err(DataError::protocol(format!(
                        "unexpected decimal scale {scale} (max 38)"
                    )));
                }
            }
            T_DATE => {
                max_len = 3;
            }
            T_TIME | T_DATETIME2 | T_DATETIMEOFFSET => {
                scale = reader.u8()?;
                if scale > 7 {
                    return Err(DataError::protocol(format!(
                        "unexpected {type_id:#04x} scale {scale}"
                    )));
                }
                let time_len = match scale {
                    0..=2 => 3,
                    3..=4 => 4,
                    _ => 5,
                };
                max_len = time_len
                    + match type_id {
                        T_TIME => 0,
                        T_DATETIME2 => 3,
                        _ => 5,
                    };
            }
            T_VARBINARY | T_BINARY => {
                max_len = reader.u16le()? as usize;
            }
            T_VARCHAR | T_CHAR | T_NVARCHAR | T_NCHAR => {
                max_len = reader.u16le()? as usize;
                reader.take(5)?;
            }
            T_IMAGE | T_TEXT | T_NTEXT => {
                return Err(DataError::Unsupported(
                    "legacy text / image columns; use nvarchar(max) or varbinary(max)",
                ));
            }
            T_XML => {
                return Err(DataError::Unsupported(
                    "xml columns; cast them to nvarchar(max) in the query",
                ));
            }
            T_SQLVARIANT => {
                return Err(DataError::Unsupported("sql_variant columns"));
            }
            other => {
                return Err(DataError::Unsupported(match other {
                    0xF0 => "user-defined types",
                    _ => "this SQL Server column type",
                }));
            }
        }

        let name = if with_name {
            reader.b_varchar()?
        } else {
            String::new()
        };

        Ok(Self {
            name,
            type_id,
            flags,
            max_len,
            scale,
            precision,
        })
    }

    pub(crate) fn read_value(&self, reader: &mut Reader<'_>) -> Result<Value, DataError> {
        match self.type_id {
            T_INT1 => Ok(Value::from(i64::from(reader.u8()?))),
            T_BIT => Ok(Value::from(reader.u8()? != 0)),
            T_INT2 => {
                let bytes = reader.take(2)?;
                Ok(Value::from(i64::from(i16::from_le_bytes([
                    bytes[0], bytes[1],
                ]))))
            }
            T_INT4 => {
                let bytes = reader.take(4)?;
                Ok(Value::from(i64::from(i32::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3],
                ]))))
            }
            T_INT8 => {
                let bytes = reader.take(8)?;
                Ok(Value::from(i64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ])))
            }
            T_FLT4 => {
                let bits = reader.u32le()?;
                Ok(Value::from(f64::from(f32::from_bits(bits))))
            }
            T_FLT8 => {
                let bits = reader.u64le()?;
                Ok(Value::from(f64::from_bits(bits)))
            }
            T_MONEY => {
                let bytes = reader.take(8)?;
                let raw = i64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                Ok(Value::from(raw as f64 / 10_000.0))
            }
            T_MONEY4 => {
                let bytes = reader.take(4)?;
                let raw = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                Ok(Value::from(f64::from(raw) / 10_000.0))
            }
            T_DATETIME => {
                let bytes = reader.take(8)?;
                let days = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let ticks = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                Ok(Value::from(datetime(days, ticks)))
            }
            T_INTN | T_FLTN | T_MONEYN | T_BITN | T_DATETIMN => {
                let length = reader.u8()? as usize;
                if length == 0 {
                    return Ok(Value::Null);
                }
                let bytes = reader.take(length)?;
                match self.type_id {
                    T_BITN => Ok(Value::from(bytes[0] != 0)),
                    T_INTN => Ok(Value::from(match length {
                        1 => i64::from(bytes[0] as i8),
                        2 => i64::from(i16::from_le_bytes([bytes[0], bytes[1]])),
                        4 => {
                            i64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                        }
                        8 => i64::from_le_bytes([
                            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
                            bytes[7],
                        ]),
                        _ => {
                            return Err(DataError::protocol(format!(
                                "int column with length {length}"
                            )));
                        }
                    })),
                    T_FLTN => Ok(Value::from(match length {
                        4 => f64::from(f32::from_bits(u32::from_le_bytes([
                            bytes[0], bytes[1], bytes[2], bytes[3],
                        ]))),
                        8 => f64::from_bits(uint64(bytes)?),
                        _ => {
                            return Err(DataError::protocol(format!(
                                "float column with length {length}"
                            )));
                        }
                    })),
                    T_MONEYN => Ok(Value::from(match length {
                        4 => {
                            f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                                / 10_000.0
                        }
                        8 => int64(bytes)? as f64 / 10_000.0,
                        _ => {
                            return Err(DataError::protocol(format!(
                                "money column with length {length}"
                            )));
                        }
                    })),
                    T_DATETIMN => Ok(match length {
                        4 => {
                            let days = u16::from_le_bytes([bytes[0], bytes[1]]);
                            let minutes = u16::from_le_bytes([bytes[2], bytes[3]]);
                            Value::from(smalldatetime(days, minutes))
                        }
                        8 => {
                            let days = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                            let ticks =
                                u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                            Value::from(datetime(days, ticks))
                        }
                        _ => {
                            return Err(DataError::protocol(format!(
                                "datetime column with length {length}"
                            )));
                        }
                    }),
                    _ => unreachable!("the outer arm only matches the five variable types"),
                }
            }
            T_DECIMALN | T_NUMERICN => {
                let length = reader.u8()? as usize;
                if length == 0 {
                    return Ok(Value::Null);
                }
                let bytes = reader.take(length)?;
                decimal(bytes, self.scale)
            }
            T_GUID => {
                let length = reader.u8()? as usize;
                if length == 0 {
                    return Ok(Value::Null);
                }
                let bytes = reader.take(length)?;
                if bytes.len() != 16 {
                    return Err(DataError::protocol("a GUID value must be 16 bytes"));
                }
                Ok(Value::from(guid(bytes)))
            }
            T_DATE => {
                let length = reader.u8()? as usize;
                if length == 0 {
                    return Ok(Value::Null);
                }
                let bytes = reader.take(length)?;
                if bytes.len() != 3 {
                    return Err(DataError::protocol(format!(
                        "a date value must be 3 bytes, got {}",
                        bytes.len()
                    )));
                }
                Ok(Value::from(format_days(
                    u64_from(bytes)? as i64 - UNIX_EPOCH_DAYS,
                )))
            }
            T_TIME => {
                let length = reader.u8()? as usize;
                if length == 0 {
                    return Ok(Value::Null);
                }
                let bytes = reader.take(length)?;
                if !(3..=5).contains(&bytes.len()) {
                    return Err(DataError::protocol(format!(
                        "a time value must be 3 to 5 bytes, got {}",
                        bytes.len()
                    )));
                }
                Ok(Value::from(format_time(u64_from(bytes)?, self.scale)))
            }
            T_DATETIME2 => {
                let length = reader.u8()? as usize;
                if length == 0 {
                    return Ok(Value::Null);
                }
                let bytes = reader.take(length)?;
                if !(6..=8).contains(&bytes.len()) {
                    return Err(DataError::protocol(format!(
                        "a datetime2 value must be 6 to 8 bytes, got {}",
                        bytes.len()
                    )));
                }
                let (time, date) = bytes.split_at(bytes.len() - 3);
                Ok(Value::from(format!(
                    "{}T{}",
                    format_days(u64_from(date)? as i64 - UNIX_EPOCH_DAYS),
                    format_time(u64_from(time)?, self.scale)
                )))
            }
            T_DATETIMEOFFSET => {
                let length = reader.u8()? as usize;
                if length == 0 {
                    return Ok(Value::Null);
                }
                let bytes = reader.take(length)?;
                if !(8..=10).contains(&bytes.len()) {
                    return Err(DataError::protocol(format!(
                        "a datetimeoffset value must be 8 to 10 bytes, got {}",
                        bytes.len()
                    )));
                }
                let (time, rest) = bytes.split_at(bytes.len() - 5);
                let date = &rest[..3];
                let offset = i16::from_le_bytes([rest[3], rest[4]]);

                let per_second = 10i128.pow(u32::from(self.scale));
                let day = 86_400 * per_second;
                let mut units =
                    i128::from(u64_from(time)?) + i128::from(offset) * 60 * per_second;
                let mut days = u64_from(date)? as i64;
                while units < 0 {
                    units += day;
                    days -= 1;
                }
                while units >= day {
                    units -= day;
                    days += 1;
                }

                Ok(Value::from(format!(
                    "{}T{}{}",
                    format_days(days - UNIX_EPOCH_DAYS),
                    format_time(units as u64, self.scale),
                    format_offset(offset)
                )))
            }
            T_VARBINARY | T_BINARY => {
                if self.max_len == 0xFFFF {
                    return Ok(match read_plp(reader)? {
                        Some(bytes) => bytes_to_value(&bytes),
                        None => Value::Null,
                    });
                }
                let length = reader.u16le()? as usize;
                if length == 0xFFFF {
                    return Ok(Value::Null);
                }
                Ok(bytes_to_value(reader.take(length)?))
            }
            T_NVARCHAR | T_NCHAR => {
                if self.max_len == 0xFFFF {
                    return Ok(match read_plp(reader)? {
                        Some(bytes) => Value::from(crate::packet::decode_utf16(&bytes)?),
                        None => Value::Null,
                    });
                }
                let length = reader.u16le()? as usize;
                if length == 0xFFFF {
                    return Ok(Value::Null);
                }
                Ok(Value::from(crate::packet::decode_utf16(
                    reader.take(length)?,
                )?))
            }
            T_VARCHAR | T_CHAR => {
                if self.max_len == 0xFFFF {
                    return Ok(match read_plp(reader)? {
                        Some(bytes) => Value::from(latin1_or_utf8(&bytes)),
                        None => Value::Null,
                    });
                }
                let length = reader.u16le()? as usize;
                if length == 0xFFFF {
                    return Ok(Value::Null);
                }
                Ok(Value::from(latin1_or_utf8(reader.take(length)?)))
            }
            other => Err(DataError::Unsupported(match other {
                0x1F => "a NULL-typed column",
                _ => "this SQL Server column type",
            })),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ParamKind {
    Null,

    Bit(bool),

    Int(i64),

    Float(f64),

    Text(String),

    Bytes(Vec<u8>),
}

pub(crate) fn classify(value: &Value) -> Result<ParamKind, DataError> {
    match value {
        Value::Null => Ok(ParamKind::Null),
        Value::Bool(value) => Ok(ParamKind::Bit(*value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(ParamKind::Int(value))
            } else if let Some(value) = number.as_u64() {
                Ok(ParamKind::Text(value.to_string()))
            } else {
                Ok(ParamKind::Float(number.as_f64()))
            }
        }
        Value::String(text) => Ok(ParamKind::Text(text.clone())),
        Value::Array(items) => {
            let mut bytes = Vec::with_capacity(items.len());
            for item in items {
                match item.as_u64() {
                    Some(byte) if byte <= u64::from(u8::MAX) => bytes.push(byte as u8),
                    _ => {
                        return Err(DataError::Unsupported(
                            "arrays may only bind as bytes (integers 0..=255)",
                        ));
                    }
                }
            }
            Ok(ParamKind::Bytes(bytes))
        }
        Value::Object(_) => Err(DataError::Unsupported(
            "binding a JSON object; serialize it to text first",
        )),
    }
}

pub(crate) fn declarations(values: &[Value]) -> Result<String, DataError> {
    let mut text = String::new();

    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        text.push_str(&format!("@P{} ", index + 1));
        text.push_str(match classify(value)? {
            ParamKind::Null => "nvarchar(4000)",
            ParamKind::Bit(_) => "bit",
            ParamKind::Int(_) => "bigint",
            ParamKind::Float(_) => "float",
            ParamKind::Text(ref value) if value.encode_utf16().count() > 4000 => "nvarchar(max)",
            ParamKind::Text(_) => "nvarchar(4000)",
            ParamKind::Bytes(ref value) if value.len() > 8000 => "varbinary(max)",
            ParamKind::Bytes(_) => "varbinary(8000)",
        });
    }

    Ok(text)
}

pub(crate) fn write_param(output: &mut Vec<u8>, kind: &ParamKind) {
    match kind {
        ParamKind::Null => {
            push_u8(output, T_NVARCHAR);
            push_u16le(output, 8000);
            output.extend_from_slice(&COLLATION_LATIN1);
            push_u16le(output, 0xFFFF);
        }
        ParamKind::Bit(value) => {
            push_u8(output, T_BITN);
            push_u8(output, 1);
            push_u8(output, 1);
            push_u8(output, u8::from(*value));
        }
        ParamKind::Int(value) => {
            push_u8(output, T_INTN);
            push_u8(output, 8);
            push_u8(output, 8);
            output.extend_from_slice(&value.to_le_bytes());
        }
        ParamKind::Float(value) => {
            push_u8(output, T_FLTN);
            push_u8(output, 8);
            push_u8(output, 8);
            output.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        ParamKind::Text(value) => {
            let mut bytes = Vec::with_capacity(value.len() * 2);
            push_utf16(&mut bytes, value);

            push_u8(output, T_NVARCHAR);
            if bytes.len() <= 8000 {
                push_u16le(output, 8000);
                output.extend_from_slice(&COLLATION_LATIN1);
                push_u16le(output, bytes.len() as u16);
                output.extend_from_slice(&bytes);
            } else {
                push_u16le(output, 0xFFFF);
                output.extend_from_slice(&COLLATION_LATIN1);
                crate::packet::push_plp(output, Some(&bytes));
            }
        }
        ParamKind::Bytes(bytes) => {
            push_u8(output, T_VARBINARY);
            if bytes.len() <= 8000 {
                push_u16le(output, 8000);
                push_u16le(output, bytes.len() as u16);
                output.extend_from_slice(bytes);
            } else {
                push_u16le(output, 0xFFFF);
                crate::packet::push_plp(output, Some(bytes));
            }
        }
    }
}

fn bytes_to_value(bytes: &[u8]) -> Value {
    Value::Array(
        bytes
            .iter()
            .map(|byte| Value::from(i64::from(*byte)))
            .collect(),
    )
}

fn latin1_or_utf8(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|byte| char::from(*byte)).collect(),
    }
}

fn uint64(bytes: &[u8]) -> Result<u64, DataError> {
    if bytes.len() > 8 {
        return Err(DataError::protocol(format!(
            "a value needs at most 8 bytes, got {}",
            bytes.len()
        )));
    }
    let mut buffer = [0u8; 8];
    buffer[..bytes.len()].copy_from_slice(bytes);
    Ok(u64::from_le_bytes(buffer))
}

fn int64(bytes: &[u8]) -> Result<i64, DataError> {
    Ok(uint64(bytes)? as i64)
}

fn u64_from(bytes: &[u8]) -> Result<u64, DataError> {
    uint64(bytes)
}

fn decimal(bytes: &[u8], scale: u8) -> Result<Value, DataError> {
    if bytes.len() > 17 {
        return Err(DataError::protocol(format!(
            "a decimal value may not exceed 17 bytes, got {}",
            bytes.len()
        )));
    }
    let negative = bytes[0] == 0;
    let mut magnitude: u128 = 0;
    for (index, byte) in bytes[1..].iter().enumerate() {
        magnitude |= u128::from(*byte) << (8 * index);
    }

    let value = if negative {
        (magnitude as i128).wrapping_neg()
    } else {
        magnitude as i128
    };

    if scale == 0 {
        if let Ok(value) = i64::try_from(value) {
            return Ok(Value::from(value));
        }
        if !negative && let Ok(value) = u64::try_from(magnitude) {
            return Ok(Value::from(value));
        }
        return Ok(Value::from(value as f64));
    }

    let divisor = 10i128.pow(u32::from(scale));
    Ok(Value::from(value as f64 / divisor as f64))
}

fn guid(bytes: &[u8]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[3],
        bytes[2],
        bytes[1],
        bytes[0],
        bytes[5],
        bytes[4],
        bytes[7],
        bytes[6],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

const UNIX_EPOCH_DAYS: i64 = 719_162;

const SQL_EPOCH_DAYS: i64 = 25_567;

fn format_days(days_since_epoch: i64) -> String {
    let (year, month, day) = civil_from_days(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    (year, month, day)
}

fn datetime(days: i32, ticks: u32) -> String {
    let seconds = u64::from(ticks) / 300;
    let remainder = u64::from(ticks) % 300;
    let millis = (remainder * 10 + 1) / 3;

    format!(
        "{}T{:02}:{:02}:{:02}.{:03}",
        format_days(i64::from(days) - SQL_EPOCH_DAYS),
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60,
        millis
    )
}

fn smalldatetime(days: u16, minutes: u16) -> String {
    format!(
        "{}T{:02}:{:02}:00",
        format_days(i64::from(days) - SQL_EPOCH_DAYS),
        minutes / 60,
        minutes % 60
    )
}

fn format_time(units: u64, scale: u8) -> String {
    let per_second = 10u64.pow(u32::from(scale));
    let total_seconds = units / per_second;
    let fraction = units % per_second;

    let mut text = format!(
        "{:02}:{:02}:{:02}",
        total_seconds / 3600,
        (total_seconds / 60) % 60,
        total_seconds % 60
    );

    if scale > 0 {
        text.push_str(&format!(".{fraction:0width$}", width = usize::from(scale)));
    }

    text
}

fn format_offset(minutes: i16) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let minutes = minutes.unsigned_abs();
    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_days_to_civil_dates() {
        assert_eq!(format_days(0), "1970-01-01");
        assert_eq!(format_days(-1), "1969-12-31");
        assert_eq!(format_days(19_723), "2024-01-01");
    }

    #[test]
    fn formats_datetime_values() {
        assert_eq!(
            datetime(45_290, (12 * 3600 + 34 * 60 + 56) * 300),
            "2024-01-01T12:34:56.000"
        );
        assert_eq!(smalldatetime(45_290, 12 * 60 + 34), "2024-01-01T12:34:00");
    }

    #[test]
    fn formats_time_at_every_scale() {
        assert_eq!(format_time(45, 0), "00:00:45");
        assert_eq!(format_time(12_345, 3), "00:00:12.345");
        assert_eq!(format_time(1_234_567_890, 7), "00:02:03.4567890");
        assert_eq!(format_offset(-330), "-05:30");
        assert_eq!(format_offset(480), "+08:00");
    }

    #[test]
    fn decodes_decimals_with_and_without_scale() {
        assert_eq!(decimal(&[1, 0x39, 0x30], 0).unwrap(), Value::from(12_345i64));
        assert_eq!(decimal(&[0, 0x39, 0x30], 0).unwrap(), Value::from(-12_345i64));
        assert_eq!(decimal(&[1, 0xD2, 0x04], 2).unwrap(), Value::from(12.34f64));
    }

    #[test]
    fn formats_guids_in_sql_server_byte_order() {
        let bytes = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ];
        assert_eq!(guid(&bytes), "04030201-0605-0807-090a-0b0c0d0e0f10");
    }

    #[test]
    fn writes_declarations_matching_the_bound_types() {
        let values = vec![
            Value::Null,
            Value::from(true),
            Value::from(7i64),
            Value::from(1.5f64),
            Value::from("ada"),
            Value::Array(vec![Value::from(1i64), Value::from(2i64)]),
        ];
        assert_eq!(
            declarations(&values).unwrap(),
            "@P1 nvarchar(4000), @P2 bit, @P3 bigint, @P4 float, @P5 nvarchar(4000), @P6 varbinary(8000)"
        );
    }

    #[test]
    fn rejects_objects_and_non_byte_arrays() {
        assert!(classify(&Value::Object(Default::default())).is_err());
        let array = Value::Array(vec![Value::from(300i64)]);
        assert!(classify(&array).is_err());
    }
}
