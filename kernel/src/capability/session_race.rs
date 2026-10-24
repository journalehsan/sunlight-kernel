//! Opt-in observations of the boot-global production protocol. No authority
//! creation, roles, tokens, alternate broker, or waiting under kernel locks.
use core::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static LAST_TRANSITION: spin::Mutex<Option<(u64, u64, u64)>> = spin::Mutex::new(None);
static RACE_SESSION: AtomicU64 = AtomicU64::new(0);
static RACE_EPOCH: AtomicU64 = AtomicU64::new(0);
static WINNER: AtomicU64 = AtomicU64::new(0);
static PARTICIPANTS: AtomicU64 = AtomicU64::new(0);
static READY: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicU64 = AtomicU64::new(0);
static GRANTS: spin::Mutex<[u64; 32]> = spin::Mutex::new([0; 32]);
static MINTED: AtomicU64 = AtomicU64::new(0);
static CONSUMED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn grant(token: super::CapabilityToken, session: u64, epoch: u64, consume: bool) {
    let mut tokens = GRANTS.lock();
    let position = tokens
        .iter()
        .position(|t| *t == token.0)
        .or_else(|| tokens.iter().position(|t| *t == 0));
    let Some(position) = position else {
        return;
    };
    if tokens[position] == 0 {
        tokens[position] = token.0;
        MINTED.fetch_add(1, Ordering::Relaxed);
    }
    if consume {
        CONSUMED.fetch_add(1, Ordering::Relaxed);
    }
    crate::serial_println!("[SESSION-E3] seq={} checkpoint=grant-observation session={} epoch={} grant_fingerprint=boot-local-{} consumed={} cpu={}", sequence(), session, epoch, position+1, u8::from(consume), crate::sched::current_cpu_id());
}

