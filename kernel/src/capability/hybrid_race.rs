//! Bounded native kernel-method contenders against an authenticated Ring 3
//! Actor's actual deferred reply. The driver never creates reply authority.
use super::{
    channel::{Error, Handle},
    delivery::RecipientIdentity,
    CAP_BROKER,
};
use crate::sched::{Scheduler, SCHEDULER};
use core::sync::atomic::{AtomicU64, Ordering};

static MODE: AtomicU64 = AtomicU64::new(0);
static ENTERED: AtomicU64 = AtomicU64::new(0);
static HELD: AtomicU64 = AtomicU64::new(0);
static CONTENDED: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicU64 = AtomicU64::new(0);
static ACTOR_CPU: AtomicU64 = AtomicU64::new(u64::MAX);
static ACTOR: AtomicU64 = AtomicU64::new(0);
static ACTOR_INC: AtomicU64 = AtomicU64::new(0);
static CHANNEL: AtomicU64 = AtomicU64::new(0);
static ENDPOINT: AtomicU64 = AtomicU64::new(0);
const PUBLISH: u64 = 1;
const RETIRE: u64 = 2;

fn wait(mut ready: impl FnMut() -> bool) {
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    let budget = crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(5);
    for n in 0..100_000_000u64 {
        if ready() {
            return;
        }
        if n % 4096 == 0 && unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) >= budget {
            break;
        }
        core::hint::spin_loop();
    }
    panic!("hybrid lifecycle bounded contention timeout");
}

fn event(kind: &str, actor: u64) {
    crate::serial_println!(
        "[FL-R2-RACE] mode={} channel={} event={} actor={} cpu={}",
        MODE.load(Ordering::Relaxed),
        CHANNEL.load(Ordering::Relaxed),
        kind,
        actor,
        crate::sched::current_cpu_id()
    );
}

pub(crate) fn arm(sched: &Scheduler, h: Handle, mode: u64) -> Result<u64, Error> {
    if !matches!(mode,1..=6|9..=14)
        || (MODE.load(Ordering::Acquire) != 0 && DONE.load(Ordering::Acquire) != 3)
    {
        return Err(Error::Denied);
    }
    let current = sched.current_process();
    let mut caps = CAP_BROKER.lock();
    let row = caps.channels.row_mut(h)?;
    let target = row.contract.hybrid_reply.ok_or(Error::Denied)?;
    if !row.live()
        || row.consumer != Some(RecipientIdentity::of(current))
        || current.contract_service != Some(row.contract.service)
        || !current
            .deferred_reply_targets
            .iter()
            .any(|e| e.target == target)
        || !crate::ipc::reply_target_is_live(target, sched)
    {
        return Err(Error::Denied);
    }
    ACTOR.store(current.pid as u64, Ordering::Relaxed);
    ACTOR_INC.store(current.foreground.incarnation, Ordering::Relaxed);
    ACTOR_CPU.store(crate::sched::current_cpu_id() as u64, Ordering::Relaxed);
    CHANNEL.store(h.generation, Ordering::Relaxed);
    ENDPOINT.store(target.endpoint_id as u64, Ordering::Relaxed);
    ENTERED.store(0, Ordering::Relaxed);
    HELD.store(0, Ordering::Relaxed);
    CONTENDED.store(0, Ordering::Relaxed);
    DONE.store(0, Ordering::Relaxed);
    MODE.store(mode, Ordering::Release);
    crate::serial_println!("[FL-R2-RACE] mode={} channel={} event=armed caller={} ci={} cv={} service={} si={} sv={} policy={} pg={} endpoint={} call_generation={} scope=authenticated-ring3/kernel-method-retirement cpu={}", mode,h.generation,row.creator.pid,row.creator.incarnation,row.creator.vm_generation,current.pid,current.foreground.incarnation,current.address_space.identity().generation,row.contract.policy_identity.unwrap().policy_id,row.contract.policy_identity.unwrap().generation,target.endpoint_id,target.call.generation,crate::sched::current_cpu_id());
    Ok(0)
}

fn contend(actor: u64) -> spin::MutexGuard<'static, Scheduler> {
    event("entered", actor);
    assert_eq!(ENTERED.fetch_or(actor, Ordering::AcqRel) & actor, 0);
    wait(|| ENTERED.load(Ordering::Acquire) == 3);
    let winner = if MODE.load(Ordering::Relaxed) % 2 == 1 {
        PUBLISH
    } else {
        RETIRE
    };
    if actor == winner {
        let sched = SCHEDULER.lock();
        HELD.store(actor, Ordering::Release);
        wait(|| CONTENDED.load(Ordering::Acquire) != 0);
        event("lock-held", actor);
        sched
    } else {
        wait(|| HELD.load(Ordering::Acquire) != 0);
        assert!(
            SCHEDULER.try_lock().is_none(),
            "hybrid native lock contention absent"
        );
        event("lock-contended", actor);
        CONTENDED.store(actor, Ordering::Release);
        wait(|| DONE.load(Ordering::Acquire) & winner != 0);
        SCHEDULER.lock()
    }
}

