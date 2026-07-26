//! LZ4 compression helpers for cold segments.
//!
//! Reuses `lz4_flex` (same crate as SIMG v2 / kernel ZRAM) when the `host`
//! or `sunlightos` feature is enabled. Decompression always validates the
//! configured maximum uncompressed size **before** allocating.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::error::MemoryError;

/// Algorithm id: no compression.
pub const COMPRESSION_NONE: u8 = 0;
/// Algorithm id: LZ4 block (lz4_flex).
pub const COMPRESSION_LZ4: u8 = 1;

/// Compress `input` with LZ4. Returns compressed bytes.
#[cfg(any(feature = "host", feature = "sunlightos"))]
pub fn compress_lz4(input: &[u8]) -> Result<Vec<u8>, MemoryError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let max = lz4_flex::block::get_maximum_output_size(input.len()).max(64);
    let mut out = vec![0u8; max];
    let written = lz4_flex::block::compress_into(input, &mut out)
        .map_err(|_| MemoryError::CompressionFailure)?;
    out.truncate(written);
    Ok(out)
}

#[cfg(not(any(feature = "host", feature = "sunlightos")))]
pub fn compress_lz4(_input: &[u8]) -> Result<Vec<u8>, MemoryError> {
    Err(MemoryError::CompressionFailure)
}

/// Decompress LZ4 into a buffer of exact `uncompressed_len`.
///
/// **Security:** `uncompressed_len` is checked against `max_allowed` before
/// any allocation. Never pass attacker-controlled lengths without this check.
#[cfg(any(feature = "host", feature = "sunlightos"))]
pub fn decompress_lz4_checked(
    input: &[u8],
    uncompressed_len: u32,
    max_allowed: u32,
) -> Result<Vec<u8>, MemoryError> {
    if uncompressed_len > max_allowed {
        return Err(MemoryError::PayloadTooLarge {
            size: uncompressed_len,
            max: max_allowed,
        });
    }
    if uncompressed_len == 0 {
        return Ok(Vec::new());
    }
    let mut out = vec![0u8; uncompressed_len as usize];
    let written = lz4_flex::block::decompress_into(input, &mut out)
        .map_err(|_| MemoryError::DecompressionFailure)?;
    if written != uncompressed_len as usize {
        return Err(MemoryError::DecompressionFailure);
    }
    Ok(out)
}

#[cfg(not(any(feature = "host", feature = "sunlightos")))]
pub fn decompress_lz4_checked(
    _input: &[u8],
    uncompressed_len: u32,
    max_allowed: u32,
) -> Result<Vec<u8>, MemoryError> {
    if uncompressed_len > max_allowed {
        return Err(MemoryError::PayloadTooLarge {
            size: uncompressed_len,
            max: max_allowed,
        });
    }
    Err(MemoryError::DecompressionFailure)
}

/// IEEE CRC32 (polynomial 0xEDB88320), matching sunlight-kv style integrity.
pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lz4_roundtrip() {
        let src = b"hello wise owl short-term memory foundation ".repeat(20);
        let c = compress_lz4(&src).unwrap();
        assert!(!c.is_empty());
        assert!(c.len() < src.len() || src.len() < 32);
        let d = decompress_lz4_checked(&c, src.len() as u32, 1024 * 1024).unwrap();
        assert_eq!(d, src);
    }

    #[test]
    fn corrupt_compressed_fails() {
        let src = b"abcdefghijklmnop".repeat(8);
        let mut c = compress_lz4(&src).unwrap();
        if !c.is_empty() {
            c[0] ^= 0xFF;
        }
        let r = decompress_lz4_checked(&c, src.len() as u32, 1024 * 1024);
        assert!(r.is_err());
    }

    #[test]
    fn oversized_uncompressed_header_rejected() {
        let src = b"tiny";
        let c = compress_lz4(src).unwrap();
        let r = decompress_lz4_checked(&c, 10_000_000, 1024);
        assert!(matches!(
            r,
            Err(MemoryError::PayloadTooLarge { size: 10_000_000, max: 1024 })
        ));
    }

    #[test]
    fn crc32_stable() {
        assert_eq!(crc32_ieee(b""), 0);
        // Known vector: CRC-32 of "123456789" is 0xCBF43926
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }
}
