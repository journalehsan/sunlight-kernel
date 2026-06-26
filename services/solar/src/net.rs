//! TCP networking abstraction layer for Solar
//!
//! This module provides a high-level socket API that translates standard TCP
//! operations (bind, listen, accept, read, write) into IPC calls to net_server.
//!
//! Architecture:
//! - TcpListener: Server socket that can bind() and accept() connections
//! - TcpStream: Connected socket for reading/writing data
//! - All operations use sunlight_ipc to communicate with net_server
//! - Errors are propagated as &'static str for no_std compatibility

use sunlight_ipc::{ipc_call, nameserver_lookup, CapabilityToken, IpcMsg};
use sunlight_net::netop::NetOp;

/// Maximum concurrent connections Solar can handle per event loop
pub const MAX_ACTIVE_CONNS: usize = 64;

/// A TCP socket server that listens for incoming connections.
pub struct TcpListener {
    socket_id: u32,
    net_endpoint: CapabilityToken,
}

impl TcpListener {
    /// Creates a TCP listener bound to the specified port.
    ///
    /// This performs three IPC operations:
    /// 1. SOCKET - Create a new socket
    /// 2. BIND - Bind to 0.0.0.0:port
    /// 3. LISTEN - Start listening with backlog=64
    ///
    /// # Arguments
    /// * `port` - The port number to bind to (e.g., 80 for HTTP)
    ///
    /// # Returns
    /// * `Ok(TcpListener)` - Ready to accept connections
    /// * `Err(&str)` - If net_server lookup or any operation fails
    pub fn bind(port: u16) -> Result<Self, &'static str> {
        // 1. Look up net_server endpoint via nameserver
        let net_endpoint =
            nameserver_lookup("net").ok_or("Could not find net_server in nameserver")?;

        // 2. Create a new socket
        let msg = IpcMsg::with_label(NetOp::SOCKET);
        let reply = ipc_call(net_endpoint, msg);

        // net_server returns socket_id in reply.words[0]
        let socket_id = reply.words[0] as u32;
        if socket_id == 0 {
            return Err("net_server returned invalid socket_id");
        }

        // 3. Bind to 0.0.0.0:port
        let mut msg = IpcMsg::with_label(NetOp::BIND);
        msg = msg.word(0, socket_id as u64);
        msg = msg.word(1, port as u64);
        let reply = ipc_call(net_endpoint, msg);

        // Check for bind error (reply.words[0] == 0 means success)
        if reply.words[0] != 0 {
            return Err("Bind failed - port may be in use");
        }

        // 4. Start listening with backlog=64
        let mut msg = IpcMsg::with_label(NetOp::LISTEN);
        msg = msg.word(0, socket_id as u64);
        msg = msg.word(1, 64); // Backlog size
        let reply = ipc_call(net_endpoint, msg);

        if reply.words[0] != 0 {
            return Err("Listen failed");
        }

        crate::solar_log!("[SOLAR-NET] Socket {} bound to port {}", socket_id, port);

        Ok(Self {
            socket_id,
            net_endpoint,
        })
    }

    /// Blocks until a new client connects, returning a connected TcpStream.
    ///
    /// This calls NetOp::ACCEPT and waits for net_server to signal an
    /// incoming connection. The returned TcpStream can be used to read/write data.
    ///
    /// # Returns
    /// * `Ok(TcpStream)` - A connected client socket
    /// * `Err(&str)` - If accept fails or IPC error occurs
    pub fn accept(&self) -> Result<TcpStream, &'static str> {
        let mut msg = IpcMsg::with_label(NetOp::ACCEPT);
        msg = msg.word(0, self.socket_id as u64);

        let reply = ipc_call(self.net_endpoint, msg);

        // net_server returns the new client socket_id in reply.words[0]
        let client_socket_id = reply.words[0] as u32;
        if client_socket_id == 0 {
            return Err("Accept returned invalid socket_id");
        }

        Ok(TcpStream {
            socket_id: client_socket_id,
            net_endpoint: self.net_endpoint,
        })
    }

    /// Non-blocking accept: returns None if no pending connections
    ///
    /// This is useful for event loops that want to accept new connections
    /// without blocking when there are no clients waiting.
    pub fn try_accept(&self) -> Result<Option<TcpStream>, &'static str> {
        let mut msg = IpcMsg::with_label(NetOp::ACCEPT);
        msg = msg.word(0, self.socket_id as u64);

        let reply = ipc_call(self.net_endpoint, msg);

        // net_server returns the new client socket_id in reply.words[0]
        let client_socket_id = reply.words[0] as u32;

        if client_socket_id == 0 {
            // No pending connections (non-blocking mode)
            return Ok(None);
        }

        Ok(Some(TcpStream {
            socket_id: client_socket_id,
            net_endpoint: self.net_endpoint,
        }))
    }

    /// Get the net_endpoint for polling operations
    pub fn net_endpoint(&self) -> CapabilityToken {
        self.net_endpoint
    }
}

