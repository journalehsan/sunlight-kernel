//! Allocation-free 4 KiB LZ4 codec and integrity helpers.

pub const PAGE_SIZE: usize = 4096;
// Matches lz4_flex 0.11's allocation-free `compress_into` bound.
pub const MAX_COMPRESSED_SIZE: usize = 20 + PAGE_SIZE * 110 / 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    CompressionFailed,
    Incompressible,
    InvalidLength,
    DecompressionFailed,
    ChecksumMismatch,
}

/// FNV-1a is a lightweight accidental-corruption detector, not an
/// authentication primitive. ZRAM data never leaves trusted kernel memory.
pub fn checksum(bytes: &[u8]) -> u32 {
    let mut value = 0x811c_9dc5u32;
    for byte in bytes {
        value ^= u32::from(*byte);
        value = value.wrapping_mul(0x0100_0193);
    }
    value
}

pub fn compress_page(
    src: &[u8; PAGE_SIZE],
    output: &mut [u8; MAX_COMPRESSED_SIZE],
) -> Result<(usize, u32), CodecError> {
    let written =
        lz4_flex::block::compress_into(src, output).map_err(|_| CodecError::CompressionFailed)?;
    if written == 0 || written >= PAGE_SIZE {
        return Err(CodecError::Incompressible);
    }
    Ok((written, checksum(src)))
}

pub fn decompress_page(
    compressed: &[u8],
    expected_checksum: u32,
    output: &mut [u8; PAGE_SIZE],
) -> Result<(), CodecError> {
    if compressed.is_empty() || compressed.len() >= PAGE_SIZE {
        return Err(CodecError::InvalidLength);
    }
    let written = lz4_flex::block::decompress_into(compressed, output)
        .map_err(|_| CodecError::DecompressionFailed)?;
    if written != PAGE_SIZE {
        output.fill(0);
        return Err(CodecError::InvalidLength);
    }
    if checksum(output) != expected_checksum {
        output.fill(0);
        return Err(CodecError::ChecksumMismatch);
    }
    Ok(())
}
