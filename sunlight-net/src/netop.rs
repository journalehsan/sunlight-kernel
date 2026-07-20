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
