//! Typed identifiers for Wise Owl short-term memory.
//!
//! # Restart-safe generation strategy (Phase 1.1)
//!
//! Each identifier is a non-zero `u64` with an explicit bit partition:
//!
//! ```text
//! bits 63..48 (16): service generation  (1..=65535)
//! bits 47..0  (48): monotonic counter   (1..=2^48-1 within a generation)
//! ```
//!
//! Generation is advanced on every daemon start (persisted when a generation
//! store is available) so a new process never reissues IDs that a prior
//! instance could have created. Within a generation the counter is monotonic
//! and advanced past any recovered cold-record IDs.
//!
//! Guarantees:
//! - zero remains invalid
//! - wrap of the counter is detected and refuses allocation
//! - generation 0 is reserved/invalid in packed form
//! - recovered IDs call [`IdAllocator::note_seen`] so the counter advances
//! - two sequential daemon generations never collide on newly issued IDs

use core::fmt;
use core::num::NonZeroU64;

/// Bits reserved for the per-daemon generation.
pub const GENERATION_BITS: u32 = 16;
/// Bits reserved for the monotonic counter within a generation.
pub const COUNTER_BITS: u32 = 48;
/// Maximum generation value (16-bit).
pub const MAX_GENERATION: u16 = u16::MAX;
/// Maximum counter within a generation (48-bit).
pub const MAX_COUNTER: u64 = (1u64 << COUNTER_BITS) - 1;
/// Shift applied to the generation field.
pub const GENERATION_SHIFT: u32 = COUNTER_BITS;

/// Error when constructing or parsing an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdError {
    /// Zero is reserved and never a valid object id.
    Zero,
    /// Counter or generation would wrap; service must refuse new allocations.
    Exhausted,
    /// Packed value has an invalid generation field.
    InvalidGeneration,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => write!(f, "identifier must be non-zero"),
            Self::Exhausted => write!(f, "identifier space exhausted"),
            Self::InvalidGeneration => write!(f, "identifier generation is zero/invalid"),
        }
    }
}

/// Pack generation and counter into a raw u64. Generation and counter must be non-zero.
pub const fn pack_id(generation: u16, counter: u64) -> Result<u64, IdError> {
    if generation == 0 {
        return Err(IdError::InvalidGeneration);
    }
    if counter == 0 || counter > MAX_COUNTER {
        return Err(IdError::Exhausted);
    }
    Ok(((generation as u64) << GENERATION_SHIFT) | counter)
}

