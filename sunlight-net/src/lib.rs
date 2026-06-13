#![no_std]

extern crate alloc;

pub mod pci;
pub mod virtio_net;
pub mod device;
pub mod dhcp;
pub mod dns;
pub mod hosts;
pub mod tcp;
pub mod icmp;
pub mod netop;
pub mod proxy_device;
pub mod simulation;

pub use virtio_net::{VirtioNet, VirtioNetHeader, NetError, QUEUE_PAGES_PER_NET_QUEUE};
pub use device::SunlightNetDevice;
pub use proxy_device::ProxyNetDevice;
pub use dhcp::{DhcpConfig, DhcpError, acquire_lease};
pub use dns::{DnsError, ResolverChain};
pub use hosts::{parse_hosts, HostsTable};
pub use tcp::{TcpConnection, TcpError};
pub use netop::NetOp;
