//! SIMG v2 — versioned lossless image container (BGRA8 + optional Sub + LZ4).
//!
//! # Legal / patent notice
//!
//! We are **not sure** of the patent-free status of this format combination
//! (LZ4 block compression and reversible per-row Sub filtering as packaged for
//! SunlightOS). This implementation is for engineering use inside the project
//! and **needs formal legal review** before any claim of “patent free” or
//! broad redistribution under that premise. This comment is not legal advice.
//!
//! See `docs/SIMG_V2.md` for the on-disk specification.

use alloc::vec;
use alloc::vec::Vec;

use crate::crc32::crc32_ieee;
use crate::{rgba_len, ImageError, ImageRgba8};

/// ASCII magic `SIMG` — not a valid TGA image-type header.
pub const MAGIC: [u8; 4] = *b"SIMG";
pub const VERSION: u16 = 2;
pub const HEADER_SIZE: u16 = 36;
pub const HEADER_SIZE_USIZE: usize = HEADER_SIZE as usize;

/// Practical maximum edge length (pixels).
pub const MAX_DIMENSION: u32 = 8192;
/// Practical maximum decoded pixel buffer size (bytes).
pub const MAX_DECODED_BYTES: u32 = 64 * 1024 * 1024;

pub const FLAG_CRC32: u32 = 1;

pub const PIXEL_FORMAT_BGRA8: u8 = 1;
pub const ALPHA_STRAIGHT: u8 = 1;

pub const COMPRESSION_NONE: u8 = 0;
pub const COMPRESSION_LZ4: u8 = 1;

pub const FILTER_NONE: u8 = 0;
pub const FILTER_SUB: u8 = 1;

const BPP: usize = 4;

/// Encoder identity string embedded in deterministic method selection docs.
pub const ENCODER_VERSION: &str = "simg-v2-1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Lz4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    None,
    Sub,
}

impl Compression {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::None => COMPRESSION_NONE,
            Self::Lz4 => COMPRESSION_LZ4,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self, ImageError> {
        match v {
            COMPRESSION_NONE => Ok(Self::None),
            COMPRESSION_LZ4 => Ok(Self::Lz4),
            _ => Err(ImageError::UnsupportedCompression),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "raw",
            Self::Lz4 => "lz4",
        }
    }
}

impl Filter {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::None => FILTER_NONE,
            Self::Sub => FILTER_SUB,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self, ImageError> {
        match v {
            FILTER_NONE => Ok(Self::None),
            FILTER_SUB => Ok(Self::Sub),
            _ => Err(ImageError::InvalidHeader),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sub => "sub",
        }
    }
}

/// Parsed SIMG v2 header fields (validated enums).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimgV2Header {
    pub version: u16,
    pub header_size: u16,
    pub flags: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: u8,
    pub alpha_mode: u8,
    pub compression: Compression,
    pub filter: Filter,
    pub uncompressed_size: u32,
    pub payload_size: u32,
    pub crc32: u32,
}

/// Result of encoding, including which candidate won.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeReport {
    pub bytes: Vec<u8>,
    pub compression: Compression,
    pub filter: Filter,
    pub raw_payload_size: usize,
    pub encoded_payload_size: usize,
    pub file_size: usize,
}

/// True when the first four bytes are the SIMG magic.
#[inline]
pub fn is_simg_v2(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == MAGIC
}

/// Validate dimensions and return the exact BGRA8 byte length.
pub fn checked_layout(width: u32, height: u32) -> Result<usize, ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::InvalidDimensions);
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(ImageError::InvalidDimensions);
    }
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .ok_or(ImageError::InvalidDimensions)?;
    let bytes = pixels
        .checked_mul(BPP as u64)
        .ok_or(ImageError::InvalidDimensions)?;
    if bytes > u64::from(MAX_DECODED_BYTES) {
        return Err(ImageError::InvalidDimensions);
    }
    usize::try_from(bytes).map_err(|_| ImageError::InvalidDimensions)
}

fn combo_valid(compression: Compression, filter: Filter) -> bool {
    matches!(
        (compression, filter),
        (Compression::None, Filter::None)
            | (Compression::Lz4, Filter::None)
            | (Compression::Lz4, Filter::Sub)
    )
}

