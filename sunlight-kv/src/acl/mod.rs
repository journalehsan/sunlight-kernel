//! ACL metadata stored inside each record.
//! Serialized with serde + bincode into the on-disk record.

#[cfg(feature = "host")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "sunlightos")]
use alloc::string::String;
#[cfg(feature = "sunlightos")]
use alloc::vec;
#[cfg(feature = "sunlightos")]
use alloc::vec::Vec; // brings vec! macro into scope for no_std

/// Access control list persisted with every key record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(Serialize, Deserialize))]
pub struct Acl {
    /// The service/identity that created the key. Owner implicitly has full rights.
    pub owner: String,
    /// Identities explicitly allowed to read the value.
    pub read: Vec<String>,
    /// Identities explicitly allowed to write (overwrite or delete).
    pub write: Vec<String>,
}

impl Acl {
    /// Create a minimal ACL for a first-time PUT by `owner`.
    /// Only the owner has read and write rights.
    pub fn new(owner: impl Into<String>) -> Self {
        let o = owner.into();
        Self {
            owner: o.clone(),
            read: vec![o.clone()],
            write: vec![o],
        }
    }

    /// Returns true if `who` is allowed to read.
    /// "root" is always allowed (admin bypass).
    pub fn allows_read(&self, who: &str) -> bool {
        if who == "root" {
            return true;
        }
        if who == self.owner {
            return true;
        }
        self.read.iter().any(|r| r == who)
    }

    /// Returns true if `who` is allowed to write or delete.
    /// "root" is always allowed (admin bypass).
    pub fn allows_write(&self, who: &str) -> bool {
        if who == "root" {
            return true;
        }
        if who == self.owner {
            return true;
        }
        self.write.iter().any(|w| w == who)
    }
}
