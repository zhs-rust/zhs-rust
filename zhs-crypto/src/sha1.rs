const INITIAL: [u32; 5] = [
    0x6745_2301,
    0xefcd_ab89,
    0x98ba_dcfe,
    0x1032_5476,
    0xc3d2_e1f0,
];

const BLOCK: usize = 64;

#[derive(Clone, Debug)]
pub struct Sha1 {
    state: [u32; 5],
    buffer: [u8; BLOCK],
    buffered: usize,
    length: u64,
}

impl Default for Sha1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha1 {
    pub const fn new() -> Self {
        Self {
            state: INITIAL,
            buffer: [0; BLOCK],
            buffered: 0,
            length: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);

        if self.buffered > 0 {
            let needed = BLOCK - self.buffered;
            let take = needed.min(data.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];

            if self.buffered == BLOCK {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }

        while data.len() >= BLOCK {
            let (block, rest) = data.split_at(BLOCK);
            let mut chunk = [0u8; BLOCK];
            chunk.copy_from_slice(block);
            self.compress(&chunk);
            data = rest;
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffered = data.len();
        }
    }

    pub fn finish(mut self) -> [u8; 20] {
        let bit_length = self.length.wrapping_mul(8);

        let mut buffer = [0u8; 128];
        buffer[..self.buffered].copy_from_slice(&self.buffer[..self.buffered]);
        buffer[self.buffered] = 0x80;
        let end = if self.buffered < 56 { 56 } else { 120 };
        buffer[end..end + 8].copy_from_slice(&bit_length.to_be_bytes());

        for block in 0..(end + 8) / BLOCK {
            let mut chunk = [0u8; BLOCK];
            chunk.copy_from_slice(&buffer[block * BLOCK..(block + 1) * BLOCK]);
            self.compress(&chunk);
        }

        let mut digest = [0u8; 20];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(&mut self, block: &[u8; BLOCK]) {
        let mut schedule = [0u32; 80];
        for (index, slot) in schedule.iter_mut().enumerate().take(16) {
            let base = index * 4;
            *slot = u32::from_be_bytes([
                block[base],
                block[base + 1],
                block[base + 2],
                block[base + 3],
            ]);
        }
        for index in 16..80 {
            schedule[index] = (schedule[index - 3]
                ^ schedule[index - 8]
                ^ schedule[index - 14]
                ^ schedule[index - 16])
                .rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = self.state;

        for (index, word) in schedule.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999_u32),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };

            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);

            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
    }
}

pub fn digest(data: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::{Sha1, digest};

    fn hex(bytes: [u8; 20]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn matches_fips_180_4_vectors() {
        assert_eq!(hex(digest(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(digest(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(digest(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn handles_block_boundaries_and_incremental_updates() {
        for length in [0_usize, 1, 55, 56, 63, 64, 65, 119, 120, 127, 128, 200] {
            let data = vec![b'a'; length];
            assert_eq!(digest(&data), digest(&vec![b'a'; length]));

            let mut hasher = Sha1::new();
            for chunk in data.chunks(7) {
                hasher.update(chunk);
            }
            assert_eq!(hasher.finish(), digest(&data), "length {length}");
        }
    }

    #[test]
    fn matches_the_million_a_vector() {
        let mut hasher = Sha1::new();
        for _ in 0..1000 {
            hasher.update(&[b'a'; 1000]);
        }
        assert_eq!(
            hex(hasher.finish()),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }
}
