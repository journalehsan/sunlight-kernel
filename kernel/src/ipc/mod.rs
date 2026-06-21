pub mod message;

use crate::capability::{CapabilityBroker, CapabilityRights, CapabilityToken};
use crate::process::ProcessState;
use crate::sched::Scheduler;
use alloc::collections::VecDeque;

pub use message::IpcMsg;

pub const INIT_NAMESERVER_ENDPOINT: u32 = 0;

#[allow(non_snake_case)]
pub mod SpawnMsg {
    pub const SPAWN: u64 = 1;
    pub const REPLY: u64 = 2;
    pub const ERROR: u64 = 3;
}

/// Global IPC bus instance.
pub static IPC_BUS: spin::Mutex<IpcBus> = spin::Mutex::new(IpcBus::new());

/// Errors returned by IPC operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidCapability = 1,
    EndpointNotFound = 2,
    WouldBlock = 3,
    InvalidArgument = 4,
    /// `IpcMsg::word_count` exceeds `IPC_MAX_WORDS` (forged register value).
    InvalidWordCount = 5,
    /// `IpcMsg::cap_count` exceeds `IPC_MAX_CAPS` (forged register value).
    InvalidCapCount = 6,
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
        let idx = self
            .reply_waiters
            .iter()
            .position(|(id, _)| *id == endpoint_id);
        if let Some(idx) = idx {
            return &mut self.reply_waiters[idx].1;
        }
        self.reply_waiters.push((endpoint_id, VecDeque::new()));
        let last = self.reply_waiters.len() - 1;
        &mut self.reply_waiters[last].1
    }

    pub fn endpoint_owner(&self, token: CapabilityToken, caps: &CapabilityBroker) -> Option<usize> {
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

    pub fn block_on_recv(&mut self, endpoint_id: u32, receiver_pid: usize, sched: &mut Scheduler) {
        let global_tick = sched.global_tick;
        if let Some(idx) = sched.processes.iter().position(|p| p.pid == receiver_pid) {
            sched.processes[idx].ipc_endpoint = Some(endpoint_id);
            sched.account_and_apply_churn_penalty();
            sched.processes[idx].state = ProcessState::BlockedOnIpc;
            sched.processes[idx].block_start_tick = global_tick;
        }
        if let Some(idx) = sched.processes.iter().position(|p| p.pid == receiver_pid) {
            sched.remove_from_ready_queues(idx);
        }
    }

    pub fn pop_pending(&mut self, endpoint_id: u32) -> Option<IpcMsg> {
        self.queue_for(endpoint_id).pop_front()
    }

    pub fn reply_waiter_pop_front(&mut self, endpoint_id: u32) -> Option<usize> {
        self.reply_waiters_for(endpoint_id).pop_front()
    }

    pub fn reply_waiter_remove(&mut self, endpoint_id: u32, caller_pid: usize) -> bool {
        let waiters = self.reply_waiters_for(endpoint_id);
        let mut removed = false;
        let mut kept = VecDeque::new();
        while let Some(pid) = waiters.pop_front() {
            if pid == caller_pid {
                removed = true;
            } else {
                kept.push_back(pid);
            }
        }
        *waiters = kept;
        removed
    }

    pub fn remove_pending_calls_for(&mut self, endpoint_id: u32, caller_pid: usize) -> usize {
        let queue = self.queue_for(endpoint_id);
        let mut removed = 0usize;
        let mut kept = VecDeque::new();
        while let Some(msg) = queue.pop_front() {
            if msg.badge == caller_pid as u64 {
                removed += 1;
            } else {
                kept.push_back(msg);
            }
        }
        *queue = kept;
        removed
    }

    pub fn pending_count(&self, endpoint_id: u32) -> usize {
        self.queues
            .iter()
            .find(|(id, _)| *id == endpoint_id)
            .map_or(0, |(_, q)| q.len())
    }

    pub fn pending_callers_count(&self, endpoint_id: u32) -> usize {
        self.reply_waiters
            .iter()
            .find(|(id, _)| *id == endpoint_id)
            .map_or(0, |(_, q)| q.len())
    }

    pub fn has_pending_call_from(&self, endpoint_id: u32, caller_pid: usize) -> bool {
        self.reply_waiters
            .iter()
            .find(|(id, _)| *id == endpoint_id)
            .is_some_and(|(_, q)| q.iter().any(|pid| *pid == caller_pid))
    }

    pub fn waiting_receiver_pid(&self, endpoint_id: u32, sched: &Scheduler) -> Option<usize> {
        sched.processes.iter().find_map(|p| {
            if p.state == ProcessState::BlockedOnIpc
                && p.ipc_endpoint == Some(endpoint_id)
                && p.pending_call.is_none()
            {
                Some(p.pid)
            } else {
                None
            }
        })
    }

    pub fn send_timer_tick(&mut self, endpoint_id: u32, sched: &mut Scheduler, server_pid: usize) {
        let msg = IpcMsg::with_label(0x1);
        self.queue_for(endpoint_id).push_back(msg);
        sched.wake_pid(server_pid);
    }

    pub fn send_keyboard_event(
        &mut self,
        endpoint_id: u32,
        event_val: u64,
        sched: &mut Scheduler,
        server_pid: usize,
    ) {
        let msg = IpcMsg::with_label(0x1).word(0, event_val);
        self.queue_for(endpoint_id).push_back(msg);
        sched.wake_pid(server_pid);
    }

    pub fn remove_endpoint(&mut self, endpoint_id: u32) {
        self.queues.retain(|(id, _)| *id != endpoint_id);
        self.reply_waiters.retain(|(id, _)| *id != endpoint_id);
    }

    pub fn remove_pid_references(&mut self, pid: usize) {
        for (_, queue) in self.queues.iter_mut() {
            queue.retain(|msg| msg.badge != pid as u64);
        }
        for (_, waiters) in self.reply_waiters.iter_mut() {
            waiters.retain(|waiter| *waiter != pid);
        }
    }
}

