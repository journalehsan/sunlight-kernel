//! Hardware topology and per-core monitoring types.
//!
//! These types are computed from the kernel [`TelemetryPage`] by
//! [`crate::Telemetry::poll`] and exposed as fields on [`crate::SystemSnapshot`].
//!
//! # Thread safety
//!
//! All types in this module are [`Copy`] and contain no raw pointers, so they
//! are automatically [`Send`] + [`Sync`].  They may be freely cloned out of a
//! snapshot and passed across thread/task boundaries.
//!
//! The *producer* ([`crate::Telemetry`]) is **not** `Sync` because it holds a
//! raw pointer to the shared-memory telemetry page.  If a process needs to
//! share a producer across threads it must guard it:
//!
//! ```ignore
//! static TELEM: spin::Mutex<Telemetry> = spin::Mutex::new(/* ... */);
//! ```

/// Maximum number of logical cores tracked by the telemetry layer.
///
/// Kept equal to `MAX_PROCESSES` so a per-core table and a per-process table
/// fit in the same stack budget.  Increase for large NUMA machines.
pub const MAX_CORES: usize = 64;

// ─── Topology ────────────────────────────────────────────────────────────────

/// Hardware topology derived from ACPI MADT and CPUID.
///
/// In the current uniprocessor kernel `physical_sockets = 1`,
/// `cores_per_socket = cpu_count`, `threads_per_core = 1`.
/// Once CPUID leaf 0x0B and MADT are fully wired these reflect the
/// true package / core / thread hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopologyInfo {
    /// Physical CPU packages (sockets) present in the system.
    pub physical_sockets: u8,
    /// Physical cores per package.
    pub cores_per_socket: u8,
    /// Logical threads per physical core (1 = no HT, 2 = hyper-threaded).
    pub threads_per_core: u8,
    /// Total schedulable logical processors (`sockets × cores × threads`).
    pub total_logical_cores: u8,
}

impl TopologyInfo {
    /// Derive a conservative topology from a flat logical-core count.
    ///
    /// Assumes single socket and no hyper-threading.  Replace once
    /// CPUID topology enumeration is implemented in the kernel.
    #[inline]
    pub fn from_cpu_count(cpu_count: u8) -> Self {
        let n = cpu_count.max(1);
        Self {
            physical_sockets: 1,
            cores_per_socket: n,
            threads_per_core: 1,
            total_logical_cores: n,
        }
    }

    /// Total logical cores as `u32` (convenience for arithmetic).
    #[inline]
    pub fn total(self) -> u32 {
        self.physical_sockets as u32
            * self.cores_per_socket as u32
            * self.threads_per_core as u32
    }
}

impl Default for TopologyInfo {
    fn default() -> Self {
        Self::from_cpu_count(1)
    }
}

// ─── Per-core snapshot ───────────────────────────────────────────────────────

/// State snapshot for one logical processor at a single point in time.
#[derive(Clone, Copy, Debug, Default)]
pub struct CoreSnapshot {
    /// Logical core index (0-based, APIC-ID order).
    pub core_id: u8,
    /// CPU utilization in basis points (0 = idle, 10 000 = 100.00 %).
    pub load_bp: u16,
    /// PID of the task currently scheduled on this core; 0 = idle loop.
    pub current_task_pid: u32,
    /// Scheduler affinity mask.  Bit N is set when the running task is
    /// allowed to execute on logical core N.  `!0u64` = unrestricted.
    pub affinity_mask: u64,
}

impl CoreSnapshot {
    /// Load as a percentage scaled by 100 (same unit as `load_bp`).
    /// 10 000 → 100.00 %; 742 → 7.42 %.
    #[inline]
    pub fn load_pct_x100(self) -> u32 {
        self.load_bp as u32
    }

    /// `true` when a real (non-idle) task is running on this core.
    #[inline]
    pub fn is_busy(self) -> bool {
        self.current_task_pid != 0
    }
}

// ─── Per-core telemetry table ─────────────────────────────────────────────────

