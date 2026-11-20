use std::io::{self, Read, Write};

use zhs_entity::DataError;

pub(crate) const HEADER_LEN: usize = 8;

pub(crate) const PACKET_SIZE: usize = 4096;

const STATUS_EOM: u8 = 0x01;

const MAX_RESPONSE: usize = 64 * 1024 * 1024;

pub(crate) const TYPE_SQL_BATCH: u8 = 0x01;

pub(crate) const TYPE_RPC: u8 = 0x03;

pub(crate) const TYPE_REPLY: u8 = 0x04;

pub(crate) const TYPE_LOGIN7: u8 = 0x10;

pub(crate) const TYPE_PRELOGIN: u8 = 0x12;

pub(crate) fn push_all_headers(output: &mut Vec<u8>, transaction_descriptor: [u8; 8]) {
    push_u32le(output, 22);
    push_u32le(output, 18);
    push_u16le(output, 0x0002);
    output.extend_from_slice(&transaction_descriptor);
    push_u32le(output, 1);
}

pub(crate) fn write_message(
    writer: &mut impl Write,
    kind: u8,
    payload: &[u8],
    packet_size: usize,
) -> io::Result<()> {
    let chunk = packet_size.saturating_sub(HEADER_LEN).max(1);
    let mut offset = 0;

    loop {
        let end = (offset + chunk).min(payload.len());
        let last = end == payload.len();

        let mut header = [0u8; HEADER_LEN];
        header[0] = kind;
        header[1] = if last { STATUS_EOM } else { 0x00 };
        header[2..4].copy_from_slice(&((HEADER_LEN + end - offset) as u16).to_be_bytes());
        header[6] = 1;

        writer.write_all(&header)?;
        writer.write_all(&payload[offset..end])?;
        offset = end;

        if last {
            return Ok(());
        }
    }
}

pub(crate) fn read_message(reader: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut payload = Vec::new();
    let mut kind = 0;
    let mut first = true;

    loop {
        let mut header = [0u8; HEADER_LEN];
        reader.read_exact(&mut header)?;

        let length = u16::from_be_bytes([header[2], header[3]]) as usize;
        if length < HEADER_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("TDS packet length {length} is smaller than its header"),
            ));
        }

        if first {
            kind = header[0];
            first = false;
        }

        let body_len = length - HEADER_LEN;
        if payload.len() + body_len > MAX_RESPONSE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("TDS message exceeds the {MAX_RESPONSE} byte limit"),
            ));
        }

        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body)?;
        payload.extend_from_slice(&body);

        if header[1] & STATUS_EOM != 0 {
            return Ok((kind, payload));
        }
    }
}

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    pub(crate) fn u8(&mut self) -> Result<u8, DataError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16le(&mut self) -> Result<u16, DataError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn u32le(&mut self) -> Result<u32, DataError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn u64le(&mut self) -> Result<u64, DataError> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub(crate) fn take(&mut self, length: usize) -> Result<&'a [u8], DataError> {
        if self.remaining() < length {
            return Err(DataError::protocol(format!(
                "TDS token needs {length} more bytes, {} left",
                self.remaining()
            )));
        }
        let slice = &self.data[self.position..self.position + length];
        self.position += length;
        Ok(slice)
    }

    pub(crate) fn b_varchar(&mut self) -> Result<String, DataError> {
        let chars = self.u8()? as usize;
        self.utf16(chars)
    }

    pub(crate) fn utf16(&mut self, chars: usize) -> Result<String, DataError> {
        let bytes = self.take(chars * 2)?;
        decode_utf16(bytes)
    }
}

pub(crate) fn decode_utf16(bytes: &[u8]) -> Result<String, DataError> {
    if bytes.len() & 1 != 0 {
        return Err(DataError::protocol(
            "UTF-16 text has an odd number of bytes",
        ));
    }

    let mut units = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        units.push(u16::from_le_bytes([pair[0], pair[1]]));
    }

    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| DataError::protocol("UTF-16 text contains an unpaired surrogate"))
}

pub(crate) fn push_u8(output: &mut Vec<u8>, value: u8) {
    output.push(value);
}