/// Parse and validate the v2 header without touching the payload.
pub fn parse_header(data: &[u8]) -> Result<SimgV2Header, ImageError> {
    if data.len() < 4 {
        return Err(ImageError::TruncatedInput);
    }
    if data[0..4] != MAGIC {
        return Err(ImageError::UnsupportedFormat);
    }
    if data.len() < 8 {
        return Err(ImageError::TruncatedInput);
    }
    let version = u16::from_le_bytes([data[4], data[5]]);
    if version != VERSION {
        return Err(ImageError::UnsupportedFormat);
    }
    let header_size = u16::from_le_bytes([data[6], data[7]]);
    if header_size != HEADER_SIZE {
        return Err(ImageError::InvalidHeader);
    }
    if data.len() < HEADER_SIZE_USIZE {
        return Err(ImageError::TruncatedInput);
    }

    let flags = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if flags & !FLAG_CRC32 != 0 {
        return Err(ImageError::InvalidHeader);
    }
    let width = u32::from_le_bytes(data[12..16].try_into().unwrap());
    let height = u32::from_le_bytes(data[16..20].try_into().unwrap());
    let pixel_format = data[20];
    let alpha_mode = data[21];
    let compression = Compression::from_u8(data[22])?;
    let filter = Filter::from_u8(data[23])?;
    let uncompressed_size = u32::from_le_bytes(data[24..28].try_into().unwrap());
    let payload_size = u32::from_le_bytes(data[28..32].try_into().unwrap());
    let crc32 = u32::from_le_bytes(data[32..36].try_into().unwrap());

    if pixel_format != PIXEL_FORMAT_BGRA8 {
        return Err(ImageError::UnsupportedBitDepth);
    }
    if alpha_mode != ALPHA_STRAIGHT {
        return Err(ImageError::InvalidHeader);
    }
    if !combo_valid(compression, filter) {
        return Err(ImageError::UnsupportedCompression);
    }

    let expected = checked_layout(width, height)?;
    if uncompressed_size as usize != expected {
        return Err(ImageError::InvalidDimensions);
    }

    let payload_end = (HEADER_SIZE_USIZE as u64)
        .checked_add(u64::from(payload_size))
        .ok_or(ImageError::InvalidHeader)?;
    if payload_end > data.len() as u64 {
        return Err(ImageError::TruncatedInput);
    }

    match compression {
        Compression::None => {
            if payload_size as usize != expected {
                return Err(ImageError::InvalidDimensions);
            }
        }
        Compression::Lz4 => {
            if payload_size == 0 {
                return Err(ImageError::InvalidHeader);
            }
        }
    }

    if flags & FLAG_CRC32 == 0 && crc32 != 0 {
        return Err(ImageError::InvalidHeader);
    }

    Ok(SimgV2Header {
        version,
        header_size,
        flags,
        width,
        height,
        pixel_format,
        alpha_mode,
        compression,
        filter,
        uncompressed_size,
        payload_size,
        crc32,
    })
}

fn write_header(
    out: &mut [u8; HEADER_SIZE_USIZE],
    width: u32,
    height: u32,
    compression: Compression,
    filter: Filter,
    uncompressed_size: u32,
    payload_size: u32,
    crc32: u32,
    flags: u32,
) {
    out[0..4].copy_from_slice(&MAGIC);
    out[4..6].copy_from_slice(&VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&HEADER_SIZE.to_le_bytes());
    out[8..12].copy_from_slice(&flags.to_le_bytes());
    out[12..16].copy_from_slice(&width.to_le_bytes());
    out[16..20].copy_from_slice(&height.to_le_bytes());
    out[20] = PIXEL_FORMAT_BGRA8;
    out[21] = ALPHA_STRAIGHT;
    out[22] = compression.as_u8();
    out[23] = filter.as_u8();
    out[24..28].copy_from_slice(&uncompressed_size.to_le_bytes());
    out[28..32].copy_from_slice(&payload_size.to_le_bytes());
    out[32..36].copy_from_slice(&crc32.to_le_bytes());
}

