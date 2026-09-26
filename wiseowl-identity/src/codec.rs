use sha2::{Digest, Sha256};

use crate::validation::IdentityError;

pub(crate) fn sha256(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().into()
}

pub(crate) fn require_exact(bytes: &[u8], expected: usize) -> Result<(), IdentityError> {
    if bytes.len() < expected {
        Err(IdentityError::InvalidLength)
    } else if bytes.len() > expected {
        Err(IdentityError::TrailingData)
    } else {
        Ok(())
    }
}

pub(crate) fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

pub(crate) fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("bounded codec"))
}

pub(crate) fn short_hex(bytes: &[u8]) -> [u8; 8] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = [0u8; 8];
    for (index, byte) in bytes.iter().take(4).copied().enumerate() {
        out[index * 2] = HEX[(byte >> 4) as usize];
        out[index * 2 + 1] = HEX[(byte & 0x0F) as usize];
    }
    out
}
