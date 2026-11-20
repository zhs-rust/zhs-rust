use std::error::Error as StdError;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HexError {
    OddLength,

    InvalidCharacter { index: usize },
}

impl fmt::Display for HexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HexError::OddLength => formatter.write_str("hex input has an odd length"),
            HexError::InvalidCharacter { index } => {
                write!(formatter, "invalid hex character at byte {index}")
            }
        }
    }
}

impl StdError for HexError {}

pub fn encode(input: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    let mut output = String::with_capacity(input.len() * 2);
    for byte in input {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn decode(input: &str) -> Result<Vec<u8>, HexError> {
    if !input.len().is_multiple_of(2) {
        return Err(HexError::OddLength);
    }

    let mut output = Vec::with_capacity(input.len() / 2);
    let bytes = input.as_bytes();

    for index in (0..bytes.len()).step_by(2) {
        let high = decode_nibble(bytes[index]).ok_or(HexError::InvalidCharacter { index })?;
        let low = decode_nibble(bytes[index + 1])
            .ok_or(HexError::InvalidCharacter { index: index + 1 })?;
        output.push((high << 4) | low);
    }

    Ok(output)
}

fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{HexError, decode, encode};

    #[test]
    fn round_trips_every_byte_value() {
        let bytes: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn accepts_both_cases_but_encodes_lowercase() {
        assert_eq!(encode(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(decode("00ABff").unwrap(), vec![0x00, 0xab, 0xff]);
    }

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(decode("abc"), Err(HexError::OddLength));
        assert_eq!(decode("zz"), Err(HexError::InvalidCharacter { index: 0 }));
        assert_eq!(decode("0g"), Err(HexError::InvalidCharacter { index: 1 }));
    }
}
