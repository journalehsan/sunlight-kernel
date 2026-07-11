#![no_std]
#![allow(dead_code)]

mod blk;
pub mod gpu;
pub mod pci;

pub use blk::{BlkError, VirtioBlk, QUEUE_PAGES};
pub use gpu::{VirtioGpu, VirtioGpuMemEntry};
pub use pci::{
    find_virtio_blk, find_virtio_gpu, find_virtio_net, find_vmxnet3, find_vmxnet3_bdf,
    probe_vmxnet3, vmxnet3_present, PciBarMemoryWidth, PciMemoryBarInfo, Vmxnet3PciInfo,
    Vmxnet3ProbeError,
};
