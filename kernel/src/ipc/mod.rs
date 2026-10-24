pub mod message;

use crate::capability::{CapabilityBroker, CapabilityRights, CapabilityToken};
use crate::process::{
    DeferredIpcReply, IpcCallId, IpcCallOutcome, IpcReplyTarget, PendingIpcCall, ProcessState,
};
use crate::sched::Scheduler;
use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub use message::IpcMsg;
pub use sunlight_ipc::ServiceCapability;

pub const INIT_NAMESERVER_ENDPOINT: u32 = 0;
const NAMESERVER_REGISTER: u64 = 1;
/// A 104-byte register message makes this about 6.5 KiB of payload metadata per
/// saturated endpoint. Existing boot/services traffic stays far below this.
pub const ENDPOINT_QUEUE_CAPACITY: usize = 64;
/// A server may retain a bounded number of asynchronous reply targets. This
/// keeps deferred service waits from consuming unbounded kernel memory.
pub const DEFERRED_REPLY_CAPACITY: usize = 64;
const NUM_SHARDS: usize = 16;

#[derive(Clone, Copy, Debug)]
struct PtyIpcEvent {
    tick: u64,
    pid: usize,
    kind: &'static str,
    generation: u64,
    endpoint: u32,
    label: u64,
    bytes: u64,
    depth: usize,
    owner: u8,
    memory: [u64; 5],
}

struct PtyIpcTrace {
    events: [Option<PtyIpcEvent>; 32],
    next: usize,
    slow_operations: [Option<PtyIpcEvent>; 8],
    slow_next: usize,
    max_mmap_hold: Option<PtyIpcEvent>,
}

static PTY_IPC_TRACE: spin::Mutex<PtyIpcTrace> = spin::Mutex::new(PtyIpcTrace {
    events: [None; 32],
    next: 0,
    slow_operations: [None; 8],
    slow_next: 0,
    max_mmap_hold: None,
});

fn trace_pty_event(
    sched: &Scheduler,
    idx: usize,
    kind: &'static str,
    generation: u64,
    endpoint: u32,
    msg: IpcMsg,
    depth: usize,
) {
    if option_env!("SUNLIGHT_PTY_TRACE") != Some("1")
        || !sched.processes[idx].name_str().starts_with("sshl")
    {
        return;
    }
    let mut trace = PTY_IPC_TRACE.lock();
    let next = trace.next;
    let event = PtyIpcEvent {
        tick: sched.global_tick,
        pid: sched.processes[idx].pid,
        kind,
        generation,
        endpoint,
        label: msg.label,
        bytes: msg.words[2],
        depth,
        owner: sched.processes[idx].owning_core,
        memory: [
            msg.words[0],
            msg.words[1],
            msg.words[3],
            msg.words[4],
            msg.words[5],
        ],
    };
    trace.events[next] = Some(event);
    trace.next = (next + 1) % trace.events.len();
    if kind == "mmap"
        && trace
            .max_mmap_hold
            .is_none_or(|previous| event.bytes > previous.bytes)
    {
        trace.max_mmap_hold = Some(event);
    }
    if matches!(kind, "mmap" | "spawn") {
        let slow_next = trace.slow_next;
        trace.slow_operations[slow_next] = Some(event);
        trace.slow_next = (slow_next + 1) % trace.slow_operations.len();
    }
}

/// Record slow startup syscalls without printing while scheduler locks are held.
/// For these event kinds generation/bytes are lock-wait/hold microseconds.
pub(crate) fn trace_pty_operation(
    sched: &Scheduler,
    kind: &'static str,
    wait_ns: u64,
    hold_ns: u64,
    memory: [u64; 5],
) {
    if wait_ns.saturating_add(hold_ns) < 50_000_000 {
        return;
    }
    if let Some(idx) = sched.current_process_index() {
        let mut msg = IpcMsg::empty();
        msg.words[0] = memory[0];
        msg.words[1] = memory[1];
        msg.words[2] = hold_ns / 1000;
        msg.words[3..6].copy_from_slice(&memory[2..5]);
        trace_pty_event(sched, idx, kind, wait_ns / 1000, 0, msg, 0);
    }
}

/// One snapshot at a guest-test failure, never logging on the hot IPC path.
pub(crate) fn trace_pty_liveness(sched: &Scheduler) {
    crate::serial_println!(
        "[PTY-DIAG] dispatch_notifications sent/busy={:?}",
        crate::sched::diagnostic_dispatch_notifications()
    );
    for cpu in 0..sched.online_cores.min(16) {
        crate::serial_println!(
            "[PTY-DIAG] cpu={} current_pid={:?}",
            cpu,
            sched.diagnostic_current_pid(cpu)
        );
    }
    let counters = diagnostic_snapshot();
    crate::serial_println!(
        "[PTY-DIAG] enqueued={} dequeued={} depth={} high={} late={} deadlines={}",
        counters.enqueue_count,
        counters.dequeue_count,
        counters.current_queue_depth,
        counters.high_watermark,
        counters.late_reply_drop_count,
        counters.deadline_expired_count
    );
    {
        let trace = PTY_IPC_TRACE.lock();
        if let Some(e) = trace.max_mmap_hold {
            crate::serial_println!("[PTY-MMAP] tick={} pid={} wait_us={} hold_max_us={} reclaimed={} scans={} activations={}",
                e.tick, e.pid, e.generation, e.bytes, e.memory[3], e.memory[2], e.memory[4]);
        }
        for offset in 0..trace.slow_operations.len() {
            if let Some(e) =
                trace.slow_operations[(trace.slow_next + offset) % trace.slow_operations.len()]
            {
                crate::serial_println!("[PTY-STARTUP] tick={} pid={} kind={} wait_us={} hold_us={} owner={} free_before={} free_after={} reclaim_scans={} reclaimed={} activations={}", e.tick, e.pid, e.kind, e.generation, e.bytes, e.owner, e.memory[0], e.memory[1], e.memory[2], e.memory[3], e.memory[4]);
            }
        }
        for offset in 0..trace.events.len() {
            if let Some(e) = trace.events[(trace.next + offset) % trace.events.len()] {
                crate::serial_println!("[PTY-EVENT] tick={} pid={} kind={} gen={} ep={} label={:#x} bytes={} depth={} owner={}",
                    e.tick, e.pid, e.kind, e.generation, e.endpoint, e.label, e.bytes, e.depth, e.owner);
            }
        }
    }
    for (idx, p) in sched
        .processes
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            (p.name_str().starts_with("sshl") || p.name_str() == "pty_server")
                && (!matches!(p.state, ProcessState::Finished | ProcessState::Reaped)
                    || p.exit_cleanup_pending)
        })
        .take(16)
    {
        let (input, output) =
            crate::process::tty_io::diagnostic_occupancy(p.tty_tab.unwrap_or(0) as usize);
        crate::serial_println!(
            "[PTY-DIAG] pid={} ppid={} name={} state={:?} owner={} queued={} tab={:?} rings={}/{} wait_child={:?} pending_present={} outcome={:?} target={:?} reply_wait_present={} blocked_ticks={} last_run_age={} nice={} burst={} queue_copies={}",
            p.pid, p.ppid, p.name_str(), p.state, p.owning_core, p.queued_on_core,
            p.tty_tab, input, output, p.wait_child, p.pending_call.is_some(), p.ipc_call_outcome,
            p.ipc_reply_target, p.pending_reply_wait.is_some(),
            sched.global_tick.saturating_sub(p.block_start_tick),
            sched.global_tick.saturating_sub(p.last_run_tick), p.nice, p.burst_score,
            sched.diagnostic_ready_occurrences(idx)
        );
        if let Some(ep) = p.ipc_endpoint {
            with_shard(ep, |bus| {
                crate::serial_println!(
                    "[PTY-DIAG] ep={} depth={} callers={} receiver={:?}",
                    ep,
                    bus.pending_count(ep),
                    bus.pending_callers_count(ep),
                    bus.waiting_receiver_pid(ep, sched)
                );
            });
        }
        if let Some((ring, writing)) = p.native_pipe_wait {
            crate::serial_println!(
                "[PTY-DIAG] pid={} native_pipe={} writing={} queued/readers/writers={:?}",
                p.pid,
                ring,
                writing,
                crate::process::pipe::pipe_state(ring)
            );
        }
        if let Some(tab) = p.tty_write_wait {
            crate::serial_println!("[PTY-DIAG] pid={} tty_write_wait={}", p.pid, tab);
        }
    }
}

