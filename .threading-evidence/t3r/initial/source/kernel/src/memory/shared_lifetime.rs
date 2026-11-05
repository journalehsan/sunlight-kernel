//! Backing lifetime, serialized by CAP_BROKER (not an atomic reference count).
//! Retirement denies new views but does not revoke existing PTEs. A caller may
//! reclaim backing only after the final view has been unmapped and its TLB
//! invalidations acknowledged. This type does not perform those VM operations.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifetimeError {
    Retired,
    Overflow,
    Underflow,
}

#[derive(Debug)]
pub struct SharedLifetime {
    retired: bool,
    views: usize,
}

impl SharedLifetime {
    pub const fn new() -> Self {
        Self {
            retired: false,
            views: 0,
        }
    }

    pub const fn is_retired(&self) -> bool {
        self.retired
    }

    pub const fn views(&self) -> usize {
        self.views
    }

    pub fn preflight_add(&self) -> Result<(), LifetimeError> {
        if self.retired {
            return Err(LifetimeError::Retired);
        }
        self.views.checked_add(1).ok_or(LifetimeError::Overflow)?;
        Ok(())
    }

    pub fn add(&mut self) -> Result<(), LifetimeError> {
        self.preflight_add()?;
        self.views += 1;
        Ok(())
    }

    /// Must follow verified unmapping and synchronous TLB acknowledgement.
    /// `true` transfers final backing ownership to the caller exactly once;
    /// the broker must remove the entry immediately in that case.
    pub fn release(&mut self) -> Result<bool, LifetimeError> {
        self.views = self.views.checked_sub(1).ok_or(LifetimeError::Underflow)?;
        Ok(self.views == 0)
    }

    /// Idempotent admission fence. Existing views continue to pin backing.
    /// `true` means an object with no mappings can be removed immediately.
    pub fn retire(&mut self) -> bool {
        self.retired = true;
        self.views == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirement_preserves_every_existing_view() {
        let mut life = SharedLifetime::new();
        for _ in 0..3 {
            life.add().unwrap();
        }
        assert!(!life.retire());
        assert!(!life.retire());
        assert_eq!(life.preflight_add(), Err(LifetimeError::Retired));
        assert_eq!(life.add(), Err(LifetimeError::Retired));
        assert_eq!(life.views(), 3);
        assert_eq!(life.release(), Ok(false));
        assert_eq!(life.release(), Ok(false));
        assert_eq!(life.release(), Ok(true));
        assert_eq!(life.release(), Err(LifetimeError::Underflow));
        assert_eq!(life.views(), 0);
    }

    #[test]
    fn overflow_and_failed_preflight_leave_state_unchanged() {
        let mut life = SharedLifetime {
            retired: false,
            views: usize::MAX,
        };
        assert_eq!(life.preflight_add(), Err(LifetimeError::Overflow));
        assert_eq!(life.add(), Err(LifetimeError::Overflow));
        assert_eq!(life.views(), usize::MAX);
        assert!(!life.retire());
        assert_eq!(life.add(), Err(LifetimeError::Retired));
    }

    #[test]
    fn unmapped_retirement_is_immediately_reclaimable() {
        let mut life = SharedLifetime::new();
        assert!(life.retire());
        assert!(life.is_retired());
        assert_eq!(life.release(), Err(LifetimeError::Underflow));
    }

    #[test]
    fn all_bounded_admission_retirement_release_interleavings() {
        // Broker serialization reduces concurrent operations to these orders.
        // This is a lifetime model, not an SMP/TLB or memory-order proof.
        fn explore(life: SharedLifetime, views: usize, retired: bool, depth: usize) {
            assert_eq!(life.views(), views);
            assert_eq!(life.is_retired(), retired);
            if depth == 0 {
                return;
            }
            for action in 0..3 {
                let mut next = SharedLifetime { views, retired };
                match action {
                    0 => {
                        let result = next.add();
                        if retired {
                            assert_eq!(result, Err(LifetimeError::Retired));
                        } else {
                            assert_eq!(result, Ok(()));
                        }
                        explore(next, views + usize::from(!retired), retired, depth - 1);
                    }
                    1 => {
                        assert_eq!(next.retire(), views == 0);
                        explore(next, views, true, depth - 1);
                    }
                    _ => {
                        let result = next.release();
                        if views == 0 {
                            assert_eq!(result, Err(LifetimeError::Underflow));
                            explore(next, 0, retired, depth - 1);
                        } else {
                            assert_eq!(result, Ok(views == 1));
                            // Final release removes the broker object. Never
                            // model subsequent admission to that same entry.
                            if views > 1 {
                                explore(next, views - 1, retired, depth - 1);
                            }
                        }
                    }
                }
            }
        }
        explore(SharedLifetime::new(), 0, false, 9);
    }
}
