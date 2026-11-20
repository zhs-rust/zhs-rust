use std::io::{Read, Write};

use zhs_entity::DataError;

const MAX_PAYLOAD: usize = 0xFF_FFFF;

const MAX_RESPONSE: usize = 64 * 1024 * 1024;

pub(crate) fn read_packet(stream: &mut impl Read, sequence: &mut u8) -> Result<Vec<u8>, DataError> {
    let mut payload = Vec::new();

    loop {
        let mut header = [0u8; 4];
        stream
            .read_exact(&mut header)
            .map_err(|error| io_error("reading a packet header", error))?;

        let length = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
        if header[3] != *sequence {
            return Err(DataError::protocol(format!(
                "packet sequence {} does not match the expected {}",
                header[3], *sequence
            )));
        }
        *sequence = header[3].wrapping_add(1);

        if payload.len() + length > MAX_RESPONSE {
            return Err(DataError::protocol(
                "response is larger than the accepted maximum",
            ));
        }

        let start = payload.len();
        payload.resize(start + length, 0);
        stream
            .read_exact(&mut payload[start..])
            .map_err(|error| io_error("reading a packet body", error))?;

        if length < MAX_PAYLOAD {
            return Ok(payload);
        }
    }
}

pub(crate) fn write_packet(
    stream: &mut impl Write,
    sequence: &mut u8,
    payload: &[u8],
) -> Result<(), DataError> {
    let mut offset = 0;

    loop {
        let chunk = (payload.len() - offset).min(MAX_PAYLOAD);
        let length = (chunk as u32).to_le_bytes();
        let header = [length[0], length[1], length[2], *sequence];
        *sequence = sequence.wrapping_add(1);

        stream
            .write_all(&header)
            .map_err(|error| io_error("writing a packet header", error))?;
        stream
            .write_all(&payload[offset..offset + chunk])
            .map_err(|error| io_error("writing a packet body", error))?;

        offset += chunk;
        if chunk < MAX_PAYLOAD {
            stream
                .flush()
                .map_err(|error| io_error("flushing a packet", error))?;
            return Ok(());
        }
    }
}

fn io_error(context: &str, error: std::io::Error) -> DataError {
    DataError::protocol(format!("{context}: {error}"))
}

pub(crate) struct PacketReader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> PacketReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub(crate) fn position(&self) -> usize {
        self.position
    }

    pub(crate) fn take(&mut self, length: usize) -> Result<&'a [u8], DataError> {
        if self.remaining() < length {
            return Err(DataError::protocol(format!(
                "packet ended after {} of {} bytes",
                self.remaining(),
                length
            )));
        }
        let start = self.position;
        self.position += length;
        Ok(&self.data[start..self.position])
    }

    pub(crate) fn byte(&mut self) -> Result<u8, DataError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16_le(&mut self) -> Result<u16, DataError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn u32_le(&mut self) -> Result<u32, DataError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn lenenc_int(&mut self) -> Result<Option<u64>, DataError> {
        let first = self.byte()?;
        match first {
            0xFB => Ok(None),
            0xFC => Ok(Some(
                self.take(2)?
                    .iter()
                    .rev()
                    .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)),
            )),
            0xFD => Ok(Some(
                self.take(3)?
                    .iter()
                    .rev()
                    .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)),
            )),
            0xFE => Ok(Some(
                self.take(8)?
                    .iter()
                    .rev()
                    .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)),
            )),
            value => Ok(Some(u64::from(value))),
        }
    }

    pub(crate) fn lenenc_bytes(&mut self) -> Result<Option<&'a [u8]>, DataError> {
        match self.byte()? {
            0xFB => Ok(None),
            0xFC => {
                let length = self.u16_le()? as usize;
                Ok(Some(self.take(length)?))
            }
            0xFD => {
                let bytes = self.take(3)?;
                let length = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]) as usize;
                Ok(Some(self.take(length)?))
            }
            0xFE => {
                let bytes = self.take(8)?;
                let length = u64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]) as usize;
                Ok(Some(self.take(length)?))
            }
            value => Ok(Some(self.take(usize::from(value))?)),
        }
    }

    pub(crate) fn null_terminated(&mut self) -> Result<&'a [u8], DataError> {
        let start = self.position;
        while self.position < self.data.len() {
            if self.data[self.position] == 0 {
                let text = &self.data[start..self.position];
                self.position += 1;
                return Ok(text);
            }
            self.position += 1;
        }
        Err(DataError::protocol(
            "packet ended inside a NUL-terminated string",
        ))
    }

    pub(crate) fn rest(&mut self) -> &'a [u8] {
        let text = &self.data[self.position..];
        self.position = self.data.len();
        text
    }

    pub(crate) fn rest_without_nul(&mut self) -> &'a [u8] {
        let text = self.rest();
        match text.last() {
            Some(0) => &text[..text.len() - 1],
            _ => text,
        }
    }
}

