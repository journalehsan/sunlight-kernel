//! TelemetryPage reader for gcd (state/name/memory only — no cpu_pct
//! computation needed).
//!
//! Mirrors the seqlock read pattern from `sunlight-top/src/telemetry.rs`.

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
    pub proc_count: u32,
    pub procs: [ProcessStat; MAX_PROCS],
}

#[derive(Clone, Copy, Default)]
pub struct ProcSample {
    pub pid: usize,
    pub name: [u8; 32],
    pub state: u8,
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
    last_snapshot: [ProcSample; MAX_PROCS],
    last_count: usize,
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
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
            last_snapshot: [ProcSample::default(); MAX_PROCS],
            last_count: 0,
            total_ram_kb: 0,
            used_ram_kb: 0,
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

            let mut samples = [ProcSample::default(); MAX_PROCS];
            let count = self.read_page(&mut samples);

            // SAFETY: same mapping as above; second seqlock read validates consistency.
            let seq2 = unsafe { vread(core::ptr::addr_of!((*self.page_ptr).sequence)) };
            if seq2 != seq1 {
                continue;
            }

            self.last_seq = seq2;
            self.last_snapshot = samples;
            self.last_count = count;
            return true;
        }
    }

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
                state: raw.state,
            };
        }

        count
    }
}

#[inline(always)]
unsafe fn vread<T: Copy>(ptr: *const T) -> T {
    // SAFETY: caller ensures `ptr` points to a valid telemetry-mapped field.
    unsafe { core::ptr::read_volatile(ptr) }
}
