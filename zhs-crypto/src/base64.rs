use std::error::Error as StdError;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Base64Error {
    InvalidLength,

    InvalidCharacter { index: usize },

    NonCanonical,
}

impl fmt::Display for Base64Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Base64Error::InvalidLength => {
                formatter.write_str("base64url input has an impossible length")
            }
            Base64Error::InvalidCharacter { index } => {
                write!(formatter, "invalid base64url character at byte {index}")
            }
            Base64Error::NonCanonical => formatter.write_str("non-canonical base64url encoding"),
        }
    }
}

impl StdError for Base64Error {}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

const ALPHABET_STANDARD: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(input: &[u8]) -> String {
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    let (chunks, remainder) = input.as_chunks::<3>();

    for chunk in chunks {
        let value = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        output.push(ALPHABET_STANDARD[(value >> 18) as usize & 0x3f] as char);
        output.push(ALPHABET_STANDARD[(value >> 12) as usize & 0x3f] as char);
        output.push(ALPHABET_STANDARD[(value >> 6) as usize & 0x3f] as char);
        output.push(ALPHABET_STANDARD[value as usize & 0x3f] as char);
    }

    match remainder {
        [first] => {
            let value = u32::from(*first) << 16;
            output.push(ALPHABET_STANDARD[(value >> 18) as usize & 0x3f] as char);
            output.push(ALPHABET_STANDARD[(value >> 12) as usize & 0x3f] as char);
            output.push_str("==");
        }
        [first, second] => {
            let value = (u32::from(*first) << 16) | (u32::from(*second) << 8);
            output.push(ALPHABET_STANDARD[(value >> 18) as usize & 0x3f] as char);
            output.push(ALPHABET_STANDARD[(value >> 12) as usize & 0x3f] as char);
            output.push(ALPHABET_STANDARD[(value >> 6) as usize & 0x3f] as char);
            output.push('=');
        }
        [] => {}
        _ => unreachable!("as_chunks(3) leaves at most two bytes"),
    }

    output
}

pub fn decode(input: &str) -> Result<Vec<u8>, Base64Error> {
    if !input.len().is_multiple_of(4) {
        return Err(Base64Error::InvalidLength);
    }

    let body = input.trim_end_matches('=');
    if input.len() - body.len() > 2 {
        return Err(Base64Error::InvalidCharacter {
            index: input.len() - 1,
        });
    }
    if body.len() % 4 == 1 {
        return Err(Base64Error::InvalidLength);
    }

    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0u32;

    for (index, byte) in body.bytes().enumerate() {
        let value = decode_standard_char(byte).ok_or(Base64Error::InvalidCharacter { index })?;

        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1u32 << bits) - 1;
        }
    }

    if accumulator != 0 {
        return Err(Base64Error::NonCanonical);
    }

    Ok(output)
}

pub fn encode_url(input: &[u8]) -> String {
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    let (chunks, remainder) = input.as_chunks::<3>();

    for chunk in chunks {
        let value = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        output.push(ALPHABET[(value >> 18) as usize & 0x3f] as char);
        output.push(ALPHABET[(value >> 12) as usize & 0x3f] as char);
        output.push(ALPHABET[(value >> 6) as usize & 0x3f] as char);
        output.push(ALPHABET[value as usize & 0x3f] as char);
    }

    match remainder {
        [first] => {
            let value = u32::from(*first) << 16;
            output.push(ALPHABET[(value >> 18) as usize & 0x3f] as char);
            output.push(ALPHABET[(value >> 12) as usize & 0x3f] as char);
        }
        [first, second] => {
            let value = (u32::from(*first) << 16) | (u32::from(*second) << 8);
            output.push(ALPHABET[(value >> 18) as usize & 0x3f] as char);
            output.push(ALPHABET[(value >> 12) as usize & 0x3f] as char);
            output.push(ALPHABET[(value >> 6) as usize & 0x3f] as char);
        }
        [] => {}
        _ => unreachable!("as_chunks(3) leaves at most two bytes"),
    }

    output
}

