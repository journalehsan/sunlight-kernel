//! IPC control interface for sunlightd
//! Defines the control opcodes and message handling

use crate::supervisor::{ServiceDiagnostics, ServiceState};
use sunlight_ipc::IpcMsg;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunlightdOp {
    // Management
    Start = 1,
    Stop = 2,
    Restart = 3,
    Reload = 4,
    Enable = 5,
    Disable = 6,
    NotifyReady = 7,
    NotifyFailed = 8,
    // Query
    Status = 10,
    List = 11,
    // Logging
    GetLog = 20,
}

impl SunlightdOp {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            1 => Some(Self::Start),
            2 => Some(Self::Stop),
            3 => Some(Self::Restart),
            4 => Some(Self::Reload),
            5 => Some(Self::Enable),
            6 => Some(Self::Disable),
            7 => Some(Self::NotifyReady),
            8 => Some(Self::NotifyFailed),
            10 => Some(Self::Status),
            11 => Some(Self::List),
            20 => Some(Self::GetLog),
            _ => None,
        }
    }
}

/// Extract unit name from IPC message words[0..4] (up to 32 bytes).
pub fn extract_unit_name(msg: &IpcMsg) -> heapless::String<64> {
    let mut name = heapless::String::new();
    for i in 0..4 {
        let word = msg.words[i];
        for j in 0..8 {
            let byte = ((word >> (j * 8)) & 0xff) as u8;
            if byte == 0 {
                return name;
            }
            let _ = name.push(byte as char);
        }
    }
    name
}

/// Pack unit name into IPC message words[0..4] (up to 32 bytes).
pub fn pack_unit_name(msg: &mut IpcMsg, name: &str) {
    let bytes = name.as_bytes();
    for i in 0..4 {
        let mut word: u64 = 0;
        for j in 0..8 {
            let idx = i * 8 + j;
            if idx < bytes.len() {
                word |= (bytes[idx] as u64) << (j * 8);
            }
        }
        msg.words[i] = word;
    }
}

/// Status reply packed into the 4 register-transmitted words only.
///
/// Register IPC silently drops words[4..]; all fields must fit in words[0..3].
///
/// words[0] =
///   state(u8)
///   | detail_kind(u8) << 8
///   | restarts(u16) << 16
///   | enabled(u1) << 32
///   | stop_unconfirmed(u1) << 33
///   | last_op(u8) << 40
///   | timed_out(u1) << 48
/// words[1] = pid
/// words[2] = started_at / transition timestamp (monotonic ms)
/// words[3] = detail_value (exit code, secondary code, etc.)
#[derive(Debug, Clone, Copy)]
pub struct StatusReply {
    pub state: u32,
    pub pid: u32,
    pub restarts: u32,
    pub started_at: u64,
    pub enabled: bool,
    pub detail_kind: u32,
    pub detail_value: u32,
    pub last_op: u8,
    pub termination_unconfirmed: bool,
    pub stop_timed_out: bool,
}

impl StatusReply {
    pub fn from_entry(
        state: ServiceState,
        enabled: bool,
        restart_count: u32,
        diagnostics: &ServiceDiagnostics,
        last_status_detail: u32,
    ) -> Self {
        let (pid, started_at, stop_timed_out) = match state {
            ServiceState::Stopped => (0, 0, false),
            ServiceState::Starting {
                pid, started_at, ..
            } => (pid, started_at, false),
            ServiceState::Running { pid, started_at } => (pid, started_at, false),
            ServiceState::Stopping {
                pid,
                started_at,
                timed_out,
                ..
            } => (pid, started_at, timed_out),
            ServiceState::Failed {
                exit_code,
                crashed_at,
                restarts,
            } => {
                let detail_kind = if diagnostics.last_error_kind != 0 {
                    diagnostics.last_error_kind
                } else if last_status_detail != 0 {
                    last_status_detail
                } else {
                    exit_code as u32
                };
                return Self {
                    state: state.wire_code(),
                    pid: 0,
                    restarts,
                    started_at: crashed_at,
                    enabled,
                    detail_kind,
                    detail_value: diagnostics
                        .last_exit_status
                        .map(|c| c as u32)
                        .unwrap_or(exit_code as u32),
                    last_op: diagnostics.last_op,
                    termination_unconfirmed: false,
                    stop_timed_out: false,
                };
            }
            ServiceState::Restarting { at } => (0, at, false),
        };

        let detail_kind = if diagnostics.last_error_kind != 0 {
            diagnostics.last_error_kind
        } else {
            last_status_detail
        };
        let detail_value = diagnostics
            .last_exit_status
            .map(|c| c as u32)
            .unwrap_or(diagnostics.last_error_detail);

        Self {
            state: state.wire_code(),
            pid,
            restarts: restart_count,
            started_at,
            enabled,
            detail_kind,
            detail_value,
            last_op: diagnostics.last_op,
            termination_unconfirmed: diagnostics.termination_unconfirmed
                || matches!(state, ServiceState::Stopping { .. }),
            stop_timed_out,
        }
    }

