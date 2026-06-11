/// Network operation IPC message opcodes
pub mod NetOp {
    pub const SOCKET: u64 = 1;      // create socket → socket_id
    pub const CONNECT: u64 = 2;     // connect(socket_id, ip, port)
    pub const BIND: u64 = 3;        // bind(socket_id, port)
    pub const LISTEN: u64 = 4;      // listen(socket_id, backlog)
    pub const ACCEPT: u64 = 5;      // accept → new socket_id
    pub const SEND: u64 = 6;        // send(socket_id, data)
    pub const RECV: u64 = 7;        // recv(socket_id) → data
    pub const CLOSE: u64 = 8;       // close(socket_id)
    pub const RESOLVE: u64 = 9;     // DNS lookup(hostname) → ip
    pub const GETIP: u64 = 10;      // get our assigned IP
}
