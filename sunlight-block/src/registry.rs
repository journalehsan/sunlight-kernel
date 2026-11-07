//! Runtime identities describe devices; they convey no trust or write authority.
use crate::partition::{Layout, PartitionTable, MAX_PARTITIONS};

pub const READ: u64 = 1;
pub const WRITE: u64 = 2;
pub const FLUSH: u64 = 4;
pub const FAT32: u64 = 8;
/// Transport metadata only; conveys no authorization.
pub const USB: u64 = 16;
pub const MAX_DEVICES: usize = 2 * (MAX_PARTITIONS + 1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: u64,
    /// Zero for the physical disk; otherwise the physical disk's runtime ID.
    pub parent: u64,
    pub start: u64,
    pub blocks: u64,
    pub operations: u64,
    /// Discovery ordinal (1-based), zero for the physical disk.
    pub partition: u64,
    /// 0 = raw/unknown, 1 = superfloppy, 2 = MBR, 3 = GPT.
    pub layout: u64,
    pub uuid: [u8; 16],
    pub type_uuid: [u8; 16],
}

pub struct DeviceRegistry {
    entries: [Option<DeviceInfo>; MAX_DEVICES],
    // Never wrap or reset when an entry is replaced. Exhaustion fails closed.
    next_id: u64,
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceRegistry {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_DEVICES],
            next_id: 1,
        }
    }

    fn insert(&mut self, mut entry: DeviceInfo) -> Option<u64> {
        let slot = self.entries.iter_mut().find(|e| e.is_none())?;
        let next = self.next_id.checked_add(1)?;
        entry.id = self.next_id;
        *slot = Some(entry);
        self.next_id = next;
        Some(entry.id)
    }

    pub fn register_disk(&mut self, blocks: u64, operations: u64) -> Option<u64> {
        if blocks == 0 {
            return None;
        }
        self.insert(DeviceInfo {
            id: 0,
            parent: 0,
            start: 0,
            blocks,
            operations: operations & (READ | WRITE | FLUSH | USB),
            partition: 0,
            layout: 0,
            uuid: [0; 16],
            type_uuid: [0; 16],
        })
    }

    /// Called once after validated discovery. Even non-FAT partitions are exposed.
    pub fn register_partitions(&mut self, disk: u64, table: &PartitionTable) -> Option<()> {
        let physical = self.get(disk)?;
        if physical.parent != 0 || physical.layout != 0 {
            return None;
        }
        let layout = match table.layout {
            Layout::Superfloppy => 1,
            Layout::Mbr => 2,
            Layout::Gpt => 3,
        };
        if table.layout == Layout::Superfloppy {
            let entry = self.entries.iter_mut().flatten().find(|e| e.id == disk)?;
            entry.layout = layout;
            entry.operations |= FAT32;
            return Some(());
        }
        if self.entries.iter().filter(|e| e.is_none()).count() < table.partitions().len() {
            return None;
        }
        self.next_id.checked_add(table.partitions().len() as u64)?;
        // Validate the entire batch before publishing any child identities.
        for part in table.partitions() {
            if part.start.checked_add(part.blocks)? > physical.blocks {
                return None;
            }
        }
        for (index, part) in table.partitions().iter().enumerate() {
            self.insert(DeviceInfo {
                id: 0,
                parent: disk,
                start: part.start,
                blocks: part.blocks,
                operations: physical.operations | if part.fat_candidate { FAT32 } else { 0 },
                partition: index as u64 + 1,
                layout,
                uuid: part.uuid,
                type_uuid: part.type_uuid,
            })?;
        }
        self.entries
            .iter_mut()
            .flatten()
            .find(|e| e.id == disk)?
            .layout = layout;
        Some(())
    }

    pub fn get(&self, id: u64) -> Option<DeviceInfo> {
        self.entries.iter().flatten().find(|e| e.id == id).copied()
    }

    pub fn enumerate(&self, index: usize) -> Option<DeviceInfo> {
        self.entries.iter().flatten().nth(index).copied()
    }

    pub fn find_extent(&self, disk: u64, start: u64, blocks: u64) -> Option<DeviceInfo> {
        self.entries
            .iter()
            .flatten()
            .find(|e| {
                (e.id == disk || e.parent == disk)
                    && e.start == start
                    && e.blocks == blocks
                    && e.operations & FAT32 != 0
            })
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_stale_replacement_and_exhausted_ids() {
        let mut registry = DeviceRegistry::new();
        assert!(registry.register_disk(0, READ).is_none());
        let first = registry.register_disk(128, READ | WRITE).unwrap();
        assert!(registry.get(0).is_none());
        assert!(registry.get(999).is_none());
        // Simulate a future replacement without adding hot-removal support.
        registry.entries[0] = None;
        let replacement = registry.register_disk(256, READ).unwrap();
        assert_ne!(first, replacement);
        assert!(registry.get(first).is_none());
        assert_eq!(registry.get(replacement).unwrap().blocks, 256);
        registry.next_id = u64::MAX;
        assert!(registry.register_disk(128, READ).is_none());
    }

    #[test]
    fn identical_extents_on_usb_and_virtio_keep_distinct_parents() {
        let mut bytes = [0u8; 512 * 100];
        bytes[510..512].copy_from_slice(&[0x55, 0xaa]);
        bytes[450] = 0x0c;
        bytes[454..458].copy_from_slice(&10u32.to_le_bytes());
        bytes[458..462].copy_from_slice(&20u32.to_le_bytes());
        let table = crate::partition::discover(&mut crate::MemDisk::new(&mut bytes)).unwrap();
        let mut registry = DeviceRegistry::new();
        let virtio = registry.register_disk(100, READ | WRITE | FLUSH).unwrap();
        registry.register_partitions(virtio, &table).unwrap();
        let usb = registry
            .register_disk(100, READ | WRITE | FLUSH | USB)
            .unwrap();
        registry.register_partitions(usb, &table).unwrap();
        let first = registry.find_extent(virtio, 10, 20).unwrap();
        let second = registry.find_extent(usb, 10, 20).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(first.parent, virtio);
        assert_eq!(second.parent, usb);
        assert_eq!(first.operations & USB, 0);
        assert_eq!(second.operations & USB, USB);
    }

    #[test]
    fn enumerates_fat_and_unknown_partitions_with_parent_and_bounds() {
        let mut bytes = [0u8; 512 * 100];
        bytes[510..512].copy_from_slice(&[0x55, 0xaa]);
        for (slot, kind, start, blocks) in [(0, 0x0c, 10u32, 20u32), (1, 0x83, 40, 30)] {
            let offset = 446 + slot * 16;
            bytes[offset + 4] = kind;
            bytes[offset + 8..offset + 12].copy_from_slice(&start.to_le_bytes());
            bytes[offset + 12..offset + 16].copy_from_slice(&blocks.to_le_bytes());
        }
        let mut disk = crate::MemDisk::new(&mut bytes);
        let table = crate::partition::discover(&mut disk).unwrap();
        let mut registry = DeviceRegistry::new();
        let parent = registry.register_disk(100, READ | WRITE | FLUSH).unwrap();
        registry.register_partitions(parent, &table).unwrap();
        let first = registry.enumerate(1).unwrap();
        let second = registry.enumerate(2).unwrap();
        assert_eq!(
            (first.parent, first.partition, first.start, first.blocks),
            (parent, 1, 10, 20)
        );
        assert_eq!(first.operations, READ | WRITE | FLUSH | FAT32);
        assert_eq!(
            (second.parent, second.partition, second.start, second.blocks),
            (parent, 2, 40, 30)
        );
        assert_eq!(second.operations, READ | WRITE | FLUSH);
        assert_eq!(
            registry.get(parent).unwrap().operations,
            READ | WRITE | FLUSH
        );
        assert!(registry.enumerate(3).is_none());
    }
}
