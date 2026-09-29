#![no_std]

mod topology;
pub use sunlight_ipc::cpu_accounting::CpuCounters;
pub use topology::{CoreSnapshot, CpuTelemetry, TaskStats, TopologyInfo, MAX_CORES};

use sunlight_ipc::{ipc_call, map_telemetry, nameserver_lookup, IpcMsg, TzMsg};

pub const TELEMETRY_MAGIC: u64 = 0x5355_4E4C_5449_4D45;
pub const TELEMETRY_VERSION_MEM_ACCT: u32 = 3;
pub const MAX_PROCESSES: usize = 64;
pub const MEM_ACCT_VERSION: u32 = 1;
pub const RAMFS_METADATA_UNAVAILABLE: u64 = u64::MAX;
pub const RETAINED_BOOT_UNMEASURED: u64 = u64::MAX;

/// Matches kernel `PhysicalMemoryAccountingSnapshotV1` (native ABI).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MemoryAccountingSnapshot {
    pub sample_generation: u64,
    pub sampled_at_ticks: u64,

    pub installed_bytes: u64,
    pub usable_bytes: u64,
    pub managed_bytes: u64,
    pub free_bytes: u64,
    pub reserved_bytes: u64,

    pub active_task_count: u32,
    pub active_user_task_count: u32,
    pub active_service_count: u32,
    pub _pad0: u32,

    pub task_private_unique_bytes: u64,
    pub shared_memory_unique_bytes: u64,

    pub kernel_core_bytes: u64,
    pub kernel_heap_bytes: u64,
    pub kernel_stack_bytes: u64,
    pub page_table_bytes: u64,

    pub ramfs_file_data_bytes: u64,
    pub ramfs_metadata_bytes: u64,
    pub retained_boot_image_bytes: u64,

    pub filesystem_cache_bytes: u64,
    pub other_reclaimable_cache_bytes: u64,

    pub graphics_buffer_bytes: u64,
    pub device_dma_bytes: u64,

    pub zram_physical_bytes: u64,
    pub zram_logical_bytes: u64,

    pub other_accounted_bytes: u64,
    pub unclassified_bytes: u64,

    pub flags: u32,
    pub conservation_delta_bytes: u32,
}

impl Default for MemoryAccountingSnapshot {
    fn default() -> Self {
        Self {
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
            ramfs_metadata_bytes: RAMFS_METADATA_UNAVAILABLE,
            retained_boot_image_bytes: RETAINED_BOOT_UNMEASURED,
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
        }
    }
}

impl MemoryAccountingSnapshot {
    pub const FLAG_RAMFS_METADATA_UNAVAILABLE: u32 = 1 << 0;
    pub const FLAG_RETAINED_BOOT_IMAGE_MEASURED: u32 = 1 << 1;
    pub const FLAG_CACHE_IS_REAL_OWNERSHIP: u32 = 1 << 2;
    pub const FLAG_GRAPHICS_PARTIAL: u32 = 1 << 3;
    pub const FLAG_LARGE_UNCLASSIFIED: u32 = 1 << 4;
    pub const FLAG_CONSERVATION_OK: u32 = 1 << 5;
    pub const FLAG_SNAPSHOT_CONSISTENT: u32 = 1 << 6;

    pub fn used_bytes(&self) -> u64 {
        self.managed_bytes.saturating_sub(self.free_bytes)
    }

    pub fn kernel_total_bytes(&self) -> u64 {
        self.kernel_core_bytes
            .saturating_add(self.kernel_heap_bytes)
            .saturating_add(self.kernel_stack_bytes)
    }

    pub fn cache_total_bytes(&self) -> u64 {
        self.filesystem_cache_bytes
            .saturating_add(self.other_reclaimable_cache_bytes)
    }

    pub fn graphics_and_device_bytes(&self) -> u64 {
        self.graphics_buffer_bytes
            .saturating_add(self.device_dma_bytes)
    }

    pub fn ramfs_metadata_available(&self) -> bool {
        self.ramfs_metadata_bytes != RAMFS_METADATA_UNAVAILABLE
    }

    pub fn conservation_ok(&self) -> bool {
        self.flags & Self::FLAG_CONSERVATION_OK != 0
    }
}

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
pub struct RawCoreStat {
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
    pub sample_time_ns: u64,
    pub proc_count: u32,
    pub procs: [ProcessStat; MAX_PROCESSES],