/// Convert host RGBA8 image to canonical on-disk BGRA8 bytes.
pub fn rgba_to_bgra(image: &ImageRgba8) -> Result<Vec<u8>, ImageError> {
    let expected = rgba_len(image.width, image.height)?;
    if image.pixels.len() != expected {
        return Err(ImageError::InvalidDimensions);
    }
    let _ = checked_layout(image.width, image.height)?;
    let mut out = Vec::with_capacity(expected);
    for chunk in image.pixels.chunks_exact(4) {
        let [r, g, b, a] = [chunk[0], chunk[1], chunk[2], chunk[3]];
        out.push(b);
        out.push(g);
        out.push(r);
        out.push(a);
    }
    Ok(out)
}

/// Convert canonical BGRA8 bytes to host RGBA8 image.
pub fn bgra_to_rgba(width: u32, height: u32, bgra: &[u8]) -> Result<ImageRgba8, ImageError> {
    let expected = checked_layout(width, height)?;
    if bgra.len() != expected {
        return Err(ImageError::InvalidDimensions);
    }
    let mut pixels = Vec::with_capacity(expected);
    for chunk in bgra.chunks_exact(4) {
        let [b, g, r, a] = [chunk[0], chunk[1], chunk[2], chunk[3]];
        pixels.push(r);
        pixels.push(g);
        pixels.push(b);
        pixels.push(a);
    }
    Ok(ImageRgba8 {
        width,
        height,
        pixels,
    })
}

/// Apply Sub filter in place (BGRA8, bpp=4).
pub fn apply_sub_filter_inplace(buf: &mut [u8], width: u32, height: u32) -> Result<(), ImageError> {
    let expected = checked_layout(width, height)?;
    if buf.len() != expected {
        return Err(ImageError::InvalidDimensions);
    }
    let row_len = width as usize * BPP;
    for y in 0..height as usize {
        let row = &mut buf[y * row_len..(y + 1) * row_len];
        // Walk backward so we still see original left neighbors... actually
        // Sub needs previous *original* pixel. Forward pass storing differences
        // requires reading original[i-bpp] which is already overwritten if we
        // go forward naively after writing. Use a 4-byte window of previous original.
        let mut prev = [0u8; BPP];
        if row_len >= BPP {
            prev.copy_from_slice(&row[0..BPP]);
        }
        let mut i = BPP;
        while i < row_len {
            let orig = [row[i], row[i + 1], row[i + 2], row[i + 3]];
            row[i] = orig[0].wrapping_sub(prev[0]);
            row[i + 1] = orig[1].wrapping_sub(prev[1]);
            row[i + 2] = orig[2].wrapping_sub(prev[2]);
            row[i + 3] = orig[3].wrapping_sub(prev[3]);
            prev = orig;
            i += BPP;
        }
    }
    Ok(())
}

/// Reverse Sub filter in place (BGRA8, bpp=4).
pub fn reverse_sub_filter_inplace(
    buf: &mut [u8],
    width: u32,
    height: u32,
) -> Result<(), ImageError> {
    let expected = checked_layout(width, height)?;
    if buf.len() != expected {
        return Err(ImageError::InvalidDimensions);
    }
    let row_len = width as usize * BPP;
    for y in 0..height as usize {
        let row = &mut buf[y * row_len..(y + 1) * row_len];
        let mut i = BPP;
        while i < row_len {
            row[i] = row[i].wrapping_add(row[i - BPP]);
            row[i + 1] = row[i + 1].wrapping_add(row[i + 1 - BPP]);
            row[i + 2] = row[i + 2].wrapping_add(row[i + 2 - BPP]);
            row[i + 3] = row[i + 3].wrapping_add(row[i + 3 - BPP]);
            i += BPP;
        }
    }
    Ok(())
}

fn lz4_compress(input: &[u8]) -> Result<Vec<u8>, ImageError> {
    let max = lz4_flex::block::get_maximum_output_size(input.len()).max(64);
    let mut out = vec![0u8; max];
    let written = lz4_flex::block::compress_into(input, &mut out)
        .map_err(|e| ImageError::EncodeFailed(alloc_string(e)))?;
    out.truncate(written);
    Ok(out)
}

fn lz4_decompress_exact(input: &[u8], output: &mut [u8]) -> Result<(), ImageError> {
    let written = lz4_flex::block::decompress_into(input, output)
        .map_err(|e| ImageError::DecodeFailed(alloc_string(e)))?;
    if written != output.len() {
        return Err(ImageError::DecodeFailed(alloc::string::String::from(
            "lz4 output length mismatch",
        )));
    }
    Ok(())
}

