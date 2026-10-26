//! Preserve max-existing-PID allocation without wrapping or ignoring the
//! exhausted maximum. The caller holds lifecycle serialization through insert.
pub(crate) fn next_pid(pids: impl Iterator<Item = usize>) -> Option<usize> {
    pids.max().unwrap_or(0).checked_add(1)
}

/// Isolated arithmetic boundary, using the real allocator, never a live PID
/// assignment. Runtime evidence comes separately from ordinary spawn/reap.
#[cfg(feature = "production_grant_test")]
pub(crate) fn native_boundary() {
    assert_eq!(next_pid([usize::MAX - 1].into_iter()), Some(usize::MAX));
    assert_eq!(next_pid([usize::MAX, 1].into_iter()), None);
    crate::serial_println!("[GRANT-F] pid-boundary max={} last=accepted exhausted=None rollover=denied scope=isolated-real-allocator", usize::MAX);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_and_exhausted_pid_spaces_never_alias() {
        assert_eq!(next_pid([].into_iter()), Some(1));
        assert_eq!(next_pid([1, 11, 6].into_iter()), Some(12));
        assert_eq!(next_pid([usize::MAX - 1].into_iter()), Some(usize::MAX));
        assert_eq!(next_pid([usize::MAX, usize::MAX - 1].into_iter()), None);
        assert_eq!(next_pid([1, usize::MAX].into_iter()), None);
        // The old filter_map formulation silently ignored the maximum and
        // selected an already occupied PID. This is an arithmetic witness,
        // not an attempt to fabricate a live native process identity.
        let occupied = [usize::MAX, usize::MAX - 1];
        let old = occupied.into_iter().filter_map(|p| p.checked_add(1)).max();
        assert_eq!(old, Some(usize::MAX));
        assert!(occupied.contains(&old.unwrap()));
    }

    #[test]
    fn dynamic_publication_preserves_maximum_even_when_reaped_slot_is_replaced() {
        // Retained reaped rows participate in selection. Overwriting any row
        // with the newly allocated PID preserves a strictly larger maximum.
        for replaced in 0..3 {
            let mut rows = [1, 7, 12];
            let mut previous = 12;
            for _ in 0..128 {
                let allocated = next_pid(rows.iter().copied()).unwrap();
                assert!(allocated > previous);
                rows[replaced] = allocated;
                previous = allocated;
            }
        }
    }
}