    pub core_count: u32,
    pub cores: [RawCoreStat; MAX_CORES],
    pub timekeeper_core: u8,
    pub drift_warning: u8,
    pub _time_diag_pad: [u8; 6],
    pub global_timekeeper_ticks: u64,
    pub monotonic_ns: u64,
    pub uptime_seconds: u64,
    pub ticks_per_core: [u64; MAX_CORES],

    pub mem_acct_version: u32,
    pub _mem_acct_pad: u32,
    pub mem_acct: MemoryAccountingSnapshot,
    /// Version 4: independently measured CPU capacity, idle, and activity.
    pub cpu_counters: [CpuCounters; MAX_CORES],
    pub task_groups: [u32; MAX_PROCESSES],
}

const _: () = assert!(core::mem::size_of::<TelemetryPage>() <= 16384);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked,
    Finished,
}

impl Default for ProcessState {
    fn default() -> Self {
        Self::Ready
    }
}

#[derive(Clone, Copy, Default)]
pub struct ProcessSnapshot {
    /// Kernel thread group ID; native single-thread tasks use their PID.
    pub tgid: u32,
    /// Measured task delta before process-group aggregation and rounding.
    pub runtime_delta_ns: u64,
    pub pid: u32,
    pub ppid: u32,
    pub state: ProcessState,
    pub name: [u8; 32],
    pub cpu_ticks: u64,
    pub cpu_bp: u16,
    /// Mapped present user pages in KiB (not unique private physical).
    pub mem_kb: u32,
    /// Address-space generation low 32 bits (with pid, identifies identity).
    pub generation: u32,
}

#[derive(Clone, Copy)]
pub struct TimeDiagnostics {
    pub timekeeper_core: u8,
    pub drift_warning: bool,
    pub global_timekeeper_ticks: u64,
    pub monotonic_ns: u64,
    pub uptime_seconds: u64,
    pub ticks_per_core: [u64; MAX_CORES],
}

impl Default for TimeDiagnostics {
    fn default() -> Self {
        Self {
            timekeeper_core: 0,
            drift_warning: false,
            global_timekeeper_ticks: 0,
            monotonic_ns: 0,
            uptime_seconds: 0,
            ticks_per_core: [0; MAX_CORES],
        }
    }
}

impl ProcessSnapshot {
    pub fn name_str(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.name.len());
        core::str::from_utf8(&self.name[..len]).unwrap_or("?")
    }
}

#[derive(Clone, Copy)]
pub struct SystemSnapshot {
    pub sample_time_ns: u64,
    pub cpu_capacity_ns: u64,
    pub accounting_version: u32,
    pub cpu_counters: [CpuCounters; MAX_CORES],
    pub sequence: u32,
    pub uptime_secs: u64,
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
    pub zram_orig_kb: u64,
    pub zram_comp_kb: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub proc_count: usize,
    pub procs: [ProcessSnapshot; MAX_PROCESSES],
    pub cpu_count: u8,
    /// Number of PCI display-class (0x03) devices detected at boot.
    pub gpu_count: u8,
    /// Aggregate CPU utilization in basis points (10 000 = 100 %).
    pub cpu_used_bp: u16,
    /// Aggregate CPU idle in basis points.
    pub cpu_idle_bp: u16,
    pub local_time: [u8; 16],
    pub local_time_len: usize,
    /// Physical / logical hardware layout derived from ACPI MADT.
    pub topology: TopologyInfo,
    /// Per-logical-core load and scheduling state.
    pub cpu_telemetry: CpuTelemetry,
    /// Aggregated task-state counts (running / ready / blocked / zombie).
    pub task_stats: TaskStats,
    /// Optional SMP-safe timekeeping diagnostics.
    pub time_diagnostics: TimeDiagnostics,
    /// Physical memory accounting Phase 1 breakdown (same snapshot generation).
    pub mem_acct: MemoryAccountingSnapshot,
}

