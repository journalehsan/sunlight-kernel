/// Network operation IPC message opcodes
pub mod NetOp {
    pub const SOCKET: u64 = 1; // create socket → socket_id
    pub const CONNECT: u64 = 2; // connect(socket_id, ip, port)
    pub const BIND: u64 = 3; // bind(socket_id, port)
    pub const LISTEN: u64 = 4; // listen(socket_id, backlog)
    pub const ACCEPT: u64 = 5; // accept → new socket_id
    pub const SEND: u64 = 6; // send(socket_id, data)
    pub const RECV: u64 = 7; // recv(socket_id) → data
    pub const CLOSE: u64 = 8; // close(socket_id)
    pub const RESOLVE: u64 = 9; // DNS lookup(hostname) → ip
    pub const GETIP: u64 = 10; // get our assigned IP
    pub const POLL: u64 = 11; // poll([socket_ids]) → [ready_socket_ids]
    pub const RELOAD_HOSTS: u64 = 12; // re-read /etc/hosts from VFS into the resolver chain
    pub const PING: u64 = 13; // ICMP echo(ip, count) → (success, received, avg_rtt_ms)
                              // Bulk transfer over one shared 4 KiB page instead of the ~16 inline payload
                              // bytes the register-IPC ABI allows. Used by fetch/tls for fast TCP I/O.
    pub const SEND_SHM: u64 = 14; // send(socket_id, len, page_cap) → sent
    pub const RECV_SHM: u64 = 15; // recv(socket_id, max_len) → (len, page_cap)
    pub const GET_BACKEND: u64 = 16; // query backend kind, MAC, MTU, link, state
    /// Block until a generation-checked socket in a bounded SHM wait-set is ready.
    pub const WAIT: u64 = 17;
    /// Read one TCP allocation diagnostic selected by `NetDiagnostic`.
    pub const GET_DIAGNOSTIC: u64 = 18;
    /// Return the executing net stack's current lease without consulting
    /// networkd policy. Intended for networkd's bounded state synchronisation.
    pub const GETIP_LIVE: u64 = 19;
    /// One-shot UDP request/response via SHM (see ipc::NetOp::UDP_EXCHANGE).
    pub const UDP_EXCHANGE: u64 = 20;
}

/// Explicit network operation status values. Legacy replies retain their
/// original result in word 0 and carry this status in word 1.
pub mod NetStatus {
    pub const OK: u64 = 0;
    pub const WOULD_BLOCK: u64 = 1;
    pub const EOF: u64 = 2;
    pub const RESET: u64 = 3;
    pub const CLOSED: u64 = 4;
    pub const TIMEOUT: u64 = 5;
    pub const INVALID_SOCKET: u64 = 6;
    pub const ACCESS_DENIED: u64 = 7;
    pub const ADDRESS_IN_USE: u64 = 8;
    pub const INVALID_STATE: u64 = 9;
    pub const NOT_CONNECTED: u64 = 10;
    pub const BACKLOG_FULL: u64 = 11;
    pub const NO_SLOTS: u64 = 12;
    pub const INTERNAL: u64 = 13;
}

/// Readiness bits returned by `WAIT` in reply word 2.
pub mod NetReady {
    pub const ACCEPT: u64 = 1 << 0;
    pub const READ: u64 = 1 << 1;
    pub const WRITE: u64 = 1 << 2;
    pub const EOF: u64 = 1 << 3;
    pub const RESET: u64 = 1 << 4;
    pub const CLOSED: u64 = 1 << 5;
    pub const ERROR: u64 = 1 << 6;
}

/// Selectors for `NetOp::GET_DIAGNOSTIC`.
pub mod NetDiagnostic {
    pub const SOCKET_ALLOC_TOTAL: u64 = 0;
    pub const SOCKET_RELEASE_TOTAL: u64 = 1;
    pub const SOCKET_LIVE: u64 = 2;
    pub const SOCKET_PEAK_LIVE: u64 = 3;
    pub const LISTENER_ALLOC_TOTAL: u64 = 4;
    pub const LISTENER_RELEASE_TOTAL: u64 = 5;
    pub const LISTENER_LIVE: u64 = 6;
    pub const STREAM_ALLOC_TOTAL: u64 = 7;
    pub const STREAM_RELEASE_TOTAL: u64 = 8;
    pub const STREAM_LIVE: u64 = 9;
    pub const RX_BUFFERS_LIVE: u64 = 10;
    pub const TX_BUFFERS_LIVE: u64 = 11;
    pub const RX_BYTES_RESERVED: u64 = 12;
    pub const TX_BYTES_RESERVED: u64 = 13;
    pub const ALLOCATION_FAILURES_TOTAL: u64 = 14;
    pub const ALLOCATION_ROLLBACKS_TOTAL: u64 = 15;
    pub const FAILED_CONNECT_CLEANUP_TOTAL: u64 = 16;
    pub const PEER_RESET_REAPS_TOTAL: u64 = 17;
    pub const HALF_CLOSE_REAPS_TOTAL: u64 = 18;
    pub const CLOSE_DEADLINE_REAPS_TOTAL: u64 = 19;
    pub const OWNER_EXIT_REAPS_TOTAL: u64 = 20;
}
