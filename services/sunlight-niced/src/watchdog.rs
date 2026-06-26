//! Watchdog (ping) protocol and per-tick responsiveness checks.
//!
//! There is no PROC_EXIT pub/sub and no `Finished` state subscription API in
//! the kernel; "responsiveness" is judged purely from the 1Hz telemetry poll
//! plus explicit `NicedOp::PING` heartbeats from supervised processes.

use sunlight_ipc::{ipc_call, nameserver_lookup, IpcMsg};

use crate::action;
use crate::ipc::{GcdOp, NicedOp};
use crate::monitor::Table;
use crate::shim::KernelOps;
use crate::telemetry::{ProcSample, Telemetry, MAX_PROCS as TEL_MAX_PROCS};

/// Number of consecutive missed pings before a process is judged
/// unresponsive.
const MISS_THRESHOLD: u32 = 3;

/// Grace period (seconds) a `BlockedOnIpc`/low-cpu process may remain
/// `not_responding` before the kill/restart ladder is triggered.
const STUCK_GRACE: u64 = 30;

/// Busy-hang CPU threshold (mirrors sunlight-top's 85% severity threshold).
const BUSY_HANG_PCT: u16 = 85;

/// Handle an incoming watchdog IPC message and produce a reply.
pub fn handle_message(table: &mut Table, msg: &IpcMsg, now: u64) -> IpcMsg {
    match msg.label {
        NicedOp::PING => {
            let pid = msg.words[0] as usize;
            if let Some(p) = table.find_mut(pid) {
                p.last_ping = now;
                p.pings_missed = 0;
                p.not_responding = false;
                p.not_responding_since = 0;
                if p.unresp_demotion_applied {
                    // Restore the nice value that was lowered due to an
                    // unresponsiveness-driven demotion (separate from the
                    // C.5 CPU-penalty machinery).
                    let shim = crate::shim::RealKernelOps;
                    shim.set_nice(p.pid, p.unresp_original_nice);
                    p.current_nice = p.unresp_original_nice;
                    p.unresp_demotion_applied = false;
                }
                p.unresp_escalation = 0;
            }
            IpcMsg::with_label(NicedOp::REPLY)
        }
        NicedOp::SET_WATCHDOG => {
            let pid = msg.words[0] as usize;
            let period = msg.words[1];
            if let Some(p) = table.find_mut(pid) {
                p.config.wd_period = period;
                p.last_ping = now;
                p.pings_missed = 0;
                IpcMsg::with_label(NicedOp::REPLY)
            } else {
                IpcMsg::with_label(NicedOp::ERROR)
            }
        }
        NicedOp::CLEAR_WD => {
            let pid = msg.words[0] as usize;
            if let Some(p) = table.find_mut(pid) {
                p.config.wd_period = 0;
                p.not_responding = false;
                IpcMsg::with_label(NicedOp::REPLY)
            } else {
                IpcMsg::with_label(NicedOp::ERROR)
            }
        }
        NicedOp::SET_NICE => {
            let pid = msg.words[0] as usize;
            let nice = msg.words[1] as i64 as i8;
            let shim = crate::shim::RealKernelOps;
            if shim.set_nice(pid, nice) {
                if let Some(p) = table.find_mut(pid) {
                    p.current_nice = nice;
                }
                IpcMsg::with_label(NicedOp::REPLY)
            } else {
                IpcMsg::with_label(NicedOp::ERROR)
            }
        }
        NicedOp::QUERY => {
            let pid = msg.words[0] as usize;
            if let Some(p) = table.find(pid) {
                IpcMsg::with_label(NicedOp::REPLY)
                    .word(0, p.current_nice as i64 as u64)
                    .word(1, 0)
                    .word(2, p.not_responding as u64)
            } else {
                IpcMsg::with_label(NicedOp::ERROR)
            }
        }
        NicedOp::TIGHTEN => {
            // gcd asks niced to tighten penalties under memory pressure.
            // Full "tighten" logic (e.g. lowering thresholds) is out of
            // scope for this pass; we just log the request.
            crate::serial_println!("[NICED] received TIGHTEN request from gcd");
            IpcMsg::with_label(NicedOp::REPLY)
        }
        _ => IpcMsg::with_label(NicedOp::ERROR),
    }
}

