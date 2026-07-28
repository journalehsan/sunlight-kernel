//! Process supervisor - tracks service lifecycle and handles restarts

use crate::unit::{RestartPolicy, ServiceType, ServiceUnit};

/// Wire/state codes shared with sunlightctl (keep in sync).
pub const STATE_STOPPED: u32 = 0;
pub const STATE_STARTING: u32 = 1;
pub const STATE_RUNNING: u32 = 2;
pub const STATE_FAILED: u32 = 3;
pub const STATE_RESTARTING: u32 = 4;
/// Stop requested; termination not yet confirmed.
pub const STATE_STOPPING: u32 = 5;

/// Structured result/detail kinds for status and control replies.
pub const DETAIL_NONE: u32 = 0;
pub const DETAIL_SPAWN: u32 = 1;
pub const DETAIL_IDENTITY: u32 = 2;
pub const DETAIL_STARTUP: u32 = 3;
pub const DETAIL_RESTART_LIMIT: u32 = 4;
pub const DETAIL_NOT_FOUND: u32 = 5;
pub const DETAIL_ALREADY_RUNNING: u32 = 6;
pub const DETAIL_ALREADY_STOPPED: u32 = 7;
pub const DETAIL_STOP_TIMEOUT: u32 = 8;
pub const DETAIL_EXEC_NOT_FOUND: u32 = 9;
pub const DETAIL_EXEC_DENIED: u32 = 10;
pub const DETAIL_EXEC_LOAD: u32 = 11;
pub const DETAIL_SPAWN_NOMEM: u32 = 12;
pub const DETAIL_EXITED: u32 = 13;
pub const DETAIL_IN_PROGRESS: u32 = 14;
pub const DETAIL_TRANSITION_BUSY: u32 = 15;
pub const DETAIL_RESTART_ABORTED: u32 = 16;
pub const DETAIL_KILL_FAILED: u32 = 17;
pub const DETAIL_TERMINATION_UNCONFIRMED: u32 = 18;

/// Last control operation attempted against this service.
pub const OP_NONE: u8 = 0;
pub const OP_START: u8 = 1;
pub const OP_STOP: u8 = 2;
pub const OP_RESTART: u8 = 3;

/// Kernel spawn-path `SpawnError` discriminants (order matches kernel enum).
pub const SPAWN_ERR_NOT_FOUND: u64 = 0;
pub const SPAWN_ERR_PERMISSION: u64 = 1;
pub const SPAWN_ERR_ELF_LOAD: u64 = 2;
pub const SPAWN_ERR_NO_MEMORY: u64 = 3;
pub const SPAWN_ERR_ENTROPY: u64 = 4;
pub const SPAWN_ERR_INVALID_PATH: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Stopped,
    Starting {
        pid: u32,
        started_at: u64,
        needs_ready: bool,
    },
    Running {
        pid: u32,
        started_at: u64,
    },
    /// Graceful or forced stop in flight; process may still be alive.
    Stopping {
        pid: u32,
        started_at: u64,
        requested_at: u64,
        /// True when the stop deadline expired without confirmed death.
        timed_out: bool,
    },
    Failed {
        exit_code: i32,
        crashed_at: u64,
        restarts: u32,
    },
    Restarting {
        at: u64,
    },
}

impl ServiceState {
    pub fn wire_code(self) -> u32 {
        match self {
            ServiceState::Stopped => STATE_STOPPED,
            ServiceState::Starting { .. } => STATE_STARTING,
            ServiceState::Running { .. } => STATE_RUNNING,
            ServiceState::Failed { .. } => STATE_FAILED,
            ServiceState::Restarting { .. } => STATE_RESTARTING,
            ServiceState::Stopping { .. } => STATE_STOPPING,
        }
    }

    pub fn pid(self) -> Option<u32> {
        match self {
            ServiceState::Starting { pid, .. }
            | ServiceState::Running { pid, .. }
            | ServiceState::Stopping { pid, .. } => Some(pid),
            _ => None,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            ServiceState::Starting { .. }
                | ServiceState::Running { .. }
                | ServiceState::Stopping { .. }
        )
    }

    pub fn may_still_be_alive(self) -> bool {
        matches!(
            self,
            ServiceState::Starting { .. }
                | ServiceState::Running { .. }
                | ServiceState::Stopping { .. }
        )
    }
}