impl Default for SystemSnapshot {
    fn default() -> Self {
        Self {
            sample_time_ns: 0,
            cpu_capacity_ns: 0,
            accounting_version: 0,
            cpu_counters: [CpuCounters::ZERO; MAX_CORES],
            sequence: 0,
            uptime_secs: 0,
            total_ram_kb: 0,
            used_ram_kb: 0,
            zram_orig_kb: 0,
            zram_comp_kb: 0,
            net_rx_bytes: 0,
            net_tx_bytes: 0,
            proc_count: 0,
            procs: [ProcessSnapshot::default(); MAX_PROCESSES],
            cpu_count: 1,
            gpu_count: 0,
            cpu_used_bp: 0,
            cpu_idle_bp: 10000,
            local_time: [0; 16],
            local_time_len: 0,
            topology: TopologyInfo::default(),
            cpu_telemetry: CpuTelemetry::default(),
            task_stats: TaskStats::default(),
            time_diagnostics: TimeDiagnostics::default(),
            mem_acct: MemoryAccountingSnapshot::default(),
        }
    }
}

pub struct Telemetry {
    page_ptr: *const TelemetryPage,
    last_seq: u32,
    last_sample_time_ns: u64,
    last_runtime_by_pid: [u64; MAX_PROCESSES],
    last_pids: [u32; MAX_PROCESSES],
    last_generations: [u32; MAX_PROCESSES],
    last_snapshot: SystemSnapshot,
}

impl Telemetry {
    pub fn init() -> Result<Self, &'static str> {
        let ptr = map_telemetry() as *const TelemetryPage;
        if ptr.is_null() {
            return Err("SYS_MAP_TELEMETRY failed");
        }

        let magic = unsafe { vread(core::ptr::addr_of!((*ptr).magic)) };
        if magic != TELEMETRY_MAGIC {
            return Err("TelemetryPage magic mismatch");
        }

