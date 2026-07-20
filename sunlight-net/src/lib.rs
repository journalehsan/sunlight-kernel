#![no_std]

extern crate alloc;

pub mod backend;
pub mod device;
pub mod dhcp;
pub mod dns;
pub mod hosts;
pub mod icmp;
pub mod netop;
pub mod pci;
pub mod proxy_device;
pub mod simulation;
pub mod tcp;
pub mod virtio_net;
pub mod vmxnet3;

pub use backend::{
    ActiveNetworkBackend, NetBackend, NetBackendState, NetDeviceCounters, NetworkBackendKind,
};
pub use device::SunlightNetDevice;
pub use dhcp::{acquire_lease, DhcpConfig, DhcpError};
pub use dns::{DnsError, ResolverChain};
pub use hosts::{parse_hosts, HostsTable};
pub use netop::{NetDiagnostic, NetOp};
pub use proxy_device::ProxyNetDevice;
pub use sunlight_ipc::{NetBackendEvent, Vmxnet3ErrorCode, Vmxnet3InitStage};
pub use tcp::{SocketIdentity, SocketReady, TcpDiagnostics, TcpError, TcpManager};
pub use virtio_net::{NetError, VirtioNet, VirtioNetHeader, QUEUE_PAGES_PER_NET_QUEUE};
pub use vmxnet3::{
    FirstTxDescriptor, Vmxnet3, Vmxnet3InitError, Vmxnet3InitEvent, Vmxnet3PersistentState,
};
