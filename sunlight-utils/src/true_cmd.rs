//! POSIX `true` behavior.

/// Return success without producing output or consulting any process service.
/// Extra arguments are ignored as a deliberate Sunlight extension to the
/// no-operand POSIX form; the command remains deterministically successful.
pub fn run(_args: &[&[u8]]) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn always_succeeds_without_output() {
        for args in [
            &[][..],
            &[b"operand".as_slice()][..],
            &[b"--help".as_slice()][..],
        ] {
            assert_eq!(run(args), 0);
        }
    }
}
