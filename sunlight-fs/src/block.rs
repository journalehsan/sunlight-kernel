//! Block-device layer of the VFS.
//!
//! The trait and cache live in the `sunlight-block` base crate so that
//! `sunlight-fat` can also depend on them without a dependency cycle
//! (`sunlight-fs` → `sunlight-fat` → `sunlight-block`). This module is the
//! canonical re-export for VFS users.

pub use sunlight_block::{
    BlockDevice, BlockError, CachedBlockDevice, MemDisk, NullDevice, BLOCK_SIZE,
};
