//! M5.2 control-plane activation only. No shared cursor is kernel authority.
use super::{
    channel::{Error, Handle, State},
    delivery::RecipientIdentity,
    CAP_BROKER,
};
use crate::sched::Scheduler;
impl Scheduler {
    pub(crate) fn fastlane_shared_ring(&mut self, h: Handle, role: u64) -> Result<(), Error> {
        self.fastlane_mapping_sweep();
        let who = RecipientIdentity::of(self.current_process());
        if role > 1 || !self.channel_vm_exclusive(who) {
            return Err(Error::Denied);
        }
        let mut caps = CAP_BROKER.lock();
        let row = caps.channels.row_mut(h)?;
        if (if role == 0 {
            Some(row.creator)
        } else {
            row.consumer
        }) != Some(who)
        {
            return Err(Error::Denied);
        }
        if !row.live() {
            return Err(Error::Terminal);
        }
        if row.state != State::Attached || !row.mapped[role as usize].iter().all(|v| *v) {
            return Err(Error::Incomplete);
        }
        if row.transfer != 0 || row.repeated.enabled {
            return Err(Error::Denied);
        }
        if row.native_ring[role as usize] {
            return Err(Error::Duplicate);
        }
        row.native_ring[role as usize] = true;
        crate::serial_println!("[FL-NATIVE-RING] event=activate generation={} role={} pid={} inc={} vm={} slots=16 payload=256", h.generation,role,who.pid,who.incarnation,who.vm_generation);
        Ok(())
    }
}

use core::sync::atomic::{AtomicU64, Ordering};
static SYSCALLS: AtomicU64 = AtomicU64::new(0);
static WORD_CALLS: AtomicU64 = AtomicU64::new(0);
static NOTIFICATIONS: AtomicU64 = AtomicU64::new(0);
pub(crate) fn syscall(number: u64, operation: u64) {
    SYSCALLS.fetch_add(1, Ordering::Relaxed);
    if number == 0xf1c1 {
        if matches!(operation, 18 | 19) {
            WORD_CALLS.fetch_add(1, Ordering::Relaxed);
        }
        if operation == 10 {
            NOTIFICATIONS.fetch_add(1, Ordering::Relaxed);
        }
    }
}
impl Scheduler {
    pub(crate) fn fastlane_shared_diagnostics(&self) {
        let switches = self.fastlane_context_switches();
        crate::serial_println!("[FL-M52-KDIAG] scope=system syscalls={} word_calls={} notification_calls={} context_switches={}",
            SYSCALLS.load(Ordering::Relaxed), WORD_CALLS.load(Ordering::Relaxed), NOTIFICATIONS.load(Ordering::Relaxed), switches);
    }
}

// Process-private counters are updated only under the existing SCHEDULER lock.
// No measurement lock or global writer is introduced. Bounds are per process
// incarnation, and this entire type is absent from production process layouts.
#[cfg(all(debug_assertions, feature = "fastlane_measurement_test"))]
#[derive(Clone, Copy)]
pub struct ActorCounters {
    enabled: bool,
    values: [u64; 4], // syscalls, notifications, task-change switch-ins, wake publications
    overflow: bool,
    generation: u64,
    runtime_start: u64,
    start_ms: u64,
}
#[cfg(all(debug_assertions, feature = "fastlane_measurement_test"))]
impl ActorCounters {
    pub const fn new() -> Self {
        Self { enabled: false, values: [0; 4], overflow: false, generation: 0, runtime_start: 0, start_ms: 0 }
    }
    fn increment(&mut self, index: usize) {
        if !self.enabled { return; }
        match self.values[index].checked_add(1) {
            Some(v) => self.values[index] = v,
            None => self.overflow = true,
        }
    }
    pub fn syscall(&mut self, number: u64, op: u64) {
        self.increment(0);
        if number == 0xf1c1 && op == 10 { self.increment(1); }
    }
    pub fn dispatch(&mut self) { self.increment(2); }
    pub fn wakeup(&mut self) { self.increment(3); }
}
#[cfg(all(debug_assertions, feature = "fastlane_measurement_test"))]
impl Scheduler {
    pub(crate) fn fastlane_measurement(&mut self, h: Handle, start: bool) -> Result<(), Error> {
        let who = RecipientIdentity::of(self.current_process());
        // The same identity, live state and role activation are required. This
        // diagnostic selector grants no mapping or transfer authority.
        let mut caps = CAP_BROKER.lock();
        let row = caps.channels.row_mut(h)?;
        let role = if row.creator == who { 0 } else if row.consumer == Some(who) { 1 } else { return Err(Error::Denied); };
        if !row.live() { return Err(Error::Terminal); }
        if !row.native_ring[role] { return Err(Error::Incomplete); }
        drop(caps);
        let idx = self.current_process_index().ok_or(Error::Denied)?;
        let now = self.now_ns();
        let runtime = self.effective_runtime_at(idx, now);
        let ms = crate::timekeeping::monotonic_ms();
        let p = &mut self.processes[idx];
        if start {
            if p.measurement.enabled { return Err(Error::Duplicate); }
            p.measurement = ActorCounters { enabled: true, generation: h.generation,
                runtime_start: runtime, start_ms: ms, ..ActorCounters::new() };
        } else {
            let m = &mut p.measurement;
            if !m.enabled || m.generation != h.generation { return Err(Error::Incomplete); }
            m.enabled = false;
            let elapsed_runtime = runtime.checked_sub(m.runtime_start);
            crate::serial_println!("[FL-M53-KACTOR] generation={} pid={} inc={} vm={} syscalls={} notifications={} context_switch_ins={} wake_publications={} overflow={} dropped=0 start_ms={} end_ms={} scheduler_runtime_ns={} runtime_delta_valid={} cpu={}", h.generation,who.pid,who.incarnation,who.vm_generation,m.values[0],m.values[1],m.values[2],m.values[3],u8::from(m.overflow),m.start_ms,ms,elapsed_runtime.unwrap_or(0),u8::from(elapsed_runtime.is_some()),crate::sched::current_cpu_id());
        }
        Ok(())
    }
}
