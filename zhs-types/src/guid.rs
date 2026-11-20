use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Guid([u8; 16]);

impl Guid {
    pub const NIL: Guid = Guid([0; 16]);

    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Guid(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn new_v4() -> Self {
        Self::try_new_v4().expect("secure random source is unavailable")
    }

    pub fn try_new_v4() -> std::io::Result<Self> {
        let mut bytes = [0_u8; 16];
        crate::random::fill(&mut bytes)?;

        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;

        Ok(Guid(bytes))
    }

    pub fn is_nil(&self) -> bool {
        self.0 == [0; 16]
    }

    pub const fn version(&self) -> u8 {
        self.0[6] >> 4
    }

    pub const fn variant(&self) -> u8 {
        (self.0[8] >> 6) & 0b11
    }
}

impl Default for Guid {
    fn default() -> Self {
        Guid::NIL
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;

        write!(
            formatter,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Guid(\"{self}\")")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidParseError {
    message: &'static str,
}

impl fmt::Display for GuidParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for GuidParseError {}

impl FromStr for Guid {
    type Err = GuidParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();

        let input = input
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
            .unwrap_or(input);

        let mut bytes = [0_u8; 16];
        let mut nibble_index = 0_usize;

        for byte in input.bytes() {
            if byte == b'-' {
                continue;
            }

            let value = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => {
                    return Err(GuidParseError {
                        message: "GUID contains a non-hexadecimal character",
                    });
                }
            };

            if nibble_index >= 32 {
                return Err(GuidParseError {
                    message: "GUID contains too many hexadecimal digits",
                });
            }

            let position = nibble_index / 2;
            if nibble_index.is_multiple_of(2) {
                bytes[position] = value << 4;
            } else {
                bytes[position] |= value;
            }

            nibble_index += 1;
        }

        if nibble_index != 32 {
            return Err(GuidParseError {
                message: "GUID must contain exactly 32 hexadecimal digits",
            });
        }

        Ok(Guid(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nil_is_all_zero() {
        assert!(Guid::NIL.is_nil());
        assert_eq!(
            Guid::NIL.to_string(),
            "00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn parses_common_forms() {
        let hyphenated = "550e8400-e29b-41d4-a716-446655440000";
        let simple = "550e8400e29b41d4a716446655440000";
        let braced = "{550E8400-E29B-41D4-A716-446655440000}";

        let expected = Guid::from_bytes([
            0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
            0x00, 0x00,
        ]);

        assert_eq!(hyphenated.parse::<Guid>().unwrap(), expected);
        assert_eq!(simple.parse::<Guid>().unwrap(), expected);
        assert_eq!(braced.parse::<Guid>().unwrap(), expected);
        assert_eq!(expected.to_string(), hyphenated);
    }

    #[test]
    fn rejects_invalid_input() {
        assert!("not-a-guid".parse::<Guid>().is_err());
        assert!(
            "550e8400-e29b-41d4-a716-44665544000"
                .parse::<Guid>()
                .is_err()
        );
        assert!(
            "550e8400-e29b-41d4-a716-4466554400000"
                .parse::<Guid>()
                .is_err()
        );
        assert!(
            "550e8400-e29b-41d4-a716-44665544000g"
                .parse::<Guid>()
                .is_err()
        );
    }

    #[test]
    fn v4_has_version_and_variant() {
        let guid = Guid::new_v4();
        assert_eq!(guid.version(), 4);
        assert_eq!(guid.variant(), 0b10);
        assert!(!guid.is_nil());
    }
}
