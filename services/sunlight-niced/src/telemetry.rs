//! TelemetryPage reader for niced.
//!
//! Mirrors the seqlock read pattern and per-process cpu_pct computation
//! from `sunlight-top/src/telemetry.rs` (`Telemetry::poll`,
//! `Telemetry::read_page`, `Telemetry::compute_cpu_pct`, `vread`).

use sunlight_ipc::map_telemetry;

pub const TELEMETRY_MAGIC: u64 = 0x5355_4E4C_5449_4D45;
pub const MAX_PROCS: usize = 64;

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
    /// Monotonic kernel sample time (ns) when counters were captured.
    pub sample_time_ns: u64,
    pub proc_count: u32,
    pub procs: [ProcessStat; MAX_PROCS],
}

/// A single process's CPU/memory snapshot, computed from telemetry deltas.
#[derive(Clone, Copy, Default)]
pub struct ProcSample {
    pub pid: usize,
    pub name: [u8; 32],
    pub cpu_pct: u16,
    pub mem_kb: u32,
    pub state: u8,
    pub cpu_ticks: u64,
}

impl ProcSample {
    pub fn name_str(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.name.len());
        core::str::from_utf8(&self.name[..len]).unwrap_or("?")
    }
}

pub struct Telemetry {
    page_ptr: *const TelemetryPage,
    last_seq: u32,
    last_pids: [u32; MAX_PROCS],
    last_ticks: [u64; MAX_PROCS],
    /// Total ram/used ram from the most recent successful poll.
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
    last_snapshot: [ProcSample; MAX_PROCS],
    last_count: usize,
}

impl Telemetry {
    pub fn init() -> Result<Self, &'static str> {
        let ptr = map_telemetry() as *const TelemetryPage;
        if ptr.is_null() {
            return Err("SYS_MAP_TELEMETRY failed");
        }

        // SAFETY: kernel maps the telemetry page read-only for this process.
        let magic = unsafe { vread(core::ptr::addr_of!((*ptr).magic)) };
        if magic != TELEMETRY_MAGIC {
            return Err("TelemetryPage magic mismatch");
        }

        Ok(Self {
            page_ptr: ptr,
            last_seq: 0,
            last_pids: [0; MAX_PROCS],
            last_ticks: [0; MAX_PROCS],
            total_ram_kb: 0,
            used_ram_kb: 0,
            last_snapshot: [ProcSample::default(); MAX_PROCS],
            last_count: 0,
        })
    }

    /// Poll the telemetry page using the seqlock pattern. Returns `true` if
    /// a new (different sequence) snapshot was read.
    pub fn poll(&mut self) -> bool {
        loop {
            // SAFETY: `page_ptr` points to a valid read-only mapping from the kernel.
            let seq1 = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sequence)) };
            if seq1 & 1 == 1 {
                core::hint::spin_loop();
                continue;
            }

            if seq1 == self.last_seq {
                return false;
            }

            let mut samples = [ProcSample::default(); MAX_PROCS];
            let count = self.read_page(&mut samples);

            // SAFETY: same mapping as above; second seqlock read validates consistency.
            let seq2 = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sequence)) };
            if seq2 != seq1 {
                continue;
            }

            self.compute_cpu_pct(&mut samples, count);

            self.last_seq = seq2;
            self.last_snapshot = samples;
            self.last_count = count;
            return true;
        }
    }

    /// Returns the most recently polled snapshot.
    pub fn snapshot(&self, out: &mut [ProcSample; MAX_PROCS]) -> usize {
        *out = self.last_snapshot;
        self.last_count
    }

    fn read_page(&mut self, samples: &mut [ProcSample; MAX_PROCS]) -> usize {
        // SAFETY: `page_ptr` is a read-only telemetry mapping.
        let page = unsafe { &*self.page_ptr };

        // SAFETY: all reads come from the kernel-owned read-only telemetry mapping.
        unsafe {
            self.total_ram_kb = vread(core::ptr::addr_of!(page.total_ram_kb));
            self.used_ram_kb = vread(core::ptr::addr_of!(page.used_ram_kb));
        }

        // SAFETY: `proc_count` is read from the same read-only telemetry mapping.
        let raw_count = unsafe { vread(core::ptr::addr_of!(page.proc_count)) } as usize;
        let count = raw_count.min(MAX_PROCS);

        for i in 0..count {
            // SAFETY: volatile copy from a fixed in-page ProcessStat slot.
            let raw = unsafe { vread(core::ptr::addr_of!(page.procs[i])) };
            samples[i] = ProcSample {
                pid: raw.pid as usize,
                name: raw.name,
                cpu_pct: 0,
                mem_kb: raw.mem_pages.saturating_mul(4),
                state: raw.state,
                cpu_ticks: raw.cpu_ticks,
            };
        }

        count
    }

    /// Compute per-process cpu_pct from cpu_ticks deltas over time, mirroring
    /// `sunlight-top`'s `compute_cpu_pct` (total-delta-relative percentage).
    fn compute_cpu_pct(&mut self, samples: &mut [ProcSample; MAX_PROCS], count: usize) {
        let mut total_delta = 0u64;
        let mut deltas = [0u64; MAX_PROCS];
        let mut next_pids = [0u32; MAX_PROCS];
        let mut next_ticks = [0u64; MAX_PROCS];

        for i in 0..count {
            let pid = samples[i].pid as u32;
            let cur_tick = samples[i].cpu_ticks;
            let mut prev_tick = 0u64;

            for j in 0..MAX_PROCS {
                if self.last_pids[j] == pid {
                    prev_tick = self.last_ticks[j];
                    break;
                }
            }

            let delta = cur_tick.saturating_sub(prev_tick);
            deltas[i] = delta;
            total_delta = total_delta.saturating_add(delta);
            next_pids[i] = pid;
            next_ticks[i] = cur_tick;
        }

        if total_delta > 0 {
            for i in 0..count {
                let pct = ((deltas[i].saturating_mul(100)) / total_delta).min(100) as u16;
                samples[i].cpu_pct = pct;
            }
        }

        self.last_pids = next_pids;
        self.last_ticks = next_ticks;
    }
}

#[inline(always)]
unsafe fn vread<T: Copy>(ptr: *const T) -> T {
    // SAFETY: caller ensures `ptr` points to a valid telemetry-mapped field.
    unsafe { core::ptr::read_volatile(ptr) }
}
