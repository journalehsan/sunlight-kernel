#![no_std]
#![allow(dead_code)]

mod blk;
pub mod gpu;
pub mod pci;
pub mod svga;
pub mod svga_regs;

pub use blk::{BlkError, VirtioBlk, QUEUE_PAGES};
pub use gpu::{VirtioGpu, VirtioGpuMemEntry};
pub use pci::{
    find_virtio_blk, find_virtio_gpu, find_virtio_net, find_vmware_svga, find_vmware_svga_bdf,
    find_vmxnet3, find_vmxnet3_bdf, probe_vmware_svga, probe_vmxnet3, vmware_svga_present,
    vmxnet3_present, PciBarMemoryWidth, PciIoBarInfo, PciMemoryBarInfo, VmwareSvgaPciInfo,
    VmwareSvgaProbeError, Vmxnet3PciInfo, Vmxnet3ProbeError,
};
pub use svga::{SvgaCounters, SvgaError, SvgaProbeInfo, SvgaStage, VmwareSvga};
pub use svga_regs::{SVGA_PCI_DEVICE, SVGA_PCI_VENDOR};