#[cfg(feature = "std")]
fn alloc_string(err: impl core::fmt::Display) -> alloc::string::String {
    alloc::format!("{err}")
}

#[cfg(not(feature = "std"))]
fn alloc_string(err: impl core::fmt::Debug) -> alloc::string::String {
    alloc::format!("{err:?}")
}

fn build_file(
    width: u32,
    height: u32,
    compression: Compression,
    filter: Filter,
    payload: &[u8],
    crc32: u32,
) -> Result<Vec<u8>, ImageError> {
    let uncompressed = checked_layout(width, height)?;
    let mut header = [0u8; HEADER_SIZE_USIZE];
    write_header(
        &mut header,
        width,
        height,
        compression,
        filter,
        uncompressed as u32,
        payload.len() as u32,
        crc32,
        FLAG_CRC32,
    );
    let mut out = Vec::with_capacity(HEADER_SIZE_USIZE + payload.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Encode an RGBA8 image to SIMG v2, selecting the smallest of raw / LZ4 / Sub+LZ4.
pub fn encode(image: &ImageRgba8) -> Result<EncodeReport, ImageError> {
    let bgra = rgba_to_bgra(image)?;
    let width = image.width;
    let height = image.height;
    let raw_len = bgra.len();
    let crc = crc32_ieee(&bgra);

    // Candidate 1: raw
    let raw_file = build_file(width, height, Compression::None, Filter::None, &bgra, crc)?;

    // Candidate 2: LZ4
    let lz4_payload = lz4_compress(&bgra)?;
    let lz4_file = build_file(
        width,
        height,
        Compression::Lz4,
        Filter::None,
        &lz4_payload,
        crc,
    )?;

    // Candidate 3: Sub + LZ4
    let mut filtered = bgra;
    apply_sub_filter_inplace(&mut filtered, width, height)?;
    let sub_lz4_payload = lz4_compress(&filtered)?;
    let sub_file = build_file(
        width,
        height,
        Compression::Lz4,
        Filter::Sub,
        &sub_lz4_payload,
        crc,
    )?;

    // Deterministic selection: smallest file; ties prefer raw, then lz4, then sub+lz4.
    let candidates = [
        (raw_file, Compression::None, Filter::None),
        (lz4_file, Compression::Lz4, Filter::None),
        (sub_file, Compression::Lz4, Filter::Sub),
    ];
    let (best, compression, filter) = candidates
        .into_iter()
        .min_by(|a, b| a.0.len().cmp(&b.0.len()))
        .expect("three candidates");

    Ok(EncodeReport {
        encoded_payload_size: best.len().saturating_sub(HEADER_SIZE_USIZE),
        file_size: best.len(),
        raw_payload_size: raw_len,
        bytes: best,
        compression,
        filter,
    })
}

/// Encode forcing a specific compression/filter pair (for tests and benchmarks).
pub fn encode_with_method(
    image: &ImageRgba8,
    compression: Compression,
    filter: Filter,
) -> Result<EncodeReport, ImageError> {
    if !combo_valid(compression, filter) {
        return Err(ImageError::UnsupportedCompression);
    }
    let mut bgra = rgba_to_bgra(image)?;
    let width = image.width;
    let height = image.height;
    let raw_len = bgra.len();
    let crc = crc32_ieee(&bgra);

    if filter == Filter::Sub {
        apply_sub_filter_inplace(&mut bgra, width, height)?;
    }

    let payload = match compression {
        Compression::None => bgra,
        Compression::Lz4 => lz4_compress(&bgra)?,
    };

    let bytes = build_file(width, height, compression, filter, &payload, crc)?;
    Ok(EncodeReport {
        encoded_payload_size: payload.len(),
        file_size: bytes.len(),
        raw_payload_size: raw_len,
        bytes,
        compression,
        filter,
    })
}

/// Decode SIMG v2 into canonical BGRA8 bytes (one allocation).
/// Magic must match; no TGA fallback.
pub fn decode_bgra(data: &[u8]) -> Result<(SimgV2Header, Vec<u8>), ImageError> {
    let header = parse_header(data)?;
    let expected = header.uncompressed_size as usize;
    let payload_off = header.header_size as usize;
    let payload = &data[payload_off..payload_off + header.payload_size as usize];

    // Single final-size allocation: LZ4 decompresses directly here; Sub reverses in place.
    let mut bgra = Vec::new();
    bgra.try_reserve_exact(expected)
        .map_err(|_| ImageError::DecodeFailed(alloc::string::String::from("out of memory")))?;
    bgra.resize(expected, 0);

    match header.compression {
        Compression::None => {
            bgra.copy_from_slice(payload);
        }
        Compression::Lz4 => {
            lz4_decompress_exact(payload, &mut bgra)?;
        }
    }

    if header.filter == Filter::Sub {
        reverse_sub_filter_inplace(&mut bgra, header.width, header.height)?;
    }

    if header.flags & FLAG_CRC32 != 0 {
        let actual = crc32_ieee(&bgra);
        if actual != header.crc32 {
            return Err(ImageError::DecodeFailed(alloc::string::String::from(
                "crc32 mismatch",
            )));
        }
    }

    Ok((header, bgra))
}

/// Decode SIMG v2 into host RGBA8. Magic must match; no TGA fallback.
pub fn decode(data: &[u8]) -> Result<ImageRgba8, ImageError> {
    let (header, bgra) = decode_bgra(data)?;
    bgra_to_rgba(header.width, header.height, &bgra)
}

/// Decode SIMG v2 into packed ARGB `u32` pixels (top-down, straight alpha).
///
/// **One allocation only:** LZ4 writes into the final `Vec<u32>` (viewed as
/// bytes), Sub reverses in place, and on little-endian hosts the on-disk BGRA
/// byte order already matches the ARGB `u32` packing so no second full-image
/// buffer is required. This keeps peak decode RAM ≈ `width*height*4` so an
/// 8 MiB userspace heap can open sample photos (~4.2 MiB) without OOM panic.
pub fn decode_argb_u32(data: &[u8]) -> Result<(u32, u32, Vec<u32>), ImageError> {
    let header = parse_header(data)?;
    let expected = header.uncompressed_size as usize;
    let width = header.width;
    let height = header.height;
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(ImageError::InvalidDimensions)?;
    if expected != pixel_count * BPP {
        return Err(ImageError::InvalidDimensions);
    }

    let payload_off = header.header_size as usize;
    let payload = &data[payload_off..payload_off + header.payload_size as usize];

    // Final buffer: aligned `u32` words, filled as BGRA bytes.
    // Use try_reserve so OOM becomes ImageError instead of process abort.
    let mut words = Vec::new();
    words
        .try_reserve_exact(pixel_count)
        .map_err(|_| ImageError::DecodeFailed(alloc::string::String::from("out of memory")))?;
    words.resize(pixel_count, 0);
    // SAFETY: `Vec<u32>` is 4-byte aligned; length*4 == expected.
    let bytes: &mut [u8] =
        unsafe { core::slice::from_raw_parts_mut(words.as_mut_ptr() as *mut u8, expected) };

    match header.compression {
        Compression::None => {
            bytes.copy_from_slice(payload);
        }
        Compression::Lz4 => {
            lz4_decompress_exact(payload, bytes)?;
        }
    }

    if header.filter == Filter::Sub {
        reverse_sub_filter_inplace(bytes, width, height)?;
    }

    if header.flags & FLAG_CRC32 != 0 {
        let actual = crc32_ieee(bytes);
        if actual != header.crc32 {
            return Err(ImageError::DecodeFailed(alloc::string::String::from(
                "crc32 mismatch",
            )));
        }
    }

    // Little-endian: memory [B,G,R,A] as u32 LE == (A<<24)|(R<<16)|(G<<8)|B.
    // Big-endian (not a Sunlight target): rebuild ARGB words from BGRA bytes.
    #[cfg(target_endian = "big")]
    {
        for i in 0..pixel_count {
            let o = i * BPP;
            let b = bytes[o] as u32;
            let g = bytes[o + 1] as u32;
            let r = bytes[o + 2] as u32;
            let a = bytes[o + 3] as u32;
            words[i] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }

    Ok((width, height, words))
}

/// Method label for logs/CLI: "raw", "lz4", "sub+lz4".
pub fn method_name(compression: Compression, filter: Filter) -> &'static str {
    match (compression, filter) {
        (Compression::None, Filter::None) => "raw",
        (Compression::Lz4, Filter::None) => "lz4",
        (Compression::Lz4, Filter::Sub) => "sub+lz4",
        _ => "invalid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImageRgba8;

    fn rgba(w: u32, h: u32, pixels: Vec<u8>) -> ImageRgba8 {
        ImageRgba8 {
            width: w,
            height: h,
            pixels,
        }
    }

    fn solid(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) -> ImageRgba8 {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            pixels.extend_from_slice(&[r, g, b, a]);
        }
        rgba(w, h, pixels)
    }

    #[test]
    fn header_roundtrip_fields() {
        let img = solid(2, 2, 10, 20, 30, 255);
        let rep = encode(&img).unwrap();
        let h = parse_header(&rep.bytes).unwrap();
        assert_eq!(h.version, 2);
        assert_eq!(h.header_size, 36);
        assert_eq!(h.width, 2);
        assert_eq!(h.height, 2);
        assert_eq!(h.pixel_format, PIXEL_FORMAT_BGRA8);
        assert_eq!(h.alpha_mode, ALPHA_STRAIGHT);
        assert_eq!(h.flags & FLAG_CRC32, FLAG_CRC32);
    }

    #[test]
    fn raw_lz4_sub_lz4_pixel_roundtrip() {
        let mut pixels = Vec::new();
        // Gradient-ish with mixed alpha including 0/1/127/128/254/255.
        for y in 0..4u8 {
            for x in 0..4u8 {
                pixels.extend_from_slice(&[x.wrapping_mul(40), y.wrapping_mul(50), 200, 255]);
            }
        }
        // Overwrite first row alphas with the critical set.
        for (i, a) in [0u8, 1, 127, 128, 254, 255, 64, 192].iter().enumerate() {
            pixels[i * 4 + 3] = *a;
        }
        // Fully transparent but non-zero RGB (must survive).
        pixels[0] = 11;
        pixels[1] = 22;
        pixels[2] = 33;
        pixels[3] = 0;
        // Saturated channels
        pixels[4..8].copy_from_slice(&[255, 0, 0, 255]);
        pixels[8..12].copy_from_slice(&[0, 255, 0, 255]);
        pixels[12..16].copy_from_slice(&[0, 0, 255, 255]);

        let img = rgba(4, 4, pixels);
        for (c, f) in [
            (Compression::None, Filter::None),
            (Compression::Lz4, Filter::None),
            (Compression::Lz4, Filter::Sub),
        ] {
            let rep = encode_with_method(&img, c, f).unwrap();
            let dec = decode(&rep.bytes).unwrap();
            assert_eq!(dec, img, "method {:?}", (c, f));
        }
    }

    #[test]
    fn sub_resets_each_row() {
        // Two rows: second row starts with same first pixel as first row end would
        // wrongly predict if filter did not reset.
        let img = rgba(
            3,
            2,
            vec![
                10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255, // row0
                10, 0, 0, 255, 50, 0, 0, 255, 90, 0, 0, 255, // row1
            ],
        );
        let rep = encode_with_method(&img, Compression::Lz4, Filter::Sub).unwrap();
        assert_eq!(decode(&rep.bytes).unwrap(), img);
    }

    #[test]
    fn one_by_one_and_wide_row() {
        let a = solid(1, 1, 1, 2, 3, 4);
        let b = solid(5, 1, 9, 8, 7, 6);
        let c = solid(1, 5, 4, 5, 6, 7);
        for img in [&a, &b, &c] {
            let rep = encode(img).unwrap();
            assert_eq!(decode(&rep.bytes).unwrap(), *img);
        }
    }

    #[test]
    fn incompressible_prefers_raw_when_smaller() {
        // High-entropy bytes: LZ4 rarely helps on tiny randomish data with header overhead.
        let mut pixels = Vec::new();
        for i in 0..64u32 {
            let v = ((i.wrapping_mul(1103515245).wrapping_add(12345)) >> 16) as u8;
            pixels.extend_from_slice(&[v, v ^ 0xA5, v.wrapping_add(3), 255]);
        }
        let img = rgba(8, 8, pixels);
        let rep = encode(&img).unwrap();
        // Either raw or compressed is fine; verify lossless and that raw candidate size
        // is considered (file size never exceeds raw file size).
        let raw = encode_with_method(&img, Compression::None, Filter::None).unwrap();
        assert!(rep.file_size <= raw.file_size);
        assert_eq!(decode(&rep.bytes).unwrap(), img);
    }

    #[test]
    fn flat_color_compresses() {
        let img = solid(32, 32, 40, 80, 120, 255);
        let raw = encode_with_method(&img, Compression::None, Filter::None).unwrap();
        let rep = encode(&img).unwrap();
        assert!(rep.file_size < raw.file_size);
        assert_ne!(rep.compression, Compression::None);
        assert_eq!(decode(&rep.bytes).unwrap(), img);
    }

    #[test]
    fn rejects_truncated_header_and_payload() {
        let img = solid(2, 2, 1, 2, 3, 255);
        let rep = encode_with_method(&img, Compression::None, Filter::None).unwrap();
        assert!(matches!(
            decode(&rep.bytes[..10]),
            Err(ImageError::TruncatedInput | ImageError::UnsupportedFormat)
        ));
        let mut short = rep.bytes.clone();
        short.truncate(HEADER_SIZE_USIZE + 4);
        assert_eq!(decode(&short), Err(ImageError::TruncatedInput));
    }

    #[test]
    fn rejects_unknown_version_format_alpha_combo() {
        let img = solid(1, 1, 0, 0, 0, 255);
        let mut bytes = encode_with_method(&img, Compression::None, Filter::None)
            .unwrap()
            .bytes;
        bytes[4] = 99;
        assert_eq!(decode(&bytes), Err(ImageError::UnsupportedFormat));

        let mut bytes = encode_with_method(&img, Compression::None, Filter::None)
            .unwrap()
            .bytes;
        bytes[20] = 99;
        assert_eq!(decode(&bytes), Err(ImageError::UnsupportedBitDepth));

        let mut bytes = encode_with_method(&img, Compression::None, Filter::None)
            .unwrap()
            .bytes;
        bytes[21] = 2;
        assert_eq!(decode(&bytes), Err(ImageError::InvalidHeader));

        let mut bytes = encode_with_method(&img, Compression::None, Filter::None)
            .unwrap()
            .bytes;
        bytes[22] = COMPRESSION_NONE;
        bytes[23] = FILTER_SUB;
        assert_eq!(decode(&bytes), Err(ImageError::UnsupportedCompression));
    }

    #[test]
    fn rejects_overflow_and_size_mismatch() {
        let img = solid(1, 1, 0, 0, 0, 255);
        let mut bytes = encode_with_method(&img, Compression::None, Filter::None)
            .unwrap()
            .bytes;
        // width = 0xFFFFFFFF, height = 0xFFFFFFFF
        bytes[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        bytes[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert_eq!(decode(&bytes), Err(ImageError::InvalidDimensions));

        let mut bytes = encode_with_method(&img, Compression::None, Filter::None)
            .unwrap()
            .bytes;
        bytes[24..28].copy_from_slice(&999u32.to_le_bytes());
        assert_eq!(decode(&bytes), Err(ImageError::InvalidDimensions));
    }

    #[test]
    fn rejects_corrupt_lz4() {
        let img = solid(8, 8, 7, 8, 9, 255);
        let mut bytes = encode_with_method(&img, Compression::Lz4, Filter::None)
            .unwrap()
            .bytes;
        // Flip payload mid-stream.
        let mid = HEADER_SIZE_USIZE + 2;
        if mid < bytes.len() {
            bytes[mid] ^= 0xFF;
        }
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn magic_not_confused_with_tga() {
        // Valid tiny TGA should not be detected as SIMG.
        let tga = crate::encode_tga_rgba32(&solid(1, 1, 255, 0, 0, 255)).unwrap();
        assert!(!is_simg_v2(&tga));
        assert!(is_simg_v2(&encode(&solid(1, 1, 1, 1, 1, 1)).unwrap().bytes));
    }

    #[test]
    fn deterministic_encode() {
        let img = solid(3, 3, 12, 34, 56, 78);
        let a = encode(&img).unwrap().bytes;
        let b = encode(&img).unwrap().bytes;
        assert_eq!(a, b);
    }
}
