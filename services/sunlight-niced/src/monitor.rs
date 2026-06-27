//! Monitored-process table and per-tick penalty/recovery logic for niced.

use heapless::{String, Vec};

use crate::config::{DynamicRule, ProcessConfig, RecoveryRule, NICE_MAX, NICE_MIN};
use crate::shim::KernelOps;
use crate::telemetry::{ProcSample, Telemetry, MAX_PROCS as TEL_MAX_PROCS};

pub const MAX_PROCS: usize = 64;

/// Ring-buffer depth for per-process CPU history.
///
/// DEVIATION (correction 4 / static-BSS sizing): the spec's default
/// WINDOW=360 would make the monitor table ~360 * 64 * 16 bytes ≈ 368 KiB,
/// which exceeds both the 256 KiB heap and is uncomfortably large for a
/// static BSS array on top of everything else this daemon needs. WINDOW is
/// reduced to 64 samples (at 1 Hz this is ~64 seconds of history, which is
/// more than enough for the cpu_window_secs=10 dynamic rule and the
/// check_interval_secs=15 recovery rule used by the default config).
pub const WINDOW: usize = 64;

#[inline]
pub fn demote(nice: i8, step: i8) -> i8 {
    (nice.saturating_add(step)).min(NICE_MAX)
}

#[inline]
pub fn promote(nice: i8, step: i8, floor: i8) -> i8 {
    (nice.saturating_sub(step)).max(floor)
}

#[derive(Clone, Copy, Default)]
pub struct CpuSample {
    pub t: u64,
    pub cpu_pct: u16,
}

#[derive(Clone)]
pub struct MonitoredProcess {
    pub pid: usize,
    pub name: String<32>,
    pub config: ProcessConfig,
    pub samples: [CpuSample; WINDOW],
    pub head: usize,
    pub filled: usize,
    pub original_nice: i8,
    pub current_nice: i8,
    pub registered_at: u64,
    pub penalty_active: bool,
    pub cooldown_until: u64,
    pub repeat_count: u32,
    pub last_recovery_check: u64,

    // Watchdog state (C.6)
    pub last_ping: u64,
    pub pings_missed: u32,
    pub unresp_escalation: u32,
    pub not_responding: bool,
    pub not_responding_since: u64,
    pub unresp_demotion_applied: bool,
    pub unresp_original_nice: i8,

    /// Cached `SunlightdOp::Status` supervised-by-sunlightd lookup result.
    /// `None` = not yet determined.
    pub supervised: Option<bool>,
}

impl MonitoredProcess {
    fn new(pid: usize, name: &str, config: ProcessConfig, now: u64) -> Self {
        let mut n = String::new();
        let _ = n.push_str(name);
        let nice = config.nice;
        Self {
            pid,
            name: n,
            config,
            samples: [CpuSample::default(); WINDOW],
            head: 0,
            filled: 0,
            original_nice: nice,
            current_nice: nice,
            registered_at: now,
            penalty_active: false,
            cooldown_until: 0,
            repeat_count: 0,
            last_recovery_check: now,
            last_ping: now,
            pings_missed: 0,
            unresp_escalation: 0,
            not_responding: false,
            not_responding_since: 0,
            unresp_demotion_applied: false,
            unresp_original_nice: nice,
            supervised: None,
        }
    }

    /// Push a new CPU sample into the ring buffer (no allocation).
    pub fn push_sample(&mut self, now: u64, cpu_pct: u16) {
        self.samples[self.head] = CpuSample { t: now, cpu_pct };
        self.head = (self.head + 1) % WINDOW;
        if self.filled < WINDOW {
            self.filled += 1;
        }
    }

    /// Average CPU% over the last `window_secs`, walking the ring buffer in
    /// place. Returns 0 if fewer than 2 samples fall within the window.
    pub fn cpu_avg_over(&self, window_secs: u64, now: u64) -> u32 {
        let cutoff = now.saturating_sub(window_secs);
        let mut sum: u64 = 0;
        let mut n: u64 = 0;
        for i in 0..self.filled {
            let idx = (self.head + WINDOW - 1 - i) % WINDOW;
            let s = &self.samples[idx];
            if s.t < cutoff {
                break;
            }
            sum += s.cpu_pct as u64;
            n += 1;
        }
        if n < 2 {
            return 0;
        }
        (sum / n) as u32
    }
}

pub struct Table {
    slots: [Option<MonitoredProcess>; MAX_PROCS],
}

impl Table {
    pub fn new() -> Self {
        Self {
            slots: [const { None }; MAX_PROCS],
        }
    }

    pub fn find_mut(&mut self, pid: usize) -> Option<&mut MonitoredProcess> {
        self.slots
            .iter_mut()
            .filter_map(|s| s.as_mut())
            .find(|p| p.pid == pid)
    }

