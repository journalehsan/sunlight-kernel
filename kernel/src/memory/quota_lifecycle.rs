//! Debug evidence only. Passive receipts preserve the production lock order.
//! No waiter or scheduler operation may be added inside these locked paths.
use super::shared_quota::{self, ProcessIdentity, SessionIdentity};
use crate::capability::{CapabilityBroker, CapabilityToken};
use crate::process::Process;
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);
static LIVE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn reserved(
    owner: ProcessIdentity,
    session: Option<SessionIdentity>,
    bytes: usize,
) -> u64 {
    let Some(SessionIdentity::Account { id, epoch }) = session else {
        return 0;
    };
    let resource = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .expect("lifecycle evidence identity exhausted");
    LIVE.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!("[R5-QUOTA] event=reserved resource={} pid={} incarnation={} session={} epoch={} bytes={} cpu={}", resource, owner.pid, owner.incarnation, id, epoch, bytes, crate::sched::current_cpu_id());
    resource
}
pub(crate) fn consumed(
    resource: u64,
    owner: ProcessIdentity,
    session: Option<SessionIdentity>,
    bytes: usize,
) {
    if resource == 0 {
        return;
    }
    let (process, account) = shared_quota::observation(owner, session);
    assert!(LIVE.fetch_sub(1, Ordering::Relaxed) > 0);
    crate::serial_println!("[R5-QUOTA] event=consumed resource={} pid={} incarnation={} bytes={} process_after={} session_after={} cpu={}", resource, owner.pid, owner.incarnation, bytes, process, account, crate::sched::current_cpu_id());
}
pub(crate) fn event(resource: u64, event: &str, value: u64, inventory: u64) {
    if resource != 0 {
        crate::serial_println!(
            "[R5-RESOURCE] event={} resource={} value={} inventory={} cpu={}",
            event,
            resource,
            value,
            inventory,
            crate::sched::current_cpu_id()
        );
    }
}
pub(crate) fn view(
    caps: &CapabilityBroker,
    p: &Process,
    token: CapabilityToken,
    va: u64,
    event: &str,
) {
    let resource = caps.lifecycle_resource(token);
    if resource != 0 {
        let vm = p.address_space.identity();
        crate::serial_println!("[R5-VIEW] event={} resource={} pid={} incarnation={} vm={} root={} va={} references={} cpu={}", event, resource, p.pid, p.foreground.incarnation, vm.generation, vm.pml4_phys, va, caps.shared_region_map_count(token).unwrap_or(0), crate::sched::current_cpu_id());
    }
}
pub(crate) fn process(p: &Process, event: &str) {
    process_vm(p, event, p.address_space.identity().generation);
}
pub(crate) fn process_vm(p: &Process, event: &str, vm: u64) {
    if let Some(reference) = p.account_membership {
        crate::serial_println!("[R5-PROCESS] event={} pid={} incarnation={} vm={} session={} epoch={} mappings={} exit={} cpu={}", event, p.pid, p.foreground.incarnation, vm, reference.id, reference.epoch, p.mapped_shared.len(), p.exit_code, crate::sched::current_cpu_id());
    }
}

