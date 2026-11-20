use std::error::Error as StdError;
use std::fmt;

use crate::random::{self, RandomError};
use crate::{base64, constant_time, pbkdf2};

pub const ALGORITHM: &str = "pbkdf2-sha256";

pub const SALT_BYTES: usize = 16;

pub const HASH_BYTES: usize = 32;

pub const MIN_ITERATIONS: u32 = 1_000;

#[derive(Debug)]
#[non_exhaustive]
pub enum PasswordError {
    InvalidIterations,

    InvalidFormat,

    Entropy(RandomError),

    Derive(pbkdf2::Pbkdf2Error),
}

impl fmt::Display for PasswordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PasswordError::InvalidIterations => {
                formatter.write_str("password hash iterations are out of range")
            }
            PasswordError::InvalidFormat => formatter.write_str("invalid password hash format"),
            PasswordError::Entropy(error) => error.fmt(formatter),
            PasswordError::Derive(error) => error.fmt(formatter),
        }
    }
}

impl StdError for PasswordError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            PasswordError::Entropy(error) => Some(error),
            PasswordError::Derive(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RandomError> for PasswordError {
    fn from(error: RandomError) -> Self {
        PasswordError::Entropy(error)
    }
}

impl From<pbkdf2::Pbkdf2Error> for PasswordError {
    fn from(error: pbkdf2::Pbkdf2Error) -> Self {
        PasswordError::Derive(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordHasher {
    iterations: u32,
}

impl PasswordHasher {
    pub fn new(iterations: u32) -> Result<Self, PasswordError> {
        if !(MIN_ITERATIONS..=pbkdf2::MAX_ITERATIONS).contains(&iterations) {
            return Err(PasswordError::InvalidIterations);
        }

        Ok(Self { iterations })
    }

    pub fn recommended() -> Self {
        Self {
            iterations: pbkdf2::DEFAULT_ITERATIONS,
        }
    }

    pub fn iterations(&self) -> u32 {
        self.iterations
    }

    pub fn hash(&self, password: &str) -> Result<String, PasswordError> {
        let salt = random::bytes(SALT_BYTES)?;
        let mut digest = [0u8; HASH_BYTES];
        pbkdf2::sha256(password.as_bytes(), &salt, self.iterations, &mut digest)?;

        Ok(format!(
            "{ALGORITHM}${}${}${}",
            self.iterations,
            base64::encode_url(&salt),
            base64::encode_url(&digest)
        ))
    }

    pub fn verify(&self, password: &str, encoded: &str) -> Result<bool, PasswordError> {
        let record = parse(encoded)?;

        let mut digest = vec![0u8; record.hash.len()];
        pbkdf2::sha256(
            password.as_bytes(),
            &record.salt,
            record.iterations,
            &mut digest,
        )?;

        Ok(constant_time::eq(&digest, &record.hash))
    }

    pub fn needs_rehash(&self, encoded: &str) -> bool {
        match parse(encoded) {
            Ok(record) => record.iterations != self.iterations,
            Err(_) => true,
        }
    }
}

impl Default for PasswordHasher {
    fn default() -> Self {
        Self::recommended()
    }
}

struct Record {
    iterations: u32,
    salt: Vec<u8>,
    hash: Vec<u8>,
}

fn parse(encoded: &str) -> Result<Record, PasswordError> {
    let mut parts = encoded.split('$');
    let (Some(algorithm), Some(iterations), Some(salt), Some(hash)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(PasswordError::InvalidFormat);
    };
    if parts.next().is_some() || algorithm != ALGORITHM {
        return Err(PasswordError::InvalidFormat);
    }

    let iterations: u32 = iterations
        .parse()
        .map_err(|_| PasswordError::InvalidFormat)?;
    if !(MIN_ITERATIONS..=pbkdf2::MAX_ITERATIONS).contains(&iterations) {
        return Err(PasswordError::InvalidFormat);
    }

    let salt = base64::decode_url(salt).map_err(|_| PasswordError::InvalidFormat)?;
    if salt.len() < 8 || salt.len() > 64 {
        return Err(PasswordError::InvalidFormat);
    }

    let hash = base64::decode_url(hash).map_err(|_| PasswordError::InvalidFormat)?;
    if hash.len() < 16 || hash.len() > 64 {
        return Err(PasswordError::InvalidFormat);
    }

    Ok(Record {
        iterations,
        salt,
        hash,
    })
}

#[cfg(test)]
mod tests {
    use super::{ALGORITHM, MIN_ITERATIONS, PasswordError, PasswordHasher};
    use crate::pbkdf2;

    const FAST: u32 = 1_000;

    fn hasher() -> PasswordHasher {
        PasswordHasher::new(FAST).unwrap()
    }

    #[test]
    fn hashes_and_verifies_passwords() {
        let hasher = hasher();
        let encoded = hasher.hash("correct horse").unwrap();

        assert!(hasher.verify("correct horse", &encoded).unwrap());
        assert!(!hasher.verify("Correct horse", &encoded).unwrap());
        assert!(!hasher.verify("", &encoded).unwrap());
    }

    #[test]
    fn every_hash_uses_a_fresh_salt() {
        let hasher = hasher();
        let first = hasher.hash("same").unwrap();
        let second = hasher.hash("same").unwrap();

        assert_ne!(first, second);
        assert!(hasher.verify("same", &first).unwrap());
        assert!(hasher.verify("same", &second).unwrap());
    }

    #[test]
    fn encodes_algorithm_and_iterations() {
        let encoded = hasher().hash("secret").unwrap();
        let parts: Vec<&str> = encoded.split('$').collect();

        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], ALGORITHM);
        assert_eq!(parts[1], FAST.to_string());
    }

    #[test]
    fn verify_uses_the_iterations_stored_in_the_record() {
        let encoded = hasher().hash("secret").unwrap();

        let stronger = PasswordHasher::new(2_000).unwrap();
        assert!(stronger.verify("secret", &encoded).unwrap());
        assert!(stronger.needs_rehash(&encoded));
        assert!(!hasher().needs_rehash(&encoded));
    }

    #[test]
    fn needs_rehash_for_unparsable_records() {
        assert!(hasher().needs_rehash("garbage"));
        assert!(hasher().needs_rehash(""));
    }

    #[test]
    fn rejects_invalid_new_iterations() {
        assert!(matches!(
            PasswordHasher::new(MIN_ITERATIONS - 1),
            Err(PasswordError::InvalidIterations)
        ));
        assert!(matches!(
            PasswordHasher::new(pbkdf2::MAX_ITERATIONS + 1),
            Err(PasswordError::InvalidIterations)
        ));
    }

    #[test]
    fn rejects_malformed_records() {
        let hasher = hasher();
        let good = hasher.hash("secret").unwrap();
        let parts: Vec<&str> = good.split('$').collect();

        let cases = [
            String::new(),
            "pbkdf2-sha256".to_owned(),
            format!("md5$1000${}${}", parts[2], parts[3]),
            format!("pbkdf2-sha256$abc${}${}", parts[2], parts[3]),
            format!("pbkdf2-sha256$0${}${}", parts[2], parts[3]),
            format!(
                "pbkdf2-sha256${}${}${}",
                MIN_ITERATIONS - 1,
                parts[2],
                parts[3]
            ),
            format!("pbkdf2-sha256${}${}$", pbkdf2::MAX_ITERATIONS + 1, parts[2]),
            format!("{good}$extra"),
            format!("pbkdf2-sha256${}$!!!${}", FAST, parts[3]),
            format!("pbkdf2-sha256${}${}$!!!", FAST, parts[2]),
            format!("pbkdf2-sha256${}$AA${}", FAST, parts[3]),
        ];

        for case in cases {
            assert!(
                matches!(
                    hasher.verify("secret", &case),
                    Err(PasswordError::InvalidFormat)
                ),
                "`{case}` must be rejected"
            );
        }
    }
}
