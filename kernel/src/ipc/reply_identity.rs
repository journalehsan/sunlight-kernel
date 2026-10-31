//! Exact publication identity. Endpoint IDs and call generations are distinct
//! checked, non-recycled allocations; PID alone is never a completion key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplyIdentity {
    pub(crate) pid: usize,
    pub(crate) incarnation: u64,
    pub(crate) endpoint: u32,
    pub(crate) generation: u64,
}

pub(crate) fn publication_allowed(
    target: ReplyIdentity,
    pending: Option<ReplyIdentity>,
    exiting: bool,
    terminal: bool,
) -> bool {
    !exiting && !terminal && pending == Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_identity_component_is_required() {
        let target = ReplyIdentity {
            pid: 4,
            incarnation: 8,
            endpoint: 12,
            generation: 7,
        };
        assert!(publication_allowed(target, Some(target), false, false));
        for pending in [
            ReplyIdentity { pid: 5, ..target },
            ReplyIdentity {
                incarnation: 9,
                ..target
            },
            ReplyIdentity {
                endpoint: 13,
                ..target
            },
            ReplyIdentity {
                generation: 8,
                ..target
            },
        ] {
            assert!(!publication_allowed(target, Some(pending), false, false));
        }
        assert!(!publication_allowed(target, None, false, false));
        assert!(!publication_allowed(target, Some(target), true, false));
        assert!(!publication_allowed(target, Some(target), false, true));
    }
    #[test]
    fn closure_and_exec_withdrawal_fence_late_publication() {
        let target = ReplyIdentity {
            pid: 4,
            incarnation: 8,
            endpoint: 12,
            generation: 7,
        };
        // Both closure and successful exec withdraw pending authority. Even
        // consuming the terminal receipt cannot make the old key live again.
        assert!(!publication_allowed(target, None, false, true));
        assert!(!publication_allowed(target, None, false, false));
    }
}
