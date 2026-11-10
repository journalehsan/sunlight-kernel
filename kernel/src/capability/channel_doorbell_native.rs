//! SCHEDULER -> CAP_BROKER. Fixed channel rows and one receipt per process.
use super::{
    channel::{Error, Handle},
    channel_doorbell::identity,
    delivery::RecipientIdentity,
    CAP_BROKER,
};
use crate::{
    process::ProcessState,
    sched::{request_park_after_sysret, Scheduler},
};

#[derive(Clone, Copy)]
pub(crate) struct Receipt {
    pub handle: Handle,
    pub who: RecipientIdentity,
    pub outcome: Option<Result<u64, Error>>,
}
impl Scheduler {
    fn bell_complete(
        &mut self,
        h: Handle,
        who: sunlight_fastlane_doorbell::Identity,
        outcome: Result<u64, Error>,
    ) {
        let Some(i) = self
            .processes
            .iter()
            .position(|p| !p.is_exiting() && identity(RecipientIdentity::of(p)) == who)
        else {
            return;
        };
        let current = RecipientIdentity::of(&self.processes[i]);
        let Some(r) = self.processes[i].fastlane_bell.as_mut() else {
            return;
        };
        if r.handle != h || r.who != current || r.outcome.is_some() {
            return;
        }
        r.outcome = Some(outcome);
        crate::serial_println!("[FL-BELL] event=complete generation={} consumer={} inc={} vm={} cpu={} blocked={} result={} sequence={}",
            h.generation, current.pid, current.incarnation, current.vm_generation,
            crate::sched::current_cpu_id(), u8::from(self.processes[i].state == ProcessState::BlockedOnIo),
            outcome.as_ref().err().map_or(0, |e| *e as u64), outcome.unwrap_or(0));
        self.wake_io_pid(who.pid as usize);
        #[cfg(all(debug_assertions, feature = "fastlane_doorbell_trace"))]
        self.trace_bell_process("wake-ready", i, self.processes[i].context_rsp,
            u64::from(self.processes[i].owning_core) | (u64::from(self.processes[i].queued_on_core) << 8));
    }

    pub(crate) fn fastlane_bell_sweep(&mut self) {
        let mut caps = CAP_BROKER.lock();
        caps.expire_sessions(self.global_tick);
        caps.channels.expire(self.global_tick);
        let mut wakes = [None; super::channel::LIMIT];
        for (i, row) in caps.channels.rows.iter_mut().enumerate() {
            row.retire_unavailable(|who, issuer| self.channel_participant_terminal(who, issuer));
            wakes[i] = row.bell_pending.take().map(|who| (row.handle, who));
        }
        drop(caps);
        for (h, who) in wakes.into_iter().flatten() {
            self.bell_complete(h, who, Err(Error::Terminal));
        }
        for p in &mut self.processes {
            if p.is_exiting()
                || p.fastlane_bell
                    .is_some_and(|r| r.who != RecipientIdentity::of(p))
            {
                p.fastlane_bell = None;
            }
        }
    }

    /// Remove a registration on unrelated I/O or signal wake; never complete twice.
    pub(crate) fn fastlane_bell_spurious(&mut self, i: usize) {
        let Some(r) = self.processes[i].fastlane_bell else {
            return;
        };
        if r.outcome.is_some() {
            return;
        }
        let mut caps = CAP_BROKER.lock();
        let mut outcome = Err(Error::Cancelled);
        if let Ok(row) = caps.channels.row_mut(r.handle) {
            if !row.live() {
                outcome = Err(Error::Terminal);
            }
            if let Some(bell) = row.bell.as_mut() {
                if outcome == Err(Error::Cancelled) {
                    if let Err(e) =
                        bell.snapshot(super::channel_doorbell::key(r.handle), identity(r.who))
                    {
                        outcome = Err(super::channel_doorbell::error(e));
                    }
                }
                let _ = bell.cancel(super::channel_doorbell::key(r.handle), identity(r.who));
            }
            row.bell_pending = None;
        }
        self.processes[i].fastlane_bell.as_mut().unwrap().outcome = Some(outcome);
        crate::serial_println!("[FL-BELL] event=complete generation={} consumer={} inc={} vm={} cpu={} blocked={} result={} sequence=0",
            r.handle.generation, r.who.pid, r.who.incarnation, r.who.vm_generation, crate::sched::current_cpu_id(),
            u8::from(self.processes[i].state == ProcessState::BlockedOnIo), outcome.unwrap_err() as u64);
    }