/// Fixed-capacity per-core telemetry table indexed by logical core ID.
///
/// [`CpuTelemetry::iter`] yields only the *populated* entries (`..self.count`).
/// In the current uniprocessor kernel `count` is always 1 and `cores[0]`
/// carries the aggregate system load.
#[derive(Clone, Copy)]
pub struct CpuTelemetry {
    /// Snapshot for each online logical core, ordered by `core_id`.
    pub cores: [CoreSnapshot; MAX_CORES],
    /// Number of valid entries in `cores`.
    pub count: usize,
}

impl Default for CpuTelemetry {
    fn default() -> Self {
        Self {
            cores: [CoreSnapshot::default(); MAX_CORES],
            count: 0,
        }
    }
}

impl CpuTelemetry {
    /// Slice over populated core snapshots (length = `self.count`).
    #[inline]
    pub fn iter(&self) -> &[CoreSnapshot] {
        &self.cores[..self.count.min(MAX_CORES)]
    }

    /// Look up a core by `core_id`; `None` if the core is not online.
    pub fn get(&self, core_id: u8) -> Option<&CoreSnapshot> {
        let slice = self.iter();
        let mut i = 0;
        while i < slice.len() {
            if slice[i].core_id == core_id {
                return Some(&slice[i]);
            }
            i += 1;
        }
        None
    }

    /// Number of cores that are currently executing a non-idle task.
    pub fn busy_count(&self) -> usize {
        let slice = self.iter();
        let mut n = 0usize;
        let mut i = 0;
        while i < slice.len() {
            if slice[i].is_busy() {
                n += 1;
            }
            i += 1;
        }
        n
    }

    /// System-wide aggregate utilization in basis points.
    ///
    /// For a 4-core machine at 25 % each this returns 2 500, not 10 000 —
    /// it is the *per-core average*, not the sum.  Use the individual
    /// `load_bp` fields to get per-core values.
    pub fn average_load_bp(&self) -> u16 {
        let n = self.count.max(1);
        let mut sum: u32 = 0;
        for c in self.iter().iter() {
            sum = sum.saturating_add(c.load_bp as u32);
        }
        (sum / n as u32).min(10_000) as u16
    }
}

// ─── Task statistics ──────────────────────────────────────────────────────────

/// Aggregated task-state counts across all processes in the system.
///
/// Computed from raw kernel state bytes in the telemetry page
/// *before* finished processes are filtered out, so `zombie` is accurate.
///
/// Kernel state-byte mapping (from `kernel/src/telemetry.rs`):
/// ```text
/// 0 = Ready
/// 1 = Running
/// 2 = BlockedOnIpc   ┐
/// 4 = Suspended       ├─ all counted as `blocked`
/// 5 = BlockedOnTimer  │
/// 6 = BlockedOnIo    ┘
/// 3 = Finished (zombie, pending reap)
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct TaskStats {
    /// Tasks currently executing on a CPU.
    pub running: u16,
    /// Runnable tasks waiting for a free CPU.
    pub ready: u16,
    /// Tasks blocked on IPC, timers, I/O, or explicitly suspended.
    pub blocked: u16,
    /// Tasks that have exited but whose exit code has not yet been reaped.
    pub zombie: u16,
    /// Live task count: `running + ready + blocked` (zombies excluded).
    pub total: u16,
}

impl TaskStats {
    /// Accumulate one raw kernel state byte into the counters.
    ///
    /// Called once per raw `ProcessStat` entry during [`crate::Telemetry::read_page`].
    #[inline]
    pub(crate) fn tally(&mut self, raw_state: u8) {
        match raw_state {
            0 => self.ready   = self.ready.saturating_add(1),
            1 => self.running = self.running.saturating_add(1),
            2 | 4 | 5 | 6 => self.blocked = self.blocked.saturating_add(1),
            3 => self.zombie  = self.zombie.saturating_add(1),
            _ => {}
        }
    }

    /// Finalise the `total` field after all entries have been tallied.
    #[inline]
    pub(crate) fn commit(&mut self) {
        self.total = self
            .running
            .saturating_add(self.ready)
            .saturating_add(self.blocked);
    }
}
