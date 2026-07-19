use crate::{vmxnet3::FirstTxDescriptor, NetError, VirtioNet, Vmxnet3};
pub use sunlight_ipc::NetworkBackendKind;

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetBackendState {
    Detected = 1,
    Initializing = 2,
    HardwareReady = 3,
    Registered = 4,
    Failed = 5,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NetDeviceCounters {
    pub device_resets: u64,
    pub device_activations: u64,
    pub tx_requests: u64,
    pub tx_submitted: u64,
    pub tx_completed: u64,
    pub tx_bytes: u64,
    pub tx_notifications: u64,
    pub tx_errors: u64,
    pub rx_buffers_posted: u64,
    pub rx_completed: u64,
    pub rx_delivered: u64,
    pub rx_bytes: u64,
    pub rx_dropped: u64,
    pub rx_errors: u64,
    pub tx_ring_full: u64,
    pub rx_bad_completion: u64,
    pub interrupts: u64,
    pub polls: u64,
}

pub enum NetBackend {
    Virtio(VirtioNet),
    Vmxnet3(Vmxnet3),
}

pub type ActiveNetworkBackend = NetBackend;

unsafe impl Send for NetBackend {}

impl NetBackend {
    pub const fn kind(&self) -> NetworkBackendKind {
        match self {
            Self::Virtio(_) => NetworkBackendKind::VirtioNet,
            Self::Vmxnet3(_) => NetworkBackendKind::Vmxnet3,
        }
    }

    pub const fn mtu(&self) -> u16 {
        1500
    }

    pub fn mac(&self) -> [u8; 6] {
        match self {
            Self::Virtio(device) => device.mac(),
            Self::Vmxnet3(device) => device.mac(),
        }
    }

    pub fn link_up(&self) -> bool {
        match self {
            Self::Virtio(_) => true,
            Self::Vmxnet3(device) => device.link_up(),
        }
    }

    pub fn tx_available(&self) -> bool {
        match self {
            Self::Virtio(_) => true,
            Self::Vmxnet3(device) => device.tx_available(),
        }
    }

    pub fn rx_available(&self) -> bool {
        match self {
            Self::Virtio(_) => true,
            Self::Vmxnet3(device) => device.rx_available(),
        }
    }

    pub fn persistent_state_valid(&self) -> bool {
        match self {
            Self::Virtio(device) => device.mac() != [0; 6],
            Self::Vmxnet3(device) => device.persistent_state_valid(),
        }
    }

    pub fn vmxnet3_persistent_state(&self) -> Option<crate::Vmxnet3PersistentState> {
        match self {
            Self::Vmxnet3(device) => Some(device.persistent_state()),
            Self::Virtio(_) => None,
        }
    }

    pub fn counters(&self) -> NetDeviceCounters {
        match self {
            Self::Virtio(device) => device.counters(),
            Self::Vmxnet3(device) => device.counters(),
        }
    }

    pub fn first_tx_descriptor(&self) -> Option<FirstTxDescriptor> {
        match self {
            Self::Virtio(_) => None,
            Self::Vmxnet3(device) => device.first_tx_descriptor(),
        }
    }

    pub fn first_rx(&self) -> Option<(u16, u16)> {
        match self {
            Self::Virtio(_) => None,
            Self::Vmxnet3(device) => device.first_rx(),
        }
    }

    pub unsafe fn send(&mut self, frame: &[u8]) -> Result<(), NetError> {
        match self {
            Self::Virtio(device) => unsafe { device.send(frame) },
            Self::Vmxnet3(device) => unsafe { device.send(frame) },
        }
    }

    pub unsafe fn recv(&mut self, frame: &mut [u8]) -> usize {
        match self {
            Self::Virtio(device) => unsafe { device.recv(frame) },
            Self::Vmxnet3(device) => unsafe { device.recv(frame) },
        }
    }
}
