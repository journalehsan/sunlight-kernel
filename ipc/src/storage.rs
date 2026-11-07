//! Native StorageCtl ABI. Mutation and legacy queries require root AND StorageAdmin.
//! INVENTORY and INVENTORY_MOUNTS are public, read-only registry/VFS snapshots.
//! IDs are boot-local opaque identities, never /state trust authorizations.
use crate::ServiceCapability;

pub const ENUMERATE: u64 = 0;
pub const MOUNT_FAT32: u64 = 1;
pub const LIST_MOUNTS: u64 = 2;
pub const UNMOUNT: u64 = 3;
pub const INVENTORY: u64 = 4;
pub const INVENTORY_MOUNTS: u64 = 5;
/// First 64 bytes match ENUMERATE; GPT UUID at 64, type UUID at 80.
pub const INVENTORY_RECORD_SIZE: usize = 96;
pub const MAX_DEVICES: usize = 258;
pub fn read_only(op: u64) -> bool {
    matches!(op, INVENTORY | INVENTORY_MOUNTS)
}
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
    fn only_explicit_inventory_operations_are_public() {
        for op in [ENUMERATE, LIST_MOUNTS, MOUNT_FAT32, UNMOUNT, 6, u64::MAX] {
            assert!(!read_only(op));
        }
        assert!(read_only(INVENTORY));
        assert!(read_only(INVENTORY_MOUNTS));
    }
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
