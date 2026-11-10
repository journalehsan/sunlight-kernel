//! SMP-safe address-space activation and synchronous TLB shootdown.
//!
//! PCID is not enabled in SunlightOS. A CR3 load therefore discards all
//! non-global translations, and CPUs that are not executing the target root
//! cannot retain a tagged translation for it.

use crate::process::mm2b_state::{
    ActiveCpuSet, AddressSpaceIdentity, MailboxRequest, ShootdownMailbox, FULL_FLUSH_PAGES,
    MAX_TRACKED_CPUS,
};
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::{PhysAddr, VirtAddr};

pub const SHOOTDOWN_VECTOR: u8 = 0xF1;
#[cfg(any(feature = "mm2b_smp_test", feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"))]
pub const TEST_VECTOR: u8 = 0xF2;
pub const RANGE_FLUSH_PAGE_THRESHOLD: u64 = 32;

const SHOOTDOWN_TIMEOUT_SPINS: u64 = 100_000_000;

static ACTIVE_CPUS: ActiveCpuSet = ActiveCpuSet::new();
static MAILBOXES: [ShootdownMailbox; MAX_TRACKED_CPUS] =
    [const { ShootdownMailbox::new() }; MAX_TRACKED_CPUS];
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static MUTATION_EPOCH: AtomicU64 = AtomicU64::new(1);
static KERNEL_PML4: AtomicU64 = AtomicU64::new(0);
static SHOOTDOWN_SERIALIZER: spin::Mutex<()> = spin::Mutex::new(());

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static TARGET_CPUS: AtomicU64 = AtomicU64::new(0);
static LOCAL_INVALIDATIONS: AtomicU64 = AtomicU64::new(0);
static REMOTE_INVALIDATIONS: AtomicU64 = AtomicU64::new(0);
static RANGE_FLUSHES: AtomicU64 = AtomicU64::new(0);
static FULL_FLUSHES: AtomicU64 = AtomicU64::new(0);
static ACKNOWLEDGEMENTS: AtomicU64 = AtomicU64::new(0);
static CONTEXT_SWITCH_RETRIES: AtomicU64 = AtomicU64::new(0);
static NO_REMOTE_TARGETS: AtomicU64 = AtomicU64::new(0);
static INVARIANT_FAILURES: AtomicU64 = AtomicU64::new(0);

#[cfg(any(feature = "mm2b_smp_test", feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"))]
struct TestMailbox {
    sequence: AtomicU64,
    acknowledgement: AtomicU64,
    command: AtomicU64,
    target_pml4: AtomicU64,
    target_generation: AtomicU64,
    address: AtomicU64,
    result: AtomicU64,
}

#[cfg(any(feature = "mm2b_smp_test", feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"))]
impl TestMailbox {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            acknowledgement: AtomicU64::new(0),
            command: AtomicU64::new(0),
            target_pml4: AtomicU64::new(0),
            target_generation: AtomicU64::new(0),
            address: AtomicU64::new(0),
            result: AtomicU64::new(0),
        }
    }
}

#[cfg(any(feature = "mm2b_smp_test", feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"))]
static TEST_MAILBOXES: [TestMailbox; MAX_TRACKED_CPUS] =
    [const { TestMailbox::new() }; MAX_TRACKED_CPUS];

#[cfg(any(feature = "mm2b_smp_test", feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"))]
static NEXT_TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationError {
    InvalidRange,
}

#[derive(Debug, Clone, Copy)]
enum FlushKind {
    Range { start: u64, pages: u64 },
    Full,
}

impl FlushKind {
    fn mailbox_fields(self) -> (u64, u64) {
        match self {
            Self::Range { start, pages } => (start, pages),
            Self::Full => (0, FULL_FLUSH_PAGES),
        }
    }
}

fn online_mask() -> u64 {
    let count = crate::sched::ONLINE_CORES
        .load(Ordering::Acquire)
        .min(MAX_TRACKED_CPUS);
    if count == MAX_TRACKED_CPUS {
        u64::MAX
    } else {
        (1u64 << count).wrapping_sub(1)
    }
}

