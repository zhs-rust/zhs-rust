use std::error::Error as StdError;
use std::fmt;

use crate::hmac::HmacSha256;

pub const DEFAULT_ITERATIONS: u32 = 600_000;

pub const MAX_ITERATIONS: u32 = 10_000_000;

pub const MAX_OUTPUT: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pbkdf2Error {
    InvalidIterations,

    EmptyOutput,

    OutputTooLong,
}

impl fmt::Display for Pbkdf2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pbkdf2Error::InvalidIterations => {
                formatter.write_str("PBKDF2 iterations must be between 1 and the maximum")
            }
            Pbkdf2Error::EmptyOutput => formatter.write_str("PBKDF2 output must not be empty"),
            Pbkdf2Error::OutputTooLong => {
                formatter.write_str("PBKDF2 output is longer than the accepted maximum")
            }
        }
    }
}

impl StdError for Pbkdf2Error {}

pub fn sha256(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    output: &mut [u8],
) -> Result<(), Pbkdf2Error> {
    if iterations == 0 || iterations > MAX_ITERATIONS {
        return Err(Pbkdf2Error::InvalidIterations);
    }
    if output.is_empty() {
        return Err(Pbkdf2Error::EmptyOutput);
    }
    if output.len() > MAX_OUTPUT {
        return Err(Pbkdf2Error::OutputTooLong);
    }

    let base = HmacSha256::new(password);

    for (block, chunk) in output.chunks_mut(32).enumerate() {
        let index = u32::try_from(block + 1).expect("output length is bounded");
        let mut mac = base.clone();
        mac.update(salt);
        mac.update(&index.to_be_bytes());
        let mut previous = mac.finish();
        let mut accumulator = previous;

        for _ in 1..iterations {
            let mut mac = base.clone();
            mac.update(&previous);
            previous = mac.finish();

            for (accumulator, previous) in accumulator.iter_mut().zip(previous) {
                *accumulator ^= previous;
            }
        }

        chunk.copy_from_slice(&accumulator[..chunk.len()]);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::sha256;
    use crate::hex;

    fn derive(password: &[u8], salt: &[u8], iterations: u32, length: usize) -> String {
        let mut output = vec![0u8; length];
        sha256(password, salt, iterations, &mut output).unwrap();
        hex::encode(&output)
    }

    #[test]
    fn matches_reference_vectors() {
        assert_eq!(
            derive(b"password", b"salt", 1, 32),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            derive(b"password", b"salt", 2, 32),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
        assert_eq!(
            derive(b"password", b"salt", 4_096, 32),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
        assert_eq!(
            derive(
                b"passwordPASSWORDpassword",
                b"saltSALTsaltSALTsaltSALTsaltSALTsalt",
                4_096,
                40
            ),
            "348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1c635518c7dac47e9"
        );
        assert_eq!(
            derive(b"passwd", b"0123456789abcdef", 1, 64),
            "93b53e8a2c86901f20095a7b35d35f081ba4220f3b7931ba23646897133ac5bfe7fddef153e9985d346d38ca7c12c8626a93861b33a207731f7c39999d8df4a6"
        );
    }

    #[test]
    fn rejects_invalid_parameters() {
        let mut output = [0u8; 32];
        assert_eq!(
            sha256(b"password", b"salt", 0, &mut output),
            Err(super::Pbkdf2Error::InvalidIterations)
        );
        assert_eq!(
            sha256(b"password", b"salt", super::MAX_ITERATIONS + 1, &mut output),
            Err(super::Pbkdf2Error::InvalidIterations)
        );
        assert_eq!(
            sha256(b"password", b"salt", 1, &mut []),
            Err(super::Pbkdf2Error::EmptyOutput)
        );
        assert_eq!(
            sha256(
                b"password",
                b"salt",
                1,
                &mut vec![0u8; super::MAX_OUTPUT + 1]
            ),
            Err(super::Pbkdf2Error::OutputTooLong)
        );
    }
}
