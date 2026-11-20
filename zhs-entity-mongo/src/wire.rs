use zhs_entity::DataError;

use crate::bson::{self, Document};

const OP_MSG: i32 = 2013;

const MAX_MESSAGE: usize = 48 * 1024 * 1024;

pub(crate) fn encode(request_id: i32, document: &Document) -> Vec<u8> {
    let body = bson::encode(document);
    let total = 16 + 4 + 1 + body.len();

    let mut buffer = Vec::with_capacity(total);
    buffer.extend_from_slice(&(total as i32).to_le_bytes());
    buffer.extend_from_slice(&request_id.to_le_bytes());
    buffer.extend_from_slice(&0_i32.to_le_bytes());
    buffer.extend_from_slice(&OP_MSG.to_le_bytes());
    buffer.extend_from_slice(&0_u32.to_le_bytes());
    buffer.push(0);
    buffer.extend_from_slice(&body);
    buffer
}

pub(crate) struct Reply {
    pub(crate) response_to: i32,

    pub(crate) document: Document,
}

pub(crate) fn decode(buffer: &[u8]) -> Result<Reply, DataError> {
    if buffer.len() < 21 {
        return Err(DataError::protocol("message is too short to be OP_MSG"));
    }

    let length = read_i32(buffer, 0)?;
    let response_to = read_i32(buffer, 8)?;
    let op_code = read_i32(buffer, 12)?;

    if length < 0 || length as usize != buffer.len() {
        return Err(DataError::protocol(format!(
            "message declares {length} bytes but {} arrived",
            buffer.len()
        )));
    }
    if op_code != OP_MSG {
        return Err(DataError::protocol(format!(
            "unsupported opcode {op_code}; only OP_MSG is implemented"
        )));
    }

    let flags = u32::from_le_bytes([buffer[16], buffer[17], buffer[18], buffer[19]]);
    let mut position = 20;
    let end = if flags & 0x1 != 0 {
        buffer.len() - 4
    } else {
        buffer.len()
    };

    let mut body = None;
    while position < end {
        let kind = buffer[position];
        position += 1;

        match kind {
            0 => {
                let size = bson_length(buffer, position)?;
                body = Some(bson::decode(&buffer[position..position + size])?);
                position += size;
            }
            1 => {
                let size = read_i32(buffer, position)?;
                if size < 4 || position + size as usize > end {
                    return Err(DataError::protocol(
                        "document sequence section is malformed",
                    ));
                }
                let section_end = position + size as usize;
                position += 4;

                while position < section_end && buffer[position] != 0 {
                    position += 1;
                }
                position += 1;
                while position < section_end {
                    position += bson_length(buffer, position)?;
                }
            }
            other => {
                return Err(DataError::protocol(format!(
                    "unknown OP_MSG section kind {other}"
                )));
            }
        }
    }

    let document = body.ok_or_else(|| DataError::protocol("OP_MSG has no body section"))?;

    Ok(Reply {
        response_to,
        document,
    })
}

fn read_i32(buffer: &[u8], position: usize) -> Result<i32, DataError> {
    if position + 4 > buffer.len() {
        return Err(DataError::protocol("message ended inside a header field"));
    }
    Ok(i32::from_le_bytes([
        buffer[position],
        buffer[position + 1],
        buffer[position + 2],
        buffer[position + 3],
    ]))
}

fn bson_length(buffer: &[u8], position: usize) -> Result<usize, DataError> {
    let length = read_i32(buffer, position)?;
    if length < 5 || position + length as usize > buffer.len() {
        return Err(DataError::protocol("document length is out of range"));
    }
    Ok(length as usize)
}

pub(crate) const fn max_message_length() -> usize {
    MAX_MESSAGE
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};
    use crate::bson::{Bson, Document};

    fn sample() -> Document {
        let mut document = Document::new();
        document.insert("ok", Bson::Double(1.0));
        document.insert("value", Bson::Int32(7));
        document
    }

    #[test]
    fn round_trips_a_body_section() {
        let encoded = encode(42, &sample());
        assert_eq!(
            i32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize,
            encoded.len()
        );
        assert_eq!(
            i32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]),
            42
        );
        assert_eq!(
            i32::from_le_bytes([encoded[12], encoded[13], encoded[14], encoded[15]]),
            2013
        );

        let reply = decode(&encoded).expect("decode");
        assert_eq!(reply.document, sample());
    }

    #[test]
    fn rejects_foreign_opcodes_and_bad_lengths() {
        let mut encoded = encode(1, &sample());
        encoded[12..16].copy_from_slice(&1_i32.to_le_bytes());
        assert!(decode(&encoded).is_err());

        let mut encoded = encode(1, &sample());
        let wrong = encoded.len() as i32 + 2;
        encoded[0..4].copy_from_slice(&wrong.to_le_bytes());
        assert!(decode(&encoded).is_err());

        assert!(decode(&[0_u8; 8]).is_err());
    }
}
