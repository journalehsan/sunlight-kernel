//! Exact IPC lifetime identities shared by native code and host security tests.
/// A schedulable process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcCallId {
    pub pid: usize,
    pub incarnation: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcCallOutcome {
    ReplyDelivered(u64),
    DeadlineExpired(u64),
    ExplicitlyCancelled(u64),
    PeerClosed(u64),
}

impl IpcCallOutcome {
    pub const fn generation(self) -> u64 {
        match self {
            Self::ReplyDelivered(generation)
            | Self::DeadlineExpired(generation)
            | Self::ExplicitlyCancelled(generation)
            | Self::PeerClosed(generation) => generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcReplyTarget {
    pub endpoint_id: u32,
    pub call: IpcCallId,
}
