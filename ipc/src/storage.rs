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
pub const ABORT_PREPARATION: u64 = 6;
pub const PUBLISH_PREPARATION: u64 = 7;
/// First 64 bytes match ENUMERATE; GPT UUID at 64, type UUID at 80.
pub const INVENTORY_RECORD_SIZE: usize = 96;
pub const MAX_DEVICES: usize = 258;
pub fn read_only(op: u64) -> bool {
    matches!(op, INVENTORY | INVENTORY_MOUNTS)
}
pub const DEVICE_RECORD_SIZE: usize = 64;
pub const MOUNT_RECORD_SIZE: usize = 160;
pub const TARGET_MAX: usize = 128;
pub const SERVICE_MOUNT_BIT: u64 = 1 << 63;

pub fn service_mount_id(epoch: u64, mount: u64) -> Option<u64> {
    (epoch != 0 && epoch < (1 << 31) && mount != 0 && mount <= u32::MAX as u64)
        .then_some(SERVICE_MOUNT_BIT | (epoch << 32) | mount)
}

pub fn local_mount_id(epoch: u64, id: u64) -> Option<u64> {
    let mount = id & u32::MAX as u64;
    (service_mount_id(epoch, mount) == Some(id)).then_some(mount)
}

pub fn mount_record(id: u64, source: u64, protected: bool, path: &str)
    -> Option<[u8; MOUNT_RECORD_SIZE]>
{
    if path.len() >= TARGET_MAX {
        return None;
    }
    let mut record = [0; MOUNT_RECORD_SIZE];
    for (slot, word) in record.chunks_exact_mut(8)
        .zip([id, source, u64::from(protected), path.len() as u64])
    {
        slot.copy_from_slice(&word.to_le_bytes());
    }
    record[32..32 + path.len()].copy_from_slice(path.as_bytes());
    Some(record)
}
pub const FIXTURE_CALLER_PID: u64 = 39000;
pub const FIXTURE_RETIRING_PID: u64 = 39001;
pub const FIXTURE_LINUX_PID: u64 = 39002;

/// Call only with credentials obtained from the kernel's current process.
/// An unrestricted legacy lookup profile is not an explicit admin grant.
pub fn authorized(uid: u32, capabilities: Option<u64>) -> bool {
    uid == 0 && capabilities.is_some_and(|mask| mask & ServiceCapability::StorageAdmin.bit() != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_ids_bind_epoch_and_mount_incarnation() {
        let old = service_mount_id(7, 2).unwrap();
        assert_eq!(local_mount_id(7, old), Some(2));
        assert_eq!(local_mount_id(8, old), None);
        assert_eq!(local_mount_id(7, 2), None);
        assert_ne!(service_mount_id(7, 10), Some(old));
        assert_eq!(service_mount_id(1 << 31, 2), None);
    }
    #[test]
    fn only_explicit_inventory_operations_are_public() {
        for op in [
            ENUMERATE, LIST_MOUNTS, MOUNT_FAT32, UNMOUNT,
            ABORT_PREPARATION, PUBLISH_PREPARATION, u64::MAX,
        ] {
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
