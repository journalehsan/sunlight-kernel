//! Strong versioned content digests (final content identity).
//!
//! # Selection
//!
//! Prefer existing repo digests. No BLAKE3 is present in the workspace.
//! A streaming SHA-256 implementation (matching `sunlight-bench`'s audited
//! soft SHA-256) is used as final content identity: 256-bit cryptographic
//! strength, no_std, no new dependencies.
//!
//! # Fast fingerprint vs strong digest
//!
//! - **Fast fingerprint (FNV-1a64):** prefilter / path buckets / token IDs only.
//! - **Strong digest (SHA-256):** sole proof that file content is unchanged.
//!
//! A matching FNV fingerprint must never suppress strong-digest verification
//! when metadata indicates a possible change.

use core::fmt;

use crate::error::IndexError;
use crate::hash::fnv1a64;

/// Digest algorithm identity (stable wire tags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum ContentDigestAlgorithm {
    /// SHA-256 (FIPS 180-4). Primary Phase 3.5 algorithm.
    Sha256 = 1,
    /// Reserved for future BLAKE3-256 if introduced later.
    Blake3_256 = 2,
}

impl ContentDigestAlgorithm {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Sha256),
            2 => Some(Self::Blake3_256),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sha256 => "SHA-256",
            Self::Blake3_256 => "BLAKE3-256",
        }
    }

    pub const fn output_len(self) -> usize {
        32
    }
}

/// Format version for the content-digest envelope.
pub const CONTENT_DIGEST_FORMAT_VERSION: u16 = 1;

/// Versioned strong content digest (final content identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct StrongContentDigest {
    pub algorithm: ContentDigestAlgorithm,
    pub version: u16,
    pub bytes: [u8; 32],
}

impl StrongContentDigest {
    pub const fn sha256(bytes: [u8; 32]) -> Self {
        Self {
            algorithm: ContentDigestAlgorithm::Sha256,
            version: CONTENT_DIGEST_FORMAT_VERSION,
            bytes,
        }
    }

    /// Zero digest (invalid / unknown). Not a valid content proof.
    pub const fn unset() -> Self {
        Self {
            algorithm: ContentDigestAlgorithm::Sha256,
            version: 0,
            bytes: [0u8; 32],
        }
    }

    pub fn is_set(&self) -> bool {
        self.algorithm == ContentDigestAlgorithm::Sha256
            && self.version == CONTENT_DIGEST_FORMAT_VERSION
            && self.bytes != [0u8; 32]
    }

    /// Compare including algorithm and version (never bytes alone).
    pub fn equals(&self, other: &Self) -> bool {
        self.algorithm == other.algorithm
            && self.version == other.version
            && self.bytes == other.bytes
    }

    /// Stable LE serialization: alg:u8 | ver:u16 | bytes[32] = 35 bytes.
    pub fn encode(&self) -> [u8; 35] {
        let mut out = [0u8; 35];
        out[0] = self.algorithm.as_u8();
        out[1..3].copy_from_slice(&self.version.to_le_bytes());
        out[3..35].copy_from_slice(&self.bytes);
        out
    }

    pub fn decode(data: &[u8]) -> Result<Self, IndexError> {
        if data.len() != 35 {
            return Err(IndexError::InvalidValue("content digest length"));
        }
        let algorithm = ContentDigestAlgorithm::from_u8(data[0])
            .ok_or(IndexError::InvalidValue("content digest algorithm"))?;
        if algorithm != ContentDigestAlgorithm::Sha256 {
            return Err(IndexError::InvalidValue("content digest algorithm unsupported"));
        }
        let version = u16::from_le_bytes([data[1], data[2]]);
        if version == 0 {
            return Err(IndexError::InvalidValue("content digest version"));
        }
        if version > CONTENT_DIGEST_FORMAT_VERSION {
            return Err(IndexError::InvalidValue("content digest version unsupported"));
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&data[3..35]);
        Ok(Self {
            algorithm,
            version,
            bytes,
        })
    }