pub fn register_kernel_root() {
    let root = x86_64::registers::control::Cr3::read()
        .0
        .start_address()
        .as_u64();
    let previous = KERNEL_PML4.compare_exchange(0, root, Ordering::AcqRel, Ordering::Acquire);
    if let Err(existing) = previous {
        assert_eq!(existing, root, "kernel CR3 root changed unexpectedly");
    }
}

/// Activate `identity` on the current CPU. Scheduler/context-switch callers
/// serialize process selection, while the epoch retry closes the race with a
/// lock-free shootdown target snapshot: a switch that overlaps a mutation
/// either joins its active mask or reloads CR3 after observing the new epoch.
pub unsafe fn activate(identity: AddressSpaceIdentity) {
    assert!(
        identity.is_valid(),
        "cannot activate an invalid address space"
    );
    let cpu_id = crate::sched::current_cpu_id();
    loop {
        let before = MUTATION_EPOCH.load(Ordering::Acquire);
        x86_64::registers::control::Cr3::write(
            x86_64::structures::paging::PhysFrame::from_start_address_unchecked(PhysAddr::new(
                identity.pml4_phys,
            )),
            x86_64::registers::control::Cr3Flags::empty(),
        );
        ACTIVE_CPUS.enter(cpu_id, identity);
        let after = MUTATION_EPOCH.load(Ordering::Acquire);
        if before == after {
            return;
        }
        CONTEXT_SWITCH_RETRIES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Put an idle CPU back on the permanent boot/kernel root so a dead process's
/// PML4 can be reclaimed rather than remaining loaded by an idle core.
pub unsafe fn activate_kernel_root() {
    let cpu_id = crate::sched::current_cpu_id();
    let root = KERNEL_PML4.load(Ordering::Acquire);
    assert_ne!(root, 0, "kernel CR3 root was not registered");
    x86_64::registers::control::Cr3::write(
        x86_64::structures::paging::PhysFrame::from_start_address_unchecked(PhysAddr::new(root)),
        x86_64::registers::control::Cr3Flags::empty(),
    );
    ACTIVE_CPUS.leave(cpu_id);
}

pub fn active_cpu_mask(identity: AddressSpaceIdentity) -> u64 {
    let (mask, retries) = ACTIVE_CPUS.mask(identity, online_mask());
    CONTEXT_SWITCH_RETRIES.fetch_add(retries, Ordering::Relaxed);
    mask
}

pub fn invalidate_page(identity: AddressSpaceIdentity, address: VirtAddr) {
    invalidate(
        identity,
        FlushKind::Range {
            start: address.as_u64(),
            pages: 1,
        },
    );
}

pub fn invalidate_full(identity: AddressSpaceIdentity) {
    invalidate(identity, FlushKind::Full);
}

pub fn invalidate_range(
    identity: AddressSpaceIdentity,
    start: u64,
    pages: u64,
) -> Result<(), InvalidationError> {
    if pages == 0 {
        return Ok(());
    }
    let bytes = pages
        .checked_mul(4096)
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(InvalidationError::InvalidRange)?;
    if start & 0xfff != 0 || crate::memory::user::UserRange::new(start, bytes).is_err() {
        return Err(InvalidationError::InvalidRange);
    }
    let kind = if pages <= RANGE_FLUSH_PAGE_THRESHOLD {
        FlushKind::Range { start, pages }
    } else {
        FlushKind::Full
    };
    invalidate(identity, kind);
    Ok(())
}

pub(crate) fn invalidate_retirement_range(
    identity: AddressSpaceIdentity, start: u64, pages: u64,
    retirement: &super::retirement::Token,
) -> Result<(), InvalidationError> {
    let bytes = pages.checked_mul(4096).and_then(|n| usize::try_from(n).ok())
        .ok_or(InvalidationError::InvalidRange)?;
    if pages == 0 || start & 4095 != 0 || crate::memory::user::UserRange::new(start, bytes).is_err() {
        return Err(InvalidationError::InvalidRange);
    }
    let kind = if pages <= RANGE_FLUSH_PAGE_THRESHOLD { FlushKind::Range { start, pages } } else { FlushKind::Full };
    invalidate_owned(identity, kind, Some(retirement));
    Ok(())
}

/// Table withdrawal covers a hierarchy, so use the full address-space flush
/// with the same exact-sequence ownership and failure protocol as leaf retire.
pub(crate) fn invalidate_retirement_all(
    identity: AddressSpaceIdentity,
    retirement: &super::retirement::Token,
) {
    invalidate_owned(identity, FlushKind::Full, Some(retirement));
}

fn invalidate(identity: AddressSpaceIdentity, kind: FlushKind) {
    invalidate_owned(identity, kind, None);
}

fn invalidate_owned(identity: AddressSpaceIdentity, kind: FlushKind, retirement: Option<&super::retirement::Token>) {
    assert!(identity.is_valid(), "shootdown target identity is invalid");
    let _serialized = SHOOTDOWN_SERIALIZER.lock();
    let sequence = NEXT_SEQUENCE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .expect("TLB shootdown sequence exhausted");
    assert_ne!(sequence, 0, "TLB sequence zero is reserved");

    // The PTE write precedes this Release update. A CPU entering concurrently
    // either appears in the active snapshot below or observes the epoch and
    // retries its CR3 load after the new page-table state is visible.
    MUTATION_EPOCH.fetch_add(1, Ordering::Release);
    REQUESTS.fetch_add(1, Ordering::Relaxed);

    let cpu_id = crate::sched::current_cpu_id();
    let local_bit = 1u64 << cpu_id;
    let active = active_cpu_mask(identity);
    if active & local_bit != 0 {
        perform_flush(kind);
        LOCAL_INVALIDATIONS.fetch_add(1, Ordering::Relaxed);
    }

    let targets = active & !local_bit;
    if let Some(owner) = retirement {
        assert!(super::retirement::OWNERS.lock().bind(owner, sequence, targets));
    }
    #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
    super::quota_lifecycle::tlb(identity, kind.mailbox_fields().0, sequence, targets, "request");
    #[cfg(all(debug_assertions, feature = "fastlane_census_trace"))]
    super::census_trace::record(
        super::census_trace::TLB_REQUEST,
        [
            sequence,
            identity.generation,
            identity.pml4_phys,
            kind.mailbox_fields().0,
            kind.mailbox_fields().1,
            targets,
            cpu_id as u64,
            active,
        ],
    );
    #[cfg(feature = "authenticated_exec_test")]
    crate::capability::exec_retirement_trace::tlb(
        identity,
        kind.mailbox_fields().0,
        "request",
        sequence,
        targets,
        cpu_id,
    );
    #[cfg(all(debug_assertions, feature = "fastlane_mapping_test"))]
    crate::capability::mapping_trace::tlb(
        identity,
        kind.mailbox_fields().0,
        "request",
        sequence,
        targets,
        cpu_id,
    );
    if targets == 0 {
        if let Some(owner) = retirement {
            assert!(super::retirement::OWNERS.lock().complete(owner, sequence, 0));
        }
        NO_REMOTE_TARGETS.fetch_add(1, Ordering::Relaxed);
        #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
        super::quota_lifecycle::tlb(identity, kind.mailbox_fields().0, sequence, targets, "complete");
        #[cfg(feature = "authenticated_exec_test")]
        crate::capability::exec_retirement_trace::tlb(
            identity,
            kind.mailbox_fields().0,
            "complete",
            sequence,
            targets,
            cpu_id,
        );
        #[cfg(all(debug_assertions, feature = "fastlane_mapping_test"))]
        crate::capability::mapping_trace::tlb(
            identity,
            kind.mailbox_fields().0,
            "complete",
            sequence,
            targets,
            cpu_id,
        );
        #[cfg(all(debug_assertions, feature = "fastlane_census_trace"))]
        super::census_trace::record(
            super::census_trace::TLB_COMPLETE,
            [
                sequence,
                identity.generation,
                identity.pml4_phys,
                kind.mailbox_fields().0,
                kind.mailbox_fields().1,
                targets,
                cpu_id as u64,
                0,
            ],
        );
        return;
    }
    TARGET_CPUS.fetch_add(targets.count_ones() as u64, Ordering::Relaxed);

    let (start, pages) = kind.mailbox_fields();
    for target_cpu in 0..MAX_TRACKED_CPUS {
        if targets & (1u64 << target_cpu) == 0 {
            continue;
        }
        let request = MailboxRequest {
            sequence,
            target: identity,
            start,
            pages,
        };
        if MAILBOXES[target_cpu].try_publish(request).is_err() {
            invariant_failure("attempted to overwrite an unacknowledged TLB mailbox");
        }
    }

    #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
    if let Some(owner) = retirement {
        let mode = super::retirement::fault_mode();
        if mode == 4 || mode == 5 {
            KFP_ACK_TEST_SEQUENCE.store(sequence, Ordering::Release);
        }
        if mode == 1 {
            crate::arch::x86_64::lapic::KFP_STUCK_ICR.store(true, Ordering::Release);
        }
        if mode == 2 {
            quarantine_owner(owner, sequence, targets, super::retirement::Failure::FixedDelivery, None);
            super::retirement::failure_receipt(owner);
            panic!("KFP2A2 injected APIC NotSent after withdrawal sequence={sequence}; reclamation forbidden");
        }
        if mode == 3 { KFP_MISSING_ACK.store(sequence, Ordering::Release); }
    }

    for target_cpu in 0..MAX_TRACKED_CPUS {
        if targets & (1u64 << target_cpu) == 0 {
            continue;
        }
        unsafe {
            // Anonymous munmap has a persistent owner; other callers can still
            // own obligations only on the stack. Neither permits resumption
            // after failed synchronization; preserve the fail-stop boundary.
            if let Err(error) = crate::arch::x86_64::lapic::send_fixed_ipi(target_cpu, SHOOTDOWN_VECTOR) {
                if let Some(owner) = retirement {
                    quarantine_owner(owner, sequence, targets, super::retirement::Failure::FixedDelivery, Some(target_cpu));
                }
                #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
                if let Some(owner) = retirement {
                    crate::serial_println!("[KFP2A2] icr-exhausted reads={}", crate::arch::x86_64::lapic::KFP_ICR_READS.load(Ordering::Acquire));
                    super::retirement::failure_receipt(owner);
                }
                panic!("TLB fixed IPI failed: cpu={target_cpu} sequence={sequence} outcome={error:?}; reclamation forbidden");
            }
            // Syscalls and fault handlers currently run with IF=0. The NMI
            // doorbell consumes the same claimed mailbox so a target spinning
            // on SCHEDULER cannot deadlock the initiator; the fixed-vector IPI
            // remains the normal, dedicated, EOI-tracked delivery path.
            if let Err(error) = crate::arch::x86_64::lapic::send_nmi(target_cpu) {
                if let Some(owner) = retirement {
                    quarantine_owner(owner, sequence, targets, super::retirement::Failure::NmiDelivery, Some(target_cpu));
                }
                panic!("TLB NMI failed: cpu={target_cpu} sequence={sequence} outcome={error:?}; reclamation forbidden");
            }
        }
    }

    let mut spins = 0u64;
    loop {
        let mut pending = 0u64;
        for target_cpu in 0..MAX_TRACKED_CPUS {
            let bit = 1u64 << target_cpu;
            if targets & bit != 0 && !MAILBOXES[target_cpu].acknowledged(sequence) {
                pending |= bit;
            }
        }
        if pending == 0 {
            if let Some(owner) = retirement {
                assert!(super::retirement::OWNERS.lock().complete(owner, sequence, targets));
            }
            break;
        }
        spins += 1;
        #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
        if retirement.is_some() && super::retirement::fault_mode() == 3 && spins == 100_000 {
            quarantine_owner(retirement.unwrap(), sequence, targets, super::retirement::Failure::MissingAck, None);
            super::retirement::failure_receipt(retirement.unwrap());
            invariant_failure("injected missing ACK exhausted diagnostic budget");
        }
        if spins >= SHOOTDOWN_TIMEOUT_SPINS {
            if let Some(owner) = retirement {
                quarantine_owner(owner, sequence, targets, super::retirement::Failure::MissingAck, None);
            }
            invariant_failure("timed out waiting for TLB shootdown acknowledgement");
        }
        core::hint::spin_loop();
    }
    #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
    super::quota_lifecycle::tlb(identity, kind.mailbox_fields().0, sequence, targets, "complete");
    #[cfg(feature = "authenticated_exec_test")]
    {
        for ack_cpu in 0..MAX_TRACKED_CPUS {
            if targets & (1u64 << ack_cpu) != 0 {
                crate::capability::exec_retirement_trace::tlb(
                    identity,
                    start,
                    "ack-observed",
                    sequence,
                    targets,
                    ack_cpu,
                );
            }
        }
        crate::capability::exec_retirement_trace::tlb(
            identity, start, "complete", sequence, targets, cpu_id,
        );
    }
    #[cfg(all(debug_assertions, feature = "fastlane_mapping_test"))]
    {
        for ack_cpu in 0..MAX_TRACKED_CPUS {
            if targets & (1u64 << ack_cpu) != 0 {
                crate::capability::mapping_trace::tlb(
                    identity, start, "ack", sequence, targets, ack_cpu,
                );
            }
        }
        crate::capability::mapping_trace::tlb(
            identity, start, "complete", sequence, targets, cpu_id,
        );
    }
    #[cfg(all(debug_assertions, feature = "fastlane_census_trace"))]
    super::census_trace::record(
        super::census_trace::TLB_COMPLETE,
        [
            sequence,
            identity.generation,
            identity.pml4_phys,
            start,
            pages,
            targets,
            cpu_id as u64,
            0,
        ],
    );
}

fn quarantine_owner(owner: &super::retirement::Token, sequence: u64, targets: u64,
    failure: super::retirement::Failure, failed_cpu: Option<usize>) {
    let mut received = 0;
    for cpu in 0..MAX_TRACKED_CPUS {
        if targets & (1u64 << cpu) != 0 && MAILBOXES[cpu].acknowledged(sequence) { received |= 1u64 << cpu; }
    }
    assert!(super::retirement::OWNERS.lock().quarantine(owner, sequence, received, failure, failed_cpu));
}

fn request_kind(request: MailboxRequest) -> FlushKind {
    if request.is_full_flush() || request.pages > RANGE_FLUSH_PAGE_THRESHOLD {
        FlushKind::Full
    } else {
        FlushKind::Range {
            start: request.start,
            pages: request.pages,
        }
    }
}

/// Consume this CPU's mailbox. Called by both the dedicated fixed-vector ISR
/// and its NMI doorbell; `try_claim` guarantees one flush and one ack.
pub fn handle_shootdown_ipi() {
    let cpu_id = crate::sched::current_cpu_id();
    let mailbox = &MAILBOXES[cpu_id];
    let Some(request) = mailbox.pending() else {
        return;
    };
    if !mailbox.try_claim(request.sequence) {
        return;
    }

    let (current, retries) = ACTIVE_CPUS.current(cpu_id);
    CONTEXT_SWITCH_RETRIES.fetch_add(retries, Ordering::Relaxed);
    if current == Some(request.target) {
        perform_flush(request_kind(request));
        REMOTE_INVALIDATIONS.fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(feature = "authenticated_exec_test")]
    crate::capability::exec_retirement_trace::ack_produced(
        request,
        current == Some(request.target),
    );
    #[cfg(all(debug_assertions, feature = "fastlane_mapping_test"))]
    crate::capability::mapping_trace::ack(request, current == Some(request.target));
    #[cfg(all(debug_assertions, feature = "fastlane_census_trace"))]
    super::census_trace::record(
        super::census_trace::TLB_ACK,
        [
            request.sequence,
            request.target.generation,
            request.target.pml4_phys,
            request.start,
            request.pages,
            0,
            cpu_id as u64,
            u64::from(current == Some(request.target)),
        ],
    );
    #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
    super::quota_lifecycle::ack(request.sequence, cpu_id, current == Some(request.target));
    #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
    if KFP_MISSING_ACK.load(Ordering::Acquire) == request.sequence { return; }
    #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
    if KFP_ACK_TEST_SEQUENCE.load(Ordering::Acquire) == request.sequence {
        match super::retirement::fault_mode() {
            4 => {
                // Bounded delay in the real claimed handler, after the actual
                // flush and before publishing its actual completion.
                for _ in 0..4096 { core::hint::spin_loop(); }
                assert!(!mailbox.acknowledged(request.sequence));
                KFP_ACK_TEST_OBSERVED[cpu_id].store(request.sequence, Ordering::Release);
            }
            5 => {
                // Simulate an old operation writing its completion. The
                // production waiter must still require exact sequence equality.
                mailbox.acknowledge(request.sequence - 1);
                assert!(!mailbox.acknowledged(request.sequence));
                mailbox.acknowledge(request.sequence - 1);
                assert!(!mailbox.acknowledged(request.sequence));
                assert!(!mailbox.try_claim(request.sequence));
                KFP_ACK_TEST_OBSERVED[cpu_id].store(request.sequence, Ordering::Release);
            }
            _ => {}
        }
    }
    mailbox.acknowledge(request.sequence);
    ACKNOWLEDGEMENTS.fetch_add(1, Ordering::Relaxed);
}

fn perform_flush(kind: FlushKind) {
    match kind {
        FlushKind::Range { start, pages } => {
            for page in 0..pages {
                x86_64::instructions::tlb::flush(VirtAddr::new(start + page * 4096));
            }
            RANGE_FLUSHES.fetch_add(1, Ordering::Relaxed);
        }
        FlushKind::Full => {
            let (frame, flags) = x86_64::registers::control::Cr3::read();
            unsafe {
                x86_64::registers::control::Cr3::write(frame, flags);
            }
            FULL_FLUSHES.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn invariant_failure(message: &str) -> ! {
    INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
    panic!("MM-2B TLB invariant failure: {message}");
}

#[cfg(any(feature = "mm2b_smp_test", feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"))]
fn test_broadcast(command: u64, identity: AddressSpaceIdentity, address: u64, targets: u64) -> u64 {
    let sequence = NEXT_TEST_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    for cpu_id in 0..MAX_TRACKED_CPUS {
        if targets & (1u64 << cpu_id) == 0 {
            continue;
        }
        let mailbox = &TEST_MAILBOXES[cpu_id];
        let previous = mailbox.sequence.load(Ordering::Acquire);
        assert_eq!(
            mailbox.acknowledgement.load(Ordering::Acquire),
            previous,
            "MM-2B test mailbox overwrite"
        );
        mailbox
            .target_pml4
            .store(identity.pml4_phys, Ordering::Relaxed);
        mailbox
            .target_generation
            .store(identity.generation, Ordering::Relaxed);
        mailbox.address.store(address, Ordering::Relaxed);
        mailbox.command.store(command, Ordering::Relaxed);
        mailbox.sequence.store(sequence, Ordering::Release);
    }
    for cpu_id in 0..MAX_TRACKED_CPUS {
        if targets & (1u64 << cpu_id) != 0 {
            unsafe {
                if crate::arch::x86_64::lapic::send_fixed_ipi(cpu_id, TEST_VECTOR).is_err() {
                    invariant_failure("MM-2B test IPI delivery failed");
                }
            }
        }
    }

    let mut spins = 0u64;
    loop {
        let complete = (0..MAX_TRACKED_CPUS).all(|cpu_id| {
            targets & (1u64 << cpu_id) == 0
                || TEST_MAILBOXES[cpu_id]
                    .acknowledgement
                    .load(Ordering::Acquire)
                    == sequence
        });
        if complete {
            return sequence;
        }
        spins += 1;
        if spins >= SHOOTDOWN_TIMEOUT_SPINS {
            invariant_failure("timed out waiting for MM-2B test IPI");
        }
        core::hint::spin_loop();
    }
}

#[cfg(any(feature = "mm2b_smp_test", feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"))]
pub fn handle_test_ipi() {
    const ENTER_AND_READ: u64 = 1;
    const READ: u64 = 2;
    const LEAVE: u64 = 3;

    let cpu_id = crate::sched::current_cpu_id();
    let mailbox = &TEST_MAILBOXES[cpu_id];
    let sequence = mailbox.sequence.load(Ordering::Acquire);
    if sequence == 0 || mailbox.acknowledgement.load(Ordering::Acquire) == sequence {
        return;
    }
    let command = mailbox.command.load(Ordering::Relaxed);
    if command == LEAVE {
        unsafe {
            activate_kernel_root();
        }
    } else {
        let identity = AddressSpaceIdentity {
            pml4_phys: mailbox.target_pml4.load(Ordering::Relaxed),
            generation: mailbox.target_generation.load(Ordering::Relaxed),
        };
        if command == ENTER_AND_READ {
            unsafe {
                activate(identity);
            }
        } else {
            assert_eq!(command, READ, "invalid MM-2B test command");
        }
        let value =
            unsafe { (mailbox.address.load(Ordering::Relaxed) as *const u64).read_volatile() };
        mailbox.result.store(value, Ordering::Relaxed);
    }
    mailbox.acknowledgement.store(sequence, Ordering::Release);
}

#[cfg(any(
    feature = "mm2d_munmap_test",
    feature = "mm2e_mprotect_test",
    feature = "fastlane_f1_audit",
    feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"
))]
pub fn test_activate_and_read(identity: AddressSpaceIdentity, address: u64, targets: u64) {
    test_broadcast(1, identity, address, targets);
}

#[cfg(any(
    feature = "mm2d_munmap_test",
    feature = "mm2e_mprotect_test",
    feature = "fastlane_f1_audit",
    feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"
))]
pub fn test_read(identity: AddressSpaceIdentity, address: u64, targets: u64) {
    test_broadcast(2, identity, address, targets);
}

