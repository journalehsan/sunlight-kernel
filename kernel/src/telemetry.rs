//! Kernel telemetry shared memory page.
//! Mapped read-only into user-space processes via SYS_MAP_TELEMETRY.

use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
use core::sync::atomic::{AtomicU64, Ordering};

pub const TELEMETRY_MAGIC: u64 = 0x5355_4E4C_5449_4D45;
pub const TELEMETRY_VERSION: u32 = 1;
pub const MAX_PROCESSES: usize = 64;

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
    pub _pad2: u32,
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
    pub _pad: [u8; 3],

    pub proc_count: u32,
    pub procs: [ProcessStat; MAX_PROCESSES],
}

const ZERO_PROC: ProcessStat = ProcessStat {
    pid: 0,
    ppid: 0,
    state: 0,
    _pad: [0; 3],
    name: [0; 32],
    cpu_ticks: 0,
    mem_pages: 0,
    _pad2: 0,
};

const _: () = assert!(core::mem::size_of::<TelemetryPage>() <= 4096);

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
    tick_hz: 100,
    cpu_count: 1,
    _pad: [0; 3],

    proc_count: 0,
    procs: [ZERO_PROC; MAX_PROCESSES],
};

pub fn record_net_rx(bytes: u64) {
    NET_RX_BYTES.fetch_add(bytes, Ordering::Relaxed);
}

pub fn record_net_tx(bytes: u64) {
    NET_TX_BYTES.fetch_add(bytes, Ordering::Relaxed);
}

/// SAFETY: caller must serialize updates (timer ISR with interrupts disabled).
pub unsafe fn update_telemetry(sched: &Scheduler, pmm: &PhysicalMemoryManager, tick_count: u64) {
    // SAFETY: synchronized by caller; sequence field is in the shared telemetry page.
    let seq = unsafe { TELEMETRY.sequence.wrapping_add(1) };
    // SAFETY: synchronized by caller; begin seqlock write (odd sequence).
    unsafe {
        TELEMETRY.sequence = seq;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);

    // SAFETY: synchronized by caller; writing shared telemetry fields.
    unsafe {
        TELEMETRY.uptime_secs = tick_count / (TELEMETRY.tick_hz as u64).max(1);
    }

    let (total_frames, free_frames) = pmm.stats();
    // SAFETY: synchronized by caller; writing PMM-derived counters.
    unsafe {
        TELEMETRY.total_ram_kb = total_frames as u64 * 4;
        TELEMETRY.used_ram_kb = total_frames.saturating_sub(free_frames) as u64 * 4;
    }

    let (_swap_total_blocks, swap_used_blocks, swap_used_bytes) = crate::memory::zram::stats();
    // SAFETY: synchronized by caller; writing ZRAM-derived counters.
    unsafe {
        TELEMETRY.zram_orig_kb = swap_used_blocks as u64 * 4;
        TELEMETRY.zram_comp_kb = (swap_used_bytes as u64 + 1023) / 1024;
        TELEMETRY.net_rx_bytes = NET_RX_BYTES.load(Ordering::Relaxed);
        TELEMETRY.net_tx_bytes = NET_TX_BYTES.load(Ordering::Relaxed);
    }

    let mut count = 0usize;
    for proc in &sched.processes {
        if count >= MAX_PROCESSES {
            break;
        }

        // SAFETY: synchronized by caller; bounded index into fixed process array.
        let entry = unsafe { &mut TELEMETRY.procs[count] };
        entry.pid = proc.pid as u32;
        entry.ppid = proc.ppid as u32;
        entry.state = match proc.state {
            crate::process::ProcessState::Ready => 0,
            crate::process::ProcessState::Running => 1,
            crate::process::ProcessState::BlockedOnIpc => 2,
            crate::process::ProcessState::Finished => 3,
        };
        entry.cpu_ticks = sched.global_tick.saturating_sub(proc.last_run_tick);
        entry.mem_pages = 0;
        entry._pad = [0; 3];
        entry._pad2 = 0;
        entry.name = [0; 32];

        let name_bytes = proc.name.as_bytes();
        let len = name_bytes.len().min(31);
        entry.name[..len].copy_from_slice(&name_bytes[..len]);
        entry.name[len] = 0;

        count += 1;
    }

    // SAFETY: synchronized by caller; remaining entries are fully reset.
    unsafe {
        for i in count..MAX_PROCESSES {
            TELEMETRY.procs[i] = ZERO_PROC;
        }
        TELEMETRY.proc_count = count as u32;
    }

    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    // SAFETY: synchronized by caller; end seqlock write (even sequence).
    unsafe {
        TELEMETRY.sequence = seq.wrapping_add(1);
    }
}
