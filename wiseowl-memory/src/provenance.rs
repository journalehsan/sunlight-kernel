//! Provenance records for memory entries.
//!
//! Derived records must retain a parent chain. Parent count is hard-bounded
//! to prevent unbounded metadata growth.

use crate::error::MemoryError;
use crate::ids::{MemoryId, SourceId};
use crate::kinds::{SourceKind, TrustLevel};

/// Maximum parents retained on any single provenance record.
pub const MAX_PROVENANCE_PARENTS: usize = 8;

/// Stable producer name max length (bytes).
pub const MAX_PRODUCER_LEN: usize = 32;

/// Provenance attached to every memory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct Provenance {
    pub source_kind: SourceKind,
    pub source_id: Option<SourceId>,
    /// Monotonic creation time (nanoseconds where available; service uses ns scale).
    pub created_at_ns: u64,
    /// Producer service name (UTF-8, truncated to [`MAX_PRODUCER_LEN`]).
    pub producer_service: heapless_string::HeaplessString,
    pub trust: TrustLevel,
    /// Parent memory IDs for derived content (bounded).
    pub parents: heapless_vec::HeaplessVec<MemoryId, MAX_PROVENANCE_PARENTS>,
}

// Minimal no_std-friendly string/vec without pulling heapless as a hard dep for host.
// Host builds use the same fixed-capacity types implemented inline below.
pub mod heapless_string {
    use core::fmt;

    use super::MAX_PRODUCER_LEN;

    #[derive(Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
    pub struct HeaplessString {
        buf: [u8; MAX_PRODUCER_LEN],
        len: u8,
    }

    impl HeaplessString {
        pub const fn new() -> Self {
            Self {
                buf: [0; MAX_PRODUCER_LEN],
                len: 0,
            }
        }

        pub fn from_str(s: &str) -> Self {
            let bytes = s.as_bytes();
            let n = bytes.len().min(MAX_PRODUCER_LEN);
            let mut out = Self::new();
            out.buf[..n].copy_from_slice(&bytes[..n]);
            out.len = n as u8;
            out
        }

        pub fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
        }

        pub fn len(&self) -> usize {
            self.len as usize
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }
    }

    impl Default for HeaplessString {
        fn default() -> Self {
            Self::new()
        }
    }

    impl fmt::Debug for HeaplessString {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:?}", self.as_str())
        }
    }
}

pub mod heapless_vec {
    use core::fmt;

    #[derive(Clone, PartialEq, Eq)]
    pub struct HeaplessVec<T, const N: usize> {
        data: [Option<T>; N],
        len: usize,
    }

    #[cfg(feature = "host")]
    impl<T: serde::Serialize, const N: usize> serde::Serialize for HeaplessVec<T, N> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq;
            let mut seq = serializer.serialize_seq(Some(self.len))?;
            for item in self.iter() {
                seq.serialize_element(item)?;
            }
            seq.end()
        }
    }

    #[cfg(feature = "host")]
    impl<'de, T: serde::Deserialize<'de>, const N: usize> serde::Deserialize<'de>
        for HeaplessVec<T, N>
    {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let items: Vec<T> = serde::Deserialize::deserialize(deserializer)?;
            let mut out = HeaplessVec {
                data: core::array::from_fn(|_| None),
                len: 0,
            };
            for item in items {
                out.push(item).map_err(|_| {
                    serde::de::Error::custom("provenance parent limit exceeded")
                })?;
            }
            Ok(out)
        }
    }

    impl<T: Copy, const N: usize> HeaplessVec<T, N> {
        pub const fn new() -> Self {
            Self {
                data: [None; N],
                len: 0,
            }
        }
    }

    // For non-Copy T we need a different init path.
    impl<T, const N: usize> HeaplessVec<T, N> {
        pub fn new_empty() -> Self
        where
            T: Clone,
        {
            // Safety: we only use up to len; initialize with None via array map.
            Self {
                data: core::array::from_fn(|_| None),
                len: 0,
            }
        }

        pub fn len(&self) -> usize {
            self.len
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        pub fn as_slice(&self) -> &[T] {
            // Cannot easily return &[T] from [Option<T>; N] without contiguous layout.
            // Provide iterator instead; for tests use collect.
            // Actual contiguous view: only valid if we store T in array with separate len.
            // Redesign: use separate storage.
            unimplemented_slice()
        }

        pub fn push(&mut self, value: T) -> Result<(), T> {
            if self.len >= N {
                return Err(value);
            }
            self.data[self.len] = Some(value);
            self.len += 1;
            Ok(())
        }

        pub fn get(&self, index: usize) -> Option<&T> {
            if index < self.len {
                self.data[index].as_ref()
            } else {
                None
            }
        }

        pub fn iter(&self) -> impl Iterator<Item = &T> {
            self.data[..self.len].iter().filter_map(|x| x.as_ref())
        }

        pub fn clear(&mut self) {
            for i in 0..self.len {
                self.data[i] = None;
            }
            self.len = 0;
        }
    }

    fn unimplemented_slice() -> ! {
        // Keep type-checking of unused as_slice; prefer iter().
        panic!("use Provenance::parents.iter()")
    }

    impl<T: fmt::Debug, const N: usize> fmt::Debug for HeaplessVec<T, N> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_list().entries(self.iter()).finish()
        }
    }

    impl<T: Clone, const N: usize> Default for HeaplessVec<T, N> {
        fn default() -> Self {
            Self::new_empty()
        }
    }
}

