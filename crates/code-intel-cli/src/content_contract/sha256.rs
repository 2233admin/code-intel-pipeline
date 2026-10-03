//! Shared SHA-256 leaf implementation, included alongside JSON content contracts.

/// Streaming SHA-256: absorbs input in pieces, so a caller can hash a
/// sequence (a whole worktree, say) without materializing it.
///
/// `sha256_hex` is the one-shot form of this. Snapshot identity and every
/// recorded artifact digest are defined over these exact bytes, so chunk
/// boundaries must never change the result.
pub(crate) struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    length: u64,
}

impl Sha256 {
    pub(crate) fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            filled: 0,
            length: 0,
        }
    }

    pub(crate) fn update(&mut self, mut bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        if self.filled > 0 {
            let take = (64 - self.filled).min(bytes.len());
            self.block[self.filled..self.filled + take].copy_from_slice(&bytes[..take]);
            self.filled += take;
            bytes = &bytes[take..];
            if self.filled < 64 {
                return;
            }
            let block = self.block;
            self.compress(&block);
            self.filled = 0;
        }
        let mut blocks = bytes.chunks_exact(64);
        for block in &mut blocks {
            self.compress(block.try_into().expect("chunks_exact yields 64 bytes"));
        }
        let rest = blocks.remainder();
        self.block[..rest.len()].copy_from_slice(rest);
        self.filled = rest.len();
    }

    /// Absorb one length-framed record: its length as a big-endian `u64`,
    /// then its bytes. Snapshot digests are defined over this framing.
    pub(crate) fn update_framed(&mut self, bytes: &[u8]) {
        self.update(&(bytes.len() as u64).to_be_bytes());
        self.update(bytes);
    }

    /// Apply the `0x80` terminator, zero padding, and big-endian bit length,
    /// then return the lowercase hex digest.
    pub(crate) fn finish(mut self) -> String {
        let bits = self.length.wrapping_mul(8);
        self.block[self.filled] = 0x80;
        self.block[self.filled + 1..].fill(0);
        if self.filled >= 56 {
            let block = self.block;
            self.compress(&block);
            self.block = [0; 64];
        }
        self.block[56..].copy_from_slice(&bits.to_be_bytes());
        let block = self.block;
        self.compress(&block);
        self.state.iter().map(|v| format!("{v:08x}")).collect()
    }

    fn compress(&mut self, chunk: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap())
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1)
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2)
        }
        for (state, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(value)
        }
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    /// FIPS 180-2 vectors around the padding boundaries: the 56-byte message
    /// forces the extra length block, and a million bytes fed in odd-sized
    /// pieces exercises the partial-block carry across `update` calls.
    #[test]
    fn streaming_sha256_matches_published_vectors() {
        assert_eq!(
            super::sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            super::sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let mut hasher = super::Sha256::new();
        let piece = [b'a'; 997];
        let mut remaining = 1_000_000;
        while remaining > 0 {
            let take = remaining.min(piece.len());
            hasher.update(&piece[..take]);
            remaining -= take;
        }
        assert_eq!(
            hasher.finish(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn streaming_sha256_ignores_chunk_boundaries() {
        let message = (0..300u32)
            .map(|i| (i * 31 % 256) as u8)
            .collect::<Vec<_>>();
        for len in [0, 1, 55, 56, 63, 64, 65, 119, 120, 128, 300] {
            let whole = super::sha256_hex(&message[..len]);
            for split in 0..=len {
                let mut hasher = super::Sha256::new();
                hasher.update(&message[..split]);
                hasher.update(&message[split..len]);
                assert_eq!(hasher.finish(), whole, "len {len} split {split}");
            }
        }
    }
}
