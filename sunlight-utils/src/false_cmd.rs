//! POSIX `false` behavior.

/// POSIX requires a non-zero status but does not select a particular value.
/// Sunlight fixes the value at one so callers can verify it exactly.
pub fn run(_args: &[&[u8]]) -> i32 {
    1
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn always_returns_sunlight_failure_status_without_output() {
        for args in [
            &[][..],
            &[b"operand".as_slice()][..],
            &[b"--help".as_slice()][..],
        ] {
            assert_eq!(run(args), 1);
        }
    }
}
