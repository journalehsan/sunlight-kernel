//! Disposition of the embedded VFS service's private backing on exec.
use crate::capability::delivery::RecipientIdentity;

/// Captured under scheduler serialization after VFS admission and image
/// provenance have been withdrawn. This does not certify IPC endpoint teardown.
pub(super) struct FencedVfsExec(RecipientIdentity);

pub(crate) struct PrivateVfsExecRetired(RecipientIdentity);

impl FencedVfsExec {
    pub(super) fn after_withdrawal(
        old: RecipientIdentity,
        was_trusted_vfs: bool,
        still_trusted_vfs: bool,
    ) -> Option<Self> {
        (was_trusted_vfs
            && !still_trusted_vfs
            && old.pid != 0
            && old.incarnation != 0
            && old.vm_generation != 0)
            .then_some(Self(old))
    }

    pub(super) fn after_reclaim(self, reclaimed: bool) -> Option<PrivateVfsExecRetired> {
        reclaimed.then_some(PrivateVfsExecRetired(self.0))
    }
}

impl PrivateVfsExecRetired {
    pub(crate) fn image(self) -> RecipientIdentity {
        self.0
    }
}