        Ok(Self {
            page_ptr: ptr,
            last_seq: 0,
            last_sample_time_ns: 0,
            last_runtime_by_pid: [0; MAX_PROCESSES],
            last_pids: [0; MAX_PROCESSES],
            last_generations: [0; MAX_PROCESSES],
            last_snapshot: SystemSnapshot::default(),
        })
    }

    pub fn poll(&mut self) -> bool {
        // A preempted writer must not turn the reader into an unbounded spin.
        for _ in 0..3 {
            let seq1 = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sequence)) };
            if seq1 & 1 == 1 {
                core::hint::spin_loop();
                continue;
            }

            if seq1 == self.last_seq {
                return false;
            }

            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
            let mut snap = self.read_page();
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
            let seq2 = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sequence)) };
            if seq2 != seq1 {
                continue;
            }

            self.compute_cpu_usage(&mut snap);
            aggregate_thread_groups(&mut snap);
            self.fill_local_time(&mut snap);

            self.last_seq = seq2;
            self.last_snapshot = snap;
            return true;
        }
        false
    }

    pub fn snapshot(&self) -> &SystemSnapshot {
        &self.last_snapshot
    }

    fn read_page(&self) -> SystemSnapshot {
        let mut snap = SystemSnapshot::default();
        let page = unsafe { &*self.page_ptr };
        snap.sample_time_ns = unsafe { vread(core::ptr::addr_of!(page.sample_time_ns)) };
        snap.accounting_version = unsafe { vread(core::ptr::addr_of!(page.version)) };
        if snap.accounting_version >= 4 {
            snap.cpu_counters = unsafe { vread(core::ptr::addr_of!(page.cpu_counters)) };
        }

        unsafe {
            snap.sequence = vread(core::ptr::addr_of!(page.sequence));
            snap.uptime_secs = vread(core::ptr::addr_of!(page.uptime_secs));
            snap.total_ram_kb = vread(core::ptr::addr_of!(page.total_ram_kb));
            snap.used_ram_kb = vread(core::ptr::addr_of!(page.used_ram_kb));
            snap.zram_orig_kb = vread(core::ptr::addr_of!(page.zram_orig_kb));
            snap.zram_comp_kb = vread(core::ptr::addr_of!(page.zram_comp_kb));
            snap.net_rx_bytes = vread(core::ptr::addr_of!(page.net_rx_bytes));
            snap.net_tx_bytes = vread(core::ptr::addr_of!(page.net_tx_bytes));
            snap.cpu_count = vread(core::ptr::addr_of!(page.cpu_count));
            snap.gpu_count = vread(core::ptr::addr_of!(page.gpu_count));
            snap.time_diagnostics.timekeeper_core =
                vread(core::ptr::addr_of!(page.timekeeper_core));
            snap.time_diagnostics.drift_warning =
                vread(core::ptr::addr_of!(page.drift_warning)) != 0;
            snap.time_diagnostics.global_timekeeper_ticks =
                vread(core::ptr::addr_of!(page.global_timekeeper_ticks));
            snap.time_diagnostics.monotonic_ns = vread(core::ptr::addr_of!(page.monotonic_ns));
            snap.time_diagnostics.uptime_seconds = vread(core::ptr::addr_of!(page.uptime_seconds));
        }

        // Topology: derived from cpu_count until CPUID leaf 0x0B is wired.
        snap.topology = TopologyInfo::from_cpu_count(snap.cpu_count);

        // Bootstrap the per-core table from kernel-provided CoreStat entries.
        // Falls back to cpu_count if the kernel hasn't written core_count yet.
        let kernel_core_count = unsafe { vread(core::ptr::addr_of!(page.core_count)) } as usize;
        let logical_count = if kernel_core_count > 0 {
            kernel_core_count.min(MAX_CORES)
        } else {
            (snap.cpu_count as usize).max(1).min(MAX_CORES)
        };
        snap.cpu_telemetry.count = logical_count;
        for i in 0..logical_count {
            snap.cpu_telemetry.cores[i].core_id = i as u8;
            snap.cpu_telemetry.cores[i].affinity_mask = !0u64;
            // Populate current_task_pid from kernel-provided per-core snapshot.
            if kernel_core_count > 0 {
                let cs = unsafe { vread(core::ptr::addr_of!(page.cores[i])) };
                snap.cpu_telemetry.cores[i].current_task_pid = cs.current_pid;
                snap.cpu_telemetry.cores[i].nice = cs.nice;
                snap.cpu_telemetry.cores[i].local_timer_ticks = cs.local_timer_ticks;
                snap.cpu_telemetry.cores[i].context_switches = cs.context_switches;
            }
            snap.time_diagnostics.ticks_per_core[i] =
                unsafe { vread(core::ptr::addr_of!(page.ticks_per_core[i])) };
            // load_bp is filled in compute_cpu_usage() after interval is known.
        }

        let raw_count = unsafe { vread(core::ptr::addr_of!(page.proc_count)) } as usize;
        snap.proc_count = raw_count.min(MAX_PROCESSES);

        // Single pass: map raw ProcessStat → ProcessSnapshot AND tally TaskStats.
        // TaskStats uses the raw kernel state byte (0-6) before the Finished
        // filter so zombie counts are accurate.
        let mut ts = TaskStats::default();
        for i in 0..snap.proc_count {
            let raw = unsafe { vread(core::ptr::addr_of!(page.procs[i])) };

            // Tally raw state for TaskStats (must happen before ProcessState mapping).
            ts.tally(raw.state);

            snap.procs[i] = ProcessSnapshot {
                runtime_delta_ns: 0,
                tgid: if snap.accounting_version >= 4 {
                    unsafe { vread(core::ptr::addr_of!(page.task_groups[i])) }
                } else {
                    raw.pid
                },
                pid: raw.pid,
                ppid: raw.ppid,
                state: match raw.state {
                    1 => ProcessState::Running,
                    2 | 4 | 5 | 6 => ProcessState::Blocked,
                    3 | 7 => ProcessState::Finished,
                    _ => ProcessState::Ready,
                },
                name: raw.name,
                cpu_ticks: raw.cpu_ticks,
                cpu_bp: 0,
                mem_kb: raw.mem_pages.saturating_mul(4),
                generation: raw.generation,
            };
        }
        ts.commit();
        snap.task_stats = ts;

        // Remove exited (Finished) processes from the visible list.
        let mut write_idx = 0;
        for read_idx in 0..snap.proc_count {
            if snap.procs[read_idx].state != ProcessState::Finished {
                if write_idx != read_idx {
                    snap.procs[write_idx] = snap.procs[read_idx];
                }
                write_idx += 1;
            }
        }
        for i in write_idx..snap.proc_count {
            snap.procs[i] = ProcessSnapshot::default();
        }
        snap.proc_count = write_idx;

        // Memory accounting (telemetry version >= 3).
        let mem_ver = unsafe { vread(core::ptr::addr_of!(page.mem_acct_version)) };
        if mem_ver == MEM_ACCT_VERSION {
            snap.mem_acct = unsafe { vread(core::ptr::addr_of!(page.mem_acct)) };
        }

        snap
    }

    fn compute_cpu_usage(&mut self, snap: &mut SystemSnapshot) {
        use sunlight_ipc::cpu_accounting::{rate, usage_bp};
        let dt = snap.sample_time_ns.saturating_sub(self.last_sample_time_ns);
        let valid = self.last_sample_time_ns != 0
            && dt != 0
            && snap.cpu_count == self.last_snapshot.cpu_count;
        let capacity = dt.saturating_mul(snap.cpu_count.max(1) as u64);
        snap.cpu_capacity_ns = if valid { capacity } else { 0 };
        let mut next_pids = [0; MAX_PROCESSES];
        let mut next_runtimes = [0; MAX_PROCESSES];
        let mut next_generations = [0; MAX_PROCESSES];
        let mut task_total = 0u64;
        for (i, p) in snap.procs[..snap.proc_count].iter_mut().enumerate() {
            let prev = (0..MAX_PROCESSES)
                .find(|j| self.last_pids[*j] == p.pid && self.last_generations[*j] == p.generation);
            // Unknown/new tasks establish a baseline, never charge lifetime
            // runtime into one interval. System totals are independent of rows.
            let delta = prev
                .map(|j| p.cpu_ticks.saturating_sub(self.last_runtime_by_pid[j]))
                .unwrap_or(0);
            p.runtime_delta_ns = if valid { delta } else { 0 };
            p.cpu_bp = usage_bp(p.runtime_delta_ns, snap.cpu_capacity_ns);
            task_total = task_total.saturating_add(delta);
            next_pids[i] = p.pid;
            next_runtimes[i] = p.cpu_ticks;
            next_generations[i] = p.generation;
        }
        let mut busy = 0u64;
        let mut measured_capacity = 0u64;
        for i in 0..snap.cpu_telemetry.count {
            let c = snap.cpu_counters[i];
            let p = self.last_snapshot.cpu_counters[i];
            let elapsed = c.elapsed_ns.saturating_sub(p.elapsed_ns);
            let b = c.busy_ns.saturating_sub(p.busy_ns);
            let core = &mut snap.cpu_telemetry.cores[i];
            core.load_bp = if valid && snap.accounting_version >= 4 {
                usage_bp(b, elapsed)
            } else {
                0
            };
            core.switches_per_second = if valid {
                rate(
                    core.context_switches
                        .saturating_sub(self.last_snapshot.cpu_telemetry.cores[i].context_switches),
                    dt,
                )
            } else {
                0
            };
            busy = busy.saturating_add(b);
            measured_capacity = measured_capacity.saturating_add(elapsed);
        }
        snap.cpu_used_bp = if !valid {
            0
        } else if snap.accounting_version >= 4 {
            usage_bp(busy, measured_capacity)
        } else {
            usage_bp(task_total, capacity)
        };
        snap.cpu_idle_bp = 10_000 - snap.cpu_used_bp;
        self.last_pids = next_pids;
        self.last_runtime_by_pid = next_runtimes;
        self.last_generations = next_generations;
        self.last_sample_time_ns = snap.sample_time_ns;
    }

    fn fill_local_time(&self, snap: &mut SystemSnapshot) {
        let Some(tz_cap) = nameserver_lookup("tz") else {
            return;
        };

        let reply = ipc_call(tz_cap, IpcMsg::with_label(TzMsg::GET_LOCAL_TIME));
        if reply.label != TzMsg::REPLY {
            return;
        }

        let packed = reply.words[0];
        let hour = ((packed >> 24) & 0xff) as u8;
        let min = ((packed >> 16) & 0xff) as u8;
        let sec = ((packed >> 8) & 0xff) as u8;

        let mut out = [0u8; 16];
        out[0] = b'0' + (hour / 10);
        out[1] = b'0' + (hour % 10);
        out[2] = b':';
        out[3] = b'0' + (min / 10);
        out[4] = b'0' + (min % 10);
        out[5] = b':';
        out[6] = b'0' + (sec / 10);
        out[7] = b'0' + (sec % 10);

        snap.local_time = out;
        snap.local_time_len = 8;
    }
}

