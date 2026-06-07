use crate::capability::{CapabilityBroker, CapabilityRights, CapabilityToken};
use crate::process::IpcMessage;
use alloc::collections::VecDeque;

/// Global IPC bus instance.
pub static IPC_BUS: spin::Mutex<IpcBus> = spin::Mutex::new(IpcBus::new());

/// Errors returned by IPC operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidCapability = 1,
    EndpointNotFound = 2,
    WouldBlock = 3,
    InvalidArgument = 4,
}

/// The IPC bus manages per-endpoint message queues.
pub struct IpcBus {
    queues: alloc::vec::Vec<(u32, VecDeque<IpcMessage>)>,
}

impl IpcBus {
    pub const fn new() -> Self {
        Self {
            queues: alloc::vec::Vec::new(),
        }
    }

    /// Get or create a queue for an endpoint.
    fn queue_for(&mut self, endpoint_id: u32) -> &mut VecDeque<IpcMessage> {
        let idx = self.queues.iter().position(|(id, _)| *id == endpoint_id);
        if let Some(idx) = idx {
            return &mut self.queues[idx].1;
        }
        self.queues.push((endpoint_id, VecDeque::new()));
        let last = self.queues.len() - 1;
        &mut self.queues[last].1
    }

    /// Send a message to an endpoint.
    pub fn send(
        &mut self,
        token: CapabilityToken,
        mut message: IpcMessage,
        caps: &CapabilityBroker,
        sender_pid: usize,
    ) -> Result<(), IpcError> {
        let endpoint_id = caps
            .check(token, CapabilityRights::SEND_ONLY)
            .map_err(|_| IpcError::InvalidCapability)?;

        message.sender_pid = sender_pid as u32;
        message.endpoint_id = endpoint_id;

        let queue = self.queue_for(endpoint_id);
        queue.push_back(message);
        Ok(())
    }

    /// Try to receive a message from an endpoint without blocking.
    pub fn try_recv(
        &mut self,
        token: CapabilityToken,
        caps: &CapabilityBroker,
    ) -> Result<IpcMessage, IpcError> {
        let endpoint_id = caps
            .check(token, CapabilityRights::SEND_ONLY)
            .map_err(|_| IpcError::InvalidCapability)?;

        let queue = self.queue_for(endpoint_id);
        queue.pop_front().ok_or(IpcError::WouldBlock)
    }

    /// Send a tick message to the timer server endpoint.
    pub fn send_timer_tick(&mut self, endpoint_id: u32) {
        let msg = IpcMessage {
            sender_pid: 0, // kernel
            endpoint_id,
            tag: 0x1,      // TimerMessage::TICK
            capability: None,
            len: 0,
            data: [0; crate::process::IPC_INLINE_MAX],
        };
        let queue = self.queue_for(endpoint_id);
        queue.push_back(msg);
    }
}
