//! Root construction using only the immutable boot inventory.
use sunlight_fs::{BlockDevice, FsError, RamEntry, RamFs, Tmpfs, Vfs};

pub enum BootRoot {
    Compatibility,
    Writable,
}

impl BootRoot {
    pub fn mount<D: BlockDevice>(
        self,
        vfs: &mut Vfs<D>,
        seed: &'static [RamEntry],
    ) -> Result<(), FsError> {
        match self {
            Self::Compatibility => vfs.mount_ramfs("/", RamFs::new(seed)),
            Self::Writable => vfs.mount_tmpfs("/", Tmpfs::new(seed)),
        }
    }
}

/// Read immutable bootstrap material without opening a filesystem description.
pub fn read_seed_file(seed: &[RamEntry], path: &str, out: &mut [u8]) -> usize {
    seed.iter()
        .find(|entry| entry.path == path && !entry.is_dir)
        .and_then(|entry| entry.read(0, out).ok())
        .unwrap_or(0)
}
