#![no_std]

mod topology;
pub use topology::{CoreSnapshot, CpuTelemetry, TaskStats, TopologyInfo, MAX_CORES};

use sunlight_ipc::{ipc_call, map_telemetry, nameserver_lookup, IpcMsg, TzMsg};

pub const TELEMETRY_MAGIC: u64 = 0x5355_4E4C_5449_4D45;
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
    pub gpu_count: u8,
    pub _pad: [u8; 2],
    pub sample_time_ns: u64,
    pub proc_count: u32,
    pub procs: [ProcessStat; MAX_PROCESSES],
}

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
    pub pid: u32,
    pub ppid: u32,
    pub state: ProcessState,
    pub name: [u8; 32],
    pub cpu_ticks: u64,
    pub cpu_bp: u16,
    pub mem_kb: u32,
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
}

impl Default for SystemSnapshot {
    fn default() -> Self {
        Self {
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
        }
    }
}

pub struct Telemetry {
    page_ptr: *const TelemetryPage,
    last_seq: u32,
    last_sample_time_ns: u64,
    last_runtime_by_pid: [u64; MAX_PROCESSES],
    last_pids: [u32; MAX_PROCESSES],
    last_ticks: [u64; MAX_PROCESSES],
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
            last_ticks: [0; MAX_PROCESSES],
            last_snapshot: SystemSnapshot::default(),
        })
    }

    pub fn poll(&mut self) -> bool {
        loop {
            let seq1 = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sequence)) };
            if seq1 & 1 == 1 {
                core::hint::spin_loop();
                continue;
            }

            if seq1 == self.last_seq {
                return false;
            }

            let mut snap = self.read_page();
            let seq2 = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sequence)) };
            if seq2 != seq1 {
                continue;
            }

            self.compute_cpu_usage(&mut snap);
            self.fill_local_time(&mut snap);

            self.last_seq = seq2;
            self.last_snapshot = snap;
            return true;
        }
    }

    pub fn snapshot(&self) -> &SystemSnapshot {
        &self.last_snapshot
    }

    fn read_page(&self) -> SystemSnapshot {
        let mut snap = SystemSnapshot::default();
        let page = unsafe { &*self.page_ptr };

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
        }

        // Topology: derived from cpu_count until CPUID leaf 0x0B is wired.
        snap.topology = TopologyInfo::from_cpu_count(snap.cpu_count);

        // Bootstrap the per-core table: one CoreSnapshot per online CPU.
        // In the uniprocessor kernel this is always exactly one entry.
        // The SMP path will extend this once APs are brought online.
        let logical_count = (snap.cpu_count as usize).max(1).min(MAX_CORES);
        snap.cpu_telemetry.count = logical_count;
        for i in 0..logical_count {
            snap.cpu_telemetry.cores[i].core_id = i as u8;
            snap.cpu_telemetry.cores[i].affinity_mask = !0u64; // unrestricted
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

            // Track the first Running task's PID as the "current" task on core 0.
            // With SMP the kernel would tell us which core each task is on.
            if raw.state == 1 && snap.cpu_telemetry.cores[0].current_task_pid == 0 {
                snap.cpu_telemetry.cores[0].current_task_pid = raw.pid;
            }

            snap.procs[i] = ProcessSnapshot {
                pid: raw.pid,
                ppid: raw.ppid,
                state: match raw.state {
                    1 => ProcessState::Running,
                    2 => ProcessState::Blocked,
                    3 => ProcessState::Finished,
                    _ => ProcessState::Ready,
                },
                name: raw.name,
                cpu_ticks: raw.cpu_ticks,
                cpu_bp: 0,
                mem_kb: raw.mem_pages.saturating_mul(4),
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

        snap
    }

    fn compute_cpu_usage(&mut self, snap: &mut SystemSnapshot) {
        let cur_sample = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sample_time_ns)) };
        let interval_ns: u64 =
            if self.last_sample_time_ns != 0 && cur_sample > self.last_sample_time_ns {
                cur_sample.saturating_sub(self.last_sample_time_ns)
            } else {
                0
            };

        let cpu_count = if snap.cpu_count > 0 {
            snap.cpu_count as u64
        } else {
            1
        };
        let capacity_delta_ns: u64 = interval_ns.saturating_mul(cpu_count);

        let mut used_delta_ns: u64 = 0;
        let mut proc_deltas = [0u64; MAX_PROCESSES];
        let mut next_pids = [0u32; MAX_PROCESSES];
        let mut next_runtimes = [0u64; MAX_PROCESSES];

        for i in 0..snap.proc_count {
            let pid = snap.procs[i].pid;
            let cur_runtime = snap.procs[i].cpu_ticks;

            let mut found = false;
            for j in 0..MAX_PROCESSES {
                if self.last_pids[j] == pid {
                    found = true;
                    break;
                }
            }
            if !found {
                for j in 0..MAX_PROCESSES {
                    if self.last_pids[j] == pid {
                        found = true;
                        break;
                    }
                }
            }

            let delta = if found {
                let mut prev_runtime = 0u64;
                for j in 0..MAX_PROCESSES {
                    if self.last_pids[j] == pid {
                        prev_runtime = self.last_runtime_by_pid[j];
                        break;
                    }
                }
                if prev_runtime == 0 {
                    for j in 0..MAX_PROCESSES {
                        if self.last_pids[j] == pid {
                            prev_runtime = self.last_ticks[j];
                            break;
                        }
                    }
                }
                cur_runtime.saturating_sub(prev_runtime)
            } else {
                0
            };
            proc_deltas[i] = delta;
            used_delta_ns = used_delta_ns.saturating_add(delta);

            next_pids[i] = pid;
            next_runtimes[i] = cur_runtime;
        }

        let mut used_bp: u32 = 0;
        let mut idle_bp: u32 = 10000;

        if interval_ns > 0 && capacity_delta_ns > 0 {
            used_bp = ((used_delta_ns as u128) * 10000u128 / (capacity_delta_ns as u128)) as u32;
            used_bp = used_bp.min(10000);
            idle_bp = 10000u32.saturating_sub(used_bp);
        } else if interval_ns == 0 {
            used_bp = self.last_snapshot.cpu_used_bp as u32;
            idle_bp = self.last_snapshot.cpu_idle_bp as u32;
        }

        if used_bp + idle_bp > 10000 {
            idle_bp = 10000 - used_bp;
        }

        snap.cpu_used_bp = used_bp as u16;
        snap.cpu_idle_bp = idle_bp as u16;

        // Per-core load: in the uniprocessor kernel all work runs on core 0,
        // so core 0 mirrors the aggregate.  When SMP is added the kernel
        // telemetry page will expose per-core counters and this loop will
        // distribute load_bp across each CoreSnapshot individually.
        for i in 0..snap.cpu_telemetry.count {
            snap.cpu_telemetry.cores[i].load_bp = if i == 0 {
                snap.cpu_used_bp
            } else {
                // APs are online but kernel counters not yet per-core.
                0
            };
        }

        if capacity_delta_ns > 0 {
            for i in 0..snap.proc_count {
                let bp =
                    ((proc_deltas[i] as u128) * 10000u128 / (capacity_delta_ns as u128)) as u32;
                snap.procs[i].cpu_bp = bp.min(10000) as u16;
            }
        } else {
            for i in 0..snap.proc_count {
                snap.procs[i].cpu_bp = 0;
            }
        }

        self.last_sample_time_ns = if cur_sample != 0 {
            cur_sample
        } else {
            self.last_sample_time_ns
        };

        for j in 0..MAX_PROCESSES {
            let lp = self.last_pids[j];
            if lp != 0 {
                let mut still_present = false;
                for k in 0..snap.proc_count {
                    if snap.procs[k].pid == lp {
                        still_present = true;
                        break;
                    }
                }
                if !still_present {
                    self.last_pids[j] = 0;
                    self.last_runtime_by_pid[j] = 0;
                    self.last_ticks[j] = 0;
                }
            }
        }

        self.last_pids = next_pids;
        self.last_runtime_by_pid = next_runtimes;
        for i in 0..snap.proc_count {
            self.last_ticks[i] = next_runtimes[i];
        }
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
unsafe fn vread<T: Copy>(ptr: *const T) -> T {
    unsafe { core::ptr::read_volatile(ptr) }
}
