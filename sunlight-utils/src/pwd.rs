//! Byte-preserving `pwd` behavior for the native libc utility.

use sunlight_libc::{Errno, MAX_PATH};

/// `pwd` historically ignores all arguments and prints the process path with
/// one trailing newline. The kernel owns CWD resolution and path spelling.
pub fn run(
    _args: &[&[u8]],
    getcwd: &mut impl FnMut(&mut [u8; MAX_PATH]) -> Result<usize, Errno>,
    write_stdout: &mut impl FnMut(&[u8]) -> Result<(), Errno>,
    write_stderr: &mut impl FnMut(&[u8]) -> Result<(), Errno>,
) -> i32 {
    let mut path = [0u8; MAX_PATH];
    let len = match getcwd(&mut path) {
        Ok(len) if len <= path.len() => len,
        _ => {
            let _ = write_stderr(b"pwd: cannot determine current directory\n");
            return 1;
        }
    };
    let _ = write_stdout(&path[..len]);
    let _ = write_stdout(b"\n");
    0
}

/// Remove argv[0] from the raw native argument vector.
pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    #[test]
    fn preserves_root_and_nested_path_bytes() {
        for expected in [b"/".as_slice(), b"/etc/sunlight".as_slice()] {
            let mut output = Vec::new();
            let status = run(
                &[],
                &mut |buf| {
                    buf[..expected.len()].copy_from_slice(expected);
                    Ok(expected.len())
                },
                &mut |bytes| {
                    output.extend_from_slice(bytes);
                    Ok(())
                },
                &mut |_| Ok(()),
            );
            assert_eq!(status, 0);
            let mut wanted = expected.to_vec();
            wanted.push(b'\n');
            assert_eq!(output, wanted);
        }
    }

    #[test]
    fn ignores_arguments_and_reports_cwd_failure() {
        let mut errors = Vec::new();
        let status = run(
            &[b"-P", b"ignored"],
            &mut |_| Err(Errno::Failed),
            &mut |_| Ok(()),
            &mut |bytes| {
                errors.extend_from_slice(bytes);
                Ok(())
            },
        );
        assert_eq!(status, 1);
        assert_eq!(errors, b"pwd: cannot determine current directory\n".to_vec());
    }
}
