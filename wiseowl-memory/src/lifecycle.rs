//! Explicit lifecycle operations and valid transitions.

use crate::entry::MemoryState;
use crate::error::MemoryError;

/// Explicit lifecycle operation (never implicit state mutation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum LifecycleOp {
    Create,
    Append,
    Read,
    Touch,
    Seal,
    Delete,
    Expire,
    PromoteToKv,
    SpillToCold,
    Rehydrate,
}

/// Result of checking whether an operation is allowed from a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionCheck {
    Allowed,
    Denied,
}

impl MemoryState {
    /// Return whether `op` is valid from this state.
    pub fn allows(self, op: LifecycleOp) -> TransitionCheck {
        use LifecycleOp::*;
        use MemoryState::*;
        let ok = match (self, op) {
            // Create is only meaningful for non-existent entries (handled outside).
            (_, Create) => false,

            (Open, Append) => true,
            (Open, Read) => true,
            (Open, Touch) => true,
            (Open, Seal) => true,
            (Open, Delete) => true,
            (Open, Expire) => true,
            (Open, PromoteToKv) => false, // must seal first
            (Open, SpillToCold) => false, // must seal first
            (Open, Rehydrate) => false,

            (Sealed, Append) => false,
            (Sealed, Read) => true,
            (Sealed, Touch) => true,
            (Sealed, Seal) => false, // already sealed
            (Sealed, Delete) => true,
            (Sealed, Expire) => true,
            (Sealed, PromoteToKv) => true,
            (Sealed, SpillToCold) => true,
            (Sealed, Rehydrate) => false,

            (Cold, Append) => false,
            (Cold, Read) => true,
            (Cold, Touch) => true,
            (Cold, Seal) => false,
            (Cold, Delete) => true,
            (Cold, Expire) => true,
            (Cold, PromoteToKv) => true,
            (Cold, SpillToCold) => false,
            (Cold, Rehydrate) => true,

            (Promoted, Append) => false,
            (Promoted, Read) => true,
            (Promoted, Touch) => true,
            (Promoted, Seal) => false,
            (Promoted, Delete) => true,
            (Promoted, Expire) => true,
            (Promoted, PromoteToKv) => true, // idempotent
            (Promoted, SpillToCold) => true,
            (Promoted, Rehydrate) => true,

            (Deleted, _) => false,
            (Expired, Touch) => false,
            (Expired, Read) => false,
            (Expired, Append) => false,
            (Expired, Seal) => false,
            (Expired, PromoteToKv) => false,
            (Expired, SpillToCold) => false,
            (Expired, Rehydrate) => false,
            (Expired, Delete) => true, // allow cleanup
            (Expired, Expire) => true, // idempotent
        };
        if ok {
            TransitionCheck::Allowed
        } else {
            TransitionCheck::Denied
        }
    }

    pub fn require(self, op: LifecycleOp) -> Result<(), MemoryError> {
        match self.allows(op) {
            TransitionCheck::Allowed => Ok(()),
            TransitionCheck::Denied => Err(MemoryError::InvalidLifecycleTransition {
                from: self.as_str(),
                op,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_cannot_mutate() {
        assert_eq!(
            MemoryState::Sealed.allows(LifecycleOp::Append),
            TransitionCheck::Denied
        );
        assert_eq!(
            MemoryState::Open.allows(LifecycleOp::Append),
            TransitionCheck::Allowed
        );
    }

    #[test]
    fn open_cannot_spill_or_promote() {
        assert_eq!(
            MemoryState::Open.allows(LifecycleOp::SpillToCold),
            TransitionCheck::Denied
        );
        assert_eq!(
            MemoryState::Open.allows(LifecycleOp::PromoteToKv),
            TransitionCheck::Denied
        );
    }

    #[test]
    fn expired_cannot_touch() {
        assert!(MemoryState::Expired.require(LifecycleOp::Touch).is_err());
    }

    #[test]
    fn deleted_cannot_promote() {
        assert!(MemoryState::Deleted
            .require(LifecycleOp::PromoteToKv)
            .is_err());
    }

    #[test]
    fn sealed_can_promote() {
        assert!(MemoryState::Sealed.require(LifecycleOp::PromoteToKv).is_ok());
    }
}
