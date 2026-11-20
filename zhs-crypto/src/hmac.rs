use crate::constant_time;
use crate::sha256::{self, Sha256};

const BLOCK: usize = 64;

#[derive(Clone)]
pub struct HmacSha256 {
    inner: Sha256,
    outer_pad: [u8; BLOCK],
}

impl HmacSha256 {
    pub fn new(key: &[u8]) -> Self {
        let mut block = [0u8; BLOCK];
        if key.len() > BLOCK {
            block[..32].copy_from_slice(&sha256::digest(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }

        let mut inner_pad = [0x36u8; BLOCK];
        let mut outer_pad = [0x5cu8; BLOCK];
        for index in 0..BLOCK {
            inner_pad[index] ^= block[index];
            outer_pad[index] ^= block[index];
        }

        let mut inner = Sha256::new();
        inner.update(&inner_pad);

        Self { inner, outer_pad }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finish(self) -> [u8; 32] {
        let inner = self.inner.finish();

        let mut outer = Sha256::new();
        outer.update(&self.outer_pad);
        outer.update(&inner);
        outer.finish()
    }
}

pub fn sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new(key);
    mac.update(data);
    mac.finish()
}

pub fn verify(key: &[u8], data: &[u8], expected: &[u8]) -> bool {
    constant_time::eq(&sha256(key, data), expected)
}

#[cfg(test)]
mod tests {
    use super::{HmacSha256, sha256, verify};
    use crate::hex;

    fn assert_tag(key: &[u8], data: &[u8], expected: &str) {
        let tag = sha256(key, data);
        assert_eq!(hex::encode(&tag), expected);
        assert!(verify(key, data, &tag));
        assert!(!verify(key, data, b"short"));
    }

    #[test]
    fn matches_rfc_4231_vectors() {
        assert_tag(
            &[0x0b; 20],
            b"Hi There",
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        );

        assert_tag(
            b"Jefe",
            b"what do ya want for nothing?",
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        );

        assert_tag(
            &[0xaa; 20],
            &[0xdd; 50],
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
        );

        let key: Vec<u8> = (0x01..=0x19).collect();
        assert_tag(
            &key,
            &[0xcd; 50],
            "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
        );

        assert_tag(
            &[0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First",
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
        );

        assert_tag(
            &[0xaa; 131],
            b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.",
            "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
        );
    }

    #[test]
    fn empty_key_and_message_are_valid() {
        assert_tag(
            b"",
            b"",
            "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad",
        );
    }

    #[test]
    fn streaming_and_cloning_match_the_one_shot_digest() {
        let mut mac = HmacSha256::new(b"key");
        mac.update(b"seg");
        mac.update(b"mented");
        assert_eq!(mac.finish(), sha256(b"key", b"segmented"));

        let base = HmacSha256::new(b"key");
        let mut first = base.clone();
        first.update(b"a");
        let mut second = base.clone();
        second.update(b"b");

        assert_eq!(first.finish(), sha256(b"key", b"a"));
        assert_eq!(second.finish(), sha256(b"key", b"b"));
    }
}
