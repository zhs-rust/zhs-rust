mod platform;

use std::error::Error as StdError;
use std::fmt;
use std::io;

use crate::base64;

#[derive(Debug)]
pub struct RandomError(io::Error);

impl RandomError {
    pub fn kind(&self) -> io::ErrorKind {
        self.0.kind()
    }
}

impl fmt::Display for RandomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "failed to read secure random bytes: {}", self.0)
    }
}

impl StdError for RandomError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.0)
    }
}

impl From<io::Error> for RandomError {
    fn from(error: io::Error) -> Self {
        Self(error)
    }
}

pub fn fill(buffer: &mut [u8]) -> Result<(), RandomError> {
    platform::fill(buffer).map_err(RandomError)
}

pub fn bytes(length: usize) -> Result<Vec<u8>, RandomError> {
    let mut buffer = vec![0u8; length];
    fill(&mut buffer)?;
    Ok(buffer)
}

pub fn token(length: usize) -> Result<String, RandomError> {
    Ok(base64::encode_url(&bytes(length)?))
}

#[cfg(test)]
mod tests {
    use super::{bytes, fill, token};

    #[test]
    fn fills_a_buffer() {
        let mut first = [0u8; 32];
        let mut second = [0u8; 32];
        fill(&mut first).unwrap();
        fill(&mut second).unwrap();

        assert_ne!(first, second, "two draws must not repeat");
        assert_ne!(first, [0u8; 32]);
    }

    #[test]
    fn reads_the_requested_length() {
        assert!(bytes(0).unwrap().is_empty());
        assert_eq!(bytes(1).unwrap().len(), 1);
        assert_eq!(bytes(64).unwrap().len(), 64);
    }

    #[test]
    fn tokens_decode_to_the_requested_length() {
        let token = token(32).unwrap();
        let decoded = crate::base64::decode_url(&token).unwrap();
        assert_eq!(decoded.len(), 32);
        assert!(!token.contains('='));
    }
}
