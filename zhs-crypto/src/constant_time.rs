use std::hint::black_box;

pub fn eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }

    black_box(difference) == 0
}

#[cfg(test)]
mod tests {
    use super::eq;

    #[test]
    fn compares_bytes() {
        assert!(eq(b"", b""));
        assert!(eq(b"secret", b"secret"));
        assert!(!eq(b"secret", b"secreT"));
        assert!(!eq(b"secret", b"secrets"));
        assert!(!eq(b"secre", b"secret"));
        assert!(!eq(&[0xff; 32], &[0x00; 32]));
    }
}
