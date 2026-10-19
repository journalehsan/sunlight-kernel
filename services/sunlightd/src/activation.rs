//! Fixed-space D1 single-flight lifecycle, independent of process syscalls.
//! The daemon owns this state serially; it never holds scheduler/registry locks.
use sunlight_ipc::demand as wire;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Phase {
    Inactive,
    Starting,
    Ready,
    Failed,
    IdleCandidate,
    Quiescing,
    Stopping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub pid: u32,
    pub process_generation: u64,
    pub endpoint: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    pub requests: u64,
    pub starts: u64,
    pub coalesced: u64,
    pub failures: u64,
    pub timeouts: u64,
    pub cold_ms: u64,
    pub ready_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Start,
    Join,
    Reuse,
}

pub struct Activation {
    pub phase: Phase,
    pub generation: u64,
    pub runtime: Option<Runtime>,
    pub requested_at: u64,
    pub spawned_at: u64,
    pub failure: u64,
    pub retry_at: u64,
    pub metrics: Metrics,
    pub spawn_attempted: bool,
    pub idle_reported: bool,
    pub ready_at: u64,
    pub quiesce_nonce: u64,
    pub quiesce_at: u64,
    pub stop_ack: bool,
}

impl Default for Activation {
    fn default() -> Self {
        Self {
            phase: Phase::Inactive,
            generation: 0,
            runtime: None,
            requested_at: 0,
            spawned_at: 0,
            failure: 0,
            retry_at: 0,
            metrics: Metrics::default(),
            spawn_attempted: false,
            idle_reported: false,
            ready_at: 0,
            quiesce_nonce: 0,
            quiesce_at: 0,
            stop_ack: false,
        }
    }
}

impl Activation {
    pub fn request(&mut self, now: u64) -> Result<Request, u64> {
        self.metrics.requests = self.metrics.requests.saturating_add(1);
        match self.phase {
            Phase::Starting => {
                self.metrics.coalesced = self.metrics.coalesced.saturating_add(1);
                return Ok(Request::Join);
            }
            Phase::Ready | Phase::IdleCandidate => {
                self.phase = Phase::Ready;
                return Ok(Request::Reuse);
            }
            Phase::Quiescing | Phase::Stopping => return Err(wire::BUSY),
            Phase::Failed if now < self.retry_at || self.runtime.is_some() => {
                return Err(wire::BUSY)
            }
            _ => {}
        }
        self.generation = self.generation.checked_add(1).ok_or(wire::EXHAUSTED)?;
        self.phase = Phase::Starting;
        self.requested_at = now;
        self.failure = 0;
        self.spawn_attempted = false;
        self.idle_reported = false;
        self.stop_ack = false;
        Ok(Request::Start)
    }

    pub fn spawned(&mut self, pid: u32, generation: u64, now: u64) {
        self.runtime = Some(Runtime {
            pid,
            process_generation: generation,
            endpoint: None,
        });
        self.spawned_at = now;
        self.metrics.starts = self.metrics.starts.saturating_add(1);
        self.metrics.cold_ms = now.saturating_sub(self.requested_at);
    }

    /// Call only after kernel validation of the live process and endpoint.
    pub fn ready(&mut self, generation: u64, runtime: Runtime, now: u64) -> Result<(), u64> {
        if self.expired(now) {
            return Err(wire::TIMED_OUT);
        }
        if self.phase != Phase::Starting
            || generation != self.generation
            || runtime.endpoint.is_none()
            || runtime.process_generation == 0
            || !self.runtime.is_some_and(|r| {
                r.pid == runtime.pid && r.process_generation == runtime.process_generation
            })
        {
            return Err(wire::STALE);
        }
        self.runtime = Some(runtime);
        self.phase = Phase::Ready;
        self.ready_at = now;
        self.metrics.ready_ms = now.saturating_sub(self.requested_at);
        Ok(())
    }

    pub fn fail(&mut self, code: u64, now: u64) {
        if self.phase != Phase::Failed {
            self.metrics.failures = self.metrics.failures.saturating_add(1);
            if code == wire::TIMED_OUT {
                self.metrics.timeouts = self.metrics.timeouts.saturating_add(1);
            }
        }
        self.phase = Phase::Failed;
        self.idle_reported = false;
        self.failure = code;
        self.retry_at = now.saturating_add(wire::RETRY_MS);
        // Retain the old runtime until confirmed dead. Failure never permits
        // replacement while an earlier instance could still own an endpoint.
        if let Some(r) = self.runtime.as_mut() {
            r.endpoint = None;
        }
    }

    pub fn expired(&self, now: u64) -> bool {
        self.phase == Phase::Starting && now.saturating_sub(self.requested_at) >= wire::TIMEOUT_MS
    }

