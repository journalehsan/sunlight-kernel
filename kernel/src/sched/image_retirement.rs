//! Last-owner exit proof. Shared objects retain their existing owners.
use crate::capability::delivery::RecipientIdentity;

/// The image has stopped executing, its owned IPC endpoints and task-local
/// IPC state have been torn down, and its private user address space has
/// been reclaimed. This says nothing about objects exported to other owners.
#[derive(Clone, Copy)]
pub(crate) struct PrivateImageRetired(RecipientIdentity);

impl PrivateImageRetired {
    /// The scheduler calls this inside the cleanup path, after endpoint and
    /// task teardown. Capture the identity before reclaim invalidates the VM.
    pub(super) fn after_exit_cleanup(
        image: RecipientIdentity,
        owned_address_space: bool,
        address_space_reclaimed: bool,
        endpoints_revoked: bool,
    ) -> Option<Self> {
        (owned_address_space
            && address_space_reclaimed
            && endpoints_revoked
            && image.pid != 0
            && image.incarnation != 0
            && image.vm_generation != 0)
            .then_some(Self(image))
    }

    pub(crate) fn image(self) -> RecipientIdentity {
        self.0
    }
}
