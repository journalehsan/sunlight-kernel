//! On-disk record format and (de)serialization helpers.
//! Binary layout must be followed exactly:
//!
//! [ RecordHeader (24 bytes LE) ]
//! [ key_bytes (key_len) ]
//! [ value_bytes (value_len) ]
//! [ acl_bytes (acl_len) ]
//!
//! CRC32 is computed over key+value+acl only.

use std::io::{self, Read, Write};

/// Magic number identifying a valid record.
pub const RECORD_MAGIC: u32 = 0xABCD1234;

/// Record format version.
pub const RECORD_VERSION: u16 = 1;

/// Flag values for the `flags` field.
pub const FLAG_PUT: u16 = 1;
pub const FLAG_DELETE: u16 = 2;

/// Fixed-size header written before key/value/acl.
#[derive(Debug, Clone, Copy)]
pub struct RecordHeader {
    pub magic: u32,
    pub version: u16,
    pub flags: u16,
    pub key_len: u32,
    pub value_len: u32,
    pub acl_len: u32,
    pub crc32: u32,
}

impl RecordHeader {
    pub const SIZE: usize = 24; // 4+2+2+4+4+4+4

    /// Serialize header to exactly 24 little-endian bytes.
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.flags.to_le_bytes());
        buf[8..12].copy_from_slice(&self.key_len.to_le_bytes());
        buf[12..16].copy_from_slice(&self.value_len.to_le_bytes());
        buf[16..20].copy_from_slice(&self.acl_len.to_le_bytes());
        buf[20..24].copy_from_slice(&self.crc32.to_le_bytes());
        buf
    }

    /// Deserialize header from exactly 24 bytes. Returns None on invalid magic/version.
    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Option<Self> {
        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if magic != RECORD_MAGIC || version != RECORD_VERSION {
            return None;
        }
        Some(Self {
            magic,
            version,
            flags: u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
            key_len: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            value_len: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            acl_len: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            crc32: u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        })
    }
}

/// Compute CRC32 over the provided payload (key || value || acl).
pub fn compute_crc(payload: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(payload);
    hasher.finalize()
}

/// Write a complete record (header + payloads) to `w`.
/// Returns the number of bytes written.
pub fn write_record<W: Write>(
    w: &mut W,
    flags: u16,
    key: &[u8],
    value: &[u8],
    acl_bytes: &[u8],
) -> io::Result<u32> {
    let payload: Vec<u8> = key
        .iter()
        .chain(value.iter())
        .chain(acl_bytes.iter())
        .copied()
        .collect();
    let crc = compute_crc(&payload);

    let header = RecordHeader {
        magic: RECORD_MAGIC,
        version: RECORD_VERSION,
        flags,
        key_len: key.len() as u32,
        value_len: value.len() as u32,
        acl_len: acl_bytes.len() as u32,
        crc32: crc,
    };

    let header_bytes = header.to_bytes();
    w.write_all(&header_bytes)?;
    w.write_all(key)?;
    w.write_all(value)?;
    w.write_all(acl_bytes)?;

    Ok(RecordHeader::SIZE as u32 + header.key_len + header.value_len + header.acl_len)
}

/// Attempt to read exactly one record from `r` at the current position.
/// On success returns (header, key, value, acl_bytes).
/// On EOF at start returns None.
/// Any other truncation/invalid data returns an error (caller decides to stop recovery).
pub fn read_record<R: Read>(r: &mut R) -> io::Result<Option<(RecordHeader, Vec<u8>, Vec<u8>, Vec<u8>)>> {
    let mut header_buf = [0u8; RecordHeader::SIZE];
    match r.read_exact(&mut header_buf) {
        Ok(()) => {}
        Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let header = match RecordHeader::from_bytes(&header_buf) {
        Some(h) => h,
        None => {
            // Invalid magic or version: signal corruption to caller (do not treat as normal EOF).
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid record magic or version",
            ));
        }
    };

    let mut key = vec![0u8; header.key_len as usize];
    let mut value = vec![0u8; header.value_len as usize];
    let mut acl = vec![0u8; header.acl_len as usize];

    r.read_exact(&mut key)?;
    r.read_exact(&mut value)?;
    r.read_exact(&mut acl)?;

    let mut payload = Vec::with_capacity(key.len() + value.len() + acl.len());
    payload.extend_from_slice(&key);
    payload.extend_from_slice(&value);
    payload.extend_from_slice(&acl);

    let actual_crc = compute_crc(&payload);
    if actual_crc != header.crc32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "crc32 mismatch",
        ));
    }

    Ok(Some((header, key, value, acl)))
}