fn set_reply_target(sched: &mut Scheduler, server_pid: usize, endpoint_id: u32, caller_pid: usize) {
    if let Some(server) = sched.process_mut_by_pid(server_pid) {
        server.ipc_endpoint = Some(endpoint_id);
        server.pending_reply_wait = None;
        server.ipc_reply_target = if caller_pid == 0 {
            None
        } else {
            Some((endpoint_id, caller_pid))
        };
    }
}

fn deliver_reply_to_current_target(
    server_pid: usize,
    reply: IpcMsg,
    sched: &mut Scheduler,
    bus: &mut IpcBus,
) -> Result<(), IpcError> {
    let Some((endpoint_id, client_pid)) = sched
        .processes
        .iter_mut()
        .find(|p| p.pid == server_pid)
        .and_then(|p| p.ipc_reply_target.take())
    else {
        return Ok(());
    };

    let _ = bus.reply_waiter_remove(endpoint_id, client_pid);

    let Some(client_idx) = sched.processes.iter().position(|p| p.pid == client_pid) else {
        crate::serial_println!(
            "[IPC] late reply dropped caller={} server={} ep={} label={:#x}",
            client_pid,
            server_pid,
            endpoint_id,
            reply.label
        );
        return Ok(());
    };

    let client = &mut sched.processes[client_idx];
    if client.pending_call.is_none() {
        client.ipc_reply = None;
        crate::serial_println!(
            "[IPC] late reply dropped caller={} server={} ep={} label={:#x}",
            client_pid,
            server_pid,
            endpoint_id,
            reply.label
        );
        return Ok(());
    }

    client.ipc_reply = Some(reply);
    client.pending_call = None;
    if client.state == ProcessState::BlockedOnIpc {
        client.state = ProcessState::Ready;
        sched.enqueue_process_once(client_idx);
    }
    Ok(())
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
    let global_tick = sched.global_tick;
    if let Some(idx) = sched.processes.iter().position(|p| p.pid == caller_pid) {
        if sched.processes[idx].pending_reply_wait.is_some() {
            // unlikely path, but handle
        }
        if let Some(reply) = sched.processes[idx].ipc_reply.take() {
            sched.processes[idx].pending_call = None;
            return Ok(reply);
        }
        if sched.processes[idx].pending_call.is_none() {
            sched.processes[idx].pending_call = Some((target_cap.0, msg));
            should_enqueue = true;
        } else if !bus.has_pending_call_from(endpoint_id, caller_pid) {
            // A retrying client must never be left blocked with only its
            // per-process pending_call set. That state means no server can
            // receive the request, so repair it by re-queueing the original
            // call once. Duplicate retries while the message is queued are
            // suppressed by has_pending_call_from().
            should_enqueue = true;
        }
        // CPU accounting + churn penalty + runnable queue hygiene when blocking on IPC
        sched.account_and_apply_churn_penalty();
        sched.processes[idx].state = ProcessState::BlockedOnIpc;
        sched.processes[idx].block_start_tick = global_tick;
    }
    // Best-effort: remove the caller from queues by pid (Phase 2).
    if let Some(idx) = sched.processes.iter().position(|p| p.pid == caller_pid) {
        sched.remove_from_ready_queues(idx);
    }
    if should_enqueue {
        bus.enqueue_call(endpoint_id, msg, caller_pid, sched, target_owner);
    }

    if sched.processes.iter().any(|p| {
        p.pid == target_owner
            && p.state == ProcessState::BlockedOnIpc
            && p.ipc_endpoint == Some(endpoint_id)
    }) && bus.pending_count(endpoint_id) > 0
    {
        sched.wake_pid(target_owner);
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
        set_reply_target(sched, receiver_pid, endpoint_id, msg.badge as usize);
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
    deliver_reply_to_current_target(server_pid, reply, sched, bus)
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
        deliver_reply_to_current_target(server_pid, reply, sched, bus)?;
    }

    if let Some(server) = sched.process_mut_by_pid(server_pid) {
        if let Some(msg) = bus.pop_pending(endpoint_id) {
            server.ipc_reply_target = if msg.badge == 0 {
                None
            } else {
                Some((endpoint_id, msg.badge as usize))
            };
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

pub fn handle_ipc_cancel(
    caller_pid: usize,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
    bus: &mut IpcBus,
) -> Result<(), IpcError> {
    let Some(idx) = sched.processes.iter().position(|p| p.pid == caller_pid) else {
        return Err(IpcError::InvalidArgument);
    };

    let pending = sched.processes[idx].pending_call;
    sched.processes[idx].ipc_reply = None;

    let Some((target_cap, _msg)) = pending else {
        sched.processes[idx].pending_call = None;
        return Ok(());
    };

    let endpoint_id = caps
        .check(CapabilityToken(target_cap), CapabilityRights::SEND)
        .map_err(|_| IpcError::InvalidCapability)?;

    bus.remove_pending_calls_for(endpoint_id, caller_pid);
    bus.reply_waiter_remove(endpoint_id, caller_pid);
    sched.processes[idx].pending_call = None;
    Ok(())
}
