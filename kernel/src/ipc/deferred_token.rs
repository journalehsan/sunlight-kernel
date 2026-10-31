//! Process-local completion handles. Bit 63 keeps success distinct from errors.

pub(crate) fn next_token(previous: u64) -> Option<u64> {
    if previous == 0 {
        Some((1u64 << 63) | 1)
    } else {
        previous.checked_add(1)
    }
}

#[cfg(test)]
mod tests {
    use super::next_token;

    #[test]
    fn first_and_successive_handles_preserve_wire_encoding() {
        let first = next_token(0).unwrap();
        assert_eq!(first, (1 << 63) | 1);
        assert_eq!(next_token(first), Some(first + 1));
    }

    #[test]
    fn final_handle_is_available_once() {
        assert_eq!(next_token(u64::MAX - 1), Some(u64::MAX));
        assert_eq!(next_token(u64::MAX), None);
    }

    #[test]
    fn exhaustion_never_reissues_a_stale_handle() {
        let stale = next_token(0).unwrap();
        assert_ne!(next_token(u64::MAX), Some(stale));
        for _ in 0..64 {
            assert_eq!(next_token(u64::MAX), None);
        }
    }
}