/// Read-only observations through the existing debug syscall. A query grants
/// no mapping, release, session or lifecycle authority. Inputs never select a
/// cleanup routine. Cross-process observation is limited to the current parent.
pub(crate) fn query(s: &str) -> Option<u64> {
    let command = s.strip_prefix("[R5] ")?;
    let sched = crate::sched::SCHEDULER.lock();
    let caller = sched.current_process();
    let reference = caller.account_membership?;
    let session = Some(SessionIdentity::Account {
        id: reference.id,
        epoch: reference.epoch,
    });
    if command == "session-live" {
        return Some(u64::from(
            crate::capability::CAP_BROKER
                .lock()
                .session_allocation_live(reference, sched.global_tick),
        ));
    }
    if let Some(pid) = command
        .strip_prefix("session-owner-reaped ")
        .and_then(|value| value.parse::<usize>().ok())
    {
        return Some(u64::from(sched.processes.iter().any(|p| {
            p.pid == pid
                && p.account_membership == Some(reference)
                && p.state == crate::process::ProcessState::Reaped
        })));
    }
    if let Some(label) = command.strip_prefix("observe ") {
        if label == "baseline" {
            crate::serial_println!(
                "[R5-START] source={} run={} cpus={}",
                option_env!("SUNLIGHT_R5_SOURCE").unwrap_or("UNSET"),
                option_env!("SUNLIGHT_R5_RUN").unwrap_or("UNSET"),
                sched.online_cores
            );
        }
        let owner = caller.shared_quota_owner.unwrap_or(ProcessIdentity {
            pid: caller.pid,
            incarnation: caller.foreground.incarnation,
        });
        let (bytes, account) = shared_quota::observation(owner, session);
        crate::serial_println!("[R5-SNAPSHOT] label={} pid={} incarnation={} vm={} session={} epoch={} process_bytes={} session_bytes={} live_receipts={} free_frames={} mappings={} cpu={}", label, caller.pid, caller.foreground.incarnation, caller.address_space.identity().generation, reference.id, reference.epoch, bytes, account, LIVE.load(Ordering::Relaxed), crate::PMM.lock().free_page_count(), caller.mapped_shared.len(), crate::sched::current_cpu_id());
        // Reuse the PMM primary classes; these are not additional consumption
        // counters. Hold the PMM lock across stats and class reads so PMM
        // allocation/free/reclassification cannot straddle this observation.
        // This does not fence services, deferred cleanup, or heap suballocations.
        let (total, free, classes) = {
            let pmm = crate::PMM.lock();
            if label == "baseline" {
                super::census_trace::lifecycle_begin();
            }
            if label == "quiescent" {
                let hhdm = x86_64::VirtAddr::new(crate::HHDM_REQ.response().expect("HHDM").offset);
                for p in &sched.processes {
                    if p.address_space.identity().is_valid() {
                        crate::serial_println!(
                            "[R51-OWNER] pid={} incarnation={} vm={} root={} name={} state={:?}",
                            p.pid,
                            p.foreground.incarnation,
                            p.address_space.identity().generation,
                            p.address_space.identity().pml4_phys,
                            p.name_str(),
                            p.state
                        );
                        unsafe {
                            p.address_space.trace_tables(
                                2,
                                (p.pid, p.foreground.incarnation),
                                hhdm,
                            );
                            super::census_trace::lifecycle_views(p, hhdm);
                        }
                    }
                }
                super::census_trace::lifecycle_end();
            }
            let (total, free) = pmm.stats();
            let classes: [u64; super::accounting::CLASS_COUNT] = core::array::from_fn(|index| {
                super::accounting::class_frame_count(
                    super::accounting::PhysicalMemoryClass::from_u8(index as u8)
                        .expect("primary class index"),
                )
            });
            (total, free, classes)
        };
        crate::serial_println!(
            "[R51-PMM] label={} managed={} free={} used={} scope=pmm-lock-snapshot",
            label,
            total,
            free,
            total - free
        );
        for (class, frames) in classes.iter().enumerate() {
            crate::serial_println!(
                "[R51-CLASS] label={} class={} frames={} scope=primary-class-overlaps-owner-and-quota",
                label,
                class,
                frames
            );
        }
        let pending = sched
            .processes
            .iter()
            .filter(|p| {
                p.account_membership == Some(reference)
                    && p.is_exiting()
                    && p.state != crate::process::ProcessState::Reaped
            })
            .count();
        let views: usize = sched
            .processes
            .iter()
            .filter(|p| p.account_membership == Some(reference))
            .map(|p| p.mapped_shared.len())
            .sum();
        crate::serial_println!("[R5-INVENTORY] label={} session={} epoch={} session_views={} session_pending_reaps={} system_shared_frames={} system_table_frames={} scope=session-and-separate-system-counters", label, reference.id, reference.epoch, views, pending, super::accounting::class_frame_count(super::accounting::PhysicalMemoryClass::SharedMemory), super::accounting::class_frame_count(super::accounting::PhysicalMemoryClass::PageTable));
        return Some(account as u64);
    }
    if let Some(pid) = command
        .strip_prefix("reaped ")
        .and_then(|s| s.parse::<usize>().ok())
    {
        return Some(match sched.processes.iter().find(|p| p.pid == pid) {
            Some(p) if p.ppid == caller.pid => {
                u64::from(p.state == crate::process::ProcessState::Reaped)
            }
            _ => 0,
        });
    }
    None
}

