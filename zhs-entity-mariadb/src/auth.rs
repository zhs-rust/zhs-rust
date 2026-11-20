use zhs_crypto::sha1;

pub(crate) fn native_password(password: &str, nonce: &[u8]) -> Vec<u8> {
    if password.is_empty() {
        return Vec::new();
    }

    let stage1 = sha1::digest(password.as_bytes());
    let stage2 = sha1::digest(&stage1);

    let mut hasher = sha1::Sha1::new();
    hasher.update(nonce);
    hasher.update(&stage2);
    let stage3 = hasher.finish();

    stage1
        .iter()
        .zip(stage3.iter())
        .map(|(left, right)| left ^ right)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::native_password;

    #[test]
    fn empty_password_produces_no_response() {
        assert!(native_password("", b"01234567890123456789").is_empty());
    }

    #[test]
    fn response_depends_on_the_nonce() {
        let first = native_password("secret", b"01234567890123456789");
        let second = native_password("secret", b"98765432109876543210");

        assert_eq!(first.len(), 20);
        assert_eq!(second.len(), 20);
        assert_ne!(first, second);
    }

    #[test]
    fn response_is_stable_for_the_same_inputs() {
        let nonce = b"01234567890123456789";
        assert_eq!(
            native_password("secret", nonce),
            native_password("secret", nonce)
        );
    }
}
