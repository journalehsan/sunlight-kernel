#![no_std]

extern crate alloc;

pub mod pci;
pub mod virtio_net;
pub mod device;
pub mod dhcp;
pub mod dns;
pub mod tcp;
pub mod icmp;
pub mod netop;
pub mod simulation;

pub use virtio_net::{VirtioNet, VirtioNetHeader, NetError, QUEUE_PAGES_PER_NET_QUEUE};
pub use device::SunlightNetDevice;
pub use dhcp::{DhcpConfig, DhcpError, acquire_lease};
pub use dns::{DnsError, resolve};
pub use tcp::{TcpConnection, TcpError};
pub use netop::NetOp;