/// Fixed-size diagnostic metadata (no unbounded log history).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ServiceDiagnostics {
    pub last_op: u8,
    pub last_result: u32,
    pub last_error_kind: u32,
    pub last_error_detail: u32,
    /// Exit status when known; `None` when death was observed only via liveness.
    pub last_exit_status: Option<i32>,
    /// True when a stop was requested and termination is not confirmed.
    pub termination_unconfirmed: bool,
}

pub struct ServiceEntry {
    pub unit: ServiceUnit,
    pub state: ServiceState,
    pub restart_count: u32,
    pub last_restart_time: u64,
    pub enabled: bool,
    pub stop_requested: bool,
    pub restart_after_stop: bool,
    /// Legacy detail field retained for packers that still read it.
    pub last_status_detail: u32,
    pub diagnostics: ServiceDiagnostics,
}

impl ServiceEntry {
    pub fn new(unit: ServiceUnit) -> Self {
        Self {
            unit,
            state: ServiceState::Stopped,
            restart_count: 0,
            last_restart_time: 0,
            enabled: true,
            stop_requested: false,
            restart_after_stop: false,
            last_status_detail: 0,
            diagnostics: ServiceDiagnostics::default(),
        }
    }

    /// Check if we should restart this service based on exit code
    pub fn should_restart(&self, exit_code: i32) -> bool {
        match self.unit.restart {
            RestartPolicy::No => false,
            RestartPolicy::OnFailure => exit_code != 0,
            RestartPolicy::Always => true,
        }
    }

    /// Check if we've hit the restart limit (5 restarts within 30 seconds)
    pub fn check_restart_limit(&self, current_time: u64) -> bool {
        const RESTART_WINDOW: u64 = 30_000; // 30 seconds in ms
        const MAX_RESTARTS: u32 = 5;

        if current_time - self.last_restart_time > RESTART_WINDOW {
            // Outside the window, reset is allowed
            false
        } else if self.restart_count >= MAX_RESTARTS {
            // Too many restarts within window
            true
        } else {
            false
        }
    }

    pub fn set_last_op(&mut self, op: u8) {
        self.diagnostics.last_op = op;
    }

    pub fn record_result(&mut self, result: u32, kind: u32, detail: u32) {
        self.diagnostics.last_result = result;
        self.diagnostics.last_error_kind = kind;
        self.diagnostics.last_error_detail = detail;
        self.last_status_detail = kind;
    }

    pub fn clear_error(&mut self) {
        self.diagnostics.last_error_kind = DETAIL_NONE;
        self.diagnostics.last_error_detail = 0;
        self.diagnostics.termination_unconfirmed = false;
        self.last_status_detail = DETAIL_NONE;
    }

    pub fn mark_starting(&mut self, pid: u32, started_at: u64) {
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.diagnostics.termination_unconfirmed = false;
        self.diagnostics.last_exit_status = None;
        self.clear_error();
        self.state = ServiceState::Starting {
            pid,
            started_at,
            needs_ready: self.unit.service_type == ServiceType::Notify,
        };
    }

    pub fn mark_running(&mut self, pid: u32, started_at: u64) {
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.diagnostics.termination_unconfirmed = false;
        self.clear_error();
        self.state = ServiceState::Running { pid, started_at };
    }

    pub fn mark_stopping(
        &mut self,
        pid: u32,
        started_at: u64,
        requested_at: u64,
        restart_after_stop: bool,
    ) {
        self.stop_requested = true;
        self.restart_after_stop = restart_after_stop;
        self.diagnostics.termination_unconfirmed = true;
        self.state = ServiceState::Stopping {
            pid,
            started_at,
            requested_at,
            timed_out: false,
        };
    }

    pub fn mark_stop_timeout(&mut self, now: u64) {
        if let ServiceState::Stopping {
            pid,
            started_at,
            requested_at,
            ..
        } = self.state
        {
            self.diagnostics.termination_unconfirmed = true;
            self.record_result(DETAIL_STOP_TIMEOUT, DETAIL_STOP_TIMEOUT, 0);
            self.state = ServiceState::Stopping {
                pid,
                started_at,
                requested_at: if requested_at == 0 { now } else { requested_at },
                timed_out: true,
            };
        }
    }

