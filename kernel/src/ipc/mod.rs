pub mod message;

use crate::capability::{CapabilityBroker, CapabilityRights, CapabilityToken};
use crate::process::ProcessState;
use crate::sched::Scheduler;
use alloc::collections::VecDeque;

pub use message::IpcMsg;

pub const INIT_NAMESERVER_ENDPOINT: u32 = 0;

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

/// The IPC bus manages per-endpoint message queues and call waiters.
pub struct IpcBus {
    queues: alloc::vec::Vec<(u32, VecDeque<IpcMsg>)>,
    reply_waiters: alloc::vec::Vec<(u32, VecDeque<usize>)>,
}

impl IpcBus {
    pub const fn new() -> Self {
        Self {
            queues: alloc::vec::Vec::new(),
            reply_waiters: alloc::vec::Vec::new(),
        }
    }

    fn queue_for(&mut self, endpoint_id: u32) -> &mut VecDeque<IpcMsg> {
        let idx = self.queues.iter().position(|(id, _)| *id == endpoint_id);
        if let Some(idx) = idx {
            return &mut self.queues[idx].1;
        }
        self.queues.push((endpoint_id, VecDeque::new()));
        let last = self.queues.len() - 1;
        &mut self.queues[last].1
    }

    fn reply_waiters_for(&mut self, endpoint_id: u32) -> &mut VecDeque<usize> {
        let idx = self.reply_waiters.iter().position(|(id, _)| *id == endpoint_id);
        if let Some(idx) = idx {
            return &mut self.reply_waiters[idx].1;
        }
        self.reply_waiters.push((endpoint_id, VecDeque::new()));
        let last = self.reply_waiters.len() - 1;
        &mut self.reply_waiters[last].1
    }

    pub fn endpoint_owner(
        &self,
        token: CapabilityToken,
        caps: &CapabilityBroker,
    ) -> Option<usize> {
        let endpoint_id = caps.check(token, CapabilityRights::SEND).ok()?;
        caps.endpoint_owner(endpoint_id)
    }

    pub fn enqueue_call(
        &mut self,
        endpoint_id: u32,
        mut msg: IpcMsg,
        caller_pid: usize,
        sched: &mut Scheduler,
        server_pid: usize,
    ) {
        msg.badge = caller_pid as u64;
        self.queue_for(endpoint_id).push_back(msg);
        let waiters = self.reply_waiters_for(endpoint_id);
        if !waiters.iter().any(|pid| *pid == caller_pid) {
            waiters.push_back(caller_pid);
        }
        sched.wake_pid(server_pid);
    }

    pub fn block_on_recv(
        &mut self,
        endpoint_id: u32,
        receiver_pid: usize,
        sched: &mut Scheduler,
    ) {
        if let Some(receiver) = sched.process_mut_by_pid(receiver_pid) {
            receiver.ipc_endpoint = Some(endpoint_id);
            receiver.state = ProcessState::BlockedOnIpc;
        }
    }

    pub fn pop_pending(&mut self, endpoint_id: u32) -> Option<IpcMsg> {
        self.queue_for(endpoint_id).pop_front()
    }

    pub fn reply_waiter_pop_front(&mut self, endpoint_id: u32) -> Option<usize> {
        self.reply_waiters_for(endpoint_id).pop_front()
    }

    pub fn send_timer_tick(&mut self, endpoint_id: u32, sched: &mut Scheduler, server_pid: usize) {
        let msg = IpcMsg::with_label(0x1);
        self.queue_for(endpoint_id).push_back(msg);
        sched.wake_pid(server_pid);
    }

    pub fn send_keyboard_event(&mut self, endpoint_id: u32, event_val: u64, sched: &mut Scheduler, server_pid: usize) {
        let msg = IpcMsg::with_label(0x1).word(0, event_val);
        self.queue_for(endpoint_id).push_back(msg);
        sched.wake_pid(server_pid);
    }
}