#[inline(always)]
/// Public rows represent processes, aggregating Linux threads after deltas
/// have been calculated with task identities. Shared address-space memory is
/// counted once (maximum thread measurement), not summed once per thread.
fn aggregate_thread_groups(snap: &mut SystemSnapshot) {
    let mut count = 0;
    for i in 0..snap.proc_count {
        let mut p = snap.procs[i];
        if p.tgid == 0 {
            p.tgid = p.pid;
        }
        if let Some(j) = (0..count).find(|j| snap.procs[*j].tgid == p.tgid) {
            let group = &mut snap.procs[j];
            group.runtime_delta_ns = group.runtime_delta_ns.saturating_add(p.runtime_delta_ns);
            group.cpu_bp = sunlight_ipc::cpu_accounting::usage_bp(
                group.runtime_delta_ns,
                snap.cpu_capacity_ns,
            );
            group.cpu_ticks = group.cpu_ticks.saturating_add(p.cpu_ticks);
            group.mem_kb = group.mem_kb.max(p.mem_kb);
            if p.state == ProcessState::Running {
                group.state = ProcessState::Running;
            }
        } else {
            p.pid = p.tgid;
            snap.procs[count] = p;
            count += 1;
        }
    }
    snap.proc_count = count;
}

unsafe fn vread<T: Copy>(ptr: *const T) -> T {
    unsafe { core::ptr::read_volatile(ptr) }
}

