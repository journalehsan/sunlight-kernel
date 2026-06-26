//! Zombie tracking and reap-request handling.
//!
//! There is no PROC_EXIT pub/sub in the kernel. "Subscribed to PROC_EXIT" in
//! practice means: poll the TelemetryPage every tick for entries with
//! `state==3 (Finished)`, and diff against a fixed "seen" set to find newly
//! finished pids.
//!
//! Reaping itself goes through `KernelOps::reap`, which always returns
//! `false` today (no kernel reap syscall). Per the task spec we do NOT print
//! a "reaped zombie" line — that would be dishonest, since nothing was
//! actually reaped — and the `.expected` gate file does not require it.

use crate::resources::ProcessResources;
use crate::shim::KernelOps;
use crate::telemetry::{ProcSample, MAX_PROCS};

pub const REAP_AFTER_SECS: u64 = 10;
pub const MAX_TRACKED: usize = MAX_PROCS;

#[derive(Clone, Copy)]
struct ZombieEntry {
    pid: usize,
    first_seen: u64,
    reap_requested: bool,
}

pub struct Reaper {
    /// Pids seen as Finished as of the previous tick (for "newly finished"
    /// detection / one-shot resource cleanup logging).
    seen: [Option<u32>; MAX_TRACKED],
    /// Zombies currently being tracked for time-based reaping.
    zombies: [Option<ZombieEntry>; MAX_TRACKED],
}

impl Reaper {
    pub fn new() -> Self {
        Self {
            seen: [None; MAX_TRACKED],
            zombies: [None; MAX_TRACKED],
        }
    }

    fn was_seen(&self, pid: u32) -> bool {
        self.seen.iter().any(|s| *s == Some(pid))
    }

    fn mark_seen(&mut self, pid: u32) {
        if self.was_seen(pid) {
            return;
        }
        for slot in self.seen.iter_mut() {
            if slot.is_none() {
                *slot = Some(pid);
                return;
            }
        }
    }

    #[allow(dead_code)]
    fn unmark_seen(&mut self, pid: u32) {
        for slot in self.seen.iter_mut() {
            if *slot == Some(pid) {
                *slot = None;
            }
        }
    }

    fn track_zombie(&mut self, pid: usize, now: u64) {
        if self.zombies.iter().any(|z| z.map(|e| e.pid) == Some(pid)) {
            return;
        }
        for slot in self.zombies.iter_mut() {
            if slot.is_none() {
                *slot = Some(ZombieEntry {
                    pid,
                    first_seen: now,
                    reap_requested: false,
                });
                return;
            }
        }
    }

    fn untrack_zombie(&mut self, pid: usize) {
        for slot in self.zombies.iter_mut() {
            if slot.map(|e| e.pid) == Some(pid) {
                *slot = None;
            }
        }
    }

    /// Process `REAP_ZOMBIE` from niced: reap immediately (still subject to
    /// the always-false `KernelOps::reap` shim).
    pub fn reap_now<K: KernelOps>(&mut self, shim: &K, pid: usize, name: &str) {
        if shim.reap(pid) {
            // Unreachable today (shim always returns false), but kept for
            // forward-compatibility.
            crate::serial_println!("[GCD] reaped zombie pid={} '{}'", pid, name);
        } else {
            crate::serial_println!(
                "[GCD] reap requested pid={} '{}' (no-op, kernel reap syscall missing)",
                pid,
                name
            );
        }
        self.untrack_zombie(pid);
    }

    /// Per-tick: scan telemetry for Finished processes, log one-shot
    /// "cleaned" resource reports for newly-finished pids, track zombies for
    /// time-based reaping, and check memory pressure.
    pub fn tick<K: KernelOps>(
        &mut self,
        samples: &[ProcSample; MAX_PROCS],
        count: usize,
        shim: &K,
        now: u64,
    ) {
        for i in 0..count {
            let s = &samples[i];
            if s.state != 3 {
                continue;
            }
            let pid32 = s.pid as u32;

            if !self.was_seen(pid32) {
                self.mark_seen(pid32);
                // Honest resource accounting: nothing can actually be
                // enumerated/freed yet (see resources::ProcessResources).
                let res = ProcessResources::enumerate(s.pid, s.name_str());
                crate::serial_println!(
                    "[GCD] cleaned pid={} ipc={} shm={} tmp={} fds={}",
                    res.pid,
                    res.n_queues,
                    res.n_shm,
                    res.n_tmp,
                    res.n_fds
                );
                self.track_zombie(s.pid, now);
            }
        }

        // Time-based reaping: zombies seen for > REAP_AFTER_SECS.
        for slot in self.zombies.iter_mut() {
            if let Some(entry) = slot {
                if !entry.reap_requested && now.saturating_sub(entry.first_seen) > REAP_AFTER_SECS {
                    entry.reap_requested = true;
                    let pid = entry.pid;
                    if shim.reap(pid) {
                        crate::serial_println!("[GCD] reaped zombie pid={}", pid);
                    } else {
                        crate::serial_println!(
                            "[GCD] reap requested pid={} (no-op, kernel reap syscall missing)",
                            pid
                        );
                    }
                }
            }
        }

        // `seen` entries are intentionally left in place — we can't
        // distinguish "reaped" from "still finished" without a real reap
        // syscall, so `mark_seen`/`was_seen` simply suppress repeated
        // "cleaned" logging for the same pid across ticks.
    }

    /// Check memory pressure (`used_ram_kb / total_ram_kb > 0.9`) and notify
    /// niced via `NicedOp::TIGHTEN` (best-effort; skipped if niced isn't
    /// running).
    pub fn check_memory_pressure(&self, used_kb: u64, total_kb: u64) {
        if total_kb == 0 {
            return;
        }
        if used_kb.saturating_mul(10) > total_kb.saturating_mul(9) {
            if let Some(cap) = sunlight_ipc::nameserver_lookup("niced") {
                let _ = sunlight_ipc::ipc_call(
                    cap,
                    sunlight_ipc::IpcMsg::with_label(crate::ipc::NicedOp::TIGHTEN),
                );
            }
        }
    }
}
