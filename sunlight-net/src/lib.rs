#![no_std]

extern crate alloc;

pub mod pci;
pub mod virtio_net;
pub mod device;
pub mod dhcp;
pub mod dns;
pub mod netop;

pub use virtio_net::VirtioNet;
pub use device::SunlightNetDevice;
pub use dhcp::{DhcpConfig, DhcpError, acquire_lease};
pub use dns::{DnsError, resolve};
pub use netop::NetOp;