#[cfg(test)]
mod cpu_tests {
    use super::*;
    const SECOND: u64 = 1_000_000_000;
    fn reader() -> Telemetry {
        Telemetry {
            page_ptr: core::ptr::null(),
            last_seq: 0,
            last_sample_time_ns: 0,
            last_runtime_by_pid: [0; MAX_PROCESSES],
            last_pids: [0; MAX_PROCESSES],
            last_generations: [0; MAX_PROCESSES],
            last_snapshot: SystemSnapshot::default(),
        }
    }
    fn sample(time: u64, busy_cores: usize) -> SystemSnapshot {
        let mut s = SystemSnapshot::default();
        s.accounting_version = 4;
        s.cpu_count = 4;
        s.sample_time_ns = time;
        s.cpu_telemetry.count = 4;
        for i in 0..4 {
            s.cpu_counters[i] = CpuCounters {
                elapsed_ns: time,
                busy_ns: if i < busy_cores { time } else { 0 },
                idle_ns: if i < busy_cores { 0 } else { time },
                ..CpuCounters::ZERO
            };
        }
        s
    }
    fn accept(r: &mut Telemetry, s: &mut SystemSnapshot) {
        r.compute_cpu_usage(s);
        r.last_snapshot = *s;
    }
    fn task(s: &mut SystemSnapshot, pid: u32, runtime: u64) {
        let i = s.proc_count;
        s.procs[i].pid = pid;
        s.procs[i].tgid = pid;
        s.procs[i].generation = 1;
        s.procs[i].cpu_ticks = runtime;
        s.proc_count += 1;
    }
    #[test]
    fn idle_one_two_four_busy_cores() {
        for busy in [0, 1, 2, 4] {
            let mut r = reader();
            let mut a = sample(SECOND, busy);
            let mut b = sample(2 * SECOND, busy);
            if busy != 0 {
                task(&mut a, 1, SECOND);
                task(&mut b, 1, 2 * SECOND);
            }
            accept(&mut r, &mut a);
            accept(&mut r, &mut b);
            assert_eq!(b.cpu_used_bp, busy as u16 * 2500);
            assert_eq!(b.cpu_idle_bp, 10_000 - busy as u16 * 2500);
            if busy != 0 {
                assert_eq!(b.procs[0].cpu_bp, 2500);
            }
            for i in 0..4 {
                assert_eq!(
                    b.cpu_telemetry.cores[i].load_bp,
                    if i < busy { 10_000 } else { 0 }
                );
            }
        }
    }
    #[test]
    fn sleeping_and_frequently_scheduled_short_tasks() {
        let mut r = reader();
        let mut a = sample(SECOND, 0);
        let mut b = sample(2 * SECOND, 0);
        task(&mut a, 1, 0);
        task(&mut b, 1, 1_000_000);
        b.cpu_telemetry.cores[0].context_switches = 1_000_000;
        accept(&mut r, &mut a);
        accept(&mut r, &mut b);
        assert_eq!(b.procs[0].cpu_bp, 2); // 0.025% machine capacity
        assert_eq!(b.cpu_telemetry.cores[0].switches_per_second, 1_000_000);
    }
    #[test]
    fn exit_and_table_truncation_do_not_remove_system_runtime() {
        let mut r = reader();
        let mut a = sample(SECOND, 1);
        let mut b = sample(2 * SECOND, 1);
        task(&mut a, 1, SECOND); // exits before b; no process rows remain
        accept(&mut r, &mut a);
        accept(&mut r, &mut b);
        assert_eq!(b.cpu_used_bp, 2500);
        assert_eq!(b.cpu_idle_bp, 7500);
    }
    #[test]
    fn reused_pid_and_new_tasks_establish_baseline() {
        let mut r = reader();
        let mut a = sample(SECOND, 1);
        let mut b = sample(2 * SECOND, 1);
        task(&mut a, 1, SECOND);
        task(&mut b, 1, 10 * SECOND);
        b.procs[0].generation = 2;
        task(&mut b, 2, 10 * SECOND);
        accept(&mut r, &mut a);
        accept(&mut r, &mut b);
        assert_eq!(b.procs[0].cpu_bp, 0);
        assert_eq!(b.procs[1].cpu_bp, 0);
        assert_eq!(b.cpu_used_bp, 2500);
    }
    #[test]
    fn repeated_samples_conserve_capacity_without_drift() {
        let mut r = reader();
        for n in 1..1000 {
            let mut s = sample(SECOND * n + SECOND * 1_000_000_000, 2);
            accept(&mut r, &mut s);
            assert_eq!(s.cpu_used_bp + s.cpu_idle_bp, 10_000);
            if n > 1 {
                assert_eq!(s.cpu_used_bp, 5000);
            }
        }
    }
    #[test]
    fn duplicate_time_and_cpu_count_change_rebaseline() {
        let mut r = reader();
        let mut s = sample(SECOND, 1);
        accept(&mut r, &mut s);
        accept(&mut r, &mut s);
        assert_eq!(s.cpu_used_bp, 0);
        s.sample_time_ns += SECOND;
        s.cpu_count = 2;
        accept(&mut r, &mut s);
        assert_eq!(s.cpu_used_bp, 0);
    }
    #[test]
    fn thread_groups_sum_cpu_once_and_share_memory() {
        let mut r = reader();
        let mut a = sample(SECOND, 2);
        let mut b = sample(2 * SECOND, 2);
        for s in [&mut a, &mut b] {
            task(s, 1, s.sample_time_ns);
            task(s, 2, s.sample_time_ns);
            s.procs[1].tgid = 1;
            s.procs[0].mem_kb = 128;
            s.procs[1].mem_kb = 128;
        }
        accept(&mut r, &mut a);
        accept(&mut r, &mut b);
        aggregate_thread_groups(&mut b);
        assert_eq!(b.proc_count, 1);
        assert_eq!(b.procs[0].cpu_bp, 5000);
        assert_eq!(b.procs[0].mem_kb, 128);
    }
    #[test]
    fn v4_layout_is_append_only_and_fits_mapping() {
        assert_eq!(core::mem::size_of::<ProcessStat>(), 60);
        assert_eq!(core::mem::size_of::<RawCoreStat>(), 32);
        assert!(
            core::mem::offset_of!(TelemetryPage, cpu_counters)
                > core::mem::offset_of!(TelemetryPage, mem_acct)
        );
        assert_eq!(core::mem::offset_of!(TelemetryPage, cpu_counters), 6752);
        assert_eq!(core::mem::size_of::<TelemetryPage>(), 12128);
    }

