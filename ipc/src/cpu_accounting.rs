//! Integer-only CPU accounting shared by the kernel and telemetry readers.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuCounters {
    pub elapsed_ns: u64,
    pub busy_ns: u64,
    pub idle_ns: u64,
    pub switches: u64,
    pub schedules: u64,
    pub wakeups: u64,
    pub idle_entries: u64,
    pub idle_exits: u64,
    pub timer_irqs: u64,
    pub ipc_wakeups: u64,
}

impl CpuCounters {
    pub const ZERO: Self = Self {
        elapsed_ns: 0,
        busy_ns: 0,
        idle_ns: 0,
        switches: 0,
        schedules: 0,
        wakeups: 0,
        idle_entries: 0,
        idle_exits: 0,
        timer_irqs: 0,
        ipc_wakeups: 0,
    };
}

/// Preserve real sub-basis-point work as <0.1% rather than manufacturing zero.
pub fn usage_bp(runtime: u64, capacity: u64) -> u16 {
    if capacity == 0 || runtime == 0 {
        return 0;
    }
    ((runtime as u128 * 10_000 / capacity as u128).clamp(1, 10_000)) as u16
}

pub fn rate(delta: u64, elapsed_ns: u64) -> u64 {
    if elapsed_ns == 0 {
        return 0;
    }
    (delta as u128 * 1_000_000_000 / elapsed_ns as u128).min(u64::MAX as u128) as u64
}

/// Open execution intervals are independent of scheduler state (a task may
/// already be marked Blocked while returning from the blocking syscall).
pub fn runtime_at(committed: u64, since: Option<u64>, now: u64) -> u64 {
    committed.saturating_add(since.map(|start| now.saturating_sub(start)).unwrap_or(0))
}

pub fn checkpoint(committed: &mut u64, since: &mut Option<u64>, now: u64, stop: bool) {
    *committed = runtime_at(*committed, *since, now);
    *since = if stop { None } else { since.map(|_| now) };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrating_task_commits_once_per_cpu() {
        let mut total = 0;
        let mut since = Some(0);
        checkpoint(&mut total, &mut since, 40, true); // CPU 0 out
        assert_eq!(runtime_at(total, since, 100), 40);
        since = Some(100); // CPU 3 in
        assert_eq!(runtime_at(total, since, 120), 60);
        checkpoint(&mut total, &mut since, 130, true);
        checkpoint(&mut total, &mut since, 200, true);
        assert_eq!(total, 70);
    }
    #[test]
    fn blocked_syscall_tail_and_running_snapshot_are_counted() {
        let mut total = 0;
        let mut since = Some(10);
        checkpoint(&mut total, &mut since, 20, false); // marked blocked
        assert_eq!(runtime_at(total, since, 25), 15);
        checkpoint(&mut total, &mut since, 30, true); // actually descheduled
        assert_eq!(runtime_at(total, since, 1_000), 20);
    }
    #[test]
    fn long_uptime_saturates_without_wrapping() {
        assert_eq!(runtime_at(u64::MAX - 1, Some(0), 10), u64::MAX);
        assert_eq!(usage_bp(u64::MAX / 4, u64::MAX), 2499);
        assert_eq!(usage_bp(u64::MAX, u64::MAX), 10_000);
        assert_eq!(rate(u64::MAX, 1), u64::MAX);
        assert_eq!(runtime_at(50, Some(100), 90), 50);
    }
    #[test]
    fn tiny_real_work_is_not_rounded_to_zero() {
        assert_eq!(usage_bp(1, 4_000_000_000), 1);
        assert_eq!(usage_bp(0, 4_000_000_000), 0);
    }
}
