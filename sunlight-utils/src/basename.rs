//! Pure behavior for the POSIX `basename` utility.

use crate::pathname;

const USAGE: &[u8] = b"basename: usage: basename string [suffix]\n";

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
    if !(1..=2).contains(&operands.len()) {
        let _ = write_stderr(USAGE);
        return 2;
    }

    let suffix = operands.get(1).copied();
    let result = match pathname::basename(operands[0], suffix) {
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
                stdout: b"kernel\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"/root/projects/sunlight/kernel"],
                stdout: b"kernel\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"relative"],
                stdout: b"relative\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"component"],
                stdout: b"component\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"path/to/file/"],
                stdout: b"file\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"path/to/file///"],
                stdout: b"file\n",
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
                args: &[b"//server/share"],
                stdout: b"share\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b""],
                stdout: b"\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"dir/with spaces/file name"],
                stdout: b"file name\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &["répertoire/été".as_bytes()],
                stdout: "été\n".as_bytes(),
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"file.txt", b".txt"],
                stdout: b"file\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"file.txt", b".md"],
                stdout: b"file.txt\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"file", b"file"],
                stdout: b"file\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"file", b"file-long"],
                stdout: b"file\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b".profile"],
                stdout: b".profile\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"archive.tar.gz", b".gz"],
                stdout: b"archive.tar\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"-x"],
                stdout: b"-x\n",
                stderr: b"",
                status: 0,
            },
            Case {
                args: &[b"--", b"/a/b"],
                stdout: b"b\n",
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
                args: &[b"a", b"b", b"c"],
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