    #[test]
    fn raw_reaped_rows_are_excluded_and_blocked_states_preserved() {
        // The shared page is all integer fields: zero is a valid fixture.
        let mut page: TelemetryPage = unsafe { core::mem::zeroed() };
        page.version = 4;
        page.cpu_count = 4;
        page.sample_time_ns = 123_456;
        page.proc_count = 4;
        for (i, state) in [7, 4, 5, 6].into_iter().enumerate() {
            page.procs[i].pid = i as u32 + 1;
            page.procs[i].state = state;
        }
        let mut r = reader();
        r.page_ptr = &page;
        let s = r.read_page();
        assert_eq!(s.sample_time_ns, 123_456);
        assert_eq!(s.proc_count, 3);
        for p in &s.procs[..3] {
            assert!(p.pid != 1);
            assert!(p.state == ProcessState::Blocked);
        }
    }

    #[test]
    fn preempted_writer_does_not_spin_reader_or_replace_sample() {
        let mut page: TelemetryPage = unsafe { core::mem::zeroed() };
        page.sequence = 3;
        let mut r = reader();
        r.page_ptr = &page;
        r.last_snapshot.sample_time_ns = 42;
        assert!(!r.poll());
        assert_eq!(r.snapshot().sample_time_ns, 42);
    }
}