pub(crate) fn push_u16le(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn push_u32le(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn push_u64le(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn push_utf16(output: &mut Vec<u8>, text: &str) {
    for unit in text.encode_utf16() {
        push_u16le(output, unit);
    }
}

pub(crate) fn push_b_varchar(output: &mut Vec<u8>, text: &str) -> Result<(), DataError> {
    let chars = text.encode_utf16().count();
    if chars > u8::MAX as usize {
        return Err(DataError::query("a TDS name may not exceed 255 characters"));
    }
    push_u8(output, chars as u8);
    push_utf16(output, text);
    Ok(())
}

pub(crate) fn push_plp(output: &mut Vec<u8>, bytes: Option<&[u8]>) {
    match bytes {
        None => push_u64le(output, u64::MAX),
        Some(bytes) => {
            push_u64le(output, bytes.len() as u64);
            if !bytes.is_empty() {
                push_u32le(output, bytes.len() as u32);
                output.extend_from_slice(bytes);
            }
            push_u32le(output, 0);
        }
    }
}

pub(crate) fn read_b_varbyte(reader: &mut Reader<'_>) -> Result<Vec<u8>, DataError> {
    let length = reader.u8()? as usize;
    Ok(reader.take(length)?.to_vec())
}

pub(crate) fn read_plp(reader: &mut Reader<'_>) -> Result<Option<Vec<u8>>, DataError> {
    let total = reader.u64le()?;
    if total == u64::MAX {
        return Ok(None);
    }

    let mut bytes = Vec::new();
    loop {
        let length = reader.u32le()? as usize;
        if length == 0 {
            return Ok(Some(bytes));
        }
        bytes.extend_from_slice(reader.take(length)?);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_large_payload_into_packets() {
        let payload = vec![0x41u8; 100];
        let mut encoded = Vec::new();
        write_message(&mut encoded, TYPE_SQL_BATCH, &payload, 40).unwrap();

        assert_eq!(encoded.len(), 100 + 4 * HEADER_LEN);
        assert_eq!(encoded[0], TYPE_SQL_BATCH);
        assert_eq!(encoded[1], 0x00);
        assert_eq!(u16::from_be_bytes([encoded[2], encoded[3]]), 40);

        let last_header = &encoded[encoded.len() - 4 - HEADER_LEN..encoded.len() - 4];
        assert_eq!(last_header[1] & STATUS_EOM, STATUS_EOM);
    }

    #[test]
    fn writes_and_reads_an_empty_message() {
        let mut encoded = Vec::new();
        write_message(&mut encoded, TYPE_PRELOGIN, &[], PACKET_SIZE).unwrap();
        assert_eq!(encoded.len(), HEADER_LEN);
        assert_eq!(u16::from_be_bytes([encoded[2], encoded[3]]), 8);

        let (kind, payload) = read_message(&mut encoded.as_slice()).unwrap();
        assert_eq!(kind, TYPE_PRELOGIN);
        assert!(payload.is_empty());
    }

    #[test]
    fn reassembles_a_multi_packet_message() {
        let payload: Vec<u8> = (0..=255u8).cycle().take(200).collect();
        let mut encoded = Vec::new();
        write_message(&mut encoded, TYPE_REPLY, &payload, 64).unwrap();

        let (kind, decoded) = read_message(&mut encoded.as_slice()).unwrap();
        assert_eq!(kind, TYPE_REPLY);
        assert_eq!(decoded, payload);
    }

    #[test]
    fn reader_rejects_truncated_tokens() {
        let mut reader = Reader::new(&[0x01, 0x02]);
        assert!(reader.u32le().is_err());
        assert_eq!(reader.remaining(), 2);
    }

    #[test]
    fn plp_round_trips() {
        let mut encoded = Vec::new();
        push_plp(&mut encoded, Some(b"hello"));
        let mut reader = Reader::new(&encoded);
        assert_eq!(
            read_plp(&mut reader).unwrap().as_deref(),
            Some(&b"hello"[..])
        );

        let mut encoded = Vec::new();
        push_plp(&mut encoded, None);
        let mut reader = Reader::new(&encoded);
        assert_eq!(read_plp(&mut reader).unwrap(), None);
    }
}