pub(crate) fn sequence() -> u64 {
    SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

/// Observe the real UAC's existing, policy-accepted timing IPC. This changes
/// timing only; all broker authorization runs again at its ordinary boundary.
pub(crate) fn arm(msg: &crate::ipc::message::IpcMsg) {
    if msg.label != 0xE301
        || msg.word_count != 4
        || !matches!((msg.words[0], msg.words[1]), (3 | 9, 0) | (10 | 11, 2))
    {
        return;
    }
    let sched = crate::sched::SCHEDULER.lock();
    let p = sched.current_process();
    if !p.trusted_auth_broker
        || p.is_exiting()
        || p.ipc_reply_target.is_none()
        || RACE_SESSION.load(Ordering::Relaxed) != 0
        || sched.online_cores < 2
    {
        return;
    }
    READY.store(0, Ordering::Relaxed);
    DONE.store(0, Ordering::Relaxed);
    RACE_EPOCH.store(msg.words[3], Ordering::Relaxed);
    let operation = if msg.words[0] >= 10 { 4 } else { 1 };
    PARTICIPANTS.store(operation | 2, Ordering::Relaxed);
    WINNER.store(
        if matches!(msg.words[0], 3 | 10) {
            operation
        } else {
            2
        },
        Ordering::Relaxed,
    );
    RACE_SESSION.store(msg.words[2], Ordering::Release);
    locked_event(
        "contention-armed",
        p,
        msg.words[2],
        msg.words[3],
        WINNER.load(Ordering::Relaxed),
    );
}

/// Both genuine syscall entrants rendezvous outside SCHEDULER/CAP_BROKER.
/// The losing entrant resumes only after the ordinary winner commits. The
/// iteration bound remains effective even if the BSP clock cannot advance.
pub(crate) fn boundary(session: u64, epoch: u64, operation: u64) {
    if RACE_SESSION.load(Ordering::Acquire) != session
        || RACE_EPOCH.load(Ordering::Relaxed) != epoch
    {
        return;
    }
    let bit = operation;
    if PARTICIPANTS.load(Ordering::Relaxed) & bit == 0 {
        return;
    }
    if READY.load(Ordering::Acquire) & bit != 0 {
        return;
    }
    {
        let sched = crate::sched::SCHEDULER.lock();
        let p = sched.current_process();
        if p.is_exiting()
            || !(if operation != 2 {
                p.trusted_auth_broker
            } else {
                p.trusted_session_service
            })
        {
            return;
        }
        locked_event("boundary-ready", p, session, epoch, bit);
        READY.fetch_or(bit, Ordering::Release);
    }
    let started = unsafe { core::arch::x86_64::_rdtsc() };
    let budget = crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(3);
    for i in 0..50_000_000u64 {
        if READY.load(Ordering::Acquire) == PARTICIPANTS.load(Ordering::Relaxed)
            && (WINNER.load(Ordering::Relaxed) == bit || DONE.load(Ordering::Acquire) != 0)
        {
            return;
        }
        if i % 4096 == 0 && unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(started) >= budget
        {
            break;
        }
        core::hint::spin_loop();
    }
    event("boundary-timeout", session, epoch, bit);
}

/// Called while the actual lifecycle serialization is still held, after its
/// state mutation (or a failed mint). No second authorization state machine.
pub(crate) fn committed(session: u64, epoch: u64, operation: u64) {
    if RACE_SESSION.load(Ordering::Acquire) == session
        && RACE_EPOCH.load(Ordering::Relaxed) == epoch
        && WINNER.load(Ordering::Relaxed) == operation
    {
        DONE.store(1, Ordering::Release);
    }
}

pub(crate) fn inventory(broker: &super::CapabilityBroker) {
    let [sessions, capacity, registrations, revoked, expired, retired, receipts, grants] =
        broker.session_accounting();
    let pending = broker
        .account_grants
        .iter()
        .filter(|(_, g)| g.delivery.is_some_and(|d| !d.delivered))
        .count();
    let (identity_bindings, session_metadata_bytes) = broker.session_metadata_observation();
    let grant_metadata_bytes = broker.account_grants.capacity()
        * core::mem::size_of::<(super::CapabilityToken, super::AccountGrant)>();
    let minted = MINTED.load(Ordering::Relaxed);
    let consumed = CONSUMED.load(Ordering::Relaxed);
    crate::serial_println!("[SESSION-E3] checkpoint=inventory sessions={} session_capacity={} registrations={} revoked={} expired={} retired_sessions={} receipts={} account_grants={} pending_account_deliveries={} observed_mints={} observed_consumes={} token_history={} token_history_limit={} scope=session-subset allocator=UNMEASURED cpu={}", sessions, capacity, registrations, revoked, expired, retired, receipts, grants, pending, MINTED.load(Ordering::Relaxed), CONSUMED.load(Ordering::Relaxed), broker.token_history.usage().0, super::token_history::MAX_PREPARED_TOKENS, crate::sched::current_cpu_id());
    crate::serial_println!("[SESSION-E3] checkpoint=metadata session_identity_bindings={} session_metadata_bytes={} grant_metadata_bytes={} delivered_unconsumed={} observed_retired_grants={} pending_commit_gauge=UNMEASURED allocator=UNMEASURED cpu={}", identity_bindings, session_metadata_bytes, grant_metadata_bytes, grants.saturating_sub(pending), minted.saturating_sub(consumed.saturating_add(grants as u64)), crate::sched::current_cpu_id());
}

pub(crate) fn event(kind: &str, session: u64, epoch: u64, result: u64) {
    let sched = crate::sched::SCHEDULER.lock();
    locked_event(kind, sched.current_process(), session, epoch, result);
}

/// Caller retains lifecycle serialization. Result is an error code or operation
/// number, never a bearer token. Sequence is sampled under that serialization.
pub(crate) fn locked_event(
    kind: &str,
    p: &crate::process::Process,
    session: u64,
    epoch: u64,
    result: u64,
) {
    if kind == "session-entry" && result != 0 && result != 3 {
        return;
    }
    if kind == "transition-committed" {
        let mut last = LAST_TRANSITION.lock();
        if *last == Some((session, epoch, result)) {
            return;
        }
        *last = Some((session, epoch, result));
    }
    let sequence = sequence();
    if sequence > 128 {
        return;
    }
    crate::serial_println!(
        "[SESSION-E3] seq={} checkpoint={} pid={} incarnation={} vm={} account={} sessiond={} tty={} session={} epoch={} result={} cpu={}",
        sequence, kind, p.pid, p.foreground.incarnation,
        p.address_space.identity().generation, u8::from(p.trusted_auth_broker),
        u8::from(p.trusted_session_service), u8::from(p.trusted_tty_session_service),
        session, epoch, result, crate::sched::current_cpu_id()
    );
}
