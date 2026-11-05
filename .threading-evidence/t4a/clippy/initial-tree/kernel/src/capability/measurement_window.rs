//! Bounded matching state only; runtime remains the existing scheduler accumulator.
use super::{
    channel::{Error, Handle},
    delivery::RecipientIdentity,
};

#[derive(Clone, Copy)]
pub(super) struct Window {
    pub active: bool,
    last: u64,
    handle: Option<Handle>,
    actor: Option<RecipientIdentity>,
    pub migrated: bool,
    pub invalid: bool,
    pub cpu: usize,
}
impl Window {
    pub const fn new() -> Self {
        Self {
            active: false,
            last: 0,
            handle: None,
            actor: None,
            migrated: false,
            invalid: false,
            cpu: 0,
        }
    }
    pub fn actor(&self) -> Option<RecipientIdentity> {
        self.actor
    }
    pub fn id(&self) -> u64 {
        self.last
    }
    pub fn start(
        &mut self,
        h: Handle,
        actor: RecipientIdentity,
        id: u64,
        cpu: usize,
    ) -> Result<(), Error> {
        if self.active {
            return Err(Error::Duplicate);
        }
        if id == 0 || id <= self.last {
            return Err(Error::Stale);
        }
        *self = Self {
            active: true,
            last: id,
            handle: Some(h),
            actor: Some(actor),
            migrated: false,
            invalid: false,
            cpu,
        };
        Ok(())
    }
    pub fn stop(&mut self, h: Handle, actor: RecipientIdentity, id: u64) -> Result<(), Error> {
        if self.last == 0 {
            return Err(Error::Incomplete);
        }
        if self.actor.is_some_and(|a| a != actor) {
            return Err(Error::Denied);
        }
        if self.handle.is_some_and(|v| v != h) || id != self.last {
            return Err(Error::Stale);
        }
        if !self.active {
            return Err(if self.last == 0 {
                Error::Incomplete
            } else {
                Error::Duplicate
            });
        }
        self.active = false;
        Ok(())
    }
    pub fn placement(&mut self, cpu: usize) {
        if self.active && cpu != self.cpu {
            self.migrated = true;
        }
    }
    pub fn charge_probe(&mut self, committed: u64, since: Option<u64>, now: u64) {
        if self.active
            && since.is_some_and(|s| {
                now.checked_sub(s)
                    .and_then(|d| committed.checked_add(d))
                    .is_none()
            })
        {
            self.invalid = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::CapabilityToken;
    use super::*;
    fn actor() -> RecipientIdentity {
        RecipientIdentity {
            pid: 7,
            incarnation: 8,
            vm_generation: 9,
        }
    }
    fn handle() -> Handle {
        Handle {
            token: CapabilityToken(1),
            generation: 2,
        }
    }
    #[test]
    fn matched_and_rejected_observations() {
        let (h, a) = (handle(), actor());
        let mut w = Window::new();
        assert_eq!(w.stop(h, a, 0), Err(Error::Incomplete));
        w.start(h, a, 1, 0).unwrap();
        assert_eq!(w.start(h, a, 2, 0), Err(Error::Duplicate));
        for foreign in [
            RecipientIdentity { pid: 99, ..a },
            RecipientIdentity {
                incarnation: 99,
                ..a
            },
            RecipientIdentity {
                vm_generation: 99,
                ..a
            },
        ] {
            assert_eq!(w.stop(h, foreign, 1), Err(Error::Denied));
            assert!(w.active);
        }
        assert_eq!(
            w.stop(
                Handle {
                    generation: 99,
                    ..h
                },
                a,
                1
            ),
            Err(Error::Stale)
        );
        assert_eq!(
            w.stop(
                Handle {
                    token: CapabilityToken(99),
                    ..h
                },
                a,
                1
            ),
            Err(Error::Stale)
        );
        assert_eq!(w.stop(h, a, 2), Err(Error::Stale));
        assert!(w.active);
        w.stop(h, a, 1).unwrap();
        assert_eq!(w.id(), 1);
        assert_eq!(w.actor(), Some(a));
        assert_eq!(w.stop(h, a, 1), Err(Error::Duplicate));
        assert_eq!(w.start(h, a, 1, 0), Err(Error::Stale));
    }
    #[test]
    fn migration_returning_to_start_and_bad_charge_are_sticky() {
        let mut w = Window::new();
        w.start(handle(), actor(), 1, 0).unwrap();
        w.placement(1);
        w.placement(0);
        assert!(w.migrated);
        w.charge_probe(0, Some(100), 90);
        assert!(w.invalid);
        w.start(handle(), actor(), 2, 0).unwrap_err();
        w.stop(handle(), actor(), 1).unwrap();
        w.start(handle(), actor(), 2, 0).unwrap();
        w.charge_probe(u64::MAX - 1, Some(0), 10);
        assert!(w.invalid);
    }
}
