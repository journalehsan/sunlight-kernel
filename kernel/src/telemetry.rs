//! Kernel telemetry shared memory page.
//! Mapped read-only into user-space processes via SYS_MAP_TELEMETRY.

use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

pub const TELEMETRY_MAGIC: u64 = 0x5355_4E4C_5449_4D45;
/// Version 3: adds PhysicalMemoryAccountingSnapshotV1 after the core page.
pub const TELEMETRY_VERSION: u32 = 3;
pub const MAX_PROCESSES: usize = 64;
pub const MAX_CORES: usize = 64;

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct ProcessStat {
    pub pid: u32,
    pub ppid: u32,
    pub state: u8,
    pub _pad: [u8; 3],
    pub name: [u8; 32],
    pub cpu_ticks: u64,
    pub mem_pages: u32,
    /// Low 32 bits of address-space generation (PID reuse discrimination).
    pub generation: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct CoreStat {
    pub core_id: u8,
    pub _pad: [u8; 3],
    pub current_pid: u32,
    pub current_ticks: u32,
    pub nice: i8,
    pub _pad2: [u8; 3],
    pub local_timer_ticks: u64,
    pub context_switches: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TelemetryPage {
    pub magic: u64,
    pub version: u32,
    pub sequence: u32,

    pub uptime_secs: u64,
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
    pub zram_orig_kb: u64,
    pub zram_comp_kb: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub tick_hz: u32,
    pub cpu_count: u8,
    pub gpu_count: u8,
    pub _pad: [u8; 2],

    /// Monotonic kernel time (ns) when this telemetry sample was captured.
    /// Used by sunlight-top for accurate interval-based CPU % computation.
    pub sample_time_ns: u64,

    pub proc_count: u32,
    pub procs: [ProcessStat; MAX_PROCESSES],

    pub core_count: u32,
    pub cores: [CoreStat; MAX_CORES],

    pub timekeeper_core: u8,
    pub drift_warning: u8,
    pub _time_diag_pad: [u8; 6],
    pub global_timekeeper_ticks: u64,
    pub monotonic_ns: u64,
    pub uptime_seconds: u64,
    pub ticks_per_core: [u64; MAX_CORES],

    /// Physical memory accounting Phase 1 (versioned nested snapshot).
    pub mem_acct_version: u32,
    pub _mem_acct_pad: u32,
    pub mem_acct: crate::memory::accounting::PhysicalMemoryAccountingSnapshotV1,
}

const ZERO_PROC: ProcessStat = ProcessStat {
    pid: 0,
    ppid: 0,
    state: 0,
    _pad: [0; 3],
    name: [0; 32],
    cpu_ticks: 0,
    mem_pages: 0,
    generation: 0,
};

const ZERO_CORE: CoreStat = CoreStat {
    core_id: 0,
    _pad: [0; 3],
    current_pid: 0,
    current_ticks: 0,
    nice: 0,
    _pad2: [0; 3],
    local_timer_ticks: 0,
    context_switches: 0,
};

const _: () = assert!(core::mem::size_of::<TelemetryPage>() <= 8192);

static NET_RX_BYTES: AtomicU64 = AtomicU64::new(0);
static NET_TX_BYTES: AtomicU64 = AtomicU64::new(0);

#[link_section = ".telemetry"]
pub static mut TELEMETRY: TelemetryPage = TelemetryPage {
    magic: TELEMETRY_MAGIC,
    version: TELEMETRY_VERSION,
    sequence: 0,

    uptime_secs: 0,
    total_ram_kb: 0,
    used_ram_kb: 0,
    zram_orig_kb: 0,
    zram_comp_kb: 0,
    net_rx_bytes: 0,
    net_tx_bytes: 0,
    tick_hz: crate::timekeeping::TICK_HZ as u32,
    cpu_count: 1,
    gpu_count: 0,
    _pad: [0; 2],

    sample_time_ns: 0,

    proc_count: 0,
    procs: [ZERO_PROC; MAX_PROCESSES],

    core_count: 0,
    cores: [ZERO_CORE; MAX_CORES],
    timekeeper_core: 0,
    drift_warning: 0,
    _time_diag_pad: [0; 6],
    global_timekeeper_ticks: 0,
    monotonic_ns: 0,
    uptime_seconds: 0,
    ticks_per_core: [0; MAX_CORES],

    mem_acct_version: crate::memory::accounting::PhysicalMemoryAccountingSnapshotV1::VERSION,
    _mem_acct_pad: 0,
    mem_acct: crate::memory::accounting::PhysicalMemoryAccountingSnapshotV1 {
        sample_generation: 0,
        sampled_at_ticks: 0,
        installed_bytes: 0,
        usable_bytes: 0,
        managed_bytes: 0,
        free_bytes: 0,
        reserved_bytes: 0,
        active_task_count: 0,
        active_user_task_count: 0,
        active_service_count: 0,
        _pad0: 0,
        task_private_unique_bytes: 0,
        shared_memory_unique_bytes: 0,
        kernel_core_bytes: 0,
        kernel_heap_bytes: 0,
        kernel_stack_bytes: 0,
        page_table_bytes: 0,
        ramfs_file_data_bytes: 0,
        ramfs_metadata_bytes: crate::memory::accounting::PhysicalMemoryAccountingSnapshotV1::RAMFS_METADATA_UNAVAILABLE,
        retained_boot_image_bytes: 0,
        filesystem_cache_bytes: 0,
        other_reclaimable_cache_bytes: 0,
        graphics_buffer_bytes: 0,
        device_dma_bytes: 0,
        zram_physical_bytes: 0,
        zram_logical_bytes: 0,
        other_accounted_bytes: 0,
        unclassified_bytes: 0,
        flags: 0,
        conservation_delta_bytes: 0,
    },
};

pub fn record_net_rx(bytes: u64) {
    NET_RX_BYTES.fetch_add(bytes, Ordering::Relaxed);
}

pub fn record_net_tx(bytes: u64) {
    NET_TX_BYTES.fetch_add(bytes, Ordering::Relaxed);
}

// === Phase 2: telemetry snapshot for lock-free expensive work ===

#[derive(Clone)]
pub struct ProcSnap {
    pub pid: u32,
    pub ppid: u32,
    pub state: u8,
    pub name: [u8; 32],
    pub cpu_ticks: u64,
    pub pml4_phys: x86_64::PhysAddr,
    pub generation: u32,
    pub is_finished_or_cleanup: bool,
}

#[derive(Clone, Copy, Default)]
pub struct CoreSnap {
    pub core_id: u8,
    pub local_timer_ticks: u64,
    pub context_switches: u64,
    pub current_pid: u32,
    pub current_ticks: u32,
    pub nice: i8,
}

#[derive(Clone)]
pub struct TelemetrySnapshot {
    pub uptime_secs: u64,
    pub uptime_seconds: u64,
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
    pub zram_orig_kb: u64,
    pub zram_comp_kb: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub sample_time_ns: u64,
    pub cpu_count: u8,
    pub gpu_count: u8,
    pub procs: Vec<ProcSnap>,
    pub cores: Vec<CoreSnap>,
    pub core_count: u32,
    pub timekeeper_core: u8,
    pub drift_warning: u8,
    pub global_timekeeper_ticks: u64,
    pub monotonic_ns: u64,
    pub ticks_per_core: [u64; MAX_CORES],
    pub mem_acct: crate::memory::accounting::PhysicalMemoryAccountingSnapshotV1,
}

/// Capture only stable scalar data while holding SCHEDULER lock (cheap copy).
pub fn capture_telemetry_snapshot(
    sched: &Scheduler,
    pmm: &PhysicalMemoryManager,
) -> TelemetrySnapshot {
    let global_timekeeper_ticks = crate::timekeeping::global_ticks();
    let uptime_secs = crate::timekeeping::uptime_secs();
    let sample_now = sched.now_ns();

    let (total_frames, free_frames) = pmm.stats();
    let (_swap_total, swap_used_blocks, swap_used_bytes) = crate::memory::zram::stats();
    let zram_agg = crate::memory::zram::aggregate_stats();
    let net_rx = NET_RX_BYTES.load(Ordering::Relaxed);
    let net_tx = NET_TX_BYTES.load(Ordering::Relaxed);

    let cpu_count = unsafe { TELEMETRY.cpu_count.max(1) };
    let gpu_count = unsafe { TELEMETRY.gpu_count };

    let mut procs = Vec::new();
    let mut active_task_count = 0u32;
    let mut active_user_task_count = 0u32;
    let mut active_service_count = 0u32;
    for idx in 0..sched.processes.len() {
        if procs.len() >= MAX_PROCESSES {
            break;
        }
        let proc = &sched.processes[idx];
        let runtime = sched.effective_runtime_ns(idx);
        let proc_state = match proc.state {
            crate::process::ProcessState::Ready => 0,
            crate::process::ProcessState::Running => 1,
            crate::process::ProcessState::BlockedOnIpc => 2,
            crate::process::ProcessState::Finished => 3,
            crate::process::ProcessState::Reaped => 7,
            crate::process::ProcessState::Suspended => 4,
            crate::process::ProcessState::BlockedOnTimer => 5,
            crate::process::ProcessState::BlockedOnIo => 6,
        };
        let is_finished_or_cleanup = proc_state == 3 || proc.exit_cleanup_pending;
        if !is_finished_or_cleanup && proc_state != 7 {
            active_task_count = active_task_count.saturating_add(1);
            // Heuristic: services typically have lower PIDs and long-lived names;
            // for Phase 1 we treat all non-finished tasks as tasks; service count
            // is the same pool (honest: no process-name attribution required).
            active_user_task_count = active_user_task_count.saturating_add(1);
        }
        procs.push(ProcSnap {
            pid: proc.pid as u32,
            ppid: proc.ppid as u32,
            state: proc_state,
            name: proc.name,
            cpu_ticks: runtime,
            pml4_phys: proc.address_space.pml4_phys,
            generation: (proc.address_space.identity().generation & 0xFFFF_FFFF) as u32,
            is_finished_or_cleanup,
        });
    }
    // Service count left at 0 unless we gain an honest service tag; do not
    // invent ownership from process names (Phase 1 discipline).
    let _ = &mut active_service_count;

    let mem_acct = crate::memory::accounting::capture_snapshot(
        total_frames as u64,
        free_frames as u64,
        active_task_count,
        active_user_task_count,
        active_service_count,
        global_timekeeper_ticks,
        // ZRAM logical = stored pages * page size; physical = allocator consumed.
        zram_agg.stored_pages.saturating_mul(4096),
        zram_agg.allocator_consumed_bytes,
    );

    let online = sched.online_cores.min(MAX_CORES);
    let mut cores = Vec::with_capacity(online);
    let mut ticks_per_core = [0u64; MAX_CORES];
    for c in 0..online {
        let mut cs = CoreSnap {
            core_id: c as u8,
            local_timer_ticks: 0,
            context_switches: 0,
            current_pid: 0,
            current_ticks: 0,
            nice: 0,
        };
        if crate::sched::SCHEDULER_MODE == crate::sched::SchedulerMode::RoundRobin {
            let core = crate::sched::CORE_STATES[c].lock();
            cs.local_timer_ticks = core.timer_ticks;
            cs.context_switches = core.context_switches;
            if let Some(idx) = core.current_task {
                if idx < sched.processes.len() {
                    let p = &sched.processes[idx];
                    cs.current_pid = p.pid as u32;
                    cs.current_ticks = core.current_ticks.min(u32::MAX as u64) as u32;
                    cs.nice = p.nice;
                }
            }
            ticks_per_core[c] = core.timer_ticks;
        } else {
            let core = &sched.cores[c];
            cs.local_timer_ticks = core.timer_ticks;
            cs.context_switches = core.context_switches;
            if let Some(idx) = core.current_task {
                if idx < sched.processes.len() {
                    let p = &sched.processes[idx];
                    cs.current_pid = p.pid as u32;
                    cs.current_ticks = core.current_ticks.min(u32::MAX as u64) as u32;
                    cs.nice = p.nice;
                }
            }
            ticks_per_core[c] = core.timer_ticks;
        }
        cores.push(cs);
    }

    TelemetrySnapshot {
        uptime_secs,
        uptime_seconds: uptime_secs,
        total_ram_kb: total_frames as u64 * 4,
        used_ram_kb: total_frames.saturating_sub(free_frames) as u64 * 4,
        zram_orig_kb: swap_used_blocks as u64 * 4,
        zram_comp_kb: (swap_used_bytes as u64 + 1023) / 1024,
        net_rx_bytes: net_rx,
        net_tx_bytes: net_tx,
        sample_time_ns: sample_now,
        cpu_count,
        gpu_count,
        procs,
        cores,
        core_count: online as u32,
        timekeeper_core: crate::timekeeping::timekeeper_core() as u8,
        drift_warning: crate::timekeeping::drift_warning_active() as u8,
        global_timekeeper_ticks,
        monotonic_ns: sample_now,
        ticks_per_core,
        mem_acct,
    }
}

/// Write snapshot to TELEMETRY page. Performs expensive page walks here (outside lock).
/// SAFETY: caller must ensure no concurrent writers; typically called from timer after releasing SCHEDULER lock.
pub unsafe fn commit_telemetry_snapshot(snap: &TelemetrySnapshot) {
    let seq = TELEMETRY.sequence.wrapping_add(1);
    TELEMETRY.sequence = seq;
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);

    TELEMETRY.uptime_secs = snap.uptime_secs;
    TELEMETRY.uptime_seconds = snap.uptime_seconds;
    TELEMETRY.sample_time_ns = snap.sample_time_ns;
    TELEMETRY.timekeeper_core = snap.timekeeper_core;
    TELEMETRY.drift_warning = snap.drift_warning;
    TELEMETRY.global_timekeeper_ticks = snap.global_timekeeper_ticks;
    TELEMETRY.monotonic_ns = snap.monotonic_ns;
    TELEMETRY.total_ram_kb = snap.total_ram_kb;
    TELEMETRY.used_ram_kb = snap.used_ram_kb;
    TELEMETRY.zram_orig_kb = snap.zram_orig_kb;
    TELEMETRY.zram_comp_kb = snap.zram_comp_kb;
    TELEMETRY.net_rx_bytes = snap.net_rx_bytes;
    TELEMETRY.net_tx_bytes = snap.net_tx_bytes;

    if TELEMETRY.cpu_count == 0 {
        TELEMETRY.cpu_count = snap.cpu_count.max(1);
    }
    TELEMETRY.gpu_count = snap.gpu_count;

    let hhdm_opt = crate::HHDM_REQ.response();

    let mut count = 0usize;
    for ps in &snap.procs {
        if count >= MAX_PROCESSES {
            break;
        }
        let entry = &mut TELEMETRY.procs[count];
        entry.pid = ps.pid;
        entry.ppid = ps.ppid;
        entry.state = ps.state;
        entry.name = ps.name;
        entry.cpu_ticks = ps.cpu_ticks;

        entry.mem_pages = if ps.is_finished_or_cleanup {
            0
        } else {
            match hhdm_opt {
                Some(resp) => {
                    let hhdm = x86_64::VirtAddr::new(resp.offset);
                    let aspace =
                        crate::process::address_space::AddressSpace::from_pml4(ps.pml4_phys);
                    if aspace.is_reclaimed() {
                        0
                    } else {
                        // Walk happens here, outside SCHEDULER lock.
                        // Counts present mapped user pages (shared counted per map).
                        unsafe { aspace.count_user_pages(hhdm) }.min(u32::MAX as usize) as u32
                    }
                }
                None => 0,
            }
        };
        entry._pad = [0; 3];
        entry.generation = ps.generation;

        count += 1;
    }

    for i in count..MAX_PROCESSES {
        TELEMETRY.procs[i] = ZERO_PROC;
    }
    TELEMETRY.proc_count = count as u32;

    let online = snap.core_count as usize;
    TELEMETRY.core_count = online as u32;
    for c in 0..online {
        let cs = &snap.cores[c];
        let entry = &mut TELEMETRY.cores[c];
        entry.core_id = cs.core_id;
        entry.local_timer_ticks = cs.local_timer_ticks;
        entry.context_switches = cs.context_switches;
        entry.current_pid = cs.current_pid;
        entry.current_ticks = cs.current_ticks;
        entry.nice = cs.nice;
        TELEMETRY.ticks_per_core[c] = snap.ticks_per_core[c];
    }
    for c in online..MAX_CORES {
        TELEMETRY.cores[c] = ZERO_CORE;
        TELEMETRY.ticks_per_core[c] = 0;
    }

    TELEMETRY.mem_acct_version =
        crate::memory::accounting::PhysicalMemoryAccountingSnapshotV1::VERSION;
    TELEMETRY.mem_acct = snap.mem_acct;

    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    TELEMETRY.sequence = seq.wrapping_add(1);
}

/// SAFETY: caller must serialize updates (timer ISR with interrupts disabled).
/// Refactored for Phase 2: captures scalars under lock, heavy work (page walks) moved out.
pub unsafe fn update_telemetry(
    sched: &mut Scheduler,
    pmm: &PhysicalMemoryManager,
    _tick_count: u64,
) {
    let snap = capture_telemetry_snapshot(sched, pmm);
    commit_telemetry_snapshot(&snap);
}
