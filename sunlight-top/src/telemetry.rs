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
    pub _pad: [u8; 3],
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
    pub cpu_pct: u8,
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
    pub cpu_usage_pct: u8,
    pub local_time: [u8; 16],
    pub local_time_len: usize,
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
            cpu_usage_pct: 0,
            local_time: [0; 16],
            local_time_len: 0,
        }
    }
}

pub struct Telemetry {
    page_ptr: *const TelemetryPage,
    last_seq: u32,
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

        // SAFETY: kernel maps the telemetry page read-only for this process.
        let magic = unsafe { vread(core::ptr::addr_of!((*ptr).magic)) };
        if magic != TELEMETRY_MAGIC {
            return Err("TelemetryPage magic mismatch");
        }

        Ok(Self {
            page_ptr: ptr,
            last_seq: 0,
            last_pids: [0; MAX_PROCESSES],
            last_ticks: [0; MAX_PROCESSES],
            last_snapshot: SystemSnapshot::default(),
        })
    }

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

            let mut snap = self.read_page();

            // SAFETY: same mapping as above; second seqlock read validates consistency.
            let seq2 = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sequence)) };
            if seq2 != seq1 {
                continue;
            }

            self.compute_cpu_pct(&mut snap);
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

        // SAFETY: `page_ptr` is a read-only telemetry mapping.
        let page = unsafe { &*self.page_ptr };

        // SAFETY: all reads come from the kernel-owned read-only telemetry mapping.
        unsafe {
            snap.sequence = vread(core::ptr::addr_of!(page.sequence));
            snap.uptime_secs = vread(core::ptr::addr_of!(page.uptime_secs));
            snap.total_ram_kb = vread(core::ptr::addr_of!(page.total_ram_kb));
            snap.used_ram_kb = vread(core::ptr::addr_of!(page.used_ram_kb));
            snap.zram_orig_kb = vread(core::ptr::addr_of!(page.zram_orig_kb));
            snap.zram_comp_kb = vread(core::ptr::addr_of!(page.zram_comp_kb));
            snap.net_rx_bytes = vread(core::ptr::addr_of!(page.net_rx_bytes));
            snap.net_tx_bytes = vread(core::ptr::addr_of!(page.net_tx_bytes));
        }

        // SAFETY: `proc_count` is read from the same read-only telemetry mapping.
        let raw_count = unsafe { vread(core::ptr::addr_of!(page.proc_count)) } as usize;
        snap.proc_count = raw_count.min(MAX_PROCESSES);

        for i in 0..snap.proc_count {
            // SAFETY: volatile copy from a fixed in-page ProcessStat slot.
            let raw = unsafe { vread(core::ptr::addr_of!(page.procs[i])) };
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
                cpu_pct: 0,
                mem_kb: raw.mem_pages.saturating_mul(4),
            };
        }

        snap
    }

    fn compute_cpu_pct(&mut self, snap: &mut SystemSnapshot) {
        let mut total_delta = 0u64;
        let mut deltas = [0u64; MAX_PROCESSES];
        let mut next_pids = [0u32; MAX_PROCESSES];
        let mut next_ticks = [0u64; MAX_PROCESSES];

        for i in 0..snap.proc_count {
            let pid = snap.procs[i].pid;
            let cur_tick = snap.procs[i].cpu_ticks;
            let mut prev_tick = 0u64;

            for j in 0..MAX_PROCESSES {
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

        let mut peak = 0u8;
        if total_delta > 0 {
            for i in 0..snap.proc_count {
                let pct = ((deltas[i].saturating_mul(100)) / total_delta).min(100) as u8;
                snap.procs[i].cpu_pct = pct;
                if pct > peak {
                    peak = pct;
                }
            }
        }

        snap.cpu_usage_pct = peak;
        self.last_pids = next_pids;
        self.last_ticks = next_ticks;
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
    // SAFETY: caller ensures `ptr` points to a valid telemetry-mapped field.
    unsafe { core::ptr::read_volatile(ptr) }
}