/// A single test-only snapshot if the driver cannot reach its own failure path.
/// Normal streams reset their block/run age; this never logs per request.
pub(crate) fn trace_pty_stall(sched: &Scheduler) {
    if option_env!("SUNLIGHT_PTY_TRACE") != Some("1")
        || !matches!(
            option_env!("SUNLIGHT_INJECT_PHASE"),
            Some("shell-pty-liveness" | "shell-phase2")
        )
        || !sched.global_tick.is_multiple_of(100)
    {
        return;
    }
    static REPORTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    // The injected test is the child of the top-level tab-0 shell. Probe
    // children also inherit tab 0 and newly spawned tasks have last_run_tick=0.
    let Some(root_shell_pid) = sched
        .processes
        .iter()
        .find(|p| p.ppid == 1 && p.name_str() == "sshl" && p.tty_tab == Some(0))
        .map(|p| p.pid)
    else {
        return;
    };
    let stalled = sched.processes.iter().any(|p| {
        p.name_str() == "sshl"
            && p.ppid == root_shell_pid
            && p.tty_tab == Some(0)
            && match p.state {
                ProcessState::BlockedOnIpc | ProcessState::BlockedOnTimer => {
                    sched.global_tick.saturating_sub(p.block_start_tick) >= 500
                }
                ProcessState::Ready => {
                    p.last_run_tick != 0 && sched.global_tick.saturating_sub(p.last_run_tick) >= 500
                }
                _ => false,
            }
    });
    if stalled && !REPORTED.swap(true, Ordering::Relaxed) {
        crate::serial_println!("[PTY-DIAG] driver stalled for 500 ticks; one watchdog snapshot");
        trace_pty_liveness(sched);
        for p in sched
            .processes
            .iter()
            .filter(|p| {
                p.name_str().starts_with("sshl")
                    && matches!(p.state, ProcessState::Finished | ProcessState::Reaped)
            })
            .take(16)
        {
            crate::serial_println!("[PTY-DIAG] terminal pid={} name={} state={:?} owner={} queued={} cleanup={} exit={}",
                p.pid, p.name_str(), p.state, p.owning_core, p.queued_on_core,
                p.exit_cleanup_pending, p.exit_code);
        }
    }
}

#[allow(non_snake_case)]
pub mod SpawnMsg {
    pub const SPAWN: u64 = 1;
    pub const REPLY: u64 = 2;
    pub const ERROR: u64 = 3;
    pub const SPAWN_AUTHENTICATED: u64 = 4;
}

/// Mutex-protected endpoint shards. Scheduler-mediated calls still take the
/// global scheduler lock; sharding does not make this path lock-free.
pub static IPC_BUS_SHARDS: [spin::Mutex<IpcBusShard>; NUM_SHARDS] = [
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
    spin::Mutex::new(IpcBusShard::new()),
];

#[inline]
pub fn shard_for(endpoint_id: u32) -> usize {
    (endpoint_id as usize) % NUM_SHARDS
}

/// Errors returned by IPC operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidCapability = 1,
    EndpointNotFound = 2,
    WouldBlock = 3,
    InvalidArgument = 4,
    InvalidWordCount = 5,
    InvalidCapCount = 6,
    QueueFull = 7,
    DeadlineExpired = 8,
    Cancelled = 9,
    PeerClosed = 10,
}

pub struct IpcDiagnosticSnapshot {
    pub current_queue_depth: usize,
    pub high_watermark: usize,
    pub enqueue_count: u64,
    pub dequeue_count: u64,
    pub queue_full_count: u64,
    pub coalesced_notification_count: u64,
    pub deadline_expired_count: u64,
    pub explicit_cancel_count: u64,
    pub late_reply_drop_count: u64,
    pub peer_closed_wake_count: u64,
    pub unauthorized_register_count: u64,
    pub conflicting_live_registration_count: u64,
    pub stale_registration_removal_count: u64,
    pub successful_dead_replacement_count: u64,
    pub stale_lookup_count: u64,
    pub registry_full_rejection_count: u64,
    pub send_only_management_reject_count: u64,
}