static FLUSHED: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
// Called in the existing NMI/IPI path. Atomics only: never log or take a lock.
pub(crate) fn ack(sequence: u64, cpu: usize, flushed: bool) {
    FLUSHED[cpu].store(if flushed { sequence } else { 0 }, Ordering::Release);
}
pub(crate) fn tlb(
    identity: crate::process::mm2b_state::AddressSpaceIdentity,
    start: u64,
    sequence: u64,
    targets: u64,
    event: &str,
) {
    if !(super::shared::SHARED_REGION_BASE
        ..super::shared::SHARED_REGION_BASE + super::shared::SHARED_REGION_MAX_SIZE as u64)
        .contains(&start)
    {
        return;
    }
    let mut flushed = 0u64;
    if event == "complete" {
        for cpu in 0..64 {
            if targets & (1u64 << cpu) != 0 && FLUSHED[cpu].load(Ordering::Acquire) == sequence {
                flushed |= 1u64 << cpu;
            }
        }
    }
    crate::serial_println!(
        "[R5-TLB] event={} request={} vm={} root={} va={} targets={} flushed={} cpu={}",
        event,
        sequence,
        identity.generation,
        identity.pml4_phys,
        start,
        targets,
        flushed,
        crate::sched::current_cpu_id()
    );
}

pub(crate) fn reservation_attempt(
    owner: ProcessIdentity,
    session: Option<SessionIdentity>,
    bytes: usize,
) {
    if let Some(SessionIdentity::Account { id, epoch }) = session {
        crate::serial_println!("[R5-BOUNDARY] event=before-reservation pid={} incarnation={} session={} epoch={} bytes={} cpu={}", owner.pid, owner.incarnation, id, epoch, bytes, crate::sched::current_cpu_id());
    }
}

pub(crate) fn charged(
    resource: u64,
    owner: ProcessIdentity,
    session: Option<SessionIdentity>,
    bytes: usize,
) {
    if resource == 0 {
        return;
    }
    let before = shared_quota::observation(owner, session);
    assert!(before.0 >= bytes && before.1 >= bytes);
    crate::serial_println!("[R5-CHARGE] resource={} pid={} incarnation={} bytes={} process_before={} session_before={}", resource, owner.pid, owner.incarnation, bytes, before.0, before.1);
}

/// Class and PMM totals from one existing allocator-lock boundary.
/// Primary classes overlap mapped/owner/quota telemetry; they are not additive.
pub(crate) fn accounting_checkpoint(label: &str, pmm: &super::pmm::PhysicalMemoryManager) {
    let (managed, free) = pmm.stats();
    crate::serial_println!(
        "[R51-PMM] label={} managed={} free={} used={} scope=pmm-lock-snapshot",
        label,
        managed,
        free,
        managed - free
    );
    for index in 0..super::accounting::CLASS_COUNT {
        let class = super::accounting::PhysicalMemoryClass::from_u8(index as u8).expect("class");
        crate::serial_println!(
            "[R51-CLASS] label={} class={} frames={} scope=primary-class-overlaps-owner-and-quota",
            label,
            index,
            super::accounting::class_frame_count(class)
        );
    }
}
