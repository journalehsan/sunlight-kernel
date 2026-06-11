/// TCP socket support stub for Phase 5.x.2
/// Full implementation requires smoltcp TcpSocket integration

use smoltcp::iface::{Interface, SocketSet};

#[derive(Debug, Clone)]
pub struct TcpConnection {
    pub remote_ip: [u8; 4],
    pub remote_port: u16,
    pub local_port: u16,
}

#[derive(Debug)]
pub enum TcpError {
    Timeout,
    Refused,
    SocketError,
}

impl TcpConnection {
    /// Connect to a remote TCP server (Phase 5.x.2 stub)
    pub fn connect(
        ip: [u8; 4],
        port: u16,
        _iface: &mut Interface,
        _sockets: &mut SocketSet,
        _device: &mut crate::device::SunlightNetDevice,
    ) -> Result<Self, TcpError> {
        // Simulate successful connection
        Ok(TcpConnection {
            remote_ip: ip,
            remote_port: port,
            local_port: 49152,
        })
    }

    /// Send data through TCP socket
    pub fn send(
        &self,
        data: &[u8],
        _sockets: &mut SocketSet,
    ) -> Result<usize, TcpError> {
        Ok(data.len())
    }

    /// Receive data from TCP socket
    pub fn recv(
        &self,
        _buf: &mut [u8],
        _sockets: &mut SocketSet,
    ) -> Result<usize, TcpError> {
        Ok(0)
    }

    /// Close TCP connection
    pub fn close(&self, _sockets: &mut SocketSet) {
        // Stub
    }
}
