#![no_std]
#![allow(dead_code)]

mod pci;
mod blk;

pub use pci::find_virtio_blk;
pub use blk::{VirtioBlk, BlkError, QUEUE_PAGES};