    pub(crate) fn fastlane_bell(
        &mut self,
        op: u64,
        h: Handle,
        observed: u64,
    ) -> Result<u64, Error> {
        self.fastlane_bell_sweep();
        let who = RecipientIdentity::of(self.current_process());
        if !self.channel_vm_exclusive(who) {
            return Err(Error::Denied);
        }
        let i = self.process_index_by_pid(who.pid).ok_or(Error::Denied)?;
        if matches!(op, 15 | 16 | 17) {
            let mut caps = CAP_BROKER.lock();
            if op == 15 {
                return super::bell_race::arm(&caps, h, who, observed).map(|_| 0);
            }
            let row = caps.channels.row_mut(h)?;
            if row.creator != who {
                return Err(Error::Denied);
            }
            if op == 17 {
                if row.bell.as_ref().is_some_and(|b| b.has_waiter()) {
                    let consumer = row.consumer.ok_or(Error::Incomplete)?;
                    let index = self
                        .processes
                        .iter()
                        .position(|p| RecipientIdentity::of(p) == consumer)
                        .ok_or(Error::Terminal)?;
                    if !self.fastlane_bell_physically_parked(index) {
                        return Err(Error::Pending);
                    }
                    crate::serial_println!("[FL-BELL-PARK] generation={} consumer={} inc={} vm={} cpu={} parked=1 owner=255 queued=255 pins=0 ready=0",
                        h.generation, consumer.pid, consumer.incarnation, consumer.vm_generation, crate::sched::current_cpu_id());
                }
                let value = u64::from(row.bell.as_ref().is_some_and(|b| b.has_waiter()))
                    | ((self
                        .processes
                        .iter()
                        .filter(|p| p.fastlane_bell.is_some_and(|r| r.handle == h))
                        .count() as u64)
                        << 32);
                crate::serial_println!("[FL-BELL-RESOURCE] generation={} rows={} capacity={} active={} waiters={} receipts={} pending={} row_bytes={} receipt_bytes={}",
                    h.generation, caps.channels.rows.len(), caps.channels.rows.capacity(), caps.channels.live_count(),
                    caps.channels.rows.iter().filter(|r| r.bell.as_ref().is_some_and(|b| b.has_waiter())).count(),
                    self.processes.iter().filter(|p| p.fastlane_bell.is_some()).count(),
                    caps.channels.rows.iter().filter(|r| r.bell_pending.is_some()).count(),
                    core::mem::size_of::<super::channel::Channel>(), core::mem::size_of::<Receipt>());
                return Ok(value);
            }
            if !row.live() {
                return Err(Error::Terminal);
            }
            if row.state != super::channel::State::Attached || observed != u64::MAX - 1 {
                return Err(Error::Denied);
            }
            drop(caps);
            // Establish the model through normal authenticated observation.
            let mut caps = CAP_BROKER.lock();
            let consumer = caps
                .channels
                .row_mut(h)?
                .consumer
                .ok_or(Error::Incomplete)?;
            caps.channel_bell(h, consumer, 11, 0)?;
            let bell = caps.channels.row_mut(h)?.bell.as_mut().unwrap();
            if bell.has_waiter() {
                return Err(Error::Duplicate);
            }
            bell.fixture_sequence(observed);
            return Ok(0);
        }
        if op == 13 {
            // Completion collection authenticates the original exact channel
            // and VM even after terminal retirement/slot reuse. It grants no
            // new operation authority on a retired object.
            let r = self.processes[i].fastlane_bell.ok_or(Error::Denied)?;
            if r.handle != h || r.who != who {
                return Err(Error::Denied);
            }
            let outcome = r.outcome.ok_or(Error::Pending)?;
            crate::serial_println!("[FL-BELL] event=take generation={} consumer={} inc={} vm={} cpu={} result={} sequence={}",
                h.generation, who.pid, who.incarnation, who.vm_generation, crate::sched::current_cpu_id(),
                outcome.as_ref().err().map_or(0, |e| *e as u64), outcome.unwrap_or(0));
            self.processes[i].fastlane_bell = None;
            return outcome;
        }
        if op == 12 && self.processes[i].fastlane_bell.is_some() {
            return Err(Error::Duplicate);
        }
        let mut caps = CAP_BROKER.lock();
        let (value, wake) = caps.channel_bell(h, who, op, observed)?;
        let outcome = if op == 14 {
            Err(Error::Cancelled)
        } else if op == 10 {
            let consumer = caps
                .channels
                .row_mut(h)?
                .consumer
                .ok_or(Error::Incomplete)?;
            caps.channel_bell(h, consumer, 11, 0).map(|r| r.0)
        } else {
            Ok(value)
        };
        drop(caps);
        if let Some(who) = wake {
            self.bell_complete(h, who, outcome);
        }
        if op == 12 {
            self.processes[i].fastlane_bell = Some(Receipt {
                handle: h,
                who,
                outcome: None,
            });
            super::bell_race::registered();
            self.processes[i].block_start_tick = self.global_tick;
            self.set_state(i, ProcessState::BlockedOnIo);
            #[cfg(all(debug_assertions, feature = "fastlane_doorbell_trace"))]
            self.trace_bell_process("wait-registered", i, observed, self.processes[i].context_rsp);
            crate::serial_println!(
                "[FL-BELL] event=registered generation={} consumer={} inc={} vm={} cpu={}",
                h.generation,
                who.pid,
                who.incarnation,
                who.vm_generation,
                crate::sched::current_cpu_id()
            );
            request_park_after_sysret();
        }
        if op == 10 {
            outcome
        } else {
            Ok(value)
        }
    }
}