    pub fn find(&self, pid: usize) -> Option<&MonitoredProcess> {
        self.slots
            .iter()
            .filter_map(|s| s.as_ref())
            .find(|p| p.pid == pid)
    }

    pub fn insert(&mut self, pid: usize, name: &str, config: ProcessConfig, now: u64) {
        if self.find(pid).is_some() {
            return;
        }
        for slot in self.slots.iter_mut() {
            if slot.is_none() {
                *slot = Some(MonitoredProcess::new(pid, name, config, now));
                return;
            }
        }
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut MonitoredProcess> {
        self.slots.iter_mut().filter_map(|s| s.as_mut())
    }
}

/// Per-tick: refresh telemetry-derived samples, register newly-seen
/// configured processes, and run the penalty/recovery logic (C.5).
pub fn tick<K: KernelOps>(
    table: &mut Table,
    tel: &mut Telemetry,
    shim: &K,
    configs: &Vec<ProcessConfig, { crate::config::MAX_CONFIGS }>,
    dyn_rules: &Vec<DynamicRule, { crate::config::MAX_DYN_RULES }>,
    recovery_rules: &Vec<RecoveryRule, { crate::config::MAX_RECOVERY_RULES }>,
    now: u64,
) {
    let mut samples = [ProcSample::default(); TEL_MAX_PROCS];
    let count = tel.snapshot(&mut samples);

    // Lazily register telemetry-visible processes whose name matches a
    // configured process and that aren't monitored yet.
    for i in 0..count {
        let s = &samples[i];
        if table.find(s.pid).is_some() {
            continue;
        }
        let pname = s.name_str();
        for c in configs.iter() {
            if c.name.as_str() == pname {
                let _ = shim.set_nice(s.pid, c.nice);
                table.insert(s.pid, pname, c.clone(), now);
                break;
            }
        }
    }

    // Push CPU samples for already-monitored processes.
    for i in 0..count {
        let s = &samples[i];
        if let Some(p) = table.find_mut(s.pid) {
            p.push_sample(now, s.cpu_pct);
        }
    }

    // Run penalty/recovery for each monitored process.
    for p in table.iter_mut() {
        // Skip self (the matched config name "sunlight-niced") and any
        // protected process.
        if p.config.protect {
            continue;
        }

        // Startup grace period.
        if now.saturating_sub(p.registered_at) < p.config.startup_grace_secs {
            continue;
        }

        if !p.penalty_active {
            for rule in dyn_rules.iter() {
                if let Some(exempt) = rule.exempt_priority {
                    if exempt == p.config.priority {
                        continue;
                    }
                }
                let avg = p.cpu_avg_over(rule.cpu_window_secs, now);
                if avg > rule.cpu_threshold_pct {
                    let target = demote(p.current_nice, rule.penalty_step);
                    if target <= p.current_nice {
                        // Already at NICE_MAX; nothing further to do.
                        continue;
                    }
                    shim.set_nice(p.pid, target);
                    let cooldown = (rule
                        .cooldown_secs
                        .checked_shl(p.repeat_count)
                        .unwrap_or(u64::MAX))
                    .min(3600);
                    crate::serial_println!(
                        "[NICED] PENALTY pid={} '{}' cpu_avg={}% nice {}->{} cooldown={}s",
                        p.pid,
                        p.name.as_str(),
                        avg,
                        p.current_nice,
                        target,
                        cooldown
                    );
                    p.current_nice = target;
                    p.penalty_active = true;
                    p.cooldown_until = now + cooldown;
                    p.repeat_count += 1;
                    break;
                }
            }
        }

        if p.penalty_active && now >= p.cooldown_until {
            for recovery in recovery_rules.iter() {
                if now.saturating_sub(p.last_recovery_check) < recovery.check_interval_secs {
                    continue;
                }
                let avg = p.cpu_avg_over(recovery.check_interval_secs, now);
                if avg < recovery.cpu_threshold_pct {
                    let new_nice = promote(p.current_nice, recovery.nice_step, p.original_nice);
                    if new_nice != p.current_nice {
                        shim.set_nice(p.pid, new_nice);
                        crate::serial_println!(
                            "[NICED] RECOVER pid={} '{}' cpu_avg={}% nice {}->{}",
                            p.pid,
                            p.name.as_str(),
                            avg,
                            p.current_nice,
                            new_nice
                        );
                        p.current_nice = new_nice;
                    }
                    if p.current_nice == p.original_nice {
                        p.penalty_active = false;
                        p.repeat_count = p.repeat_count.saturating_sub(1);
                    }
                }
                p.last_recovery_check = now;
            }
        }
    }

    // Lazily register newly-seen telemetry processes that match a configured
    // name and aren't already monitored. NICE_MIN/NICE_MAX bound checking is
    // performed by the kernel's clamp_nice; we don't need to re-derive it
    // here beyond the demote/promote helpers above.
    let _ = (NICE_MIN, NICE_MAX);
}
