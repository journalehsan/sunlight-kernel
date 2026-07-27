#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod block;
pub mod error;
pub mod fstab;
pub mod passwd;
pub mod path;
pub mod permission;
pub mod policy;
pub mod ramfs;
pub mod vfs;

pub use block::{BlockDevice, BlockError, CachedBlockDevice, NullDevice, BLOCK_SIZE};
pub use error::FsError;
pub use fstab::{parse_fstab, FstabEntry, FstabTable, MAX_FSTAB_ENTRIES};
pub use passwd::{
    lookup_by_name, lookup_by_uid, parse_group, parse_passwd, parse_shadow, GroupEntry,
    PasswdEntry, ShadowEntry,
};
pub use permission::{check_permission, Credential, PermCheck};
pub use policy::{
    broker_mint_fs_capability, can_write, Actor, CapabilityRights, Decision, FsCapability,
    FsOperation, PolicyReason,
};
pub use ramfs::{
    initramfs_static_file_data_bytes, RamEntry, RamFs, INITRAMFS,
    INITRAMFS_EMBEDDED_IN_KERNEL_IMAGE,
};
pub use vfs::{
    mode, FatFs, FileHandle, FileStat, FileSystem, FileType, FsNode, Vfs, VfsDirEntry, VFS_NAME_MAX,
};
