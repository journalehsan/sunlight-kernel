#[cfg(target_os = "none")]
use alloc::string::String;
#[cfg(target_os = "none")]
use alloc::vec;
#[cfg(target_os = "none")]
use spin::Mutex;
use sunlight_block::registry::{DeviceInfo, DeviceRegistry, FAT32, FLUSH, MAX_DEVICES, WRITE};
use sunlight_fs::fstab;
#[cfg(target_os = "none")]
use sunlight_fs::RamEntry;
#[cfg(target_os = "none")]
use x86_64::VirtAddr;

const CAPACITY: usize = fstab::MAX_FSTAB_ENTRIES;

struct Entry {
    uuid: [u8; 16],
    #[cfg(target_os = "none")]
    target: String,
    optional: bool,
}

struct Policy {
    entries: [Option<Entry>; CAPACITY],
    #[cfg(target_os = "none")]
    selected: Option<u64>,
}

#[cfg(target_os = "none")]
static POLICY: Mutex<Option<Policy>> = Mutex::new(None);

#[cfg(target_os = "none")]
pub(crate) fn configure(seed: &[RamEntry]) {
    let mut buffer = vec![0; fstab::MAX_FSTAB_BYTES];
    let Ok(table) = sunlight_fs::parse_boot_fstab(seed, buffer.as_mut_slice().try_into().unwrap()) else {
        crate::serial_println!("[VFS-C2I1] invalid or unreadable boot fstab");
        return;
    };
    let entries = table.map(|entry| {
        let entry = entry?;
        Some(Entry {
            uuid: fstab::parse_partuuid(entry.device)?,
            target: String::from(entry.mountpoint),
            optional: fstab::optional(&entry),
        })
    });
    *POLICY.lock() = Some(Policy { entries, selected: None });
}

fn devices(registry: &DeviceRegistry) -> impl Iterator<Item = DeviceInfo> + '_ {
    (0..MAX_DEVICES).filter_map(|index| registry.enumerate(index))
}

fn fences(policy: Option<&Policy>, registry: &DeviceRegistry, parent: u64) -> bool {
    devices(registry).any(|row| {
        row.parent == parent
            && row.uuid != [0; 16]
            && policy.is_none_or(|policy| {
                policy.entries.iter().flatten().any(|entry| entry.uuid == row.uuid)
            })
    })
}

#[cfg(target_os = "none")]
pub(crate) fn fences_parent(parent: u64) -> bool {
    let policy = POLICY.lock();
    fences(policy.as_ref(), &crate::storage::DEVICES.lock(), parent)
}

#[cfg(target_os = "none")]
pub(crate) fn claims_mnt() -> bool {
    POLICY.lock().as_ref().is_none_or(|policy| policy.entries.iter().any(Option::is_some))
}

fn select(
    policy: &Policy,
    registry: &DeviceRegistry,
    transport_parent: u64,
) -> Result<Option<u64>, &'static str> {
    let mut selected = None;
    for entry in policy.entries.iter().flatten() {
        let source = match fstab::resolve_uuid(
            entry.uuid,
            devices(registry).map(|row| (row.id, row.uuid)),
        ) {
            Ok(id) => id,
            Err(fstab::ResolveError::Missing) if entry.optional => continue,
            Err(_) => return Err("missing or ambiguous PARTUUID"),
        };
        let (parent, extent) = registry.resolve_extent(source).ok_or("invalid extent")?;
        if extent.parent == 0
            || parent.id != transport_parent
            || extent.operations & (FAT32 | WRITE | FLUSH) != FAT32 | WRITE | FLUSH
        {
            return Err("source is outside the eligible transport");
        }
        if selected.replace(source).is_some() {
            return Err("configured ownership exceeds the single transport slot");
        }
    }
    Ok(selected)
}

#[cfg(target_os = "none")]
pub(crate) fn reserve(boot: (u8, u8, u8, u16), hhdm: VirtAddr) {
    let mut policy = POLICY.lock();
    let Some(policy) = policy.as_mut().filter(|p| p.entries.iter().any(Option::is_some)) else {
        return;
    };
    let result = (|| {
        let address = option_env!("SUNLIGHT_VFS_C2B_PCI").ok_or("no candidate PCI address")?;
        let candidate = unsafe {
            sunlight_virtio::find_virtio_blk_where(|(bus, slot, function, _)| {
                address == alloc::format!("{bus:02x}:{slot:02x}.{function}")
            })
        }.ok_or("candidate PCI function unavailable")?;
        if candidate == boot || candidate.3 == 0 || unsafe {
            sunlight_virtio::find_virtio_blk_where(|other| other != candidate && other.3 == candidate.3)
        }.is_some() {
            return Err("boot-device or I/O BAR conflict");
        }
        let (mut disk, queue, bounce) =
            crate::init_discovered_virtio(candidate, hhdm).ok_or("candidate initialization failed")?;
        let mut marker = [0; 512];
        let disposable = unsafe { disk.read_block(0, &mut marker) }
            && marker.starts_with(b"VFS-N1O-DISPOSABLE-ONLY!");
        let parent = crate::storage::register_virtio_disk(&disk).ok_or("registry full")?;
        crate::storage::discover_virtio_partitions(&mut disk, parent)
            .map_err(|_| "candidate partition discovery failed")?;
        if !disposable || disk.block_count() != 131072
            || disk.geometry().logical_block != 512
            || !disk.supports_write() || !disk.supports_flush()
        {
            return Err("candidate lacks disposable marker or write/flush support");
        }
        let source = select(policy, &crate::storage::DEVICES.lock(), parent)?;
        if let Some(source) = source {
            crate::vfs_n1j_test::install(disk, queue, bounce, source);
            policy.selected = Some(source);
            crate::serial_println!("[VFS-C2I1] reserved parent={} source={}", parent, source);
        }
        Ok(())
    })();
    if let Err(reason) = result {
        crate::serial_println!("[VFS-C2I1] reservation refused: {}", reason);
    }
}

