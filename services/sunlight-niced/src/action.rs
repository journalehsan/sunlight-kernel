//! Kill ladder / restart orchestration.
//!
//! HONEST STATUS: actual process termination does NOT work yet. The kernel
//! has no `send_signal`, `force_terminate`, or `reap` syscalls — those go
//! through `KernelOps` shims that log a `TODO: kernel syscall — ...` message
//! and return `false`. What IS implemented here is the orchestration,
//! timing, and logging: the sequence SIGTERM -> wait -> SIGKILL -> wait ->
//! force_terminate -> notify gcd, plus an attempt to use
//! `SunlightdOp::Restart` for supervised services. When the kernel gains
//! real signal/terminate/reap support, this orchestration will start having
//! real effects without further changes here.
//!
//! Because niced runs as a 1Hz tick loop (no blocking sleeps), the "wait"
//! periods described in the spec (KILL_GRACE=5s, then 2s) are NOT
//! implemented as inline blocking waits. Instead, `kill_ladder` performs ONE
//! step per call and is intended to be re-invoked on subsequent ticks if the
//! process is still alive — i.e. it is a simple state machine driven by the
//! watchdog tick, not a single synchronous call chain. For this pass,
//! `trigger_restart_or_kill` performs the SIGTERM step immediately (logged,
//! shimmed) and, if the unit is supervised by sunlightd, additionally
//! requests `SunlightdOp::Restart`. Escalation to SIGKILL/force_terminate on
//! subsequent ticks is left as a documented follow-up (the `unresp_escalation`
//! / `not_responding` counters in `MonitoredProcess` already provide the
//! "still alive after N ticks" signal needed to drive that escalation).

use sunlight_ipc::{ipc_call, nameserver_lookup, process_is_alive, IpcMsg};

use crate::ipc::GcdOp;
use crate::shim::KernelOps;

/// sunlightd's control opcode for Restart (mirrors
/// services/sunlightd/src/ipc.rs `SunlightdOp::Restart = 3`). Hardcoded here
/// to avoid a cross-crate dependency on sunlightd.
const SUNLIGHTD_OP_RESTART: u64 = 3;

const SIGTERM: u8 = 15;
const SIGKILL: u8 = 9;

/// Entry point called from the watchdog tick when a process has been judged
/// unresponsive for too long (busy-hang escalation >= 3, or stuck-hang grace
/// period exceeded).
///
/// `supervised`: cached result of a `SunlightdOp::Status` lookup for this
/// process's unit name (`None` = not yet determined — treated as
/// unsupervised for this call).
pub fn trigger_restart_or_kill<K: KernelOps>(
    unit_name: &str,
    pid: usize,
    supervised: Option<bool>,
    shim: &K,
) {
    let is_supervised = supervised.unwrap_or(false);

    if is_supervised {
        if let Some(cap) = nameserver_lookup("sunlightd") {
            let mut msg = IpcMsg::with_label(SUNLIGHTD_OP_RESTART);
            // Pack unit name into words[0..4], matching
            // services/sunlightd/src/ipc.rs::pack_unit_name.
            let bytes = unit_name.as_bytes();
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
            let reply = ipc_call(cap, msg);
            crate::serial_println!(
                "[NICED] NOTRESP pid={} '{}' action=restart via sunlightd (reply.label={})",
                pid,
                unit_name,
                reply.label
            );
            return;
        }
    }

    // Unsupervised (or sunlightd unavailable): start the kill ladder at
    // SIGTERM.
    kill_ladder(shim, pid, false, unit_name);
}

/// One step of the kill ladder: SIGTERM -> (next tick) SIGKILL -> (next
/// tick) force_terminate -> notify gcd. See module-level doc comment for the
/// honest status of this orchestration.
pub fn kill_ladder<K: KernelOps>(shim: &K, pid: usize, supervised: bool, unit_name: &str) {
    if !process_is_alive(pid as u64) {
        // Already gone — notify gcd and stop.
        notify_gcd_exited(pid);
        return;
    }

    crate::serial_println!(
        "[NICED] KILL pid={} '{}' supervised={} step=SIGTERM",
        pid,
        unit_name,
        supervised
    );
    if shim.send_signal(pid, SIGTERM) {
        return;
    }

    // SIGTERM shim returned false (always, today) — escalate immediately to
    // SIGKILL in the same tick since real delivery never happens yet. Once
    // send_signal is real, this should instead wait ~KILL_GRACE seconds
    // (driven by re-invocation on subsequent ticks) before escalating.
    crate::serial_println!("[NICED] KILL pid={} '{}' step=SIGKILL", pid, unit_name);
    if shim.send_signal(pid, SIGKILL) {
        return;
    }

    crate::serial_println!(
        "[NICED] KILL pid={} '{}' step=force_terminate",
        pid,
        unit_name
    );
    shim.force_terminate(pid);

    notify_gcd_exited(pid);
}

fn notify_gcd_exited(pid: usize) {
    if let Some(cap) = nameserver_lookup("gcd") {
        let _ = ipc_call(
            cap,
            IpcMsg::with_label(GcdOp::PROCESS_EXITED).word(0, pid as u64),
        );
    }
}