pub fn decode_url(input: &str) -> Result<Vec<u8>, Base64Error> {
    if input.len() % 4 == 1 {
        return Err(Base64Error::InvalidLength);
    }

    let mut output = Vec::with_capacity(input.len() / 4 * 3 + 2);
    let mut accumulator = 0u32;
    let mut bits = 0u32;

    for (index, byte) in input.bytes().enumerate() {
        let value = decode_char(byte).ok_or(Base64Error::InvalidCharacter { index })?;

        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1u32 << bits) - 1;
        }
    }

    if accumulator != 0 {
        return Err(Base64Error::NonCanonical);
    }

    Ok(output)
}

fn decode_char(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

fn decode_standard_char(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Base64Error, decode, decode_url, encode, encode_url};

    #[test]
    fn matches_the_standard_alphabet_vectors() {
        let cases: [(&[u8], &str); 7] = [
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];

        for (bytes, encoded) in cases {
            assert_eq!(encode(bytes), encoded);
            assert_eq!(decode(encoded).unwrap(), bytes);
        }

        assert_eq!(encode(&[0xfb, 0xff]), "+/8=");
        assert_eq!(decode("+/8=").unwrap(), vec![0xfb, 0xff]);
    }

    #[test]
    fn standard_decoder_rejects_bad_padding_and_foreign_characters() {
        assert_eq!(decode("Zg="), Err(Base64Error::InvalidLength));
        assert_eq!(decode("Zg"), Err(Base64Error::InvalidLength));
        assert_eq!(decode("Zg==="), Err(Base64Error::InvalidLength));
        assert_eq!(
            decode("Zm9v===="),
            Err(Base64Error::InvalidCharacter { index: 7 })
        );
        assert_eq!(
            decode("Z=g="),
            Err(Base64Error::InvalidCharacter { index: 1 })
        );
        assert_eq!(
            decode("Zm9-"),
            Err(Base64Error::InvalidCharacter { index: 3 })
        );
        assert_eq!(decode("Zh=="), Err(Base64Error::NonCanonical));
    }

    #[test]
    fn standard_round_trips_every_byte_value() {
        let bytes: Vec<u8> = (0..=255u8).collect();

        for length in 0..=bytes.len() {
            let input = &bytes[..length];
            assert_eq!(decode(&encode(input)).unwrap(), input, "length {length}");
        }
    }

    #[test]
    fn matches_rfc_4648_vectors() {
        let cases: [(&[u8], &str); 7] = [
            (b"", ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (b"fooba", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy"),
        ];

        for (bytes, encoded) in cases {
            assert_eq!(encode_url(bytes), encoded);
            assert_eq!(decode_url(encoded).unwrap(), bytes);
        }
    }

    #[test]
    fn uses_the_url_safe_alphabet() {
        assert_eq!(encode_url(&[0xfb, 0xff]), "-_8");
        assert_eq!(decode_url("-_8").unwrap(), vec![0xfb, 0xff]);
    }

    #[test]
    fn round_trips_every_byte_value() {
        let bytes: Vec<u8> = (0..=255u8).collect();

        for length in 0..=bytes.len() {
            let input = &bytes[..length];
            let encoded = encode_url(input);
            assert_eq!(decode_url(&encoded).unwrap(), input);
            assert!(!encoded.contains('='));
            assert!(!encoded.contains('+'));
            assert!(!encoded.contains('/'));
        }
    }

    #[test]
    fn rejects_padding_and_foreign_characters() {
        assert_eq!(
            decode_url("Zg=="),
            Err(Base64Error::InvalidCharacter { index: 2 })
        );
        assert_eq!(
            decode_url("Zg="),
            Err(Base64Error::InvalidCharacter { index: 2 })
        );
        assert_eq!(
            decode_url("Zm9+"),
            Err(Base64Error::InvalidCharacter { index: 3 })
        );
        assert_eq!(
            decode_url("Zm9/"),
            Err(Base64Error::InvalidCharacter { index: 3 })
        );
        assert_eq!(
            decode_url("Zm\nv"),
            Err(Base64Error::InvalidCharacter { index: 2 })
        );

        assert_eq!(decode_url("Z"), Err(Base64Error::InvalidLength));
        assert_eq!(decode_url("Zm9vY"), Err(Base64Error::InvalidLength));
    }

    #[test]
    fn rejects_non_canonical_trailing_bits() {
        assert_eq!(decode_url("Zh"), Err(Base64Error::NonCanonical));

        assert_eq!(decode_url("Zm9"), Err(Base64Error::NonCanonical));

        assert_eq!(decode_url("Zg").unwrap(), b"f");
    }
}
