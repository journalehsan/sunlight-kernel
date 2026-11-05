//! Final, opt-in exhaustion of CAP_BROKER itself. Only the history capacity is
//! reduced. Real entropy and production insertion/lookup/removal are used.
//! This is kernel broker execution, not a claim of userspace syscall coverage.
use super::super::{
    token_history::MAX_PREPARED_TOKENS, CapError, CapabilityRights, CapabilityToken, CAP_BROKER,
};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static ARMED: AtomicBool = AtomicBool::new(false);
static FINISHED: AtomicBool = AtomicBool::new(false);
static MASK: AtomicUsize = AtomicUsize::new(0);
static ARRIVED: AtomicUsize = AtomicUsize::new(0);
static READY: AtomicUsize = AtomicUsize::new(0);
static CLAIMED: AtomicUsize = AtomicUsize::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);
static WINNERS: AtomicUsize = AtomicUsize::new(0);
static STATE: spin::Mutex<(CapabilityToken, CapabilityToken, u32, usize)> =
    spin::Mutex::new((CapabilityToken::INVALID, CapabilityToken::INVALID, 0, 0));

pub(super) fn arm(cpus: usize) {
    let mut broker = CAP_BROKER.lock();
    let baseline = broker.active_tokens();
    let (_, oldest) = broker
        .create_endpoint(0)
        .expect("global history below boundary");
    broker.destroy_endpoint(0, oldest).unwrap();
    let (endpoint, live) = broker.create_endpoint(0).unwrap();
    while broker.token_history.usage().0 < MAX_PREPARED_TOKENS - 1 {
        // Production aborted preparations burn identities; no history edits,
        // deterministic entropy, replacement broker or authority-table clearing.
        drop(broker.prepare_shared_region().unwrap());
    }
    assert_eq!(broker.active_tokens(), baseline + 1);
    assert_eq!(
        broker.check(oldest, CapabilityRights::SEND),
        Err(CapError::InvalidToken)
    );
    assert_eq!(broker.check(live, CapabilityRights::SEND), Ok(endpoint));
    crate::serial_println!("[HISTORY-G] armed scope=global-production-broker capacity={} production_capacity=65536 history={} record_bytes=8 baseline_live={} sentinel=usable oldest=denied entropy=production identity_domain=kernel-boot reset=UNSUPPORTED cpus={}", MAX_PREPARED_TOKENS, broker.token_history.usage().0, baseline, cpus);
    drop(broker);
    *STATE.lock() = (oldest, live, endpoint, baseline);
    MASK.store((1 << cpus) - 1, Ordering::Relaxed);
    ARMED.store(true, Ordering::Release);
}

pub(crate) fn on_tick(cpu: usize) {
    if !ARMED.load(Ordering::Acquire) || FINISHED.load(Ordering::Acquire) {
        return;
    }
    let bit = 1 << cpu;
    let mask = MASK.load(Ordering::Relaxed);
    if bit & mask == 0 {
        return;
    }
    if ARRIVED.fetch_or(bit, Ordering::AcqRel) & bit == 0 {
        crate::serial_println!("[HISTORY-G] ready cpu={} locks=none", cpu);
        READY.fetch_or(bit, Ordering::Release);
    }
    // Nonblocking two-stage rendezvous: no spinlock is held over a barrier and
    // a one-CPU run proceeds on its next timer entry like every other run.
    if READY.load(Ordering::Acquire) != mask || CLAIMED.fetch_or(bit, Ordering::AcqRel) & bit != 0 {
        return;
    }
    let (oldest, live, endpoint, _) = *STATE.lock();
    {
        let mut broker = CAP_BROKER.lock();
        let before = broker.active_tokens();
        let endpoints = broker.endpoints.len();
        let result = broker.create_endpoint(0);
        match result {
            Ok((_, token)) => {
                assert_eq!(WINNERS.fetch_add(1, Ordering::Relaxed), 0);
                assert_eq!(broker.token_history.usage().0, MAX_PREPARED_TOKENS);
                broker.destroy_endpoint(0, token).unwrap();
                assert_eq!(
                    broker.check(token, CapabilityRights::SEND),
                    Err(CapError::InvalidToken)
                );
                crate::serial_println!("[HISTORY-G] attempt cpu={} result=last-slot newest=denied recycled_slot=retired", cpu);
            }
            Err(error) => {
                assert_eq!(error, CapError::TokenHistoryFull);
                crate::serial_println!(
                    "[HISTORY-G] attempt cpu={} result=TokenHistoryFull partial=0",
                    cpu
                );
            }
        }
        assert_eq!(broker.active_tokens(), before);
        assert_eq!(broker.endpoints.len(), endpoints);
        let prepared = broker.issuance_stats().prepared;
        assert!(matches!(
            broker.create_endpoint(0),
            Err(CapError::TokenHistoryFull)
        ));
        assert_eq!(broker.issuance_stats().prepared, prepared);
        assert_eq!(broker.active_tokens(), before);
        assert_eq!(broker.endpoints.len(), endpoints);
        assert_eq!(
            broker.check(oldest, CapabilityRights::SEND),
            Err(CapError::InvalidToken)
        );
        assert_eq!(broker.check(live, CapabilityRights::SEND), Ok(endpoint));
    }
    let done = DONE.fetch_or(bit, Ordering::AcqRel) | bit;
    if done == mask && !FINISHED.swap(true, Ordering::AcqRel) {
        assert_eq!(WINNERS.load(Ordering::Relaxed), 1);
        let (_, live, _, baseline) = *STATE.lock();
        let mut broker = CAP_BROKER.lock();
        broker.destroy_endpoint(0, live).unwrap();
        assert_eq!(
            broker.check(live, CapabilityRights::SEND),
            Err(CapError::InvalidToken)
        );
        assert!(matches!(
            broker.create_endpoint(0),
            Err(CapError::TokenHistoryFull)
        ));
        assert_eq!(
            broker.active_tokens(),
            baseline,
            "unattributed live authority changed during exhaustion"
        );
        let (len, capacity) = broker.token_history.usage();
        assert_eq!(len, MAX_PREPARED_TOKENS);
        crate::serial_println!("[HISTORY-G] COMPLETE PASS ready_mask={} done_mask={} winners=1 rejected={} history={} capacity={} retained_bytes={} final_live={} oldest=denied newest=denied existing=retired rollback=zero-partial recovery=none syscall_error_transport=NOT_RUN", mask, done, mask.count_ones() - 1, len, capacity, capacity * 8, baseline);
        drop(broker);
        unsafe {
            x86_64::instructions::port::Port::<u32>::new(0xf4).write(0x10);
        }
    }
}