#[cfg(target_os = "none")]
pub(crate) fn permits_target(source: u64, path: &str) -> bool {
    let policy = POLICY.lock();
    let Some(policy) = policy.as_ref().filter(|p| p.selected == Some(source)) else {
        return false;
    };
    let Some(row) = crate::storage::DEVICES.lock().get(source) else { return false };
    policy.entries.iter().flatten().any(|entry| entry.uuid == row.uuid && entry.target == path)
}

#[cfg(target_os = "none")]
pub(crate) fn ready() -> bool {
    let policy = POLICY.lock();
    let Some(policy) = policy.as_ref() else { return false };
    let registry = crate::storage::DEVICES.lock();
    let needs_mount = policy.entries.iter().flatten().any(|entry| {
        !entry.optional || devices(&registry).any(|row| row.uuid == entry.uuid)
    });
    drop(registry);
    !needs_mount || policy.selected.is_some_and(crate::vfs_n1j_test::committed_storage_live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_block::{partition, MemDisk};

    fn crc(bytes: &[u8]) -> u32 {
        !bytes.iter().fold(!0u32, |mut sum, byte| {
            sum ^= u32::from(*byte);
            for _ in 0..8 {
                sum = (sum >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(sum & 1));
            }
            sum
        })
    }

    fn gpt(uuid: [u8; 16]) -> partition::PartitionTable {
        let mut disk = [0u8; 100 * 512];
        disk[510..512].copy_from_slice(&[0x55, 0xaa]);
        disk[450] = 0xee;
        disk[454..458].copy_from_slice(&1u32.to_le_bytes());
        disk[458..462].copy_from_slice(&99u32.to_le_bytes());
        let mut entries = [0u8; 512];
        let esp = fstab::parse_partuuid("PARTUUID=c12a7328-f81f-11d2-ba4b-00a0c93ec93b").unwrap();
        entries[..16].copy_from_slice(&esp);
        entries[16..32].copy_from_slice(&uuid);
        entries[32..40].copy_from_slice(&10u64.to_le_bytes());
        entries[40..48].copy_from_slice(&29u64.to_le_bytes());
        for (header, peer, array) in [(1, 99, 2), (99, 1, 98)] {
            let bytes = &mut disk[header * 512..(header + 1) * 512];
            bytes[..8].copy_from_slice(b"EFI PART");
            bytes[8..12].copy_from_slice(&0x10000u32.to_le_bytes());
            bytes[12..16].copy_from_slice(&92u32.to_le_bytes());
            for (offset, word) in [(24, header), (32, peer), (40, 3), (48, 97), (72, array)] {
                bytes[offset..offset + 8].copy_from_slice(&(word as u64).to_le_bytes());
            }
            bytes[56] = 42;
            bytes[80..84].copy_from_slice(&4u32.to_le_bytes());
            bytes[84..88].copy_from_slice(&128u32.to_le_bytes());
            bytes[88..92].copy_from_slice(&crc(&entries).to_le_bytes());
            let checksum = crc(&bytes[..92]);
            bytes[16..20].copy_from_slice(&checksum.to_le_bytes());
            disk[array * 512..(array + 1) * 512].copy_from_slice(&entries);
        }
        partition::discover(&mut MemDisk::new(&mut disk)).unwrap()
    }

    #[test]
    fn configured_gpt_selects_exact_extent_and_fences_both_parent_aliases() {
        let uuid = fstab::parse_partuuid("PARTUUID=7e435232-1fd5-4704-a0bf-21de06ea232c").unwrap();
        let table = gpt(uuid);
        let mut registry = DeviceRegistry::new();
        let boot = registry.register_disk(100, WRITE | FLUSH).unwrap();
        let transport = registry.register_disk(100, WRITE | FLUSH).unwrap();
        let mut policy = Policy { entries: core::array::from_fn(|_| None) };
        policy.entries[0] = Some(Entry { uuid, optional: false });
        assert!(select(&policy, &registry, transport).is_err());
        policy.entries[0].as_mut().unwrap().optional = true;
        assert_eq!(select(&policy, &registry, transport), Ok(None));
        registry.register_partitions(transport, &table).unwrap();
        let source = select(&policy, &registry, transport).unwrap().unwrap();
        let (parent, extent) = registry.resolve_extent(source).unwrap();
        assert_eq!((parent.id, extent.start, extent.blocks), (transport, 10, 20));
        assert!(fences(Some(&policy), &registry, parent.id));
        assert!(fences(Some(&policy), &registry, extent.parent));
        assert!(!fences(Some(&policy), &registry, boot));
        assert!(select(&policy, &registry, boot).is_err());
        registry.register_partitions(boot, &table).unwrap();
        assert!(select(&policy, &registry, transport).is_err());
        assert!(fences(Some(&policy), &registry, boot));
        assert!(fences(None, &registry, boot));
    }
}
