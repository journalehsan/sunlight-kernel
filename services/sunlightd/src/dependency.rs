//! Manager-owned static dependency reservations. No userspace lease payloads,
//! expiry, locks or process syscalls: only the serialized manager mutates these.
use crate::activation::Runtime;
use sunlight_ipc::demand as wire;

pub const MAX_DEPENDENCY_LEASES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
    pub unit: usize,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub consumer: Instance,
    pub provider: Instance,
    pub runtime: Option<Runtime>,
}

pub struct Dependencies {
    slots: [Option<Lease>; MAX_DEPENDENCY_LEASES],
}

impl Default for Dependencies {
    fn default() -> Self {
        Self {
            slots: [None; MAX_DEPENDENCY_LEASES],
        }
    }
}

impl Dependencies {
    /// Atomic reservation: capacity and incarnation conflicts fail before edits.
    /// The adapter deduplicates the static plan; duplicates here are harmless.
    pub fn reserve(&mut self, plan: &[Lease]) -> Result<(), u64> {
        let mut additions = 0;
        for (i, edge) in plan.iter().enumerate() {
            if edge.consumer == edge.provider
                || edge.consumer.generation == 0
                || edge.provider.generation == 0
            {
                return Err(wire::DEPENDENCY);
            }
            if let Some(old) = self.slots.iter().flatten().find(|old| {
                old.consumer == edge.consumer && old.provider.unit == edge.provider.unit
            }) {
                if old.provider != edge.provider {
                    return Err(wire::STALE);
                }
            } else if !plan[..i]
                .iter()
                .any(|old| old.consumer == edge.consumer && old.provider == edge.provider)
            {
                additions += 1;
            }
        }
        if additions > self.slots.iter().filter(|s| s.is_none()).count() {
            return Err(wire::BUSY);
        }
        for edge in plan {
            if !self
                .slots
                .iter()
                .flatten()
                .any(|old| old.consumer == edge.consumer && old.provider == edge.provider)
            {
                let slot = self.slots.iter_mut().find(|s| s.is_none()).unwrap();
                // Binding is performed separately after kernel identity checks.
                *slot = Some(Lease {
                    runtime: None,
                    ..*edge
                });
            }
        }
        Ok(())
    }

    /// A reservation binds once. Replacement endpoints/processes cannot inherit it.
    pub fn bind(
        &mut self,
        consumer: Instance,
        provider: Instance,
        runtime: Runtime,
    ) -> Result<(), u64> {
        if runtime.process_generation == 0 || runtime.endpoint.is_none() {
            return Err(wire::STALE);
        }
        let edge = self
            .slots
            .iter_mut()
            .flatten()
            .find(|edge| edge.consumer == consumer && edge.provider == provider)
            .ok_or(wire::STALE)?;
        if edge.runtime.is_some_and(|old| old != runtime) {
            return Err(wire::STALE);
        }
        edge.runtime = Some(runtime);
        Ok(())
    }

    pub fn matches(&self, consumer: Instance, provider: Instance, runtime: Runtime) -> bool {
        self.slots.iter().flatten().any(|edge| {
            edge.consumer == consumer && edge.provider == provider && edge.runtime == Some(runtime)
        })
    }

    pub fn busy(&self, provider: Instance) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|edge| edge.provider == provider)
    }

    /// Only the manager's retained-runtime/activation observation can release.
    /// Repeated cleanup is idempotent; client and service messages cannot release.
    pub fn retain(&mut self, mut alive: impl FnMut(Instance) -> bool) {
        for slot in &mut self.slots {
            if slot.is_some_and(|edge| !alive(edge.consumer)) {
                *slot = None;
            }
        }
    }

    pub fn counts(&self, unit: usize) -> (usize, usize) {
        (
            self.slots
                .iter()
                .flatten()
                .filter(|e| e.consumer.unit == unit)
                .count(),
            self.slots
                .iter()
                .flatten()
                .filter(|e| e.provider.unit == unit)
                .count(),
        )
    }

    pub fn count(&self) -> usize {
        self.slots.iter().flatten().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn instance(unit: usize, generation: u64) -> Instance {
        Instance { unit, generation }
    }
    fn edge(a: usize, b: usize) -> Lease {
        Lease {
            consumer: instance(a, 1),
            provider: instance(b, 1),
            runtime: None,
        }
    }
    fn runtime() -> Runtime {
        Runtime {
            pid: 4,
            process_generation: 9,
            endpoint: Some(8),
        }
    }
    #[test]
    fn shared_provider_survives_one_consumer_and_duplicate_release() {
        let mut d = Dependencies::default();
        d.reserve(&[edge(0, 2), edge(1, 2), edge(0, 2)]).unwrap();
        assert_eq!(d.count(), 2);
        d.retain(|i| i.unit == 1);
        assert!(d.busy(instance(2, 1)));
        d.retain(|i| i.unit == 1);
        assert_eq!(d.count(), 1);
        d.retain(|_| false);
        assert!(!d.busy(instance(2, 1)));
    }
    #[test]
    fn transitive_holds_release_in_consumer_exit_order() {
        let mut d = Dependencies::default();
        d.reserve(&[edge(0, 1), edge(1, 2)]).unwrap();
        d.retain(|i| i.unit != 0);
        assert!(!d.busy(instance(1, 1)));
        assert!(d.busy(instance(2, 1)));
        d.retain(|_| false);
        assert_eq!(d.count(), 0);
    }
    #[test]
    fn binding_never_accepts_replacement_identity_or_generation() {
        let mut d = Dependencies::default();
        d.reserve(&[edge(0, 1)]).unwrap();
        let r = runtime();
        d.bind(instance(0, 1), instance(1, 1), r).unwrap();
        assert!(d.matches(instance(0, 1), instance(1, 1), r));
        assert!(!d.matches(instance(0, 2), instance(1, 1), r));
        assert!(!d.busy(instance(1, 2)));
        assert_eq!(
            d.bind(
                instance(0, 1),
                instance(1, 1),
                Runtime {
                    process_generation: 10,
                    ..r
                }
            ),
            Err(wire::STALE)
        );
        assert_eq!(
            d.reserve(&[Lease {
                provider: instance(1, 2),
                ..edge(0, 1)
            }]),
            Err(wire::STALE)
        );
        d.retain(|_| false);
        d.reserve(&[Lease {
            consumer: instance(0, 2),
            provider: instance(1, 2),
            runtime: None,
        }])
        .unwrap();
        assert!(!d.matches(instance(0, 2), instance(1, 2), r));
    }
    #[test]
    fn capacity_failure_is_atomic_and_partial_failure_cleanup_is_bounded() {
        let mut d = Dependencies::default();
        for n in 0..MAX_DEPENDENCY_LEASES - 1 {
            d.reserve(&[edge(n, 200)]).unwrap();
        }
        assert_eq!(
            d.reserve(&[edge(300, 301), edge(302, 303)]),
            Err(wire::BUSY)
        );
        assert_eq!(d.count(), MAX_DEPENDENCY_LEASES - 1);
        assert!(!d.busy(instance(301, 1)));
        d.retain(|_| false);
        d.reserve(&[edge(0, 1), edge(0, 2)]).unwrap();
        d.retain(|_| false);
        assert_eq!(d.count(), 0);
    }
}
