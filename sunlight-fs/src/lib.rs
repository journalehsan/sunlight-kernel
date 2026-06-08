#![no_std]

#[cfg(test)]
extern crate std;

pub mod error;
pub mod path;
pub mod ramfs;
pub mod vfs;

pub use error::FsError;
pub use ramfs::{RamEntry, RamFs, INITRAMFS};
pub use vfs::{DirIter, FileHandle, FileStat, FileSystem, FileType, FsNode, Vfs};
