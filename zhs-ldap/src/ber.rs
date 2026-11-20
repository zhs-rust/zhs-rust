use crate::LdapError;

pub(crate) const BOOLEAN: u8 = 0x01;
pub(crate) const INTEGER: u8 = 0x02;
pub(crate) const OCTET_STRING: u8 = 0x04;
pub(crate) const ENUMERATED: u8 = 0x0A;
pub(crate) const SEQUENCE: u8 = 0x30;

pub(crate) fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(content.len() + 6);
    output.push(tag);
    push_length(&mut output, content.len());
    output.extend_from_slice(content);
    output
}

pub(crate) fn sequence(content: &[u8]) -> Vec<u8> {
    tlv(SEQUENCE, content)
}

pub(crate) fn integer(value: i64) -> Vec<u8> {
    let mut bytes = value.to_be_bytes().to_vec();
    while bytes.len() > 1
        && ((bytes[0] == 0 && bytes[1] & 0x80 == 0) || (bytes[0] == 0xFF && bytes[1] & 0x80 != 0))
    {
        bytes.remove(0);
    }
    tlv(INTEGER, &bytes)
}

pub(crate) fn enumerated(value: u8) -> Vec<u8> {
    tlv(ENUMERATED, &[value])
}

pub(crate) fn octet_string(value: &[u8]) -> Vec<u8> {
    tlv(OCTET_STRING, value)
}

pub(crate) fn boolean(value: bool) -> Vec<u8> {
    tlv(BOOLEAN, &[if value { 0xFF } else { 0x00 }])
}

fn push_length(output: &mut Vec<u8>, length: usize) {
    if length < 128 {
        output.push(length as u8);
        return;
    }

    let bytes = length.to_be_bytes();
    let first = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1);
    let bytes = &bytes[first..];
    output.push(0x80 | bytes.len() as u8);
    output.extend_from_slice(bytes);
}

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.position >= self.data.len()
    }

    pub(crate) fn read(&mut self) -> Result<(u8, &'a [u8]), LdapError> {
        let tag = *self
            .data
            .get(self.position)
            .ok_or_else(|| LdapError::Protocol("truncated BER tag".to_owned()))?;
        self.position += 1;

        let first = *self
            .data
            .get(self.position)
            .ok_or_else(|| LdapError::Protocol("truncated BER length".to_owned()))?;
        self.position += 1;

        let length = if first & 0x80 == 0 {
            first as usize
        } else {
            let count = (first & 0x7F) as usize;
            if count == 0 || count > 4 {
                return Err(LdapError::Protocol("invalid BER length".to_owned()));
            }

            let mut length = 0usize;
            for _ in 0..count {
                let byte = *self
                    .data
                    .get(self.position)
                    .ok_or_else(|| LdapError::Protocol("truncated BER length".to_owned()))?;
                self.position += 1;
                length = (length << 8) | byte as usize;
            }
            length
        };

        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| LdapError::Protocol("truncated BER value".to_owned()))?;

        let content = &self.data[self.position..end];
        self.position = end;
        Ok((tag, content))
    }

    pub(crate) fn read_tag(&mut self, expected: u8) -> Result<&'a [u8], LdapError> {
        let (tag, content) = self.read()?;
        if tag != expected {
            return Err(LdapError::Protocol(format!(
                "expected BER tag {expected:#04x}, got {tag:#04x}"
            )));
        }
        Ok(content)
    }

    pub(crate) fn read_integer(&mut self) -> Result<i64, LdapError> {
        let content = self.read_tag(INTEGER)?;
        if content.is_empty() || content.len() > 8 {
            return Err(LdapError::Protocol("invalid BER integer".to_owned()));
        }

        let mut value = if content[0] & 0x80 != 0 { -1i64 } else { 0i64 };
        for byte in content {
            value = (value << 8) | *byte as i64;
        }
        Ok(value)
    }

    pub(crate) fn read_enumerated(&mut self) -> Result<u8, LdapError> {
        let content = self.read_tag(ENUMERATED)?;
        if content.len() != 1 {
            return Err(LdapError::Protocol(format!(
                "invalid BER enumerated length {}",
                content.len()
            )));
        }
        Ok(content[0])
    }

    pub(crate) fn read_octet_string(&mut self) -> Result<&'a [u8], LdapError> {
        self.read_tag(OCTET_STRING)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_integer_and_octet_string() {
        let encoded = integer(300);
        let mut reader = Reader::new(&encoded);
        assert_eq!(reader.read_integer().unwrap(), 300);

        let encoded = octet_string(b"alice");
        let mut reader = Reader::new(&encoded);
        assert_eq!(reader.read_octet_string().unwrap(), b"alice");
    }

    #[test]
    fn encodes_long_lengths() {
        let content = vec![b'x'; 200];
        let encoded = octet_string(&content);
        let mut reader = Reader::new(&encoded);
        assert_eq!(reader.read_octet_string().unwrap().len(), 200);
    }
}