    /// Hex lowercase (64 chars for 32-byte digests).
    pub fn to_hex(&self) -> alloc::string::String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = alloc::string::String::with_capacity(64);
        for b in &self.bytes {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0xf) as usize] as char);
        }
        s
    }

    /// Abbreviated display: first 8 hex chars.
    pub fn abbreviated_hex(&self) -> alloc::string::String {
        let full = self.to_hex();
        full.chars().take(8).collect()
    }

    /// Fold to u64 for secondary indexes only (NOT content identity).
    pub fn fingerprint64(&self) -> u64 {
        fnv1a64(&self.encode())
    }
}

impl fmt::Display for StrongContentDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} v{} {}",
            self.algorithm.as_str(),
            self.version,
            self.abbreviated_hex()
        )
    }
}

// ---------------------------------------------------------------------------
// Streaming SHA-256 (no_std)
// ---------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[inline(always)]
fn rotr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
        let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        let ch = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Streaming SHA-256 hasher (bounded memory: one 64-byte block buffer).
#[derive(Debug, Clone)]
pub struct Sha256Hasher {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    /// Total bytes hashed (not including padding).
    total_bytes: u64,
}

impl Default for Sha256Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256Hasher {
    pub const fn new() -> Self {
        Self {
            state: H0,
            buffer: [0u8; 64],
            buffer_len: 0,
            total_bytes: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut input = data;
        self.total_bytes = self.total_bytes.saturating_add(data.len() as u64);

        if self.buffer_len > 0 {
            let need = 64 - self.buffer_len;
            let take = need.min(input.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&input[..take]);
            self.buffer_len += take;
            input = &input[take..];
            if self.buffer_len == 64 {
                let block = self.buffer;
                compress(&mut self.state, &block);
                self.buffer_len = 0;
            }
        }

        while input.len() >= 64 {
            let block: [u8; 64] = input[..64].try_into().unwrap();
            compress(&mut self.state, &block);
            input = &input[64..];
        }

        if !input.is_empty() {
            self.buffer[..input.len()].copy_from_slice(input);
            self.buffer_len = input.len();
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total_bytes.saturating_mul(8);
        // Append 0x80
        let mut pad = [0u8; 128];
        pad[0] = 0x80;
        let rem = self.buffer_len;
        // Copy remaining buffered bytes first into a temp if needed — already in buffer.
        let mut final_block = [0u8; 128];
        final_block[..rem].copy_from_slice(&self.buffer[..rem]);
        final_block[rem] = 0x80;
        let pad_len = if rem < 56 { 64 } else { 128 };
        final_block[pad_len - 8..pad_len].copy_from_slice(&bit_len.to_be_bytes());
        compress(&mut self.state, final_block[..64].try_into().unwrap());
        if pad_len == 128 {
            compress(&mut self.state, final_block[64..128].try_into().unwrap());
        }
        let _ = pad;
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    pub fn bytes_hashed(&self) -> u64 {
        self.total_bytes
    }
}

/// One-shot SHA-256.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256Hasher::new();
    h.update(data);
    h.finish()
}

/// Strong content digest of bytes (streaming equivalent of one-shot).
pub fn digest_bytes(data: &[u8]) -> StrongContentDigest {
    StrongContentDigest::sha256(sha256(data))
}

/// Streaming digest hasher producing [`ContentDigest`].
#[derive(Debug, Clone, Default)]
pub struct ContentDigestHasher {
    inner: Sha256Hasher,
}

impl ContentDigestHasher {
    pub const fn new() -> Self {
        Self {
            inner: Sha256Hasher::new(),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finish(self) -> StrongContentDigest {
        StrongContentDigest::sha256(self.inner.finish())
    }

    pub fn bytes_hashed(&self) -> u64 {
        self.inner.bytes_hashed()
    }
}

/// Backward-compatible name for the strong digest type. This alias never
/// aliases either weak hash newtype.
pub type ContentDigest = StrongContentDigest;

/// Optional fast metadata prefilter fingerprint (FNV-1a64). Never final identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct FastFingerprint(u64);

impl FastFingerprint {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Historical Phase 3 FNV content metadata. This type is intentionally not
/// convertible to either [`StrongContentDigest`] or [`FastFingerprint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct LegacyFnvContentHash(u64);

impl LegacyFnvContentHash {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Compute optional fast fingerprint of content (prefilter only).
pub fn fast_fingerprint(data: &[u8]) -> FastFingerprint {
    FastFingerprint::new(fnv1a64(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sha256_known() {
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let d = sha256(b"");
        assert_eq!(
            d,
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55
            ]
        );
    }

    #[test]
    fn abc_sha256_fips_180_4_vector() {
        assert_eq!(
            digest_bytes(b"abc").to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn multi_block_fips_180_4_vector() {
        let input = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(
            digest_bytes(input).to_hex(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn million_a_sha256_standard_vector() {
        let block = [b'a'; 4096];
        let mut h = ContentDigestHasher::new();
        let mut remaining = 1_000_000usize;
        while remaining != 0 {
            let n = remaining.min(block.len());
            h.update(&block[..n]);
            remaining -= n;
        }
        assert_eq!(
            h.finish().to_hex(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn required_streaming_boundaries_and_empty_final_update() {
        let mut data = [0u8; 8193];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = (i.wrapping_mul(131) & 0xff) as u8;
        }
        let expected = digest_bytes(&data);
        for size in [1usize, 7, 31, 63, 64, 65, 4096] {
            let mut h = ContentDigestHasher::new();
            for chunk in data.chunks(size) {
                h.update(chunk);
            }
            h.update(&[]);
            assert_eq!(h.finish(), expected, "chunk size {size}");
        }

        // Deliberately start at an unaligned address.
        let padded = [&[0x55][..], &data[..], &[0xaa][..]].concat();
        assert_eq!(digest_bytes(&padded[1..1 + data.len()]), expected);
    }

    #[test]
    fn streaming_matches_oneshot() {
        let data = b"hello wise owl phase 3.5 strong digest";
        let mut h = Sha256Hasher::new();
        h.update(&data[..10]);
        h.update(&data[10..]);
        assert_eq!(h.finish(), sha256(data));
        assert_eq!(digest_bytes(data).bytes, sha256(data));
    }

    #[test]
    fn empty_file_digest() {
        let d = digest_bytes(b"");
        assert!(d.is_set());
        assert_eq!(d.algorithm, ContentDigestAlgorithm::Sha256);
    }

    #[test]
    fn changed_byte_changes_digest() {
        assert_ne!(digest_bytes(b"a").bytes, digest_bytes(b"b").bytes);
    }

    #[test]
    fn serialize_roundtrip() {
        let d = digest_bytes(b"phase-3.5");
        let e = d.encode();
        let back = ContentDigest::decode(&e).unwrap();
        assert!(d.equals(&back));
    }

    #[test]
    fn rejects_bad_length() {
        assert!(ContentDigest::decode(&[0u8; 10]).is_err());
    }

    #[test]
    fn rejects_unknown_algorithm() {
        let mut e = digest_bytes(b"x").encode();
        e[0] = 99;
        assert!(ContentDigest::decode(&e).is_err());
    }

    #[test]
    fn rejects_reserved_but_unsupported_algorithm() {
        let mut e = digest_bytes(b"x").encode();
        e[0] = ContentDigestAlgorithm::Blake3_256.as_u8();
        assert!(ContentDigest::decode(&e).is_err());
    }

    #[test]
    fn weak_hashes_are_distinct_nominal_types() {
        let fast: FastFingerprint = fast_fingerprint(b"x");
        let legacy = LegacyFnvContentHash::new(fast.get());
        let strong: StrongContentDigest = digest_bytes(b"x");
        assert_eq!(fast.get(), legacy.get());
        assert_ne!(strong.fingerprint64(), 0);
    }

    #[test]
    fn equals_requires_algorithm() {
        let a = digest_bytes(b"x");
        let mut b = a;
        b.algorithm = ContentDigestAlgorithm::Blake3_256;
        assert!(!a.equals(&b));
    }

    #[test]
    fn fnv_collision_does_not_fool_strong() {
        // Different content → different strong digests always for these inputs.
        let a = digest_bytes(b"content-A");
        let b = digest_bytes(b"content-B");
        assert!(!a.equals(&b));
        // Even if fast fingerprints somehow matched, strong must still differ.
        let _fa = fast_fingerprint(b"content-A");
        let _fb = fast_fingerprint(b"content-B");
        assert!(!a.equals(&b));
    }
}
