//! Configuration for sunlight-niced.
//!
//! The in-memory defaults below correspond to a hypothetical
//! `/etc/nice/config.d/00-sunlight-core.cfg`. Per the task spec, attempting
//! a best-effort VFS read of that file is OPTIONAL; to keep the build
//! simple and robust we use the in-memory defaults as the SOLE source for
//! now.
//!
//! TODO: VFS config file loading — optionally read
//! `/etc/nice/config.d/00-sunlight-core.cfg` (INI-style: `#`/`;` comments,
//! `[Section]`, `key=value`) and merge/override these defaults. Skipped for
//! C.2 to avoid VFS-read complexity/risk in this pass; the in-memory
//! defaults are fully functional on their own.

use heapless::Vec;

pub const NICE_MIN: i8 = -10;
pub const NICE_MAX: i8 = 10;

#[derive(Clone, Copy, PartialEq)]
pub enum Priority {
    Critical,
    High,
    Normal,
    Low,
    Idle,
}

impl Priority {
    pub fn base_nice(self) -> i8 {
        match self {
            Priority::Critical => -10,
            Priority::High => -5,
            Priority::Normal => 0,
            Priority::Low => 5,
            Priority::Idle => 10,
        }
    }
}

#[derive(Clone)]
pub struct ProcessConfig {
    pub name: heapless::String<32>,
    pub nice: i8,
    pub priority: Priority,
    pub protect: bool,
    /// Watchdog ping period in seconds. 0 = opt-out (never judged
    /// unresponsive).
    pub wd_period: u64,
    pub startup_grace_secs: u64,
}

#[derive(Clone, Copy)]
pub struct DynamicRule {
    pub cpu_window_secs: u64,
    pub cpu_threshold_pct: u32,
    pub penalty_step: i8,
    pub cooldown_secs: u64,
    pub exempt_priority: Option<Priority>,
}

#[derive(Clone, Copy)]
pub struct RecoveryRule {
    pub check_interval_secs: u64,
    pub nice_step: i8,
    pub cpu_threshold_pct: u32,
}

pub const MAX_CONFIGS: usize = 16;
pub const MAX_DYN_RULES: usize = 4;
pub const MAX_RECOVERY_RULES: usize = 4;

fn name32(s: &str) -> heapless::String<32> {
    let mut out = heapless::String::new();
    let _ = out.push_str(s);
    out
}

fn cfg(name: &str, priority: Priority, protect: bool, wd_period: u64) -> ProcessConfig {
    ProcessConfig {
        name: name32(name),
        nice: priority.base_nice(),
        priority,
        protect,
        wd_period,
        startup_grace_secs: 5,
    }
}

/// Build the hardcoded default configuration set, equivalent to a
/// `00-sunlight-core.cfg` unit. Protected core services default to
/// `wd_period=0` (opt-out) until they explicitly opt in via
/// `WatchdogSec=` in their sunlightd unit file.
pub fn load_defaults() -> (
    Vec<ProcessConfig, MAX_CONFIGS>,
    Vec<DynamicRule, MAX_DYN_RULES>,
    Vec<RecoveryRule, MAX_RECOVERY_RULES>,
) {
    let mut configs: Vec<ProcessConfig, MAX_CONFIGS> = Vec::new();

    let _ = configs.push(cfg("tty_server", Priority::High, true, 0));
    let _ = configs.push(cfg("sshl", Priority::High, true, 0));
    let _ = configs.push(cfg("net_server", Priority::High, true, 0));
    let _ = configs.push(cfg("sunlight-mouse", Priority::Critical, true, 0));
    let _ = configs.push(cfg("display_server", Priority::Critical, true, 0));
    let _ = configs.push(cfg("vfs_server", Priority::Critical, true, 0));
    let _ = configs.push(cfg("sunlight-niced", Priority::High, true, 0));

    let mut dyn_rules: Vec<DynamicRule, MAX_DYN_RULES> = Vec::new();
    let _ = dyn_rules.push(DynamicRule {
        cpu_window_secs: 10,
        cpu_threshold_pct: 85,
        penalty_step: 2,
        cooldown_secs: 30,
        exempt_priority: Some(Priority::Critical),
    });

    let mut recovery_rules: Vec<RecoveryRule, MAX_RECOVERY_RULES> = Vec::new();
    let _ = recovery_rules.push(RecoveryRule {
        check_interval_secs: 15,
        nice_step: 1,
        cpu_threshold_pct: 30,
    });

    (configs, dyn_rules, recovery_rules)
}
