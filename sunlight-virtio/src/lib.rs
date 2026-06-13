#![no_std]
#![allow(dead_code)]

pub mod pci;
mod blk;

pub use pci::{find_virtio_blk, find_virtio_net};
pub use blk::{VirtioBlk, BlkError, QUEUE_PAGES};