    pub fn pack(&self, msg: &mut IpcMsg) {
        let mut w0 = (self.state as u64) & 0xff;
        w0 |= ((self.detail_kind as u64) & 0xff) << 8;
        w0 |= ((self.restarts as u64) & 0xffff) << 16;
        w0 |= (self.enabled as u64) << 32;
        w0 |= (self.termination_unconfirmed as u64) << 33;
        w0 |= (self.last_op as u64) << 40;
        w0 |= (self.stop_timed_out as u64) << 48;
        msg.words[0] = w0;
        msg.words[1] = self.pid as u64;
        msg.words[2] = self.started_at;
        msg.words[3] = self.detail_value as u64;
        msg.word_count = 4;
    }
}

/// Control-operation result packed into words[0..3] (register-safe).
///
/// words[0] = result_kind (DETAIL_*)
/// words[1] = pid (when relevant)
/// words[2] = detail_value
/// words[3] = flags (bit0 = termination_unconfirmed)
#[derive(Debug, Clone, Copy)]
pub struct ControlReply {
    pub kind: u32,
    pub pid: u32,
    pub detail: u32,
    pub termination_unconfirmed: bool,
}

impl ControlReply {
    pub fn pack(&self, msg: &mut IpcMsg) {
        msg.words[0] = self.kind as u64;
        msg.words[1] = self.pid as u64;
        msg.words[2] = self.detail as u64;
        msg.words[3] = self.termination_unconfirmed as u64;
        msg.word_count = 4;
    }
}

/// List entry packed into words[0..4] (transport-safe: IPC carries words[0..4] only).
///
/// words[0] = total(u32) | state(u8)<<32 | enabled(u1)<<40 | restarts(u8)<<48
/// words[1] = pid(u32)
/// words[2] = name bytes  0..8  little-endian
/// words[3] = name bytes  8..16 little-endian
#[derive(Debug, Clone)]
pub struct ListEntry {
    pub name: heapless::String<64>,
    pub state: u32,
    pub pid: u32,
    pub restarts: u32,
    pub enabled: bool,
}

impl ListEntry {
    pub fn pack(&self, msg: &mut IpcMsg, total: usize) {
        msg.words[0] = (total as u64) & 0xFFFF_FFFF
            | ((self.state as u64 & 0xFF) << 32)
            | ((self.enabled as u64) << 40)
            | ((self.restarts as u64 & 0xFF) << 48);
        msg.words[1] = self.pid as u64;

        let bytes = self.name.as_bytes();
        let mut w2: u64 = 0;
        let mut w3: u64 = 0;
        for i in 0..8.min(bytes.len()) {
            w2 |= (bytes[i] as u64) << (i * 8);
        }
        for i in 8..16.min(bytes.len()) {
            w3 |= (bytes[i] as u64) << ((i - 8) * 8);
        }
        msg.words[2] = w2;
        msg.words[3] = w3;
        msg.word_count = 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::{
        ServiceDiagnostics, ServiceState, DETAIL_STOP_TIMEOUT, OP_STOP, STATE_STOPPING,
    };

    #[test]
    fn status_reply_fits_register_words_and_round_trips_fields() {
        let diag = ServiceDiagnostics {
            last_op: OP_STOP,
            last_result: DETAIL_STOP_TIMEOUT,
            last_error_kind: DETAIL_STOP_TIMEOUT,
            last_error_detail: 0,
            last_exit_status: None,
            termination_unconfirmed: true,
        };
        let status = StatusReply::from_entry(
            ServiceState::Stopping {
                pid: 99,
                started_at: 1000,
                requested_at: 2000,
                timed_out: true,
            },
            true,
            2,
            &diag,
            DETAIL_STOP_TIMEOUT,
        );
        let mut msg = IpcMsg::empty();
        status.pack(&mut msg);
        assert_eq!(msg.word_count, 4);
        assert_eq!((msg.words[0] & 0xff) as u32, STATE_STOPPING);
        assert_eq!(((msg.words[0] >> 8) & 0xff) as u32, DETAIL_STOP_TIMEOUT);
        assert_eq!(msg.words[1] as u32, 99);
        assert_eq!(msg.words[2], 1000);
        assert_eq!((msg.words[0] >> 33) & 1, 1);
        assert_eq!((msg.words[0] >> 48) & 1, 1);
    }
}