pub(crate) fn acquire(token: u64) -> (spin::MutexGuard<'static, Scheduler>, bool) {
    let sched = SCHEDULER.lock();
    let p = sched.current_process();
    let selected = MODE.load(Ordering::Acquire) != 0
        && DONE.load(Ordering::Acquire) & PUBLISH == 0
        && p.pid as u64 == ACTOR.load(Ordering::Relaxed)
        && p.foreground.incarnation == ACTOR_INC.load(Ordering::Relaxed)
        && p.deferred_reply_targets.iter().any(|e| e.token == token);
    if !selected {
        return (sched, false);
    }
    drop(sched);
    (contend(PUBLISH), true)
}

pub(crate) fn finished(selected: bool, result: u64, token: u64, sched: &mut Scheduler) {
    if selected {
        let mode = MODE.load(Ordering::Relaxed);
        let pid = ACTOR.load(Ordering::Relaxed) as usize;
        let endpoint = ENDPOINT.load(Ordering::Relaxed) as u32;
        let mut caps = CAP_BROKER.lock();
        assert!(
            crate::ipc::with_shard(endpoint, |bus| crate::ipc::complete_deferred_reply(
                pid,
                token,
                crate::ipc::IpcMsg::empty(),
                sched,
                &mut caps,
                bus
            ))
            .is_err()
        );
        crate::serial_println!(
            "[FL-R2-RACE] mode={} channel={} event={} result={} duplicate=rejected cpu={}",
            mode,
            CHANNEL.load(Ordering::Relaxed),
            if mode >= 13 {
                "reply-retire-return"
            } else {
                "publish-return"
            },
            result,
            crate::sched::current_cpu_id()
        );
        DONE.fetch_or(PUBLISH, Ordering::Release);
    }
}

pub(crate) fn retire_current(token: u64) -> Result<u64, Error> {
    let (mut sched, selected) = acquire(token);
    if !selected || MODE.load(Ordering::Relaxed) < 13 {
        return Err(Error::Denied);
    }
    let pid = sched.current_process().pid;
    crate::ipc::retire_exec_replies(pid, None, &mut sched);
    finished(true, 0, token, &mut sched);
    Ok(0)
}

pub(crate) fn idle(cpu: usize) {
    // Caller is pinned on CPU 0, Actor on CPU 1. Only the idle original caller
    // CPU drives the method contender; no scheduler guard spans rendezvous.
    if cpu != 0 || MODE.load(Ordering::Acquire) == 0 || DONE.load(Ordering::Acquire) & RETIRE != 0 {
        return;
    }
    assert_ne!(cpu as u64, ACTOR_CPU.load(Ordering::Acquire));
    let mut sched = contend(RETIRE);
    let caps = CAP_BROKER.lock();
    let row = caps
        .channels
        .rows
        .iter()
        .find(|r| r.handle.generation == CHANNEL.load(Ordering::Relaxed))
        .unwrap();
    let h = row.handle;
    let caller = row.creator;
    let service = row.consumer.unwrap();
    let mode = MODE.load(Ordering::Relaxed);
    drop(caps);
    match mode {
        1 | 2 => {
            // Use the ordinary cancellation method on the exact original
            // caller; this is explicitly not a foreign cancellation syscall.
            let endpoint = ENDPOINT.load(Ordering::Relaxed) as u32;
            let mut caps = CAP_BROKER.lock();
            crate::ipc::with_shard(endpoint, |bus| {
                crate::ipc::handle_ipc_cancel(caller.pid, &mut sched, &mut caps, bus)
            })
            .unwrap();
        }
        3 | 4 | 13 | 14 => {
            CAP_BROKER.lock().channel_revoke(h, caller).unwrap();
        }
        5 | 6 => {
            CAP_BROKER
                .lock()
                .channels
                .policies
                .retire_process(service.pid);
        }
        9 | 10 => {
            assert!(sched.terminate_process_by_pid(caller.pid, 0, "hybrid-caller-exit"));
        }
        11 | 12 => {
            assert!(sched.terminate_process_by_pid(service.pid, 0, "hybrid-service-exit"));
        }
        _ => unreachable!(),
    }
    sched.fastlane_hybrid_sweep(&mut CAP_BROKER.lock());
    event("retire-return", RETIRE);
    DONE.fetch_or(RETIRE, Ordering::Release);
}