pub fn handle_ipc_call(
    caller_pid: usize,
    target_cap: CapabilityToken,
    msg: IpcMsg,
    caps: &CapabilityBroker,
    sched: &mut Scheduler,
    bus: &mut IpcBus,
) -> Result<IpcMsg, IpcError> {
    let (endpoint_id, target_owner) = caps
        .token_owner(target_cap, CapabilityRights::SEND)
        .map_err(|_| IpcError::InvalidCapability)?;
    let fastpath_eligible = caps.check(target_cap, CapabilityRights::SEND).is_ok()
        && sched.is_blocked_on_recv(target_owner)
        && msg.word_count <= message::IPC_REG_WORDS as u32;

    if fastpath_eligible {
        // FASTPATH: will bypass scheduler in Phase 4. For now this falls through.
    }

    let mut should_enqueue = false;
    if let Some(process) = sched.process_mut_by_pid(caller_pid) {
        if let Some(reply) = process.ipc_reply.take() {
            process.pending_call = None;
            return Ok(reply);
        }
        if process.pending_call.is_none() {
            process.pending_call = Some((target_cap.0, msg));
            should_enqueue = true;
        }
        process.state = ProcessState::BlockedOnIpc;
    }
    if should_enqueue {
        bus.enqueue_call(endpoint_id, msg, caller_pid, sched, target_owner);
    }

    Err(IpcError::WouldBlock)
}

pub fn handle_ipc_recv(
    receiver_pid: usize,
    endpoint_id: u32,
    sched: &mut Scheduler,
    bus: &mut IpcBus,
) -> Result<IpcMsg, IpcError> {
    if let Some(msg) = bus.pop_pending(endpoint_id) {
        return Ok(msg);
    }
    bus.block_on_recv(endpoint_id, receiver_pid, sched);
    Err(IpcError::WouldBlock)
}

pub fn handle_ipc_reply(
    server_pid: usize,
    reply: IpcMsg,
    sched: &mut Scheduler,
    bus: &mut IpcBus,
) -> Result<(), IpcError> {
    let endpoint_id = sched
        .processes
        .iter()
        .find(|p| p.pid == server_pid)
        .and_then(|p| p.ipc_endpoint)
        .ok_or(IpcError::InvalidArgument)?;
    let Some(client_pid) = bus.reply_waiter_pop_front(endpoint_id) else {
        return Err(IpcError::WouldBlock);
    };
    if let Some(client) = sched.process_mut_by_pid(client_pid) {
        client.ipc_reply = Some(reply);
        client.pending_call = None;
        client.state = ProcessState::Ready;
    }
    Ok(())
}

pub fn handle_ipc_reply_wait(
    server_pid: usize,
    endpoint_id: u32,
    reply: IpcMsg,
    sched: &mut Scheduler,
    bus: &mut IpcBus,
) -> Result<IpcMsg, IpcError> {
    let already_waiting = sched
        .processes
        .iter()
        .find(|p| p.pid == server_pid)
        .is_some_and(|p| p.pending_reply_wait.is_some());

    if !already_waiting {
        if let Some(client_pid) = bus.reply_waiter_pop_front(endpoint_id) {
            if let Some(client) = sched.process_mut_by_pid(client_pid) {
                client.ipc_reply = Some(reply);
                client.pending_call = None;
                client.state = ProcessState::Ready;
            }
        }
    }

    if let Some(server) = sched.process_mut_by_pid(server_pid) {
        if let Some(msg) = bus.pop_pending(endpoint_id) {
            server.ipc_endpoint = Some(endpoint_id);
            server.pending_reply_wait = None;
            return Ok(msg);
        }

        if server.pending_reply_wait.is_none() {
            server.pending_reply_wait = Some((endpoint_id, reply));
        }
    }

    bus.block_on_recv(endpoint_id, server_pid, sched);
    Err(IpcError::WouldBlock)
}
