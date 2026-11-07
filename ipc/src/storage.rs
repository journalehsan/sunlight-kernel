//! Native StorageCtl ABI. All operations require uid 0 AND StorageAdmin.
//! IDs are boot-local opaque identities, never /state trust authorizations.
use crate::ServiceCapability;

pub const ENUMERATE: u64 = 0;
pub const MOUNT_FAT32: u64 = 1;
pub const LIST_MOUNTS: u64 = 2;
pub const UNMOUNT: u64 = 3;
pub const DEVICE_RECORD_SIZE: usize = 64;
pub const MOUNT_RECORD_SIZE: usize = 160;
pub const TARGET_MAX: usize = 128;

/// Call only with credentials obtained from the kernel's current process.
/// An unrestricted legacy lookup profile is not an explicit admin grant.
pub fn authorized(uid: u32, capabilities: Option<u64>) -> bool {
    uid == 0 && capabilities.is_some_and(|mask| mask & ServiceCapability::StorageAdmin.bit() != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_real_root_and_explicit_capability() {
        let admin = ServiceCapability::StorageAdmin.bit();
        assert!(!authorized(0, None));
        assert!(!authorized(0, Some(0)));
        assert!(!authorized(1000, Some(admin)));
        assert!(!authorized(0, Some(ServiceCapability::Vfs.bit())));
        assert!(authorized(0, Some(admin)));
    }
}
