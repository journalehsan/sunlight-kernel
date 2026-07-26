//! Pure behavior for the POSIX `dirname` utility.

use crate::pathname;

const USAGE: &[u8] = b"dirname: usage: dirname string\n";

pub fn run(
    args: &[&[u8]],
    write_stdout: &mut impl FnMut(&[u8]) -> Result<(), ()>,
    write_stderr: &mut impl FnMut(&[u8]) -> Result<(), ()>,
) -> i32 {
    let operands = if args.first() == Some(&b"--".as_slice()) {
        &args[1..]
    } else {
        args
    };
    if operands.len() != 1 {
        let _ = write_stderr(USAGE);
        return 2;
    }

    let result = match pathname::dirname(operands[0]) {
        Ok(result) => result.bytes,
        Err(_) => {
            let _ = write_stderr(USAGE);
            return 2;
        }
    };
    if write_stdout(result).is_err() || write_stdout(b"\n").is_err() {
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    struct Case<'a> {
        args: &'a [&'a [u8]],
        stdout: &'a [u8],
        stderr: &'a [u8],
        status: i32,
    }

    #[test]
    fn table_matches_lexical_and_argument_rules() {
        let cases = [
            Case {
                args: &[b"/root/projects/sunlight/kernel"],
                stdout: b"/root/projects/sunlight\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"relative/path"],
                stdout: b"relative\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"component"],
                stdout: b".\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"/"],
                stdout: b"/\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"////"],
                stdout: b"/\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"path/to/file/"],
                stdout: b"path/to\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"path/to/file///"],
                stdout: b"path/to\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"a//b///c"],
                stdout: b"a//b\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"//server/share"],
                stdout: b"//server\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"."],
                stdout: b".\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b".."],
                stdout: b".\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"dir/.hidden"],
                stdout: b"dir\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"-x"],
                stdout: b".\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"dir/with spaces/file name"],
                stdout: b"dir/with spaces\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &["répertoire/été".as_bytes()],
                stdout: "répertoire\n".as_bytes(),
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b""],
                stdout: b".\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"--", b"/a/b"],
                stdout: b"/a\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[],
                stdout: b"",
                stderr: USAGE,
                status: 2,
            },
            Case {
                args: &[b"a", b"b"],
                stdout: b"",
                stderr: USAGE,
                status: 2,
            },
            Case {
                args: &[b"--"],
                stdout: b"",
                stderr: USAGE,
                status: 2,
            },
        ];

        for case in cases {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status = run(
                case.args,
                &mut |bytes| {
                    stdout.extend_from_slice(bytes);
                    Ok(())
                },
                &mut |bytes| {
                    stderr.extend_from_slice(bytes);
                    Ok(())
                },
            );
            assert_eq!(stdout, case.stdout);
            assert_eq!(stderr, case.stderr);
            assert_eq!(status, case.status);
        }
    }
}