/// Per-tick responsiveness check (the decision table from the task spec),
/// for processes with `wd_period > 0` (watchdog opt-in).
pub fn tick<K: KernelOps>(table: &mut Table, tel: &mut Telemetry, shim: &K, now: u64) {
    let mut samples = [ProcSample::default(); TEL_MAX_PROCS];
    let count = tel.snapshot(&mut samples);

    for p in table.iter_mut() {
        if p.config.wd_period == 0 {
            continue;
        }

        if now.saturating_sub(p.last_ping) >= p.config.wd_period {
            p.pings_missed += 1;
        }

        if p.pings_missed < MISS_THRESHOLD {
            continue;
        }

        // Find this process's current telemetry sample (state + cpu_pct).
        let mut state = 0u8;
        let mut cpu_pct = 0u16;
        for i in 0..count {
            if samples[i].pid == p.pid {
                state = samples[i].state;
                cpu_pct = samples[i].cpu_pct;
                break;
            }
        }

        match state {
            3 => {
                // Finished (zombie) — ask gcd to reap.
                crate::serial_println!(
                    "[NICED] NOTRESP pid={} '{}' state={} action=reap",
                    p.pid,
                    p.name.as_str(),
                    state
                );
                if let Some(gcd_cap) = nameserver_lookup("gcd") {
                    let _ = ipc_call(
                        gcd_cap,
                        IpcMsg::with_label(GcdOp::REAP_ZOMBIE).word(0, p.pid as u64),
                    );
                }
            }
            1 if cpu_pct >= BUSY_HANG_PCT => {
                // Running + busy-hang. Don't double-demote if the C.5
                // CPU-penalty machinery is already active for this pid —
                // just escalate the unresponsiveness counter.
                if !p.penalty_active && !p.unresp_demotion_applied {
                    let target = crate::monitor::demote(p.current_nice, 2);
                    if target > p.current_nice {
                        shim.set_nice(p.pid, target);
                        p.unresp_original_nice = p.current_nice;
                        p.current_nice = target;
                        p.unresp_demotion_applied = true;
                    }
                }
                p.unresp_escalation += 1;
                crate::serial_println!(
                    "[NICED] NOTRESP pid={} '{}' state={} action=demote escalation={}",
                    p.pid,
                    p.name.as_str(),
                    state,
                    p.unresp_escalation
                );
                if p.unresp_escalation >= 3 {
                    action::trigger_restart_or_kill(table_entry_name(p), p.pid, p.supervised, shim);
                }
            }
            _ => {
                // BlockedOnIpc, or low-cpu Ready/Running — flag only.
                if !p.not_responding {
                    p.not_responding = true;
                    p.not_responding_since = now;
                    crate::serial_println!(
                        "[NICED] NOTRESP pid={} '{}' state={} action=flag",
                        p.pid,
                        p.name.as_str(),
                        state
                    );
                } else if now.saturating_sub(p.not_responding_since) >= STUCK_GRACE {
                    crate::serial_println!(
                        "[NICED] NOTRESP pid={} '{}' state={} action=restart",
                        p.pid,
                        p.name.as_str(),
                        state
                    );
                    action::trigger_restart_or_kill(table_entry_name(p), p.pid, p.supervised, shim);
                }
            }
        }
    }
}

/// Small helper to avoid borrow-checker friction when passing the process
/// name into `action::trigger_restart_or_kill`.
fn table_entry_name(p: &crate::monitor::MonitoredProcess) -> &str {
    p.name.as_str()
}