static CURRENT_QUEUE_DEPTH: AtomicUsize = AtomicUsize::new(0);
static HIGH_WATERMARK: AtomicUsize = AtomicUsize::new(0);
static ENQUEUE_COUNT: AtomicU64 = AtomicU64::new(0);
static DEQUEUE_COUNT: AtomicU64 = AtomicU64::new(0);
static QUEUE_FULL_COUNT: AtomicU64 = AtomicU64::new(0);
static COALESCED_NOTIFICATION_COUNT: AtomicU64 = AtomicU64::new(0);
static DEADLINE_EXPIRED_COUNT: AtomicU64 = AtomicU64::new(0);
static EXPLICIT_CANCEL_COUNT: AtomicU64 = AtomicU64::new(0);
static LATE_REPLY_DROP_COUNT: AtomicU64 = AtomicU64::new(0);
static PEER_CLOSED_WAKE_COUNT: AtomicU64 = AtomicU64::new(0);
static UNAUTHORIZED_REGISTER_COUNT: AtomicU64 = AtomicU64::new(0);
static CONFLICTING_LIVE_REGISTRATION_COUNT: AtomicU64 = AtomicU64::new(0);
static STALE_REGISTRATION_REMOVAL_COUNT: AtomicU64 = AtomicU64::new(0);
static SUCCESSFUL_DEAD_REPLACEMENT_COUNT: AtomicU64 = AtomicU64::new(0);
static STALE_LOOKUP_COUNT: AtomicU64 = AtomicU64::new(0);
static REGISTRY_FULL_REJECTION_COUNT: AtomicU64 = AtomicU64::new(0);
static SEND_ONLY_MANAGEMENT_REJECT_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn note_send_only_management_reject() {
    SEND_ONLY_MANAGEMENT_REJECT_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn note_nameserver_diagnostic(event: u64) {
    match event {
        1 => CONFLICTING_LIVE_REGISTRATION_COUNT.fetch_add(1, Ordering::Relaxed),
        2 => STALE_REGISTRATION_REMOVAL_COUNT.fetch_add(1, Ordering::Relaxed),
        3 => SUCCESSFUL_DEAD_REPLACEMENT_COUNT.fetch_add(1, Ordering::Relaxed),
        4 => STALE_LOOKUP_COUNT.fetch_add(1, Ordering::Relaxed),
        5 => REGISTRY_FULL_REJECTION_COUNT.fetch_add(1, Ordering::Relaxed),
        _ => return,
    };
}

pub fn diagnostic_snapshot() -> IpcDiagnosticSnapshot {
    IpcDiagnosticSnapshot {
        current_queue_depth: CURRENT_QUEUE_DEPTH.load(Ordering::Relaxed),
        high_watermark: HIGH_WATERMARK.load(Ordering::Relaxed),
        enqueue_count: ENQUEUE_COUNT.load(Ordering::Relaxed),
        dequeue_count: DEQUEUE_COUNT.load(Ordering::Relaxed),
        queue_full_count: QUEUE_FULL_COUNT.load(Ordering::Relaxed),
        coalesced_notification_count: COALESCED_NOTIFICATION_COUNT.load(Ordering::Relaxed),
        deadline_expired_count: DEADLINE_EXPIRED_COUNT.load(Ordering::Relaxed),
        explicit_cancel_count: EXPLICIT_CANCEL_COUNT.load(Ordering::Relaxed),
        late_reply_drop_count: LATE_REPLY_DROP_COUNT.load(Ordering::Relaxed),
        peer_closed_wake_count: PEER_CLOSED_WAKE_COUNT.load(Ordering::Relaxed),
        unauthorized_register_count: UNAUTHORIZED_REGISTER_COUNT.load(Ordering::Relaxed),
        conflicting_live_registration_count: CONFLICTING_LIVE_REGISTRATION_COUNT
            .load(Ordering::Relaxed),
        stale_registration_removal_count: STALE_REGISTRATION_REMOVAL_COUNT.load(Ordering::Relaxed),
        successful_dead_replacement_count: SUCCESSFUL_DEAD_REPLACEMENT_COUNT
            .load(Ordering::Relaxed),
        stale_lookup_count: STALE_LOOKUP_COUNT.load(Ordering::Relaxed),
        registry_full_rejection_count: REGISTRY_FULL_REJECTION_COUNT.load(Ordering::Relaxed),
        send_only_management_reject_count: SEND_ONLY_MANAGEMENT_REJECT_COUNT
            .load(Ordering::Relaxed),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageKind {
    Ordinary(IpcCallId),
    Timer,
    Input,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingMessage {
    pub msg: IpcMsg,
    pub call: Option<IpcCallId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotificationStatus {
    None,
    Queued,
    Deferred,
}

struct QueuedMessage {
    msg: IpcMsg,
    kind: MessageKind,
}

struct EndpointQueue {
    messages: VecDeque<QueuedMessage>,
    timer_status: NotificationStatus,
    timer_base_tick: Option<u64>,
    timer_latest_tick: u64,
    input_status: NotificationStatus,
}

impl EndpointQueue {
    fn new() -> Self {
        Self {
            messages: VecDeque::new(),
            timer_status: NotificationStatus::None,
            timer_base_tick: None,
            timer_latest_tick: 0,
            input_status: NotificationStatus::None,
        }
    }

    fn timer_sequence(&self) -> u64 {
        self.timer_base_tick
            .map_or(0, |base| self.timer_latest_tick.saturating_sub(base) + 1)
    }
}

/// Per-shard IPC bus with O(log n) endpoint lookup.
pub struct IpcBusShard {
    queues: BTreeMap<u32, EndpointQueue>,
    reply_waiters: BTreeMap<u32, VecDeque<IpcCallId>>,
}

impl IpcBusShard {
    pub const fn new() -> Self {
        Self {
            queues: BTreeMap::new(),
            reply_waiters: BTreeMap::new(),
        }
    }

    fn queue_for(&mut self, endpoint_id: u32) -> &mut EndpointQueue {
        self.queues
            .entry(endpoint_id)
            .or_insert_with(EndpointQueue::new)
    }

    fn reply_waiters_for(&mut self, endpoint_id: u32) -> &mut VecDeque<IpcCallId> {
        self.reply_waiters
            .entry(endpoint_id)
            .or_insert_with(VecDeque::new)
    }

    pub fn endpoint_owner(&self, token: CapabilityToken, caps: &CapabilityBroker) -> Option<usize> {
        let endpoint_id = caps.check(token, CapabilityRights::SEND).ok()?;
        caps.endpoint_owner(endpoint_id)
    }

    pub fn enqueue_call(
        &mut self,
        endpoint_id: u32,
        mut msg: IpcMsg,
        call: IpcCallId,
    ) -> Result<(), IpcError> {
        let queue = self.queue_for(endpoint_id);
        if queue.messages.len() >= ENDPOINT_QUEUE_CAPACITY {
            QUEUE_FULL_COUNT.fetch_add(1, Ordering::Relaxed);
            return Err(IpcError::QueueFull);
        }
        msg.badge = call.pid as u64;
        queue.messages.push_back(QueuedMessage {
            msg,
            kind: MessageKind::Ordinary(call),
        });
        record_enqueue(queue.messages.len());
        let waiters = self.reply_waiters_for(endpoint_id);
        if !waiters.iter().any(|waiter| *waiter == call) {
            waiters.push_back(call);
        }
        Ok(())
    }

    pub fn block_on_recv(&mut self, endpoint_id: u32, receiver_pid: usize, sched: &mut Scheduler) {
        let global_tick = sched.global_tick;
        if let Some(idx) = sched.processes.iter().position(|p| p.pid == receiver_pid) {
            if sched.processes[idx].is_exiting() {
                return;
            }
            sched.processes[idx].ipc_endpoint = Some(endpoint_id);
            sched.account_and_apply_churn_penalty();
            sched.set_state(idx, ProcessState::BlockedOnIpc);
            sched.processes[idx].block_start_tick = global_tick;
        }
        if let Some(idx) = sched.processes.iter().position(|p| p.pid == receiver_pid) {
            sched.remove_from_ready_queues(idx);
        }
    }

    pub fn pop_pending(&mut self, endpoint_id: u32) -> Option<PendingMessage> {
        let queue = self.queues.get_mut(&endpoint_id)?;
        let queued = queue.messages.pop_front()?;
        CURRENT_QUEUE_DEPTH.fetch_sub(1, Ordering::Relaxed);
        DEQUEUE_COUNT.fetch_add(1, Ordering::Relaxed);
        let (mut msg, call) = match queued.kind {
            MessageKind::Ordinary(call) => (queued.msg, Some(call)),
            MessageKind::Timer => {
                queue.timer_status = NotificationStatus::None;
                let mut msg = queued.msg;
                msg.words[0] = queue.timer_sequence();
                msg.word_count = 1;
                (msg, None)
            }
            MessageKind::Input => {
                queue.input_status = NotificationStatus::None;
                (queued.msg, None)
            }
        };
        materialize_deferred(queue);
        if call.is_none() {
            msg.badge = 0;
        }
        Some(PendingMessage { msg, call })
    }

    pub fn reply_waiter_pop_front(&mut self, endpoint_id: u32) -> Option<IpcCallId> {
        self.reply_waiters.get_mut(&endpoint_id)?.pop_front()
    }

    pub fn reply_waiter_remove(&mut self, endpoint_id: u32, call: IpcCallId) -> bool {
        let Some(waiters) = self.reply_waiters.get_mut(&endpoint_id) else {
            return false;
        };
        let mut removed = false;
        let mut kept = VecDeque::new();
        while let Some(waiter) = waiters.pop_front() {
            if waiter == call {
                removed = true;
            } else {
                kept.push_back(waiter);
            }
        }
        *waiters = kept;
        removed
    }

    pub fn remove_pending_call(&mut self, endpoint_id: u32, call: IpcCallId) -> bool {
        let Some(queue) = self.queues.get_mut(&endpoint_id) else {
            return false;
        };
        let mut removed = false;
        let mut kept = VecDeque::new();
        while let Some(msg) = queue.messages.pop_front() {
            if msg.kind == MessageKind::Ordinary(call) {
                removed = true;
                CURRENT_QUEUE_DEPTH.fetch_sub(1, Ordering::Relaxed);
            } else {
                kept.push_back(msg);
            }
        }
        queue.messages = kept;
        materialize_deferred(queue);
        removed
    }

    pub fn pending_count(&self, endpoint_id: u32) -> usize {
        self.queues
            .get(&endpoint_id)
            .map_or(0, |q| q.messages.len())
    }

    pub fn pending_callers_count(&self, endpoint_id: u32) -> usize {
        self.reply_waiters.get(&endpoint_id).map_or(0, |w| w.len())
    }

    pub fn has_pending_call(&self, endpoint_id: u32, call: IpcCallId) -> bool {
        self.reply_waiters
            .get(&endpoint_id)
            .is_some_and(|q| q.iter().any(|waiter| *waiter == call))
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

    pub fn send_timer_tick(&mut self, endpoint_id: u32, global_tick: u64) {
        let queue = self.queue_for(endpoint_id);
        queue.timer_base_tick.get_or_insert(global_tick);
        queue.timer_latest_tick = global_tick;
        if queue.timer_status != NotificationStatus::None {
            COALESCED_NOTIFICATION_COUNT.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if queue.messages.len() < ENDPOINT_QUEUE_CAPACITY {
            queue.messages.push_back(QueuedMessage {
                msg: IpcMsg::with_label(0x1).word(0, queue.timer_sequence()),
                kind: MessageKind::Timer,
            });
            queue.timer_status = NotificationStatus::Queued;
            record_enqueue(queue.messages.len());
        } else {
            queue.timer_status = NotificationStatus::Deferred;
        }
    }

    pub fn send_input_notification(&mut self, endpoint_id: u32) {
        let queue = self.queue_for(endpoint_id);
        if queue.input_status != NotificationStatus::None {
            COALESCED_NOTIFICATION_COUNT.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if queue.messages.len() < ENDPOINT_QUEUE_CAPACITY {
            queue.messages.push_back(QueuedMessage {
                msg: IpcMsg::with_label(0x1),
                kind: MessageKind::Input,
            });
            queue.input_status = NotificationStatus::Queued;
            record_enqueue(queue.messages.len());
        } else {
            queue.input_status = NotificationStatus::Deferred;
        }
    }

    pub fn remove_endpoint(&mut self, endpoint_id: u32) -> Vec<IpcCallId> {
        if let Some(queue) = self.queues.remove(&endpoint_id) {
            CURRENT_QUEUE_DEPTH.fetch_sub(queue.messages.len(), Ordering::Relaxed);
        }
        self.reply_waiters
            .remove(&endpoint_id)
            .map_or_else(Vec::new, VecDeque::into)
    }

    pub fn remove_pid_references(&mut self, pid: usize) {
        for queue in self.queues.values_mut() {
            let before = queue.messages.len();
            queue.messages.retain(
                |queued| !matches!(queued.kind, MessageKind::Ordinary(call) if call.pid == pid),
            );
            CURRENT_QUEUE_DEPTH.fetch_sub(before - queue.messages.len(), Ordering::Relaxed);
            materialize_deferred(queue);
        }
        for waiters in self.reply_waiters.values_mut() {
            waiters.retain(|waiter| waiter.pid != pid);
        }
    }

    /// Count pending messages and waiter entries that reference this pid (best effort).
    pub fn pending_count_for_pid(&mut self, pid: usize) -> usize {
        let mut n = 0usize;
        for queue in self.queues.values() {
            n += queue
                .messages
                .iter()
                .filter(|msg| matches!(msg.kind, MessageKind::Ordinary(call) if call.pid == pid))
                .count();
        }
        for waiters in self.reply_waiters.values() {
            n += waiters.iter().filter(|w| w.pid == pid).count();
        }
        n
    }

    /// Total number of waiter pids across all endpoints in this shard.
    pub fn total_waiter_count(&self) -> usize {
        let mut n = 0usize;
        for waiters in self.reply_waiters.values() {
            n += waiters.len();
        }
        n
    }
}

impl Drop for IpcBusShard {
    fn drop(&mut self) {
        let remaining = self
            .queues
            .values()
            .map(|queue| queue.messages.len())
            .sum::<usize>();
        if remaining != 0 {
            CURRENT_QUEUE_DEPTH.fetch_sub(remaining, Ordering::Relaxed);
        }
    }
}

fn record_enqueue(endpoint_depth: usize) {
    ENQUEUE_COUNT.fetch_add(1, Ordering::Relaxed);
    CURRENT_QUEUE_DEPTH.fetch_add(1, Ordering::Relaxed);
    HIGH_WATERMARK.fetch_max(endpoint_depth, Ordering::Relaxed);
}

fn materialize_deferred(queue: &mut EndpointQueue) {
    if queue.messages.len() >= ENDPOINT_QUEUE_CAPACITY {
        return;
    }
    if queue.timer_status == NotificationStatus::Deferred {
        queue.messages.push_back(QueuedMessage {
            msg: IpcMsg::with_label(0x1).word(0, queue.timer_sequence()),
            kind: MessageKind::Timer,
        });
        queue.timer_status = NotificationStatus::Queued;
        record_enqueue(queue.messages.len());
    } else if queue.input_status == NotificationStatus::Deferred {
        queue.messages.push_back(QueuedMessage {
            msg: IpcMsg::with_label(0x1),
            kind: MessageKind::Input,
        });
        queue.input_status = NotificationStatus::Queued;
        record_enqueue(queue.messages.len());
    }
}

pub type IpcBus = IpcBusShard;

/// Acquire a shard lock for a given endpoint_id.
#[inline]
pub fn with_shard<F, R>(endpoint_id: u32, f: F) -> R
where
    F: FnOnce(&mut IpcBusShard) -> R,
{
    let shard_idx = shard_for(endpoint_id);
    let mut shard = IPC_BUS_SHARDS[shard_idx].lock();
    f(&mut shard)
}

/// Execute an operation across all shards (for cleanup operations).
pub fn for_all_shards<F>(mut f: F)
where
    F: FnMut(&mut IpcBusShard),
{
    for shard in &IPC_BUS_SHARDS {
        f(&mut shard.lock());
    }
}

fn set_reply_target(
    sched: &mut Scheduler,
    server_pid: usize,
    endpoint_id: u32,
    call: Option<IpcCallId>,
) {
    if let Some(server) = sched.process_mut_by_pid(server_pid) {
        server.ipc_endpoint = Some(endpoint_id);
        server.pending_reply_wait = None;
        server.ipc_reply_target = call.map(|call| IpcReplyTarget { endpoint_id, call });
    }
}

pub(crate) fn cancel_reply_target(
    reply_target: &mut Option<IpcReplyTarget>,
    target: IpcReplyTarget,
) -> bool {
    if *reply_target == Some(target) {
        *reply_target = None;
        true
    } else {
        false
    }
}

fn deliver_reply_to_current_target(
    server_pid: usize,
    reply: IpcMsg,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
    bus: &mut IpcBus,
) -> Result<(), IpcError> {
    let Some(target) = sched
        .processes
        .iter_mut()
        .find(|p| p.pid == server_pid)
        .and_then(|p| p.ipc_reply_target.take())
    else {
        return Ok(());
    };

    #[cfg(feature = "cap_issuance_test")]
    crate::capability::gate::before_delivery(server_pid, target, reply, sched, caps, bus);
    #[cfg(feature = "production_grant_test")]
    crate::capability::production_gate::before_delivery(target, sched, caps, bus);
    let result = deliver_reply_to_target(target, reply, sched, caps, bus);
    #[cfg(feature = "production_grant_test")]
    crate::capability::production_gate::after_delivery(target, caps);
    #[cfg(feature = "scheduler_timer_trace")]
    if result.is_ok() && sched.processes.iter().any(|p| p.pid == target.call.pid && matches!(p.ipc_call_outcome, Some(IpcCallOutcome::ReplyDelivered(g)) if g == target.call.generation)) {
        if let Some(idx) = sched.processes.iter().position(|p| p.pid == server_pid && p.name_str() == "timer_server") {
            crate::scheduler_timer_trace::event(sched, 7, server_pid, target.call.pid as u64, target.call.generation, sched.effective_runtime_at(idx, sched.now_ns()));
        }
    }
    result
}

pub(crate) fn deliver_reply_to_target(
    target: IpcReplyTarget,
    reply: IpcMsg,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
    bus: &mut IpcBus,
) -> Result<(), IpcError> {
    let _ = bus.reply_waiter_remove(target.endpoint_id, target.call);
    // Abandonment paths below can never adopt this key into a replacement PID.

    let Some(client_idx) = sched
        .processes
        .iter()
        .position(|process| process.pid == target.call.pid)
    else {
        caps.settle_grant_delivery(
            target.call.pid,
            target.call.incarnation,
            target.call.generation,
            None,
            &[],
        );
        LATE_REPLY_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    };

    let valid = !sched.processes[client_idx].is_exiting()
        && sched.processes[client_idx].foreground.incarnation == target.call.incarnation
        && sched.processes[client_idx]
            .pending_call
            .is_some_and(|p| p.endpoint_id == target.endpoint_id)
        && terminal_transition_allowed(
            sched.processes[client_idx]
                .pending_call
                .map(|pending| pending.generation),
            sched.processes[client_idx].ipc_call_outcome,
            target.call.generation,
        );
    if !valid {
        caps.settle_grant_delivery(
            target.call.pid,
            target.call.incarnation,
            target.call.generation,
            None,
            &[],
        );
        LATE_REPLY_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }

    if deadline_should_expire(
        sched.processes[client_idx].ipc_deadline,
        Some(target.call.generation),
        sched.global_tick,
    ) {
        finish_call(
            sched,
            client_idx,
            IpcCallOutcome::DeadlineExpired(target.call.generation),
            caps,
        );
        DEADLINE_EXPIRED_COUNT.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }

    caps.settle_grant_delivery(
        target.call.pid,
        target.call.incarnation,
        target.call.generation,
        Some(crate::capability::delivery::RecipientIdentity::of(
            &sched.processes[client_idx],
        )),
        &[reply.caps[0].0, reply.caps[1].0][..(reply.cap_count as usize).min(2)],
    );
    sched.processes[client_idx].ipc_reply = Some((target.call.generation, reply));
    trace_pty_event(
        sched,
        client_idx,
        "reply",
        target.call.generation,
        target.endpoint_id,
        reply,
        bus.pending_count(target.endpoint_id),
    );
    finish_call(
        sched,
        client_idx,
        IpcCallOutcome::ReplyDelivered(target.call.generation),
        caps,
    );
    Ok(())
}

fn remove_deferred_target(sched: &mut Scheduler, target: IpcReplyTarget) {
    for process in &mut sched.processes {
        process
            .deferred_reply_targets
            .retain(|entry| entry.target != target);
    }
}

pub(crate) fn take_terminal_result(
    sched: &mut Scheduler,
    idx: usize,
) -> Option<Result<IpcMsg, IpcError>> {
    let outcome = sched.processes[idx].ipc_call_outcome?;
    sched.processes[idx].ipc_call_outcome = None;
    sched.processes[idx].ipc_deadline = None;
    match outcome {
        IpcCallOutcome::ReplyDelivered(generation) => {
            let reply = sched.processes[idx].ipc_reply.take();
            #[cfg(feature = "scheduler_timer_trace")]
            if sched.processes[idx].name.starts_with(b"schedbench") {
                crate::scheduler_timer_trace::event(
                    sched,
                    8,
                    sched.processes[idx].pid,
                    generation,
                    reply.map_or(0, |(_, msg)| msg.label),
                    0,
                );
            }
            Some(match reply {
                Some((reply_generation, msg)) if reply_generation == generation => Ok(msg),
                _ => Err(IpcError::InvalidArgument),
            })
        }
        IpcCallOutcome::DeadlineExpired(_) => Some(Err(IpcError::DeadlineExpired)),
        IpcCallOutcome::ExplicitlyCancelled(_) => Some(Err(IpcError::Cancelled)),
        IpcCallOutcome::PeerClosed(_) => Some(Err(IpcError::PeerClosed)),
    }
}

fn finish_call(
    sched: &mut Scheduler,
    idx: usize,
    outcome: IpcCallOutcome,
    caps: &mut CapabilityBroker,
) {
    if terminal_transition_allowed(
        sched.processes[idx]
            .pending_call
            .map(|pending| pending.generation),
        sched.processes[idx].ipc_call_outcome,
        outcome.generation(),
    ) {
        let target = sched.processes[idx]
            .pending_call
            .map(|pending| IpcReplyTarget {
                endpoint_id: pending.endpoint_id,
                call: IpcCallId {
                    pid: sched.processes[idx].pid,
                    incarnation: sched.processes[idx].foreground.incarnation,
                    generation: pending.generation,
                },
            });
        if !matches!(outcome, IpcCallOutcome::ReplyDelivered(_)) {
            if let Some(target) = target {
                caps.settle_grant_delivery(
                    target.call.pid,
                    target.call.incarnation,
                    target.call.generation,
                    None,
                    &[],
                );
            }
        }
        sched.processes[idx].pending_call = None;
        if !matches!(outcome, IpcCallOutcome::ReplyDelivered(_)) {
            sched.processes[idx].ipc_reply = None;
        }
        sched.processes[idx].ipc_deadline = None;
        sched.processes[idx].ipc_call_outcome = Some(outcome);
        // Successful delivery has already consumed the server's exclusive
        // reply target (take for an ordinary reply, remove for a deferred
        // reply). Only externally terminated calls need to search all servers
        // for a retained deferred target. Keep this scan off the success path
        // while holding the global scheduler lock.
        if !matches!(outcome, IpcCallOutcome::ReplyDelivered(_)) {
            if let Some(target) = target {
                remove_deferred_target(sched, target);
            }
        }
        wake_terminal(sched, idx);
    }
}

pub(crate) fn terminal_transition_allowed(
    pending_generation: Option<u64>,
    current_outcome: Option<IpcCallOutcome>,
    candidate_generation: u64,
) -> bool {
    current_outcome.is_none() && pending_generation == Some(candidate_generation)
}

/// Prove that a server currently owns the reply path for a caller's exact
/// in-flight IPC generation. This remains stable while the server handles the
/// request, unlike the caller's scheduler state, which may change as part of
/// SMP wakeup and timeout bookkeeping.
pub(crate) fn reply_target_matches_pending_request(
    requester_pid: usize,
    incarnation: u64,
    pending: Option<PendingIpcCall>,
    reply_target: Option<IpcReplyTarget>,
) -> bool {
    let (Some(pending), Some(reply_target)) = (pending, reply_target) else {
        return false;
    };
    reply_target.call.incarnation == incarnation
        && reply_target.endpoint_id == pending.endpoint_id
        && reply_target.call.pid == requester_pid
        && reply_target.call.generation == pending.generation
}

pub(crate) fn deadline_should_expire(
    deadline_entry: Option<(u64, u64)>,
    pending_generation: Option<u64>,
    now_tick: u64,
) -> bool {
    deadline_entry.is_some_and(|(generation, deadline)| {
        pending_generation == Some(generation) && now_tick >= deadline
    })
}

pub(crate) fn recv_deadline_should_expire(
    deadline_entry: Option<(u64, u64, u32)>,
    current_generation: u64,
    endpoint_id: u32,
    now_tick: u64,
) -> bool {
    deadline_entry.is_some_and(|(generation, deadline, deadline_endpoint)| {
        generation == current_generation && deadline_endpoint == endpoint_id && now_tick >= deadline
    })
}

fn wake_terminal(sched: &mut Scheduler, idx: usize) {
    if sched.processes[idx].state != ProcessState::BlockedOnIpc {
        return;
    }
    sched.record_wakeup(idx, true);
    trace_pty_event(
        sched,
        idx,
        "wake",
        sched.processes[idx].ipc_call_generation,
        sched.processes[idx].ipc_endpoint.unwrap_or(0),
        IpcMsg::empty(),
        0,
    );
    sched.processes[idx].state = ProcessState::Ready;
    sched.processes[idx].priority_wake_pending = true;
    sched.remove_from_ready_queues(idx);
    if let Some(cpu_id) = sched.live_owner_core(idx) {
        crate::sched::request_reschedule_on(cpu_id);
    } else {
        sched.enqueue_ready(idx);
    }
}

pub fn handle_ipc_call(
    caller_pid: usize,
    target_cap: CapabilityToken,
    mut msg: IpcMsg,
    caps: &mut CapabilityBroker,
    sched: &mut Scheduler,
    bus: &mut IpcBus,
) -> Result<IpcMsg, IpcError> {
    let Some(idx) = sched.processes.iter().position(|p| p.pid == caller_pid) else {
        return Err(IpcError::InvalidArgument);
    };
    // SIGKILL may have finished this task while it was entering the syscall
    // on another CPU. Never enqueue work or replace its terminal state.
    if sched.processes[idx].is_exiting() {
        trace_pty_event(sched, idx, "reject_exit", 0, 0, msg, 0);
        return Err(IpcError::Cancelled);
    }
    if let Some(result) = take_terminal_result(sched, idx) {
        return result;
    }

    let (endpoint_id, target_owner) = match caps.token_owner(target_cap, CapabilityRights::SEND) {
        Ok(target) => target,
        Err(_) => {
            sched.processes[idx].ipc_next_deadline_tick = None;
            return Err(IpcError::InvalidCapability);
        }
    };
    // A diagnostic successor can publish its own endpoint, never control survivors.
    if sched.processes[idx].trusted_service_manager
        && !sched.active_manager(&sched.processes[idx])
        && !(target_owner == 1 && msg.label == NAMESERVER_REGISTER)
    {
        sched.processes[idx].ipc_next_deadline_tick = None;
        return Err(IpcError::InvalidCapability);
    }
    if sched.processes[idx].pending_call.is_none()
        && target_owner == 1
        && msg.label == NAMESERVER_REGISTER
    {
        mediate_nameserver_register(caller_pid, &mut msg, caps, sched)?;
    }
    if target_owner == 1 && msg.label == sunlight_ipc::InitMsg::LOOKUP {
        if let Err(error) = mediate_nameserver_lookup(caller_pid, &msg, sched) {
            // A denied bounded lookup never enters pending_call. Consume its
            // armed deadline so the caller can issue another bounded request.
            sched.processes[idx].ipc_next_deadline_tick = None;
            return Err(error);
        }
    }
    // D1 checks apply on every call, including callers holding a leaked SEND
    // token. The message payload and caller-supplied badge confer no authority.
    if target_owner != 1
        && msg.label == sunlight_ipc::demand::DISCOVER
        && sched
            .processes
            .iter()
            .any(|p| p.pid == target_owner && p.trusted_service_manager)
        && !sunlight_ipc::demand::discovery_authorized(
            sched.processes[idx].service_lookup_restrictions,
        )
    {
        sched.processes[idx].ipc_next_deadline_tick = None;
        return Err(IpcError::InvalidCapability);
    }
    if target_owner != 1
        && (matches!(
            msg.label,
            sunlight_ipc::demand::ACTIVATE
                | sunlight_ipc::demand::LEASE_ACQUIRE
                | sunlight_ipc::demand::IDLE_ACCOUNT
                | sunlight_ipc::supervision::RPC_RECORD
                | sunlight_ipc::supervision::RPC_DETAIL
                | sunlight_ipc::supervision::RPC_ACCOUNT
                | sunlight_ipc::supervision::RPC_PATH
                | sunlight_ipc::supervision::TEST_ARM
                | sunlight_ipc::supervision::TEST_PROBE
                | sunlight_ipc::supervision::TEST_CRASH
        ) || matches!(msg.label, 1..=6))
        && !sunlight_ipc::demand::activation_words_authorized(
            sched.processes[idx].service_lookup_restrictions,
            msg.label,
            msg.word_count,
            &msg.words,
        )
        && sched
            .processes
            .iter()
            .any(|p| p.pid == target_owner && p.trusted_service_manager)
    {
        sched.processes[idx].ipc_next_deadline_tick = None;
        return Err(IpcError::InvalidCapability);
    }
    if let Some(pending) = sched.processes[idx].pending_call {
        if pending.target_cap != target_cap.0 || pending.endpoint_id != endpoint_id {
            return Err(IpcError::InvalidArgument);
        }
    }
    let global_tick = sched.global_tick;
    if sched.processes[idx].pending_call.is_none() {
        let deadline = sched.processes[idx].ipc_next_deadline_tick.take();
        if deadline.is_some_and(|deadline| global_tick >= deadline) {
            DEADLINE_EXPIRED_COUNT.fetch_add(1, Ordering::Relaxed);
            return Err(IpcError::DeadlineExpired);
        }
        let generation = sched.processes[idx]
            .ipc_call_generation
            .checked_add(1)
            .ok_or(IpcError::InvalidArgument)?;
        let call = IpcCallId {
            pid: caller_pid,
            incarnation: sched.processes[idx].foreground.incarnation,
            generation,
        };
        if let Err(error) = bus.enqueue_call(endpoint_id, msg, call) {
            return Err(error);
        }
        sched.processes[idx].ipc_call_generation = generation;
        #[cfg(feature = "scheduler_timer_trace")]
        if sched
            .processes
            .iter()
            .any(|p| p.pid == target_owner && p.name_str() == "timer_server")
        {
            crate::scheduler_timer_trace::event(
                sched,
                5,
                target_owner,
                caller_pid as u64,
                generation,
                msg.label,
            );
        }
        sched.processes[idx].pending_call = Some(PendingIpcCall {
            target_cap: target_cap.0,
            endpoint_id,
            msg,
            generation,
        });
        trace_pty_event(
            sched,
            idx,
            "request",
            generation,
            endpoint_id,
            msg,
            bus.pending_count(endpoint_id),
        );
        sched.processes[idx].ipc_deadline = deadline.map(|deadline| (generation, deadline));
    }

    sched.account_and_apply_churn_penalty();
    sched.set_state(idx, ProcessState::BlockedOnIpc);
    sched.processes[idx].ipc_endpoint = Some(endpoint_id);
    sched.processes[idx].block_start_tick = global_tick;
    sched.remove_from_ready_queues(idx);

    Err(IpcError::WouldBlock)
}

fn mediate_nameserver_register(
    caller_pid: usize,
    msg: &mut IpcMsg,
    caps: &mut CapabilityBroker,
    sched: &Scheduler,
) -> Result<(), IpcError> {
    if msg.word_count < 2 {
        UNAUTHORIZED_REGISTER_COUNT.fetch_add(1, Ordering::Relaxed);
        return Err(IpcError::InvalidArgument);
    }
    let source = CapabilityToken(msg.words[1]);
    let (endpoint_id, owner_pid) = caps
        .token_owner(source, CapabilityRights::SEND_RECV)
        .map_err(|_| {
            UNAUTHORIZED_REGISTER_COUNT.fetch_add(1, Ordering::Relaxed);
            IpcError::InvalidCapability
        })?;
    let (process_name, trusted_display_service) = sched
        .processes
        .iter()
        .find(|process| process.pid == caller_pid)
        .map(|process| (process.name_str(), process.trusted_display_service))
        .ok_or(IpcError::InvalidArgument)?;
    if !registration_authorized(owner_pid, caller_pid, process_name, msg.words[0]) {
        UNAUTHORIZED_REGISTER_COUNT.fetch_add(1, Ordering::Relaxed);
        return Err(IpcError::InvalidCapability);
    }
    if msg.words[0] == name_hash("sunlightd") {
        let process = sched.processes.iter().find(|p| p.pid == caller_pid).ok_or(IpcError::InvalidCapability)?;
        if !sched.supervision.current(Scheduler::supervision_identity(process)) {
            return Err(IpcError::InvalidCapability);
        }
    }
    if msg.words[0] == name_hash("display_server") {
        if !trusted_display_service {
            UNAUTHORIZED_REGISTER_COUNT.fetch_add(1, Ordering::Relaxed);
            return Err(IpcError::InvalidCapability);
        }
        crate::memory::security::register_display_authority(owner_pid, endpoint_id, caps);
    }
    let public = caps
        .derive(source, CapabilityRights::SEND_ONLY)
        .map_err(|_| IpcError::InvalidCapability)?;
    msg.words[1] = public.0;
    msg.words[2] = endpoint_id as u64;
    msg.word_count = msg.word_count.max(3);
    Ok(())
}

fn mediate_nameserver_lookup(
    caller_pid: usize,
    msg: &IpcMsg,
    sched: &Scheduler,
) -> Result<(), IpcError> {
    if msg.word_count < 1 {
        return Err(IpcError::InvalidArgument);
    }
    let process = sched
        .processes
        .iter()
        .find(|process| process.pid == caller_pid)
        .ok_or(IpcError::InvalidArgument)?;
    if msg.words[0] == name_hash(sunlight_ipc::WISEOWL_DISPLAY_LIFECYCLE_ENDPOINT) {
        return if process.trusted_display_service {
            Ok(())
        } else {
            Err(IpcError::InvalidCapability)
        };
    }
    if msg.words[0] == name_hash(sunlight_ipc::WISEOWL_CONTROL_PANEL_LIFECYCLE_ENDPOINT) {
        return if process.trusted_control_panel {
            Ok(())
        } else {
            Err(IpcError::InvalidCapability)
        };
    }
    let Some(mask) = process.service_lookup_restrictions else {
        return Ok(());
    };
    if sunlight_ipc::service_capability_allows_hashed_name(mask, msg.words[0]) {
        return Ok(());
    }
    // msg.words[0] is the FNV name hash; process.name is the caller, not the target.
    crate::serial_println!(
        "[IPC] denied nameserver lookup pid={} caller='{}' name_hash={:#x} mask={:#x}",
        caller_pid,
        process.name_str(),
        msg.words[0],
        mask
    );
    Err(IpcError::InvalidCapability)
}

pub(crate) fn registration_authorized(
    owner_pid: usize,
    caller_pid: usize,
    process_name: &str,
    registered_name: u64,
) -> bool {
    owner_pid == caller_pid && registration_identity_matches(process_name, registered_name)
}

fn registration_identity_matches(process_name: &str, registered_name: u64) -> bool {
    let expected_process = match registered_name {
        name if name == name_hash("display_server") => "sunlight-display",
        name if name == name_hash("mouse_driver") => "sunlight-mouse",
        name if name == name_hash("time") => "timer_server",
        name if name == name_hash("net") => "net_server",
        name if name == name_hash("pty") => "pty_server",
        name if name == name_hash("vfs") => "vfs_server",
        name if name == name_hash("tty") => "tty_server",
        name if name == name_hash("tz") => "timezone_service",
        name if name == name_hash("uac") => "uac_service",
        name if name == name_hash("sm") => "sunlight-sm",
        name if name == name_hash("rand") => "rand_service",
        name if name == name_hash("clipd") => "sunlight-clipd",
        name if name == name_hash("wiseowl-memoryd") => "wiseowl-memoryd",
        name if name == name_hash("wiseowl-memorydb") => "wiseowl-memorydb",
        name if name == name_hash("wiseowl.memorydb.v1") => "wiseowl-memorydb",
        name if name == name_hash("wiseowl.memorydb.authority.v1") => "wiseowl-memorydb",
        name if name == name_hash("wiseowl-indexd") => "wiseowl-indexd",
        name if name == name_hash("wiseowl.index.v1") => "wiseowl-indexd",
        name if name == name_hash("wiseowl-braind") => "wiseowl-braind",
        name if name == name_hash("wiseowl.brain.v1") => "wiseowl-braind",
        name if name == name_hash(sunlight_ipc::WISEOWL_DISPLAY_LIFECYCLE_ENDPOINT) => {
            "wiseowl-braind"
        }
        name if name == name_hash(sunlight_ipc::WISEOWL_CONTROL_PANEL_LIFECYCLE_ENDPOINT) => {
            "wiseowl-braind"
        }
        name if name == name_hash("dialogd") => "sunlight-dialogd",
        name if name == name_hash("thumbd") => "sunlight-thumbd",
        name if name == name_hash("docserv") => "sunlight-docserv",
        name if name == name_hash("gcd") || name == name_hash("proc") => "gcd",
        name if name == name_hash("solar") => "solar",
        name if name == name_hash("sunlight-kv") => "sunlight-kv",
        name if name == name_hash("mezzo") => "mezzo",
        name if name == name_hash("sunlight.session.v1") => "sunlight-sessiond",
        name if name == name_hash("sunlight-tls") => "sunlight-tls",
        name if name == name_hash("sunlightd") => "sunlightd",
        name if name == name_hash("niced") => "niced",
        name if name == name_hash("resolved") => "resolved",
        name if name == name_hash("powerd") => "powerd",
        name if name == name_hash("thermald") => "thermald",
        name if name == name_hash("audiod") => "audiod",
        name if name == name_hash("audiod.capture") => "audiod",
        name if name == name_hash("networkd") => "networkd",
        name if name == name_hash("deviced") => "deviced",
        name if name == name_hash("timed") => "timed",
        name if name == name_hash("sshl") => "sshl",
        _ if process_name == "sshl" && registered_name_has_prefix(registered_name, "sshl") => {
            "sshl"
        }
        _ => return registered_name == name_hash(process_name),
    };
    process_name == expected_process
}

fn registered_name_has_prefix(registered_name: u64, prefix: &str) -> bool {
    let mut suffix = 0u64;
    while suffix < 1_024 {
        let mut name = heapless::String::<16>::new();
        if name.push_str(prefix).is_err() {
            return false;
        }
        use core::fmt::Write;
        if write!(&mut name, "{}", suffix).is_err() {
            return false;
        }
        if name_hash(name.as_str()) == registered_name {
            return true;
        }
        suffix += 1;
    }
    false
}

pub(crate) const fn name_hash(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 1;
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

pub fn handle_ipc_recv(
    receiver_pid: usize,
    endpoint_id: u32,
    sched: &mut Scheduler,
    bus: &mut IpcBus,
) -> Result<IpcMsg, IpcError> {
    let Some(idx) = sched
        .processes
        .iter()
        .position(|process| process.pid == receiver_pid)
    else {
        return Err(IpcError::InvalidArgument);
    };
    if sched.processes[idx].is_exiting() {
        return Err(IpcError::Cancelled);
    }
    if sched.processes[idx].ipc_reply_target.is_some() {
        // A synchronous server must resolve (or attempt) its current reply
        // before receiving another call, otherwise reply identity is ambiguous.
        sched.processes[idx].ipc_next_deadline_tick = None;
        return Err(IpcError::InvalidArgument);
    }

    if let Some((_generation, timeout_endpoint)) = sched.processes[idx].ipc_recv_timeout {
        if timeout_endpoint != endpoint_id {
            return Err(IpcError::InvalidArgument);
        }
        sched.processes[idx].ipc_recv_timeout = None;
        return Err(IpcError::DeadlineExpired);
    }

    let now_tick = sched.global_tick;
    if let Some((generation, deadline, deadline_endpoint)) = sched.processes[idx].ipc_recv_deadline
    {
        if deadline_endpoint != endpoint_id
            || generation != sched.processes[idx].ipc_recv_generation
        {
            sched.processes[idx].ipc_recv_deadline = None;
            return Err(IpcError::InvalidArgument);
        }
        // This syscall-side recheck covers a retry that runs before the next
        // BSP timer interrupt can perform the scheduler transition.
        if now_tick >= deadline {
            sched.processes[idx].ipc_recv_deadline = None;
            DEADLINE_EXPIRED_COUNT.fetch_add(1, Ordering::Relaxed);
            return Err(IpcError::DeadlineExpired);
        }
    } else if let Some(deadline) = sched.processes[idx].ipc_next_deadline_tick.take() {
        if now_tick >= deadline {
            DEADLINE_EXPIRED_COUNT.fetch_add(1, Ordering::Relaxed);
            return Err(IpcError::DeadlineExpired);
        }
        let generation = sched.processes[idx]
            .ipc_recv_generation
            .wrapping_add(1)
            .max(1);
        sched.processes[idx].ipc_recv_generation = generation;
        sched.processes[idx].ipc_recv_deadline = Some((generation, deadline, endpoint_id));
    }

    if let Some(msg) = bus.pop_pending(endpoint_id) {
        sched.processes[idx].ipc_next_deadline_tick = None;
        sched.processes[idx].ipc_recv_deadline = None;
        set_reply_target(sched, receiver_pid, endpoint_id, msg.call);
        #[cfg(feature = "scheduler_timer_trace")]
        crate::scheduler_timer_trace::receive(
            sched,
            receiver_pid,
            msg.call.map_or(0, |c| c.pid),
            msg.call.map_or(msg.msg.words[0], |c| c.generation),
        );
        return Ok(msg.msg);
    }
    bus.block_on_recv(endpoint_id, receiver_pid, sched);
    Err(IpcError::WouldBlock)
}

pub fn handle_ipc_try_recv(
    receiver_pid: usize,
    endpoint_id: u32,
    sched: &mut Scheduler,
    bus: &mut IpcBus,
) -> Result<IpcMsg, IpcError> {
    let Some(idx) = sched
        .processes
        .iter()
        .position(|process| process.pid == receiver_pid)
    else {
        return Err(IpcError::InvalidArgument);
    };
    if sched.processes[idx].is_exiting() {
        return Err(IpcError::Cancelled);
    }
    if sched.processes[idx].ipc_reply_target.is_some() {
        return Err(IpcError::InvalidArgument);
    }
    let Some(msg) = bus.pop_pending(endpoint_id) else {
        return Err(IpcError::WouldBlock);
    };
    set_reply_target(sched, receiver_pid, endpoint_id, msg.call);
    Ok(msg.msg)
}

pub fn handle_ipc_reply(
    server_pid: usize,
    reply: IpcMsg,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
    bus: &mut IpcBus,
) -> Result<(), IpcError> {
    deliver_reply_to_current_target(server_pid, reply, sched, caps, bus)
}

pub fn defer_current_reply(server_pid: usize, sched: &mut Scheduler) -> Result<u64, IpcError> {
    let Some(server_idx) = sched
        .processes
        .iter()
        .position(|process| process.pid == server_pid)
    else {
        return Err(IpcError::InvalidArgument);
    };
    let target = sched.processes[server_idx]
        .ipc_reply_target
        .take()
        .ok_or(IpcError::InvalidArgument)?;
    if sched.processes[server_idx].deferred_reply_targets.len() >= DEFERRED_REPLY_CAPACITY {
        sched.processes[server_idx].ipc_reply_target = Some(target);
        return Err(IpcError::QueueFull);
    }
    let token = sched.processes[server_idx]
        .next_deferred_reply_token
        .wrapping_add(1)
        .max(1)
        | (1u64 << 63);
    sched.processes[server_idx].next_deferred_reply_token = token;
    sched.processes[server_idx]
        .deferred_reply_targets
        .push_back(DeferredIpcReply { token, target });
    Ok(token)
}

pub fn deferred_reply_endpoint(
    server_pid: usize,
    token: u64,
    sched: &Scheduler,
) -> Result<u32, IpcError> {
    sched
        .processes
        .iter()
        .find(|process| process.pid == server_pid)
        .and_then(|process| {
            process
                .deferred_reply_targets
                .iter()
                .find(|entry| entry.token == token)
                .map(|entry| entry.target.endpoint_id)
        })
        .ok_or(IpcError::InvalidArgument)
}

pub fn deferred_reply_is_live(server_pid: usize, token: u64, sched: &Scheduler) -> bool {
    let Some(entry) = sched
        .processes
        .iter()
        .find(|process| process.pid == server_pid)
        .and_then(|process| {
            process
                .deferred_reply_targets
                .iter()
                .find(|entry| entry.token == token)
                .copied()
        })
    else {
        return false;
    };
    sched.processes.iter().any(|process| {
        process.pid == entry.target.call.pid
            && process.foreground.incarnation == entry.target.call.incarnation
            && !process.is_exiting()
            && process.pending_call.is_some_and(|pending| {
                pending.endpoint_id == entry.target.endpoint_id
                    && pending.generation == entry.target.call.generation
            })
    })
}

pub fn complete_deferred_reply(
    server_pid: usize,
    token: u64,
    reply: IpcMsg,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
    bus: &mut IpcBus,
) -> Result<(), IpcError> {
    let Some(server_idx) = sched
        .processes
        .iter()
        .position(|process| process.pid == server_pid)
    else {
        return Err(IpcError::InvalidArgument);
    };
    let Some(entry_idx) = sched.processes[server_idx]
        .deferred_reply_targets
        .iter()
        .position(|entry| entry.token == token)
    else {
        return Err(IpcError::InvalidArgument);
    };
    let target = sched.processes[server_idx]
        .deferred_reply_targets
        .remove(entry_idx)
        .ok_or(IpcError::InvalidArgument)?
        .target;
    deliver_reply_to_target(target, reply, sched, caps, bus)
}

pub fn handle_ipc_reply_wait(
    server_pid: usize,
    endpoint_id: u32,
    reply: IpcMsg,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
    bus: &mut IpcBus,
) -> Result<IpcMsg, IpcError> {
    let Some(server) = sched.processes.iter().find(|p| p.pid == server_pid) else {
        return Err(IpcError::InvalidArgument);
    };
    if server.is_exiting() {
        return Err(IpcError::Cancelled);
    }
    let already_waiting = sched
        .processes
        .iter()
        .find(|p| p.pid == server_pid)
        .is_some_and(|p| p.pending_reply_wait.is_some());

    if !already_waiting {
        #[cfg(feature = "scheduler_timer_trace")]
        if let Some(idx) = sched
            .processes
            .iter()
            .position(|p| p.pid == server_pid && p.name_str() == "timer_server")
        {
            crate::scheduler_timer_trace::event(
                sched,
                10,
                server_pid,
                0,
                0,
                sched.effective_runtime_at(idx, sched.now_ns()),
            );
        }
        deliver_reply_to_current_target(server_pid, reply, sched, caps, bus)?;
    }

    if let Some(server) = sched.process_mut_by_pid(server_pid) {
        if let Some(msg) = bus.pop_pending(endpoint_id) {
            server.ipc_reply_target = msg.call.map(|call| IpcReplyTarget { endpoint_id, call });
            server.ipc_endpoint = Some(endpoint_id);
            server.pending_reply_wait = None;
            #[cfg(feature = "scheduler_timer_trace")]
            crate::scheduler_timer_trace::receive(
                sched,
                server_pid,
                msg.call.map_or(0, |c| c.pid),
                msg.call.map_or(msg.msg.words[0], |c| c.generation),
            );
            return Ok(msg.msg);
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
    caps: &mut CapabilityBroker,
    bus: &mut IpcBus,
) -> Result<(), IpcError> {
    let Some(idx) = sched.processes.iter().position(|p| p.pid == caller_pid) else {
        return Err(IpcError::InvalidArgument);
    };

    if matches!(
        sched.processes[idx].ipc_call_outcome,
        Some(IpcCallOutcome::ReplyDelivered(_))
    ) {
        return Ok(());
    }

    let pending = sched.processes[idx].pending_call;

    let Some(pending) = pending else {
        return Ok(());
    };

    let call = IpcCallId {
        pid: caller_pid,
        incarnation: sched.processes[idx].foreground.incarnation,
        generation: pending.generation,
    };
    bus.remove_pending_call(pending.endpoint_id, call);
    bus.reply_waiter_remove(pending.endpoint_id, call);
    // If the server already consumed the request, retain its generation-tagged
    // reply target as a one-entry tombstone. Its eventual reply is then counted
    // and dropped before reply-and-wait installs a target for the next call.
    finish_call(
        sched,
        idx,
        IpcCallOutcome::ExplicitlyCancelled(pending.generation),
        caps,
    );
    EXPLICIT_CANCEL_COUNT.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub fn arm_ipc_deadline(
    caller_pid: usize,
    absolute_deadline_tick: u64,
    sched: &mut Scheduler,
) -> Result<(), IpcError> {
    let Some(caller) = sched.process_mut_by_pid(caller_pid) else {
        return Err(IpcError::InvalidArgument);
    };
    if caller.pending_call.is_some()
        || caller.ipc_call_outcome.is_some()
        || caller.ipc_next_deadline_tick.is_some()
        || caller.ipc_recv_deadline.is_some()
        || caller.ipc_recv_timeout.is_some()
    {
        return Err(IpcError::InvalidArgument);
    }
    caller.ipc_next_deadline_tick = Some(absolute_deadline_tick);
    Ok(())
}

pub fn clear_next_ipc_deadline(caller_pid: usize, sched: &mut Scheduler) {
    if let Some(caller) = sched.process_mut_by_pid(caller_pid) {
        caller.ipc_next_deadline_tick = None;
    }
}

/// Called from the BSP scheduler tick. Generation comparisons are the stale
/// deadline rejection: a deadline can terminate only the operation that armed it.
pub fn expire_deadlines(sched: &mut Scheduler, now_tick: u64) {
    for idx in 0..sched.processes.len() {
        let Some((generation, _deadline)) = sched.processes[idx].ipc_deadline else {
            continue;
        };
        let Some(pending) = sched.processes[idx].pending_call else {
            sched.processes[idx].ipc_deadline = None;
            continue;
        };
        if !deadline_should_expire(
            sched.processes[idx].ipc_deadline,
            Some(pending.generation),
            now_tick,
        ) {
            if pending.generation == generation {
                continue;
            }
            sched.processes[idx].ipc_deadline = None;
            continue;
        }

        let call = IpcCallId {
            pid: sched.processes[idx].pid,
            incarnation: sched.processes[idx].foreground.incarnation,
            generation,
        };
        let mut caps = crate::capability::CAP_BROKER.lock();
        with_shard(pending.endpoint_id, |bus| {
            bus.remove_pending_call(pending.endpoint_id, call);
            bus.reply_waiter_remove(pending.endpoint_id, call);
        });
        finish_call(
            sched,
            idx,
            IpcCallOutcome::DeadlineExpired(generation),
            &mut caps,
        );
        #[cfg(feature = "cpu_accounting_diag")]
        {
            let count = &mut sched.processes[idx].wake_audit[3];
            *count = count.saturating_add(1);
        }
        DEADLINE_EXPIRED_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    for idx in 0..sched.processes.len() {
        let Some((generation, _deadline, endpoint_id)) = sched.processes[idx].ipc_recv_deadline
        else {
            continue;
        };
        if !recv_deadline_should_expire(
            sched.processes[idx].ipc_recv_deadline,
            sched.processes[idx].ipc_recv_generation,
            endpoint_id,
            now_tick,
        ) {
            if generation != sched.processes[idx].ipc_recv_generation {
                sched.processes[idx].ipc_recv_deadline = None;
            }
            continue;
        }
        sched.processes[idx].ipc_recv_deadline = None;
        sched.processes[idx].ipc_recv_timeout = Some((generation, endpoint_id));
        #[cfg(feature = "cpu_accounting_diag")]
        {
            let count = &mut sched.processes[idx].wake_audit[2];
            *count = count.saturating_add(1);
        }
        DEADLINE_EXPIRED_COUNT.fetch_add(1, Ordering::Relaxed);
        wake_terminal(sched, idx);
    }
}

pub fn finish_peer_closed_calls(
    endpoint_id: u32,
    calls: impl IntoIterator<Item = IpcCallId>,
    sched: &mut Scheduler,
) {
    let mut caps = crate::capability::CAP_BROKER.lock();
    for call in calls {
        let Some(idx) = sched
            .processes
            .iter()
            .position(|p| p.pid == call.pid && p.foreground.incarnation == call.incarnation)
        else {
            continue;
        };
        if !sched.processes[idx].pending_call.is_some_and(|pending| {
            pending.endpoint_id == endpoint_id && pending.generation == call.generation
        }) {
            continue;
        }
        finish_call(
            sched,
            idx,
            IpcCallOutcome::PeerClosed(call.generation),
            &mut caps,
        );
        #[cfg(feature = "native_session_race_test")]
        if sched.processes[idx].name_str() == "session-race-probe" {
            crate::serial_println!("[SESSION-E3] seq={} checkpoint=peer-closed endpoint={} recipient={} recipient_inc={} request_generation={} pending=0 cpu={}", crate::capability::session_race::sequence(), endpoint_id, call.pid, call.incarnation, call.generation, crate::sched::current_cpu_id());
        }
        PEER_CLOSED_WAKE_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cancel_reply_target, deadline_should_expire, recv_deadline_should_expire,
        reply_target_matches_pending_request, terminal_transition_allowed, IpcBus, IpcError,
        IpcMsg, ENDPOINT_QUEUE_CAPACITY,
    };
    use crate::process::{IpcCallId, IpcCallOutcome, IpcReplyTarget, PendingIpcCall};

    fn call(pid: usize, generation: u64) -> IpcCallId {
        IpcCallId { pid, incarnation: 0, generation }
    }

    #[test]
    fn cancel_reply_target_clears_only_matching_in_flight_call() {
        let expected = IpcReplyTarget {
            endpoint_id: 13,
            call: call(12, 7),
        };
        let mut target = Some(expected);
        assert!(cancel_reply_target(&mut target, expected));
        assert_eq!(target, None);

        let mut other_endpoint = Some(IpcReplyTarget {
            endpoint_id: 9,
            call: call(12, 7),
        });
        assert!(!cancel_reply_target(&mut other_endpoint, expected));

        let mut newer_call = Some(IpcReplyTarget {
            endpoint_id: 13,
            call: call(12, 8),
        });
        assert!(!cancel_reply_target(&mut newer_call, expected));
    }

    #[test]
    fn auth_reply_target_is_bound_to_exact_pending_generation() {
        let pending = PendingIpcCall {
            target_cap: 0x1234,
            endpoint_id: 13,
            msg: IpcMsg::with_label(7),
            generation: 9,
        };
        let target = IpcReplyTarget {
            endpoint_id: 13,
            call: call(4, 9),
        };
        assert!(reply_target_matches_pending_request(4, 0,
            Some(pending),
            Some(target)
        ));

        let stale = IpcReplyTarget {
            endpoint_id: 13,
            call: call(4, 8),
        };
        assert!(!reply_target_matches_pending_request(4, 0,
            Some(pending),
            Some(stale)
        ));
        assert!(!reply_target_matches_pending_request(5, 0,
            Some(pending),
            Some(target)
        ));
        assert!(!reply_target_matches_pending_request(4, 0, None, Some(target)));
    }

    #[test]
    fn multiple_senders_keep_fifo_order() {
        let mut bus = IpcBus::new();
        for pid in [11, 22, 33] {
            bus.enqueue_call(4, IpcMsg::with_label(pid as u64), call(pid, 1))
                .unwrap();
        }
        for pid in [11, 22, 33] {
            let pending = bus.pop_pending(4).unwrap();
            assert_eq!(pending.msg.label, pid as u64);
            assert_eq!(pending.call, Some(call(pid, 1)));
        }
    }

    #[test]
    fn queue_bound_and_receive_reuse_are_explicit() {
        let mut bus = IpcBus::new();
        for i in 0..ENDPOINT_QUEUE_CAPACITY {
            bus.enqueue_call(5, IpcMsg::with_label(i as u64), call(i + 1, 1))
                .unwrap();
        }
        assert_eq!(bus.pending_count(5), ENDPOINT_QUEUE_CAPACITY);
        assert_eq!(
            bus.enqueue_call(5, IpcMsg::empty(), call(999, 1)),
            Err(IpcError::QueueFull)
        );
        assert_eq!(bus.pop_pending(5).unwrap().msg.label, 0);
        bus.enqueue_call(5, IpcMsg::with_label(99), call(999, 1))
            .unwrap();
        assert_eq!(bus.pending_count(5), ENDPOINT_QUEUE_CAPACITY);
    }

    #[test]
    fn queue_space_is_reused_after_cancel_and_sender_exit() {
        let mut bus = IpcBus::new();
        for i in 0..ENDPOINT_QUEUE_CAPACITY {
            bus.enqueue_call(6, IpcMsg::empty(), call(i + 1, 1))
                .unwrap();
        }
        assert!(bus.remove_pending_call(6, call(1, 1)));
        bus.reply_waiter_remove(6, call(1, 1));
        bus.enqueue_call(6, IpcMsg::empty(), call(1000, 1)).unwrap();

        bus.remove_pid_references(2);
        bus.enqueue_call(6, IpcMsg::empty(), call(1001, 1)).unwrap();
        assert_eq!(bus.pending_count(6), ENDPOINT_QUEUE_CAPACITY);
    }

    #[test]
    fn endpoint_removal_clears_queue_waiters_and_notification_state() {
        let mut bus = IpcBus::new();
        bus.enqueue_call(7, IpcMsg::empty(), call(42, 3)).unwrap();
        bus.send_input_notification(7);
        let closed = bus.remove_endpoint(7);
        assert_eq!(closed, [call(42, 3)]);
        assert_eq!(bus.pending_count(7), 0);

        bus.send_input_notification(7);
        assert_eq!(bus.pending_count(7), 1);
    }

    #[test]
    fn timer_notifications_are_bounded_and_preserve_elapsed_ticks() {
        let mut bus = IpcBus::new();
        for tick in 10..=1_000 {
            bus.send_timer_tick(8, tick);
        }
        assert_eq!(bus.pending_count(8), 1);
        let tick = bus.pop_pending(8).unwrap();
        assert_eq!(tick.msg.words[0], 991);
    }

    #[test]
    fn input_notification_is_level_triggered_and_rearmable() {
        let mut bus = IpcBus::new();
        for _ in 0..1_000 {
            bus.send_input_notification(9);
        }
        assert_eq!(bus.pending_count(9), 1);
        bus.pop_pending(9).unwrap();
        bus.send_input_notification(9);
        assert_eq!(bus.pending_count(9), 1);
    }

    #[test]
    fn first_terminal_transition_wins_and_stale_generations_lose() {
        assert!(terminal_transition_allowed(Some(5), None, 5));
        assert!(!terminal_transition_allowed(
            Some(5),
            Some(IpcCallOutcome::ReplyDelivered(5)),
            5
        ));
        assert!(!terminal_transition_allowed(
            Some(5),
            Some(IpcCallOutcome::DeadlineExpired(5)),
            5
        ));
        assert!(!terminal_transition_allowed(Some(6), None, 5));
        assert!(!deadline_should_expire(Some((5, 100)), Some(5), 99));
        assert!(deadline_should_expire(Some((5, 100)), Some(5), 100));
        assert!(!deadline_should_expire(Some((5, 100)), Some(6), 1_000));
        assert!(recv_deadline_should_expire(Some((7, 100, 9)), 7, 9, 100));
        assert!(!recv_deadline_should_expire(Some((7, 100, 9)), 8, 9, 1_000));
    }
}