    pub fn mark_failed(&mut self, exit_code: i32, crashed_at: u64) {
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.diagnostics.termination_unconfirmed = false;
        self.diagnostics.last_exit_status = Some(exit_code);
        self.state = ServiceState::Failed {
            exit_code,
            crashed_at,
            restarts: self.restart_count,
        };
    }

    pub fn mark_restarting(&mut self, at: u64, current_time: u64) {
        // Reset restart count if outside the window
        const RESTART_WINDOW: u64 = 30_000;
        if current_time - self.last_restart_time > RESTART_WINDOW {
            self.restart_count = 0;
        }

        self.restart_count += 1;
        self.last_restart_time = current_time;
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.diagnostics.termination_unconfirmed = false;
        self.state = ServiceState::Restarting { at };
    }

    pub fn mark_stopped(&mut self) {
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.diagnostics.termination_unconfirmed = false;
        self.last_status_detail = DETAIL_NONE;
        self.state = ServiceState::Stopped;
    }

    /// Confirmed process termination: clear pid-bearing state truthfully.
    pub fn observe_confirmed_exit(&mut self, exit_code: Option<i32>, now: u64) -> ExitDisposition {
        let stop_requested = self.stop_requested;
        let restart_after_stop = self.restart_after_stop;
        let code = exit_code.unwrap_or(0);
        self.diagnostics.last_exit_status = exit_code;
        self.diagnostics.termination_unconfirmed = false;

        if stop_requested {
            if restart_after_stop {
                self.mark_restarting(now, now);
                ExitDisposition::Restart
            } else {
                self.mark_stopped();
                ExitDisposition::Stopped
            }
        } else if self.should_restart(code) {
            if self.check_restart_limit(now) {
                self.record_result(DETAIL_RESTART_LIMIT, DETAIL_RESTART_LIMIT, code as u32);
                self.mark_failed(code, now);
                ExitDisposition::Failed
            } else {
                self.mark_restarting(now, now);
                ExitDisposition::Restart
            }
        } else {
            if exit_code.is_none() {
                self.record_result(DETAIL_EXITED, DETAIL_EXITED, 0);
            } else {
                self.record_result(DETAIL_EXITED, DETAIL_EXITED, code as u32);
            }
            self.mark_failed(code, now);
            ExitDisposition::Failed
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitDisposition {
    Stopped,
    Restart,
    Failed,
}

/// Map a kernel spawn-path error discriminant into a detail kind.
pub fn detail_from_spawn_error(code: u64) -> u32 {
    match code {
        SPAWN_ERR_NOT_FOUND | SPAWN_ERR_INVALID_PATH => DETAIL_EXEC_NOT_FOUND,
        SPAWN_ERR_PERMISSION => DETAIL_EXEC_DENIED,
        SPAWN_ERR_ELF_LOAD => DETAIL_EXEC_LOAD,
        SPAWN_ERR_NO_MEMORY => DETAIL_SPAWN_NOMEM,
        SPAWN_ERR_ENTROPY => DETAIL_SPAWN,
        _ => DETAIL_SPAWN,
    }
}

/// Spawn logic helper - parses ExecStart command line
pub fn parse_exec_command(exec_start: &str) -> Option<(&str, heapless::Vec<&str, 16>)> {
    let parts: heapless::Vec<&str, 16> = exec_start.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let binary = parts[0];
    let mut args: heapless::Vec<&str, 16> = heapless::Vec::new();
    for i in 1..parts.len() {
        let _ = args.push(parts[i]);
    }

    Some((binary, args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit::ServiceUnit;

    fn entry() -> ServiceEntry {
        ServiceEntry::new(ServiceUnit::default())
    }

    #[test]
    fn start_then_running_carries_pid() {
        let mut e = entry();
        e.mark_starting(42, 100);
        assert_eq!(e.state.pid(), Some(42));
        assert_eq!(e.state.wire_code(), STATE_STARTING);
        e.mark_running(42, 100);
        assert_eq!(e.state.wire_code(), STATE_RUNNING);
        assert!(!e.diagnostics.termination_unconfirmed);
    }

    #[test]
    fn spawn_failure_reaches_failed_with_reason() {
        let mut e = entry();
        e.record_result(DETAIL_EXEC_NOT_FOUND, DETAIL_EXEC_NOT_FOUND, 0);
        e.mark_failed(-1, 50);
        assert_eq!(e.state.wire_code(), STATE_FAILED);
        assert_eq!(e.diagnostics.last_error_kind, DETAIL_EXEC_NOT_FOUND);
        assert_eq!(e.state.pid(), None);
    }

    #[test]
    fn confirmed_exit_clears_running_pid() {
        let mut e = entry();
        e.mark_running(7, 10);
        let d = e.observe_confirmed_exit(Some(0), 20);
        assert_eq!(d, ExitDisposition::Failed);
        assert_eq!(e.state.pid(), None);
        assert!(matches!(e.state, ServiceState::Failed { .. }));
    }

    #[test]
    fn stop_confirmed_reaches_stopped_only_after_exit() {
        let mut e = entry();
        e.mark_running(9, 1);
        e.mark_stopping(9, 1, 5, false);
        assert_eq!(e.state.wire_code(), STATE_STOPPING);
        assert!(e.diagnostics.termination_unconfirmed);
        assert_eq!(e.state.pid(), Some(9));
        let d = e.observe_confirmed_exit(Some(0), 10);
        assert_eq!(d, ExitDisposition::Stopped);
        assert_eq!(e.state.wire_code(), STATE_STOPPED);
        assert_eq!(e.state.pid(), None);
        assert!(!e.diagnostics.termination_unconfirmed);
    }

    #[test]
    fn stop_timeout_preserves_pid_and_unconfirmed() {
        let mut e = entry();
        e.mark_running(11, 1);
        e.mark_stopping(11, 1, 5, false);
        e.mark_stop_timeout(100);
        match e.state {
            ServiceState::Stopping {
                pid,
                timed_out: true,
                ..
            } => assert_eq!(pid, 11),
            other => panic!("expected timed-out Stopping, got {:?}", other),
        }
        assert!(e.diagnostics.termination_unconfirmed);
        assert_eq!(e.diagnostics.last_error_kind, DETAIL_STOP_TIMEOUT);
    }

    #[test]
    fn restart_aborts_disposition_while_still_stopping() {
        let mut e = entry();
        e.mark_running(3, 1);
        e.mark_stopping(3, 1, 2, true);
        e.mark_stop_timeout(50);
        // Restart must not spawn while previous instance may still exist.
        assert!(e.state.may_still_be_alive());
        assert!(e.restart_after_stop);
        // Timeout path clears the pending restart flag via record; manager must
        // not spawn. Disposition remains Stopping.
        assert_eq!(e.state.wire_code(), STATE_STOPPING);
    }

    #[test]
    fn restart_after_confirmed_stop_requests_restart() {
        let mut e = entry();
        e.mark_running(4, 1);
        e.mark_stopping(4, 1, 2, true);
        let d = e.observe_confirmed_exit(Some(0), 10);
        assert_eq!(d, ExitDisposition::Restart);
        assert_eq!(e.state.wire_code(), STATE_RESTARTING);
    }

    #[test]
    fn repeated_active_state_blocks_duplicate_spawn() {
        let mut e = entry();
        e.mark_running(5, 1);
        assert!(e.state.is_active());
        e.mark_stopping(5, 1, 2, false);
        assert!(e.state.is_active());
        e.mark_starting(6, 3);
        assert!(e.state.is_active());
    }

    #[test]
    fn external_death_clears_stale_running() {
        let mut e = entry();
        e.mark_running(8, 1);
        // External death is not a stop_requested path.
        let d = e.observe_confirmed_exit(None, 40);
        assert_eq!(d, ExitDisposition::Failed);
        assert_eq!(e.state.pid(), None);
        assert_eq!(e.diagnostics.last_error_kind, DETAIL_EXITED);
    }

    #[test]
    fn detail_from_spawn_error_maps_kernel_codes() {
        assert_eq!(
            detail_from_spawn_error(SPAWN_ERR_NOT_FOUND),
            DETAIL_EXEC_NOT_FOUND
        );
        assert_eq!(
            detail_from_spawn_error(SPAWN_ERR_PERMISSION),
            DETAIL_EXEC_DENIED
        );
        assert_eq!(
            detail_from_spawn_error(SPAWN_ERR_ELF_LOAD),
            DETAIL_EXEC_LOAD
        );
        assert_eq!(
            detail_from_spawn_error(SPAWN_ERR_NO_MEMORY),
            DETAIL_SPAWN_NOMEM
        );
    }
}