    pub fn resolution(&self) -> sunlight_ipc::IpcMsg {
        let r = self.runtime;
        sunlight_ipc::IpcMsg::with_label(wire::OK)
            .word(0, self.generation)
            .word(1, r.map_or(0, |r| r.pid as u64))
            .word(2, r.map_or(0, |r| r.process_generation))
            .word(3, r.and_then(|r| r.endpoint).map_or(0, |id| id as u64))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::unit::{parse_service_unit, ActivationPolicy};
    use std::{
        sync::{Arc, Barrier, Mutex},
        thread,
    };

    fn runtime(pid: u32, generation: u64, endpoint: Option<u32>) -> Runtime {
        Runtime {
            pid,
            process_generation: generation,
            endpoint,
        }
    }
    #[test]
    fn policy_is_explicit_and_invalid_demand_definitions_fail_closed() {
        assert_eq!(
            parse_service_unit(b"[Service]\nExecStart=/sbin/a\n")
                .unwrap()
                .activation,
            ActivationPolicy::Resident
        );
        let valid = b"[Service]\nExecStart=/sbin/a\nType=notify\nActivation=on-demand\n";
        assert_eq!(
            parse_service_unit(valid).unwrap().activation,
            ActivationPolicy::OnDemand
        );
        for invalid in [
            b"[Service]\nActivation=typo\n".as_slice(),
            b"[Service]\nActivation=on-demand\nExecStart=/sbin/a\n",
            b"[Service]\nActivation=on-demand\nType=notify\nExecStart=/sbin/a\nRestart=always\n",
        ] {
            assert!(parse_service_unit(invalid).is_err());
        }
    }
    #[test]
    fn actual_concurrent_clients_reserve_exactly_one_generation() {
        let state = Arc::new(Mutex::new(Activation::default()));
        let barrier = Arc::new(Barrier::new(16));
        let mut threads = std::vec::Vec::new();
        for _ in 0..16 {
            let (state, barrier) = (state.clone(), barrier.clone());
            threads.push(thread::spawn(move || {
                barrier.wait();
                state.lock().unwrap().request(10).unwrap()
            }));
        }
        let starts = threads
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|r| *r == Request::Start)
            .count();
        let state = state.lock().unwrap();
        assert_eq!(starts, 1);
        assert_eq!(state.generation, 1);
        assert_eq!(state.metrics.coalesced, 15);
        assert_eq!(state.metrics.requests, 16);
    }
    #[test]
    fn spawn_is_not_ready_and_stale_readiness_cannot_cross_restarts() {
        let mut a = Activation::default();
        a.request(0).unwrap();
        a.spawned(4, 10, 2);
        assert_eq!(a.phase, Phase::Starting);
        assert!(a.ready(1, runtime(4, 10, None), 3).is_err());
        assert!(a.ready(1, runtime(4, 9, Some(5)), 3).is_err());
        a.ready(1, runtime(4, 10, Some(5)), 3).unwrap();
        a.fail(wire::DIED, 4);
        a.runtime = None;
        a.request(1_004).unwrap();
        a.spawned(4, 11, 1_005);
        assert!(a.ready(1, runtime(4, 11, Some(6)), 1_006).is_err());
        assert!(a.ready(2, runtime(4, 10, Some(5)), 1_006).is_err());
        a.ready(2, runtime(4, 11, Some(6)), 1_006).unwrap();
        assert_eq!(a.request(1_007), Ok(Request::Reuse));
    }
    #[test]
    fn timeout_and_live_failed_instance_bound_retries() {
        let mut a = Activation::default();
        a.request(0).unwrap();
        a.spawned(1, 2, 10);
        assert!(!a.expired(wire::TIMEOUT_MS - 1));
        assert!(a.expired(wire::TIMEOUT_MS));
        a.fail(wire::TIMED_OUT, wire::TIMEOUT_MS);
        assert!(!a.expired(9_000));
        assert_eq!(a.metrics.timeouts, 1);
        assert_eq!(a.request(10_000), Err(wire::BUSY)); // old process still alive
        a.runtime = None;
        assert_eq!(a.request(5_001), Err(wire::BUSY));
        assert_eq!(a.request(10_000), Ok(Request::Start));
    }
    #[test]
    fn late_readiness_cannot_extend_the_deadline() {
        let mut a = Activation::default();
        a.request(0).unwrap();
        a.spawned(1, 2, 10);
        assert_eq!(
            a.ready(1, runtime(1, 2, Some(4)), wire::TIMEOUT_MS),
            Err(wire::TIMED_OUT)
        );
        assert_eq!(a.phase, Phase::Starting);
        a.fail(wire::TIMED_OUT, wire::TIMEOUT_MS);
        assert_eq!(a.phase, Phase::Failed);
    }

    #[test]
    fn generation_exhaustion_never_wraps() {
        let mut a = Activation::default();
        a.generation = u64::MAX;
        assert_eq!(a.request(0), Err(wire::EXHAUSTED));
        assert_eq!(a.phase, Phase::Inactive);
    }
    #[test]
    fn activation_authority_is_independent_of_operation_access() {
        use sunlight_ipc::{
            demand, name_to_u64, service_capability_allows_hashed_name as allows,
            ServiceCapability as C,
        };
        assert!(!demand::activation_authorized(Some(C::DemandFixture.bit())));
        assert!(!demand::activation_authorized(Some(C::ServiceNotify.bit())));
        assert!(allows(C::ServiceNotify.bit(), name_to_u64("sunlightd")));
        assert!(demand::activation_authorized(Some(
            C::ServiceLifecycle.bit()
        )));
        assert!(!allows(
            C::ServiceLifecycle.bit(),
            name_to_u64("d1-fixture")
        ));
        assert!(allows(C::DemandFixture.bit(), name_to_u64("d1-fixture")));
    }
}
