//! Opt-in mediated selector. No mapping, transfer, or scheduling authority.
use super::{ActorCounters, Error, Handle, RecipientIdentity, Scheduler, State, CAP_BROKER};
use sunlight_ipc::measurement_clock as mc;

impl ActorCounters {
    pub fn placement(&mut self, cpu: usize) {
        self.window.placement(cpu);
    }
    pub fn charge_probe(&mut self, committed: u64, since: Option<u64>, now: u64) {
        self.window.charge_probe(committed, since, now);
    }
    pub fn abort(&mut self) {
        if self.window.active {
            crate::serial_println!(
                "[FL-M61A-ABORT] w={} generation={} pid={} inc={} vm={} reason=actor-exit qualified=0",
                self.window.id(),
                self.generation, self.window.actor().unwrap().pid, self.window.actor().unwrap().incarnation, self.window.actor().unwrap().vm_generation
            );
            self.window.active = false;
            self.enabled = false;
        }
    }
}
fn converted(w: [u64; 7]) -> mc::Stamp {
    mc::Stamp {
        cpu: w[0] as u32,
        epoch: w[1] as u32,
        ns: w[2],
        flags: w[5] as u32,
    }
}
fn raw(
    event: &str,
    who: RecipientIdentity,
    h: Handle,
    window: u64,
    runtime: u64,
    words: Option<[u64; 7]>,
) {
    let w = words.unwrap_or([0; 7]);
    crate::serial_println!("[FL-M61A-STAMP] event={} pid={} inc={} vm={} generation={} w={} cpu={} epoch={} ns={} hz={} ppm={} flags={} spread={} runtime={}",event,who.pid,who.incarnation,who.vm_generation,h.generation,window,w[0],w[1],w[2],w[3],w[4],w[5],w[6],runtime);
}
impl Scheduler {
    pub(crate) fn fastlane_mediated_accounting(
        &mut self,
        h: Handle,
        op: u64,
        id: u64,
    ) -> Result<(), Error> {
        self.fastlane_mapping_sweep();
        let who = RecipientIdentity::of(self.current_process());
        if !self.channel_vm_exclusive(who) {
            return Err(Error::Denied);
        }
        let mut caps = CAP_BROKER.lock();
        let row = caps.channels.row_mut(h)?;
        let role = if row.creator == who {
            0
        } else if row.consumer == Some(who) {
            1
        } else {
            return Err(Error::Denied);
        };
        if !row.live() {
            return Err(Error::Terminal);
        }
        if row.state != State::Attached || !row.mapped[role].iter().all(|v| *v) {
            return Err(Error::Incomplete);
        }
        if row.native_ring[role] || row.transfer != 0 || !row.repeated.enabled {
            return Err(Error::Denied);
        }
        drop(caps);
        // Local controls exercise the same matching and exhaustion code, without
        // accepting an identity, clock or counter value from the caller.
        if op == 30 {
            let mut m = ActorCounters::new();
            assert_eq!(m.window.stop(h, who, 0), Err(Error::Incomplete));
            m.window.start(h, who, 1, 0)?;
            assert_eq!(
                m.window.stop(
                    h,
                    RecipientIdentity {
                        incarnation: who.incarnation + 1,
                        ..who
                    },
                    1
                ),
                Err(Error::Denied)
            );
            assert_eq!(
                m.window.stop(
                    h,
                    RecipientIdentity {
                        pid: who.pid + 1,
                        ..who
                    },
                    1
                ),
                Err(Error::Denied)
            );
            assert_eq!(
                m.window.stop(
                    h,
                    RecipientIdentity {
                        vm_generation: who.vm_generation + 1,
                        ..who
                    },
                    1
                ),
                Err(Error::Denied)
            );
            m.enabled = true;
            m.values[0] = u64::MAX;
            m.increment(0);
            assert!(m.overflow);
            m.window.charge_probe(u64::MAX - 1, Some(0), 10);
            assert!(m.window.invalid);
            let a = mc::Stamp {
                cpu: 0,
                epoch: 1,
                ns: 10,
                flags: mc::FLAG_PIT_CALIBRATED | mc::FLAG_FREQUENCY_STABILITY_CHECKED,
            };
            assert!(mc::elapsed_ns(&a, &mc::Stamp { ns: 9, ..a }).is_err());
            assert!(mc::elapsed_ns(&a, &mc::Stamp { cpu: 1, ..a }).is_err());
            crate::serial_println!("[FL-M61A-CONTROL] pid={} inc={} vm={} stale_inc=reject foreign=reject stale_vm=reject exhausted=reject invalid_pair=reject cross_cpu=reject",who.pid,who.incarnation,who.vm_generation);
            return Ok(());
        }
        let idx = self.current_process_index().ok_or(Error::Denied)?;
        let words = crate::arch::x86_64::measurement_clock::stamp_words();
        let now = self.now_ns();
        let runtime = self.effective_runtime_at(idx, now);
        let cpu = crate::sched::current_cpu_id();
        let p = &mut self.processes[idx];
        p.measurement
            .charge_probe(p.cpu_runtime_ns, p.cpu_charge_since, now);
        if op == 28 {
            if p.measurement.enabled {
                return Err(Error::Duplicate);
            }
            let mut window = p.measurement.window;
            window.start(h, who, id, cpu)?;
            p.measurement = ActorCounters {
                enabled: true,
                generation: h.generation,
                runtime_start: runtime,
                cpu_start: cpu,
                raw_start: words,
                window,
                ..ActorCounters::new()
            };
            raw("start", who, h, id, runtime, words);
        } else {
            p.measurement.window.stop(h, who, id)?;
            let m = &mut p.measurement;
            m.enabled = false;
            m.window.placement(cpu);
            // Endpoint placement plus every switch-in observation detects a
            // migration away and back. Migration never qualifies wall duration.
            let migrated = m.window.migrated || cpu != m.cpu_start;
            let charged = runtime
                .checked_sub(m.runtime_start)
                .filter(|_| !m.overflow && !m.window.invalid && runtime != u64::MAX);
            let wall = m.raw_start.zip(words).and_then(|(a, b)| {
                if migrated || a[3..] != b[3..] || a[4] == 0 || a[4] > 20_000 || a[5] & 36 != 36 {
                    return None;
                }
                mc::elapsed_ns(&converted(a), &converted(b))
                    .ok()
                    .filter(|v| *v > 0)
            });
            let consistent = charged.zip(wall).is_some_and(|(c, w)| {
                matches!(
                    sunlight_ipc::cpu_accounting::reconcile(&[c], w),
                    sunlight_ipc::cpu_accounting::Reconcile::Consistent { .. }
                )
            });
            raw("stop", who, h, id, runtime, words);
            crate::serial_println!("[FL-M61A-CPU] pid={} inc={} vm={} generation={} w={} charged={} valid={} wall={} wall_valid={} switches={} cpu_start={} cpu_stop={} migrated={} discontinuities={} overflow={} reconcile={}",who.pid,who.incarnation,who.vm_generation,h.generation,id,charged.unwrap_or(0),u8::from(charged.is_some()&&!migrated),wall.unwrap_or(0),u8::from(wall.is_some()),m.values[2],m.cpu_start,cpu,u8::from(migrated),u8::from(m.window.invalid),u8::from(m.overflow),if consistent {"consistent"} else {"not-qualified"});
        }
        Ok(())
    }
}