/// A connected TCP stream for reading and writing data.
pub struct TcpStream {
    pub socket_id: u32,
    pub net_endpoint: CapabilityToken,
}

impl TcpStream {
    /// Get the socket ID for this stream
    pub fn socket_id(&self) -> u32 {
        self.socket_id
    }

    /// Reads data from the TCP stream into the provided buffer.
    ///
    /// This calls NetOp::RECV_SHM and copies the received data into `buffer`.
    ///
    /// # Arguments
    /// * `buffer` - Destination buffer to fill with received data
    ///
    /// # Returns
    /// * `Ok(usize)` - Number of bytes read
    /// * `Err(&str)` - If recv fails or connection closed
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, &'static str> {
        let mut msg = IpcMsg::with_label(NetOp::RECV_SHM);
        msg = msg.word(0, self.socket_id as u64);
        msg = msg.word(1, buffer.len() as u64);

        let reply = ipc_call(self.net_endpoint, msg);

        // reply.words[0] contains the number of bytes received
        let bytes_read = reply.words[0] as usize;
        if bytes_read == 0 {
            // Connection closed by peer
            return Ok(0);
        }

        let shm_token = reply.caps[0];
        if shm_token == CapabilityToken::INVALID {
            return Err("Recv failed - missing SHM page");
        }

        let shm_ptr =
            sunlight_ipc::shm_map(shm_token).map_err(|_| "Failed to map received SHM page")?;
        let copy_len = bytes_read.min(buffer.len()).min(4096);
        unsafe {
            core::ptr::copy_nonoverlapping(shm_ptr as *const u8, buffer.as_mut_ptr(), copy_len);
        }
        let _ = sunlight_ipc::shm_free(shm_token);