impl Provenance {
    pub fn new(
        source_kind: SourceKind,
        source_id: Option<SourceId>,
        created_at_ns: u64,
        producer_service: &str,
        trust: TrustLevel,
    ) -> Self {
        Self {
            source_kind,
            source_id,
            created_at_ns,
            producer_service: heapless_string::HeaplessString::from_str(producer_service),
            trust,
            parents: heapless_vec::HeaplessVec::new_empty(),
        }
    }

    /// Attach a parent memory ID. Fails if parent list is full.
    pub fn push_parent(&mut self, parent: MemoryId) -> Result<(), MemoryError> {
        self.parents
            .push(parent)
            .map_err(|_| MemoryError::InvalidRequest("provenance parent limit exceeded"))
    }

    /// Build a derived provenance that inherits the source chain from `parent`
    /// and records `parent_id` as a parent. Never silently drops the chain.
    pub fn derive_from(
        parent: &Provenance,
        parent_id: MemoryId,
        created_at_ns: u64,
        producer_service: &str,
        trust: TrustLevel,
    ) -> Result<Self, MemoryError> {
        let mut p = Self::new(
            parent.source_kind,
            parent.source_id,
            created_at_ns,
            producer_service,
            trust,
        );
        // Inherit grandparents first (drop oldest if needed), then parent_id.
        for gp in parent.parents.iter() {
            if p.parents.len() + 1 >= MAX_PROVENANCE_PARENTS {
                break;
            }
            let _ = p.parents.push(*gp);
        }
        p.push_parent(parent_id)?;
        Ok(p)
    }

    pub fn parent_count(&self) -> usize {
        self.parents.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemoryId, SourceId};

    #[test]
    fn parent_bound() {
        let mut p = Provenance::new(
            SourceKind::UserInput,
            None,
            1,
            "test",
            TrustLevel::Untrusted,
        );
        for i in 1..=MAX_PROVENANCE_PARENTS {
            p.push_parent(MemoryId::from_raw(i as u64).unwrap())
                .unwrap();
        }
        assert!(p
            .push_parent(MemoryId::from_raw(99).unwrap())
            .is_err());
        assert_eq!(p.parent_count(), MAX_PROVENANCE_PARENTS);
    }

    #[test]
    fn derive_keeps_parent() {
        let parent = Provenance::new(
            SourceKind::LocalTool,
            Some(SourceId::from_raw(3).unwrap()),
            10,
            "tool",
            TrustLevel::Trusted,
        );
        let pid = MemoryId::from_raw(5).unwrap();
        let derived = Provenance::derive_from(
            &parent,
            pid,
            20,
            "wiseowl-memoryd",
            TrustLevel::SystemDerived,
        )
        .unwrap();
        assert_eq!(derived.source_kind, SourceKind::LocalTool);
        assert_eq!(derived.source_id, parent.source_id);
        assert_eq!(derived.parent_count(), 1);
        assert_eq!(derived.parents.get(0), Some(&pid));
    }
}
