//! Process supervisor - tracks service lifecycle and handles restarts

use crate::unit::{RestartPolicy, ServiceType, ServiceUnit};

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
    Failed {
        exit_code: i32,
        crashed_at: u64,
        restarts: u32,
    },
    Restarting {
        at: u64,
    },
}

pub struct ServiceEntry {
    pub unit: ServiceUnit,
    pub state: ServiceState,
    pub restart_count: u32,
    pub last_restart_time: u64,
    pub enabled: bool,
    pub stop_requested: bool,
    pub restart_after_stop: bool,
    pub last_status_detail: u32,
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

    pub fn mark_starting(&mut self, pid: u32, started_at: u64) {
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.last_status_detail = 0;
        self.state = ServiceState::Starting {
            pid,
            started_at,
            needs_ready: self.unit.service_type == ServiceType::Notify,
        };
    }

    pub fn mark_running(&mut self, pid: u32, started_at: u64) {
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.last_status_detail = 0;
        self.state = ServiceState::Running { pid, started_at };
    }

    pub fn mark_failed(&mut self, exit_code: i32, crashed_at: u64) {
        self.stop_requested = false;
        self.restart_after_stop = false;
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
        self.state = ServiceState::Restarting { at };
    }

    pub fn mark_stopped(&mut self) {
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.last_status_detail = 0;
        self.state = ServiceState::Stopped;
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