pub(crate) fn inventory(sched: &mut Scheduler, h: Handle) -> Result<u64, Error> {
    let who = RecipientIdentity::of(sched.current_process());
    let mut caps = CAP_BROKER.lock();
    let row = caps.channels.row_mut(h)?;
    if who != row.creator
        && !sched.processes.iter().any(|p| {
            p.pid == row.creator.pid
                && p.foreground.incarnation == row.creator.incarnation
                && p.ppid == who.pid
                && p.is_exiting()
        })
    {
        return Err(Error::Denied);
    }
    let creator = row.creator;
    let consumer = row.consumer.ok_or(Error::Incomplete)?;
    let pending_reply = usize::from(row.contract.hybrid_reply.is_some());
    let channels = usize::from(row.live());
    let receipt = usize::from(row.consumer_receipt.is_some());
    let waiter = usize::from(row.bell.as_ref().is_some_and(|b| b.has_waiter()))
        + usize::from(row.bell_pending.is_some());
    let backing = row.backing.iter().filter(|b| b.is_some()).count();
    let endpoint = row.contract.hybrid_endpoint.ok_or(Error::Incomplete)?;
    let policy_refs = usize::from(row.live() && row.contract.policy_identity.is_some());
    let participants = [creator.pid, consumer.pid];
    let policies = caps.channels.policies.scoped_count(participants);
    let deliveries = caps
        .account_grants
        .iter()
        .filter(|(_, g)| {
            g.delivery.is_some_and(|d| {
                !d.delivered
                    && (participants.contains(&d.recipient.pid)
                        || participants.contains(&d.issuer_pid))
            })
        })
        .count()
        + caps
            .auth_session_grants
            .iter()
            .filter(|g| {
                g.delivery.is_some_and(|d| {
                    !d.delivered
                        && (participants.contains(&d.recipient.pid)
                            || participants.contains(&d.issuer_pid))
                })
            })
            .count()
        + caps
            .vfs_caps
            .iter()
            .filter(|g| {
                g.delivery.is_some_and(|d| {
                    !d.delivered
                        && (participants.contains(&d.recipient.pid)
                            || participants.contains(&d.issuer_pid))
                })
            })
            .count();
    let (queued, ipc_waiters) = crate::ipc::with_shard(endpoint, |bus| {
        (
            bus.pending_count(endpoint),
            bus.pending_callers_count(endpoint),
        )
    });
    let grants: usize = participants
        .iter()
        .map(|pid| caps.recipient_grant_count(*pid))
        .sum();
    let mut pending = 0;
    let mut deferred = 0;
    let mut ordinary = 0;
    let mut views = 0;
    for p in &sched.processes {
        if participants.contains(&p.pid) {
            pending += usize::from(p.pending_call.is_some());
            deferred += p.deferred_reply_targets.len();
            ordinary += usize::from(p.ipc_reply_target.is_some());
            views += p.mapped_shared.len();
        }
    }
    let total = channels
        + receipt
        + waiter
        + backing
        + pending_reply
        + grants
        + pending
        + deferred
        + ordinary
        + views
        + policies
        + policy_refs
        + deliveries
        + queued
        + ipc_waiters;
    if total == 0 {
        crate::serial_println!("[FL-R2-INVENTORY] channel={} channels={} receipts={} waiters={} backing={} pending_reply={} grants={} pending={} deferred={} ordinary={} views={} policies={} policy_refs={} deliveries={} queued={} ipc_waiters={} total={} scope=participants",h.generation,channels,receipt,waiter,backing,pending_reply,grants,pending,deferred,ordinary,views,policies,policy_refs,deliveries,queued,ipc_waiters,total);
    }
    Ok(total as u64)
}

pub(crate) fn stale_policy(sched: &Scheduler, h: Handle, generation: u64) -> Result<u64, Error> {
    let who = RecipientIdentity::of(sched.current_process());
    let mut caps = CAP_BROKER.lock();
    let row = caps.channels.row_mut(h)?;
    if !row.live() || who != row.creator {
        return Err(Error::Denied);
    }
    let peer = row.consumer.ok_or(Error::Incomplete)?;
    let service = row.contract.service;
    let original = row.contract.policy_identity.ok_or(Error::Incomplete)?;
    let identity = super::channel_contract::PolicyIdentity {
        generation,
        ..original
    };
    caps.channels
        .policies
        .resolve(identity, peer, service)
        .map(|v| v as u64)
}
