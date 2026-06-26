#![no_std]
#![allow(dead_code)]

mod blk;
pub mod pci;

pub use blk::{BlkError, VirtioBlk, QUEUE_PAGES};
pub use pci::{find_virtio_blk, find_virtio_net};
