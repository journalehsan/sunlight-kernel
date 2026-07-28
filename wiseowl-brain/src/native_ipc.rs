#![allow(dead_code)]

use crate::error::BrainError;

pub const NATIVE_PROTOCOL_VERSION: u16 = 1;
pub const MAX_REQUEST_BODY: u32 = 64 * 1024;
pub const MAX_REPLY_BODY: u32 = 64 * 1024;
/// Max payload that may be packed into a single SHM page after the BrainIpcHeader.
pub const INLINE_PAYLOAD_THRESHOLD: u32 = 3072;
/// Register IPC ABI only carries `words[0..4]` (see sunlight_ipc). With
/// `words[0] = body_len`, at most 3×8 = 24 bytes of body can travel inline.
/// Anything larger **must** use SHM (cap0 + BrainIpcHeader).
pub const REG_INLINE_BODY_MAX: usize = 24;
/// Max `word_count` accepted by the kernel register transport.
pub const IPC_REG_WORDS: u32 = 4;
pub const SHM_PAGE_SIZE: u32 = 4096;
pub const BRAIN_IPC_HEADER_LEN: usize = 24;
pub const REQUIRED_FLAGS_MASK: u32 = 0xFFFF_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct BrainIpcHeader {
    pub protocol_version: u16,
    pub operation: u16,
    pub flags: u32,
    pub request_id: u64,
    pub body_len: u32,
    pub reserved: u32,
}

impl BrainIpcHeader {
    pub fn encode(&self) -> [u8; BRAIN_IPC_HEADER_LEN] {
        let mut out = [0u8; BRAIN_IPC_HEADER_LEN];
        out[0..2].copy_from_slice(&self.protocol_version.to_le_bytes());
        out[2..4].copy_from_slice(&self.operation.to_le_bytes());
        out[4..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..16].copy_from_slice(&self.request_id.to_le_bytes());
        out[16..20].copy_from_slice(&self.body_len.to_le_bytes());
        out[20..24].copy_from_slice(&self.reserved.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, BrainError> {
        if bytes.len() < BRAIN_IPC_HEADER_LEN {
            return Err(BrainError::TruncatedHeader);
        }
        let protocol_version = u16::from_le_bytes([bytes[0], bytes[1]]);
        let operation = u16::from_le_bytes([bytes[2], bytes[3]]);
        let flags = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let request_id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let body_len = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let reserved = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let h = Self {
            protocol_version,
            operation,
            flags,
            request_id,
            body_len,
            reserved,
        };
        h.validate()?;
        Ok(h)
    }

    pub fn validate(&self) -> Result<(), BrainError> {
        if self.protocol_version != NATIVE_PROTOCOL_VERSION {
            return Err(BrainError::UnsupportedProtocolVersion {
                got: self.protocol_version,
                want: NATIVE_PROTOCOL_VERSION,
            });
        }
        if self.flags & REQUIRED_FLAGS_MASK != 0 {
            return Err(BrainError::InvalidRequest("unknown required flags"));
        }
        if self.body_len > MAX_REQUEST_BODY {
            return Err(BrainError::PayloadTooLarge {
                size: self.body_len,
                max: MAX_REQUEST_BODY,
            });
        }
        Ok(())
    }
}

/// Service endpoint name registered with nameserver.
pub const BRAIN_ENDPOINT: &str = "wiseowl.brain.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum BrainOp {
    Greeting = 0xB001,
    Summary = 0xB002,
    Suggestion = 0xB003,
    Context = 0xB004,
    /// Get own preferences (body: none; subject from request uid).
    PreferencesGet = 0xB010,
    /// Set one preference field (inline words: field code + value).
    PreferencesSet = 0xB011,
    /// Explicit Welcome completion notification (not inferred from greeting).
    WelcomeCompleted = 0xB012,
    Health = 0xB00E,
    Stats = 0xB00F,
    ConsoleUi = 0xB020,
    Reply = 0xBF80,
    Error = 0xBFFF,
}

impl BrainOp {
    pub const fn from_u16(v: u16) -> Option<Self> {
        match v {
            0xB001 => Some(Self::Greeting),
            0xB002 => Some(Self::Summary),
            0xB003 => Some(Self::Suggestion),
            0xB004 => Some(Self::Context),
            0xB010 => Some(Self::PreferencesGet),
            0xB011 => Some(Self::PreferencesSet),
            0xB012 => Some(Self::WelcomeCompleted),
            0xB00E => Some(Self::Health),
            0xB00F => Some(Self::Stats),
            0xB020 => Some(Self::ConsoleUi),
            0xBF80 => Some(Self::Reply),
            0xBFFF => Some(Self::Error),
            _ => None,
        }
    }

    pub const fn as_u16(self) -> u16 {
        self as u16
    }

    pub const fn label(self) -> u64 {
        self as u16 as u64
    }
}

pub fn encode_error_body(code: u32, request_id: u64) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&code.to_le_bytes());
    out[4..12].copy_from_slice(&request_id.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = BrainIpcHeader {
            protocol_version: NATIVE_PROTOCOL_VERSION,
            operation: BrainOp::Greeting.as_u16(),
            flags: 0,
            request_id: 42,
            body_len: 16,
            reserved: 0,
        };
        let enc = h.encode();
        let dec = BrainIpcHeader::decode(&enc).unwrap();
        assert_eq!(dec, h);
    }

    #[test]
    fn rejects_unknown_version() {
        let h = BrainIpcHeader {
            protocol_version: 99,
            operation: 1,
            flags: 0,
            request_id: 1,
            body_len: 0,
            reserved: 0,
        };
        assert!(h.validate().is_err());
    }

    #[test]
    fn rejects_unknown_required_flags() {
        let h = BrainIpcHeader {
            protocol_version: NATIVE_PROTOCOL_VERSION,
            operation: 1,
            flags: 0x0001_0000,
            request_id: 1,
            body_len: 0,
            reserved: 0,
        };
        assert!(h.validate().is_err());
    }

    #[test]
    fn inline_threshold_fits_page() {
        assert!((BRAIN_IPC_HEADER_LEN as u32 + INLINE_PAYLOAD_THRESHOLD) <= SHM_PAGE_SIZE);
    }

    #[test]
    fn reg_inline_body_fits_register_words() {
        // words[0]=len + words[1..3]=body bytes; max 3 words of body.
        assert_eq!(REG_INLINE_BODY_MAX, 24);
        assert!(REG_INLINE_BODY_MAX <= (IPC_REG_WORDS as usize - 1) * 8);
    }

    #[test]
    fn op_code_ranges_distinct() {
        let greeting = BrainOp::Greeting.as_u16();
        let health = BrainOp::Health.as_u16();
        let reply = BrainOp::Reply.as_u16();
        let error = BrainOp::Error.as_u16();
        assert_ne!(greeting, health);
        assert_ne!(greeting, reply);
        assert_ne!(greeting, error);
        assert_ne!(health, reply);
        assert_ne!(health, error);
        assert_ne!(reply, error);
        assert!(greeting < 0xB010);
        assert!(health < 0xB020);
        assert!(reply > 0xBF00);
        assert_eq!(error, 0xBFFF);
    }
}
