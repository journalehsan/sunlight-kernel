#![no_std]

#[cfg(test)]
extern crate std;

pub mod error;
pub mod passwd;
pub mod path;
pub mod permission;
pub mod ramfs;
pub mod vfs;

pub use error::FsError;
pub use passwd::{
    parse_group, parse_passwd, parse_shadow, lookup_by_name, lookup_by_uid,
    GroupEntry, PasswdEntry, ShadowEntry,
};
pub use permission::{check_permission, Credential, PermCheck};
pub use ramfs::{RamEntry, RamFs, INITRAMFS};
pub use vfs::{mode, DirIter, FileHandle, FileStat, FileSystem, FileType, FsNode, Vfs};
