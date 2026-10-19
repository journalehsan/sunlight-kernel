//! Bounded D2 accounting and event deadlines. Syscalls live in idle_runtime.
use crate::activation::{Activation, Phase, Runtime};
use crate::unit::IdlePolicy;
use sunlight_ipc::demand as wire;

pub const MAX_LEASES: usize = 32;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Unit(usize),
    Lease(usize),
}
#[derive(Clone, Copy, Debug)]
pub struct Deadline {
    pub key: Key,
    pub at: u64,
}

/// One event per unit or lease. Cancellation physically removes the entry:
/// stale timer churn cannot exhaust capacity. No periodic traversal is needed.
pub struct Deadlines(heapless::Vec<Deadline, 64>);
impl Default for Deadlines {
    fn default() -> Self {
        Self(heapless::Vec::new())
    }
}
impl Deadlines {
    pub fn cancel(&mut self, key: Key) {
        self.0.retain(|d| d.key != key);
    }
    pub fn set(&mut self, key: Key, at: u64) {
        self.cancel(key);
        self.0
            .push(Deadline { key, at })
            .expect("one deadline per bounded owner");
        self.0.sort_unstable_by(|a, b| b.at.cmp(&a.at));
    }
    pub fn next(&self) -> Option<u64> {
        self.0.last().map(|d| d.at)
    }
    pub fn due(&mut self, now: u64) -> Option<Deadline> {
        if self.next().is_some_and(|at| at <= now) {
            self.0.pop()
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Lease {
    pub id: u64,
    pub unit: usize,
    pub activation: u64,
    pub runtime: Runtime,
    pub owner: Owner,
    pub expires: u64,
}
pub struct Leases {
    pub slots: [Option<Lease>; MAX_LEASES],
    next: u64,
}
impl Default for Leases {
    fn default() -> Self {
        Self {
            slots: [None; MAX_LEASES],
            next: 0,
        }
    }
}
impl Leases {
    pub fn acquire(
        &mut self,
        unit: usize,
        activation: u64,
        runtime: Runtime,
        owner: Owner,
        now: u64,
        lifetime: u64,
    ) -> Result<(usize, Lease), u64> {
        if !(50..=wire::LEASE_MAX_MS).contains(&lifetime) || owner.generation == 0 {
            return Err(wire::STALE);
        }
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(wire::BUSY)?;
        let id = self.next.checked_add(1).ok_or(wire::EXHAUSTED)?;
        let lease = Lease {
            id,
            unit,
            activation,
            runtime,
            owner,
            expires: now.saturating_add(lifetime),
        };
        self.next = id;
        self.slots[slot] = Some(lease);
        Ok((slot, lease))
    }
    pub fn release(&mut self, id: u64, activation: u64, owner: Owner) -> Result<usize, u64> {
        let slot = self
            .slots
            .iter()
            .position(|l| {
                l.is_some_and(|l| l.id == id && l.activation == activation && l.owner == owner)
            })
            .ok_or(wire::STALE)?;
        self.slots[slot] = None;
        Ok(slot)
    }
    pub fn busy(&self, unit: usize, generation: u64) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|l| l.unit == unit && l.activation == generation)
    }
}

impl Activation {
    pub fn nominate_idle(&mut self, policy: IdlePolicy, now: u64, pinned: bool) -> Option<u64> {
        if !policy.unload_when_idle || !self.idle_reported || pinned || self.phase != Phase::Ready {
            return None;
        }
        self.phase = Phase::IdleCandidate;
        Some(
            now.saturating_add(policy.idle_grace_ms)
                .max(self.ready_at.saturating_add(policy.minimum_uptime_ms)),
        )
    }
    pub fn begin_quiesce(&mut self, pinned: bool) -> Result<u64, u64> {
        if self.phase != Phase::IdleCandidate || !self.idle_reported || pinned {
            return Err(wire::BUSY);
        }
        self.quiesce_nonce = self.quiesce_nonce.checked_add(1).ok_or(wire::EXHAUSTED)?;
        self.phase = Phase::Quiescing;
        Ok(self.quiesce_nonce)
    }
    /// A trusted, closed-admission and drained acknowledgement is the commit
    /// boundary. It never reopens this generation. Activation returns Busy.
    pub fn drained(&mut self, generation: u64, nonce: u64, pinned: bool) -> Result<(), u64> {
        if self.phase != Phase::Quiescing
            || generation != self.generation
            || nonce != self.quiesce_nonce
            || pinned
            || !self.idle_reported
        {
            return Err(wire::STALE);
        }
        self.phase = Phase::Stopping;
        self.idle_reported = false;
        Ok(())
    }
    /// Caller must have obtained the exact kernel reaping receipt.
    pub fn unloaded(&mut self, runtime: Runtime) -> Result<(), u64> {
        if self.phase != Phase::Stopping || !self.stop_ack || self.runtime != Some(runtime) {
            return Err(wire::STALE);
        }
        self.runtime = None;
        self.phase = Phase::Inactive;
        self.failure = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};
    fn ready() -> Activation {
        let mut a = Activation::default();
        a.request(0).unwrap();
        a.spawned(7, 11, 0);
        a.ready(
            1,
            Runtime {
                pid: 7,
                process_generation: 11,
                endpoint: Some(9),
            },
            10,
        )
        .unwrap();
        a
    }
    fn policy() -> IdlePolicy {
        IdlePolicy {
            unload_when_idle: true,
            ..IdlePolicy::default()
        }
    }
    #[test]
    fn policy_is_opt_in_bounded_and_resident_rejected() {
        use crate::unit::parse_service_unit as parse;
        assert!(
            !parse(b"[Service]\nExecStart=/sbin/a\n")
                .unwrap()
                .idle
                .unload_when_idle
        );
        for invalid in [
            "UnloadWhenIdle=yes",
            "IdleGraceMs=0",
            "MinimumUptimeMs=60001",
            "ShutdownTimeoutMs=bad",
        ] {
            assert!(
                parse(std::format!("[Service]\nExecStart=/sbin/a\n{invalid}\n").as_bytes())
                    .is_err()
            );
        }
        assert!(parse(b"[Service]\nExecStart=/sbin/a\nType=notify\nActivation=on-demand\nUnloadWhenIdle=yes\n").unwrap().idle.unload_when_idle);
    }
    #[test]
    fn grace_cancellation_minimum_uptime_and_no_expired_event_churn() {
        let mut a = ready();
        let mut p = policy();
        p.minimum_uptime_ms = 5_000;
        a.idle_reported = true;
        assert_eq!(a.nominate_idle(p, 20, false), Some(5010));
        let mut q = Deadlines::default();
        q.set(Key::Unit(0), 5010);
        a.request(100).unwrap();
        q.cancel(Key::Unit(0));
        assert_eq!(a.phase, Phase::Ready);
        assert!(q.due(6000).is_none());
        for n in 0..10000 {
            q.set(Key::Unit(0), n);
            q.cancel(Key::Unit(0));
        }
        assert!(q.next().is_none());
    }
    #[test]
    fn work_and_leases_block_quiesce_and_owner_is_generation_qualified() {
        let mut a = ready();
        assert_eq!(a.nominate_idle(policy(), 20, false), None);
        a.idle_reported = true;
        assert_eq!(a.nominate_idle(policy(), 20, true), None);
        let mut leases = Leases::default();
        let owner = Owner {
            pid: 9,
            generation: 20,
        };
        let (_, l) = leases
            .acquire(0, 1, a.runtime.unwrap(), owner, 0, 1000)
            .unwrap();
        assert!(leases.busy(0, 1));
        assert!(!leases.busy(0, 2));
        assert!(leases
            .release(
                l.id,
                1,
                Owner {
                    pid: 9,
                    generation: 21
                }
            )
            .is_err());
        assert!(leases.release(l.id, 2, owner).is_err());
        leases.release(l.id, 1, owner).unwrap();
        assert!(!leases.busy(0, 1));
    }
    #[test]
    fn lease_capacity_ids_and_expiry_are_bounded() {
        let a = ready();
        let mut l = Leases::default();
        let owner = Owner {
            pid: 2,
            generation: 3,
        };
        for _ in 0..MAX_LEASES {
            l.acquire(0, 1, a.runtime.unwrap(), owner, 10, 50).unwrap();
        }
        assert_eq!(
            l.acquire(0, 1, a.runtime.unwrap(), owner, 10, 50)
                .unwrap_err(),
            wire::BUSY
        );
        l.slots[0] = None;
        l.next = u64::MAX;
        assert_eq!(
            l.acquire(0, 1, a.runtime.unwrap(), owner, 10, 50)
                .unwrap_err(),
            wire::EXHAUSTED
        );
        assert!(l.slots.iter().flatten().all(|l| l.expires == 60));
    }
    #[test]
    fn stale_ack_and_old_readiness_never_commit_or_reuse() {
        let mut a = ready();
        a.idle_reported = true;
        a.nominate_idle(policy(), 20, false);
        let n = a.begin_quiesce(false).unwrap();
        assert_eq!(a.request(30), Err(wire::BUSY));
        assert!(a.drained(0, n, false).is_err());
        assert!(a.drained(1, n + 1, false).is_err());
        assert!(a.drained(1, n, true).is_err());
        a.drained(1, n, false).unwrap();
        assert_eq!(a.request(40), Err(wire::BUSY));
        let r = a.runtime.unwrap();
        assert!(a.ready(1, r, 50).is_err());
        assert!(a.unloaded(r).is_err());
        a.stop_ack = true;
        a.unloaded(r).unwrap();
        a.request(60).unwrap();
        a.spawned(8, 12, 60);
        assert!(a
            .ready(
                1,
                Runtime {
                    pid: 8,
                    process_generation: 12,
                    endpoint: Some(10)
                },
                70
            )
            .is_err());
    }
    #[test]
    fn two_clients_race_with_timeout_without_ready_after_commit() {
        for _ in 0..100 {
            let mut a = ready();
            a.idle_reported = true;
            a.nominate_idle(policy(), 20, false);
            let a = Arc::new(Mutex::new(a));
            let barrier = Arc::new(Barrier::new(3));
            let mut threads = std::vec::Vec::new();
            for timeout in [false, false, true] {
                let (a, b) = (a.clone(), barrier.clone());
                threads.push(std::thread::spawn(move || {
                    b.wait();
                    let mut a = a.lock().unwrap();
                    if timeout {
                        if let Ok(n) = a.begin_quiesce(false) {
                            a.drained(1, n, false).unwrap();
                        }
                        None
                    } else {
                        Some(a.request(2000))
                    }
                }));
            }
            let outcomes: std::vec::Vec<_> =
                threads.into_iter().map(|t| t.join().unwrap()).collect();
            if a.lock().unwrap().phase == Phase::Stopping {
                assert!(outcomes.iter().flatten().all(|r| *r == Err(wire::BUSY)));
            }
        }
    }
    #[test]
    fn repeated_cycles_and_failure_quarantine() {
        let mut a = ready();
        for n in 0..1000 {
            a.idle_reported = true;
            a.nominate_idle(policy(), 20, false);
            let nonce = a.begin_quiesce(false).unwrap();
            a.drained(a.generation, nonce, false).unwrap();
            a.stop_ack = true;
            let r = a.runtime.unwrap();
            a.unloaded(r).unwrap();
            a.request(100 + n).unwrap();
            a.spawned(7, 100 + n, 100 + n);
            a.ready(
                a.generation,
                Runtime {
                    pid: 7,
                    process_generation: 100 + n,
                    endpoint: Some(9),
                },
                101 + n,
            )
            .unwrap();
        }
        a.fail(wire::TIMED_OUT, 2000);
        assert_eq!(a.request(100000), Err(wire::BUSY));
        assert!(a.runtime.is_some());
    }
}