/// Unpack a raw id into (generation, counter). Rejects zero.
pub const fn unpack_id(raw: u64) -> Result<(u16, u64), IdError> {
    if raw == 0 {
        return Err(IdError::Zero);
    }
    let generation = (raw >> GENERATION_SHIFT) as u16;
    let counter = raw & MAX_COUNTER;
    if generation == 0 || counter == 0 {
        return Err(IdError::InvalidGeneration);
    }
    Ok((generation, counter))
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
                    Some(nz) => {
                        // Reject generation 0 / counter 0 packing (except legacy plain counters
                        // are only accepted via from_raw for wire parse of historical tests when
                        // high bits are zero — Phase 1.1 requires generation != 0 for *new* IDs;
                        // from_raw still accepts any non-zero for reading persisted values that
                        // predate partitioning so recovery can quarantine or note them).
                        Ok(Self(nz))
                    }
                    None => Err(IdError::Zero),
                }
            }

            /// Infallible constructor for service-internal known-good values.
            ///
            /// # Panics
            /// Panics only if `raw == 0` — never call with untrusted input.
            pub const fn from_raw_unchecked(raw: u64) -> Self {
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

            /// Generation field (0 if pre-partition legacy value with high bits clear).
            pub const fn generation(self) -> u16 {
                (self.get() >> GENERATION_SHIFT) as u16
            }

            /// Counter field (low 48 bits).
            pub const fn counter(self) -> u64 {
                self.get() & MAX_COUNTER
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    concat!(stringify!($name), "(g={},c={})"),
                    self.generation(),
                    self.counter()
                )
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

/// Restart-safe monotonic non-zero ID allocator used by the service.
#[derive(Debug, Clone)]
pub struct IdAllocator {
    /// Current service generation (1..=MAX_GENERATION).
    generation: u16,
    /// Next counter to issue within this generation (1..=MAX_COUNTER).
    next_counter: u64,
}

impl IdAllocator {
    /// Create an allocator for a fresh generation starting at counter 1.
    pub const fn new() -> Self {
        Self {
            generation: 1,
            next_counter: 1,
        }
    }

    /// Create with an explicit generation (must be non-zero).
    pub const fn with_generation(generation: u16) -> Result<Self, IdError> {
        if generation == 0 {
            return Err(IdError::InvalidGeneration);
        }
        Ok(Self {
            generation,
            next_counter: 1,
        })
    }

    /// Current generation.
    pub const fn generation(&self) -> u16 {
        self.generation
    }

    /// Next counter that would be issued (for diagnostics).
    pub const fn peek_next_counter(&self) -> u64 {
        self.next_counter
    }

    /// Legacy alias used by older call sites / tests.
    pub fn peek_next(&self) -> u64 {
        pack_id(self.generation, self.next_counter).unwrap_or(0)
    }

    /// Advance generation for a daemon restart. Returns error if generation wraps.
    ///
    /// Counter resets to 1. Call [`note_seen`] after recovery for any IDs that
    /// share the new generation (should not happen if generation always bumps).
    pub fn bump_generation(&mut self) -> Result<u16, IdError> {
        let next = self
            .generation
            .checked_add(1)
            .filter(|&g| g != 0)
            .ok_or(IdError::Exhausted)?;
        self.generation = next;
        self.next_counter = 1;
        Ok(next)
    }

    /// Set generation explicitly (e.g. loaded from persistent store + 1).
    pub fn set_generation(&mut self, generation: u16) -> Result<(), IdError> {
        if generation == 0 {
            return Err(IdError::InvalidGeneration);
        }
        self.generation = generation;
        self.next_counter = 1;
        Ok(())
    }

    /// Note a recovered or externally observed ID so the allocator never
    /// reissues it within the same generation. IDs from other generations are
    /// recorded only for max-tracking of same-generation counters.
    pub fn note_seen(&mut self, raw: u64) {
        if raw == 0 {
            return;
        }
        let generation = (raw >> GENERATION_SHIFT) as u16;
        let counter = raw & MAX_COUNTER;
        if generation == self.generation && counter >= self.next_counter {
            // Advance past the seen counter.
            self.next_counter = counter.saturating_add(1);
            if self.next_counter == 0 || self.next_counter > MAX_COUNTER {
                // Mark exhausted by parking at MAX+1 sentinel via MAX_COUNTER+1 clamp.
                self.next_counter = MAX_COUNTER.saturating_add(1);
            }
        }
        // Also accept legacy plain counters (generation field 0): treat low 48
        // bits as counter under current generation so recovery cannot collide
        // with a reissued packed ID that reuses that counter.
        if generation == 0 && counter > 0 && counter >= self.next_counter {
            self.next_counter = counter.saturating_add(1);
        }
    }

    /// Note a typed id.
    pub fn note_seen_id(&mut self, id: impl Into<u64>) {
        self.note_seen(id.into());
    }

    fn alloc_raw(&mut self) -> Result<u64, IdError> {
        if self.generation == 0 {
            return Err(IdError::InvalidGeneration);
        }
        if self.next_counter == 0 || self.next_counter > MAX_COUNTER {
            return Err(IdError::Exhausted);
        }
        let counter = self.next_counter;
        self.next_counter = match counter.checked_add(1) {
            Some(n) if n <= MAX_COUNTER + 1 => n,
            _ => return Err(IdError::Exhausted),
        };
        pack_id(self.generation, counter)
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
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

// Allow note_seen_id with typed wrappers.
impl From<MemoryId> for u64 {
    fn from(v: MemoryId) -> u64 {
        v.get()
    }
}
impl From<SessionId> for u64 {
    fn from(v: SessionId) -> u64 {
        v.get()
    }
}
impl From<SegmentId> for u64 {
    fn from(v: SegmentId) -> u64 {
        v.get()
    }
}
impl From<ClientId> for u64 {
    fn from(v: ClientId) -> u64 {
        v.get()
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
        let id = MemoryId::from_raw(pack_id(1, 42).unwrap()).unwrap();
        assert_eq!(MemoryId::from_le_bytes(id.to_le_bytes()).unwrap(), id);
    }

    #[test]
    fn allocator_monotonic_no_reuse() {
        let mut a = IdAllocator::new();
        let a1 = a.alloc_memory().unwrap().get();
        let a2 = a.alloc_memory().unwrap().get();
        assert!(a2 > a1);
        assert_ne!(a1, 0);
        assert_eq!(MemoryId::from_raw(a1).unwrap().generation(), 1);
    }

    #[test]
    fn parse_display() {
        let id = SessionId::from_raw(pack_id(1, 7).unwrap()).unwrap();
        assert_eq!(format!("{id}"), format!("{}", pack_id(1, 7).unwrap()));
    }

    #[test]
    fn generation_bump_avoids_collision() {
        let mut a = IdAllocator::new();
        let first = a.alloc_memory().unwrap();
        a.bump_generation().unwrap();
        let second = a.alloc_memory().unwrap();
        assert_ne!(first.get(), second.get());
        assert_eq!(first.generation(), 1);
        assert_eq!(second.generation(), 2);
        assert_eq!(second.counter(), 1);
    }

    #[test]
    fn note_seen_advances_counter() {
        let mut a = IdAllocator::with_generation(3).unwrap();
        let high = pack_id(3, 1000).unwrap();
        a.note_seen(high);
        let next = a.alloc_memory().unwrap();
        assert_eq!(next.counter(), 1001);
        assert_eq!(next.generation(), 3);
    }

    #[test]
    fn note_seen_other_generation_ignored_for_counter() {
        let mut a = IdAllocator::with_generation(5).unwrap();
        a.note_seen(pack_id(4, 9_000).unwrap());
        let next = a.alloc_memory().unwrap();
        assert_eq!(next.counter(), 1);
        assert_eq!(next.generation(), 5);
    }

    #[test]
    fn counter_exhaustion_detected() {
        let mut a = IdAllocator::with_generation(1).unwrap();
        a.next_counter = MAX_COUNTER;
        let last = a.alloc_memory().unwrap();
        assert_eq!(last.counter(), MAX_COUNTER);
        assert!(a.alloc_memory().is_err());
    }

    #[test]
    fn pack_rejects_zero_generation() {
        assert!(pack_id(0, 1).is_err());
    }
}
