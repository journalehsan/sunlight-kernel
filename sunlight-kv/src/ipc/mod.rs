//! IPC protocol definitions for sunlight-kv.
//! Requests and Responses are serialized with bincode over a length-prefixed
//! binary framing protocol on the transport (Unix domain socket for the daemon).

mod protocol;

pub use protocol::{Request, Response};
