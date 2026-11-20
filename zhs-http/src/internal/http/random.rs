use zhs_crypto::{hex, random};

pub(crate) fn random_hex(bytes: usize) -> String {
    hex::encode(&random::bytes(bytes).expect("secure random source failed"))
}

#[cfg(test)]
mod tests {
    use super::random_hex;

    #[test]
    fn produces_the_requested_length() {
        assert_eq!(random_hex(0), "");
        assert_eq!(random_hex(1).len(), 2);
        assert_eq!(random_hex(16).len(), 32);
        assert!(random_hex(16).bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn successive_values_differ() {
        assert_ne!(random_hex(32), random_hex(32));
    }
}