pub(crate) fn push_lenenc_int(buffer: &mut Vec<u8>, value: u64) {
    if value < 0xFB {
        buffer.push(value as u8);
    } else if value <= u64::from(u16::MAX) {
        buffer.push(0xFC);
        buffer.extend_from_slice(&(value as u16).to_le_bytes());
    } else if value <= 0xFF_FFFF {
        buffer.push(0xFD);
        let bytes = (value as u32).to_le_bytes();
        buffer.extend_from_slice(&bytes[..3]);
    } else {
        buffer.push(0xFE);
        buffer.extend_from_slice(&value.to_le_bytes());
    }
}

pub(crate) fn push_lenenc_bytes(buffer: &mut Vec<u8>, bytes: &[u8]) {
    push_lenenc_int(buffer, bytes.len() as u64);
    buffer.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::{PacketReader, push_lenenc_bytes, push_lenenc_int};
    use std::io::Cursor;

    #[test]
    fn lenenc_ints_round_trip() {
        for value in [
            0_u64,
            1,
            250,
            251,
            65_535,
            65_536,
            0xFF_FFFF,
            0x100_0000,
            u64::MAX,
        ] {
            let mut buffer = Vec::new();
            push_lenenc_int(&mut buffer, value);
            let mut reader = PacketReader::new(&buffer);
            assert_eq!(reader.lenenc_int().unwrap(), Some(value), "value {value}");
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn lenenc_bytes_round_trip() {
        for length in [0_usize, 1, 250, 251, 65_535, 65_536] {
            let data = vec![7_u8; length];
            let mut buffer = Vec::new();
            push_lenenc_bytes(&mut buffer, &data);
            let mut reader = PacketReader::new(&buffer);
            assert_eq!(reader.lenenc_bytes().unwrap(), Some(data.as_slice()));
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn null_is_reported_as_none() {
        let mut ints = PacketReader::new(&[0xFB]);
        assert_eq!(ints.lenenc_int().unwrap(), None);

        let mut bytes = PacketReader::new(&[0xFB]);
        assert_eq!(bytes.lenenc_bytes().unwrap(), None);
    }

    #[test]
    fn null_terminated_strings_stop_at_the_terminator() {
        let mut reader = PacketReader::new(b"users\0rest");
        assert_eq!(reader.null_terminated().unwrap(), b"users");
        assert_eq!(reader.rest(), b"rest");
    }

    #[test]
    fn packets_split_on_the_16mb_boundary() {
        use super::{read_packet, write_packet};

        let payload = vec![9_u8; 0xFF_FFFF];
        let mut outbound = Vec::new();
        let mut sequence = 0_u8;
        write_packet(&mut outbound, &mut sequence, &payload).expect("write");

        assert_eq!(outbound.len(), 0xFF_FFFF + 4 + 4);
        assert_eq!(sequence, 2);

        let mut cursor = Cursor::new(outbound);
        let mut read_sequence = 0_u8;
        let decoded = read_packet(&mut cursor, &mut read_sequence).expect("read");
        assert_eq!(decoded.len(), payload.len());
        assert_eq!(read_sequence, 2);
    }

    #[test]
    fn rejects_packets_with_an_unexpected_sequence() {
        use super::{read_packet, write_packet};

        let mut outbound = Vec::new();
        let mut sequence = 0_u8;
        write_packet(&mut outbound, &mut sequence, b"hi").expect("write");

        let mut cursor = Cursor::new(outbound);
        let mut read_sequence = 5_u8;
        assert!(read_packet(&mut cursor, &mut read_sequence).is_err());
    }
}
