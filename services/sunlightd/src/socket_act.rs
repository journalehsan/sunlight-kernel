//! Socket activation support
//! Handles .socket units with ListenStream

use crate::unit::SocketUnit;

pub struct SocketListener {
    pub unit: SocketUnit,
    pub fd: Option<u32>,
}

impl SocketListener {
    pub fn new(unit: SocketUnit) -> Self {
        Self {
            unit,
            fd: None,
        }
    }

    /// Setup socket listener (TCP port)
    /// TODO: Implement actual net_server IPC for socket bind
    pub fn setup(&mut self) -> Result<(), &'static str> {
        // For now, just mark as setup
        // Real implementation needs NetOp::SocketBind IPC
        Ok(())
    }

    /// Check for incoming connections
    /// TODO: Implement NetOp::Accept notification
    pub fn check_accept(&self) -> Option<u32> {
        // TODO: requires net_server accept notification
        None
    }
}