        Ok(copy_len)
    }

    /// Writes all data from `data` to the TCP stream.
    ///
    /// This calls NetOp::SEND with the data payload. For small payloads,
    /// data is sent inline in IPC registers. For larger payloads (> 48 bytes),
    /// we allocate an SHM page from the pool (TODO: Phase 1.6).
    ///
    /// # Arguments
    /// * `data` - The bytes to send
    ///
    /// # Returns
    /// * `Ok(())` - All data sent successfully
    /// * `Err(&str)` - If send fails or connection error
    pub fn write_all(
        &mut self,
        data: &[u8],
        shm_pool: &crate::ShmPagePool,
    ) -> Result<(), &'static str> {
        // For small payloads (≤48 bytes), send inline in IPC registers
        if data.len() <= 48 {
            return self.write_inline(data);
        }

        // For larger payloads, use SHM
        self.write_shm(data, shm_pool)
    }

    /// Write small data inline (≤48 bytes) in IPC registers
    fn write_inline(&mut self, data: &[u8]) -> Result<(), &'static str> {
        let mut msg = IpcMsg::with_label(NetOp::SEND);
        msg = msg.word(0, self.socket_id as u64);
        msg = msg.word(1, data.len() as u64);

        // Pack data into words[2..7] (6 words × 8 bytes = 48 bytes max)
        for i in 0..data.len() {
            let word_idx = 2 + (i / 8);
            let byte_idx = i % 8;
            if word_idx < msg.words.len() {
                let val = msg.words[word_idx] | ((data[i] as u64) << (byte_idx * 8));
                msg = msg.word(word_idx, val);
            }
        }

        let reply = ipc_call(self.net_endpoint, msg);

        let bytes_sent = reply.words[0] as usize;
        if bytes_sent == 0 {
            return Err("Send failed - connection may be closed");
        }

        Ok(())
    }

    /// Write large data (>48 bytes) via shared memory
    fn write_shm(
        &mut self,
        data: &[u8],
        shm_pool: &crate::ShmPagePool,
    ) -> Result<(), &'static str> {
        // Acquire a shared memory page from the pool
        let shm_token = shm_pool.acquire().ok_or("SHM pool exhausted")?;

        // Map the SHM page to get a pointer
        let shm_ptr = sunlight_ipc::shm_map(shm_token).map_err(|_| "Failed to map SHM page")?;

        // Copy data to SHM (max 4096 bytes per page)
        let copy_len = data.len().min(4096);
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), shm_ptr as *mut u8, copy_len);
        }

        // Send via IPC with SHM capability
        let mut msg = IpcMsg::with_label(NetOp::SEND_SHM);
        msg = msg.word(0, self.socket_id as u64);
        msg = msg.word(1, copy_len as u64);
        msg = msg.with_cap(0, shm_token);

        let reply = ipc_call(self.net_endpoint, msg);

        shm_pool.release(shm_token);

        // reply.words[0] contains bytes sent (0 = error)
        let bytes_sent = reply.words[0] as usize;
        if bytes_sent == 0 {
            return Err("Send failed - connection may be closed");
        }

        Ok(())
    }

    /// Gracefully closes the TCP connection.
    ///
    /// This calls NetOp::CLOSE to release the socket in net_server.
    /// The socket_id becomes invalid after this call.
    pub fn close(&mut self) {
        let mut msg = IpcMsg::with_label(NetOp::CLOSE);
        msg = msg.word(0, self.socket_id as u64);
        let _ = ipc_call(self.net_endpoint, msg);

        // Mark socket as closed
        self.socket_id = 0;
    }
}

/// Poll multiple sockets to check which ones are ready for I/O
///
/// This is the core of Solar's async event loop. It sends a list of socket IDs
/// to net_server via NetOp::POLL and receives back which ones are ready.
///
/// # Arguments
/// * `net_endpoint` - The net_server capability token
/// * `socket_ids` - Slice of socket IDs to check (up to 8)
///
/// # Returns
/// Array of ready socket IDs (0 = no more ready sockets)
pub fn poll_sockets(
    net_endpoint: CapabilityToken,
    socket_ids: &[u32],
) -> Result<[u32; 8], &'static str> {
    let count = socket_ids.len().min(8);

    let mut msg = IpcMsg::with_label(NetOp::POLL);
    msg = msg.word(0, count as u64);

    for (i, &socket_id) in socket_ids.iter().take(8).enumerate() {
        msg = msg.word(i + 1, socket_id as u64);
    }

    let reply = ipc_call(net_endpoint, msg);

    let ready_count = reply.words[0] as usize;
    let mut ready = [0u32; 8];

    for i in 0..ready_count.min(8) {
        ready[i] = reply.words[i + 1] as u32;
    }

    Ok(ready)
}

/// Check if a socket ID is in the ready list
pub fn is_socket_ready(socket_id: u32, ready: &[u32; 8]) -> bool {
    for &id in ready.iter() {
        if id == 0 {
            break; // End of ready list
        }
        if id == socket_id {
            return true;
        }
    }
    false
}

impl Drop for TcpStream {
    /// Automatically close the socket when TcpStream goes out of scope.
    /// This prevents socket leaks via RAII.
    fn drop(&mut self) {
        if self.socket_id != 0 {
            self.close();
        }
    }
}
