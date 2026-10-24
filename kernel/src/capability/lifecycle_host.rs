//! Host-only process-table model for exercising the production issuance methods.
//! Not a replacement for native Scheduler/Process/IPC lifecycle validation.
use alloc::vec::Vec;
#[derive(Clone, Copy)]
pub struct Identity {
    pub generation: u64,
}
pub struct AddressSpace {
    pub generation: u64,
}
impl AddressSpace {
    pub fn identity(&self) -> Identity {
        Identity {
            generation: self.generation,
        }
    }
}
pub struct Foreground {
    pub incarnation: u64,
}
#[derive(Clone, Copy)]
pub struct HostCall {
    pub pid: usize,
    pub incarnation: u64,
    pub generation: u64,
}
#[derive(Clone, Copy)]
pub struct HostReply {
    pub call: HostCall,
}
pub fn reply(pid: usize, generation: u64, incarnation: u64) -> HostReply {
    HostReply {
        call: HostCall {
            pid,
            generation,
            incarnation,
        },
    }
}
pub struct Process {
    pub pid: usize,
    pub trusted_auth_broker: bool,
    pub trusted_vfs_service: bool,
    pub trusted_service_manager: bool,
    pub trusted_tty_session_service: bool,
    pub trusted_lock_service: bool,
    pub exiting: bool,
    pub foreground: Foreground,
    pub address_space: AddressSpace,
    pub pending_call: Option<(usize, u64)>,
    pub ipc_reply_target: Option<HostReply>,
}
impl Process {
    pub fn is_exiting(&self) -> bool {
        self.exiting
    }
}
pub struct Scheduler {
    pub processes: Vec<Process>,
    pub global_tick: u64,
}
impl Scheduler {
    pub fn current_process(&self) -> &Process {
        &self.processes[0]
    }
}
pub fn reply_target_matches_pending_request(
    pid: usize,
    incarnation: u64,
    pending: Option<(usize, u64)>,
    reply: Option<HostReply>,
) -> bool {
    reply.is_some_and(|r| {
        r.call.pid == pid
            && r.call.incarnation == incarnation
            && pending == Some((pid, r.call.generation))
    })
}
