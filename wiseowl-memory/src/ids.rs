//! Typed identifiers for Wise Owl short-term memory.
//!
//! # Generation strategy
//!
//! Each identifier is a non-zero `u64` allocated by the service via a
//! per-kind monotonic counter that starts at 1 on every process start.
//! Counters never wrap in practice (u64 range); if they would, allocation
//! fails rather than reusing a live ID.
//!
//! Identifiers:
//! - are never process pointers or addresses
//! - survive little-endian serialization as raw `u64`
//! - reject zero / malformed external values at parse boundaries
//! - are not reused for new objects during the active service lifetime

use core::fmt;
use core::num::NonZeroU64;

/// Error when constructing or parsing an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdError {
    /// Zero is reserved and never a valid object id.
    Zero,
    /// Counter would wrap; service must refuse new allocations.
    Exhausted,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => write!(f, "identifier must be non-zero"),
            Self::Exhausted => write!(f, "identifier space exhausted"),
        }
    }
}

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name(NonZeroU64);

        impl $name {
            /// Construct from a validated non-zero raw value.
            pub const fn from_raw(raw: u64) -> Result<Self, IdError> {
                match NonZeroU64::new(raw) {
                    Some(nz) => Ok(Self(nz)),
                    None => Err(IdError::Zero),
                }
            }

            /// Infallible constructor for service-internal known-good values.
            ///
            /// # Panics
            /// Panics only if `raw == 0` — never call with untrusted input.
            pub const fn from_raw_unchecked(raw: u64) -> Self {
                // Const path: NonZeroU64::new is not const-stable for unwrap in all
                // editions; use match for clarity.
                match NonZeroU64::new(raw) {
                    Some(nz) => Self(nz),
                    None => panic!("zero id"),
                }
            }

            /// Raw numeric value for wire/disk encoding.
            pub const fn get(self) -> u64 {
                self.0.get()
            }

            /// Little-endian 8-byte encoding.
            pub fn to_le_bytes(self) -> [u8; 8] {
                self.get().to_le_bytes()
            }

            /// Decode from little-endian bytes; rejects zero.
            pub fn from_le_bytes(bytes: [u8; 8]) -> Result<Self, IdError> {
                Self::from_raw(u64::from_le_bytes(bytes))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.get())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.get())
            }
        }
    };
}

define_id!(
    /// Unique memory entry identity for the active service lifetime.
    MemoryId
);
define_id!(
    /// Cognitive session grouping many memory entries.
    SessionId
);
define_id!(
    /// Provenance source identity (user, tool, local service, …).
    SourceId
);
define_id!(
    /// Optional episode grouping within a session.
    EpisodeId
);
define_id!(
    /// Sealed segment of immutable cold records.
    SegmentId
);
define_id!(
    /// Opaque token-stream identity (tokenizer is out of scope for Phase 0/1).
    TokenStreamId
);
define_id!(
    /// Client connection / peer identity for ownership and cleanup.
    ClientId
);

/// Monotonic non-zero ID allocator used by the service.
#[derive(Debug)]
pub struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    pub fn alloc_raw(&mut self) -> Result<u64, IdError> {
        let id = self.next;
        if id == 0 {
            return Err(IdError::Exhausted);
        }
        self.next = match id.checked_add(1) {
            Some(n) => n,
            None => return Err(IdError::Exhausted),
        };
        Ok(id)
    }

    pub fn alloc_memory(&mut self) -> Result<MemoryId, IdError> {
        MemoryId::from_raw(self.alloc_raw()?)
    }

    pub fn alloc_session(&mut self) -> Result<SessionId, IdError> {
        SessionId::from_raw(self.alloc_raw()?)
    }

    pub fn alloc_segment(&mut self) -> Result<SegmentId, IdError> {
        SegmentId::from_raw(self.alloc_raw()?)
    }

    pub fn alloc_client(&mut self) -> Result<ClientId, IdError> {
        ClientId::from_raw(self.alloc_raw()?)
    }

    pub fn alloc_source(&mut self) -> Result<SourceId, IdError> {
        SourceId::from_raw(self.alloc_raw()?)
    }

    pub fn peek_next(&self) -> u64 {
        self.next
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero() {
        assert_eq!(MemoryId::from_raw(0), Err(IdError::Zero));
        assert_eq!(SessionId::from_le_bytes([0; 8]), Err(IdError::Zero));
    }

    #[test]
    fn roundtrip_le() {
        let id = MemoryId::from_raw(42).unwrap();
        assert_eq!(MemoryId::from_le_bytes(id.to_le_bytes()).unwrap(), id);
    }

    #[test]
    fn allocator_monotonic_no_reuse() {
        let mut a = IdAllocator::new();
        let a1 = a.alloc_memory().unwrap().get();
        let a2 = a.alloc_memory().unwrap().get();
        assert!(a2 > a1);
        assert_ne!(a1, 0);
    }

    #[test]
    fn parse_display() {
        let id = SessionId::from_raw(7).unwrap();
        assert_eq!(format!("{id}"), "7");
    }
}