#[cfg(any(
    feature = "mm2d_munmap_test",
    feature = "mm2e_mprotect_test",
    feature = "fastlane_f1_audit",
    feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"
))]
pub fn test_leave(targets: u64) {
    test_broadcast(3, AddressSpaceIdentity::INVALID, 0, targets);
}

#[cfg(any(
    feature = "mm2d_munmap_test",
    feature = "mm2e_mprotect_test",
    feature = "fastlane_f1_audit",
    feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"
))]
pub fn test_result(cpu_id: usize) -> u64 {
    TEST_MAILBOXES[cpu_id].result.load(Ordering::Acquire)
}

#[cfg(any(feature = "mm2e_mprotect_test", feature = "fastlane_f1_audit", feature = "fastlane_hybrid_lifecycle_test", feature = "shared_quota_test"))]
pub fn test_remote_invalidation_count() -> u64 {
    REMOTE_INVALIDATIONS.load(Ordering::Acquire)
}

#[cfg(feature = "mm2b_smp_test")]
pub fn run_smp_regression_gate(hhdm: VirtAddr) {
    use crate::process::address_space::{
        AddressSpace, ExpectedMapping, OwnershipTransition, ReplacementMapping,
    };
    use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};

    const OLD_VALUE: u64 = 0x4d4d_3242_4f4c_4421;
    const NEW_VALUE: u64 = 0x4d4d_3242_4e45_5721;
    const TEST_ADDRESS: u64 = 0x0000_6fff_ff00_0000;

    let online = crate::sched::ONLINE_CORES.load(Ordering::Acquire);
    assert_eq!(online, 12, "MM-2B gate requires exactly 12 online CPUs");
    let all_cpus = (1u64 << online) - 1;
    let local_cpu = crate::sched::current_cpu_id();
    let remote_cpus = all_cpus & !(1u64 << local_cpu);
    let root = PhysAddr::new(KERNEL_PML4.load(Ordering::Acquire));
    let mut address_space = AddressSpace::test_root(root);
    let identity = address_space.identity();
    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(TEST_ADDRESS))
        .expect("MM-2B test address alignment");
    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::NO_EXECUTE;

    let mut pmm = crate::PMM.lock();
    let baseline = pmm.free_page_count();
    let old_frame = pmm.alloc_frame().expect("MM-2B old frame allocation");
    let new_frame = pmm.alloc_frame().expect("MM-2B new frame allocation");
    unsafe {
        (hhdm + old_frame.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(OLD_VALUE);
        (hhdm + new_frame.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(NEW_VALUE);
        address_space
            .map_page(
                page,
                PhysFrame::from_start_address_unchecked(old_frame),
                flags,
                &mut pmm,
                hhdm,
            )
            .expect("MM-2B test mapping");
    }

    ACTIVE_CPUS.enter(local_cpu, identity);
    assert_eq!(
        unsafe { (TEST_ADDRESS as *const u64).read_volatile() },
        OLD_VALUE
    );
    test_broadcast(1, identity, TEST_ADDRESS, remote_cpus);
    for cpu_id in 0..online {
        if cpu_id != local_cpu {
            assert_eq!(
                TEST_MAILBOXES[cpu_id].result.load(Ordering::Acquire),
                OLD_VALUE,
                "remote CPU did not pre-touch old mapping"
            );
        }
    }

    let (_, touched_flags) = unsafe {
        address_space
            .lookup_entry(page, hhdm)
            .expect("MM-2B touched mapping disappeared")
    };

    unsafe {
        address_space
            .replace_mapping(
                page,
                ExpectedMapping::Present {
                    frame: old_frame,
                    flags: touched_flags,
                },
                ReplacementMapping::Present {
                    frame: PhysFrame::from_start_address_unchecked(new_frame),
                    flags: touched_flags,
                },
                OwnershipTransition::ReleaseOldFrame,
                &mut pmm,
                hhdm,
            )
            .expect("MM-2B coherent replacement");
    }

    assert_eq!(
        unsafe { (TEST_ADDRESS as *const u64).read_volatile() },
        NEW_VALUE
    );
    test_broadcast(2, identity, TEST_ADDRESS, remote_cpus);
    for cpu_id in 0..online {
        if cpu_id != local_cpu {
            assert_eq!(
                TEST_MAILBOXES[cpu_id].result.load(Ordering::Acquire),
                NEW_VALUE,
                "remote CPU retained stale replacement content"
            );
        }
    }

    unsafe {
        address_space
            .rollback_mapped_page(page, new_frame, &mut pmm, hhdm)
            .expect("MM-2B coherent test cleanup");
    }
    pmm.free_frame(new_frame);
    test_broadcast(3, AddressSpaceIdentity::INVALID, 0, remote_cpus);
    ACTIVE_CPUS.leave(local_cpu);
    assert_eq!(pmm.free_page_count(), baseline);
    drop(pmm);

    diagnostic_report();
    crate::serial_println!(
        "[MM-2B] 12 CPUs pre-touched, replaced, acknowledged, and observed new content: OK"
    );
    crate::serial_println!("[MM-2B] PMM accounting returned to baseline: OK");
}

pub fn diagnostic_report() {
    crate::serial_println!(
        "[MM-2B-DIAG] requests={} targets={} local={} remote={} range={} full={} acks={} switch_retries={} no_remote={} failures={}",
        REQUESTS.load(Ordering::Relaxed),
        TARGET_CPUS.load(Ordering::Relaxed),
        LOCAL_INVALIDATIONS.load(Ordering::Relaxed),
        REMOTE_INVALIDATIONS.load(Ordering::Relaxed),
        RANGE_FLUSHES.load(Ordering::Relaxed),
        FULL_FLUSHES.load(Ordering::Relaxed),
        ACKNOWLEDGEMENTS.load(Ordering::Relaxed),
        CONTEXT_SWITCH_RETRIES.load(Ordering::Relaxed),
        NO_REMOTE_TARGETS.load(Ordering::Relaxed),
        INVARIANT_FAILURES.load(Ordering::Relaxed),
    );
}

#[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
static KFP_MISSING_ACK: AtomicU64 = AtomicU64::new(0);

#[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
static KFP_ACK_TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
static KFP_ACK_TEST_OBSERVED: [AtomicU64; MAX_TRACKED_CPUS] =
    [const { AtomicU64::new(0) }; MAX_TRACKED_CPUS];

#[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
pub(crate) fn ack_test_receipt(token: &super::retirement::Token) {
    let r = super::retirement::OWNERS.lock().snapshot(token);
    let mode = super::retirement::fault_mode();
    if mode != 4 && mode != 5 { return; }
    assert_eq!(r.state, super::retirement::State::Synchronized);
    let mut observed = 0u64;
    for cpu in 0..MAX_TRACKED_CPUS {
        if r.targets & (1u64 << cpu) != 0 {
            assert_eq!(KFP_ACK_TEST_OBSERVED[cpu].load(Ordering::Acquire), r.sequence);
            assert!(MAILBOXES[cpu].acknowledged(r.sequence));
            observed |= 1u64 << cpu;
        }
    }
    crate::serial_println!("[KFP2A3-ACK] mode={} root={} vm={} sequence={} targets={} received={} observed={} state={:?}",
        mode, r.root, r.vm, r.sequence, r.targets, r.received, observed, r.state);
}
