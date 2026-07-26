//! Byte-preserving `head` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `head [-n number] [file...]`
//!
//! Default count: 10 lines. Zero or negative count: empty output.
//! Multiple files: each file output preceded by `==> path <==\n`.
//! Stdin is used when no file operand is given (fd 0).
//! Does not decode input as UTF-8; treats input as a byte stream and
//! counts only newline (0x0A) characters.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

const READ_RETRY_LIMIT: usize = 8;
const BUFFER_SIZE: usize = 512;

pub trait Io {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn yield_now(&mut self);
}

/// Remove argv[0] from the raw native argument vector.
pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (limit, files) = match parse_args(args) {
        Ok(v) => v,
        Err(code) => return code,
    };

    if files.is_empty() {
        return head_fd(io, STDIN, None, limit);
    }

    let multiple = files.len() > 1;
    let mut code = 0i32;

    for &path in files {
        let (label, label2) = if multiple {
            (Some(b"==> " as &[u8]), Some(b" <==\n" as &[u8]))
        } else {
            (None, None)
        };

        if let Some(l) = label {
            let _ = io.write_stdout(l);
            let _ = io.write_stdout(path);
            let _ = io.write_stdout(label2.unwrap());
        }

        let fd = match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                let _ = io.write_stderr(b"head: cannot open '");
                let _ = io.write_stderr(path);
                let _ = io.write_stderr(b"' for reading: No such file or directory\n");
                code = 1;
                continue;
            }
        };

        let file_code = head_fd(io, fd, None, limit);
        let _ = io.close(fd);
        if file_code != 0 {
            code = 1;
        }
    }

    code
}

fn parse_args<'a>(args: &'a [&'a [u8]]) -> Result<(u64, &'a [&'a [u8]]), i32> {
    let mut limit = 10u64;
    let mut rest = args;

    while let Some((first, tail)) = rest.split_first() {
        if *first == b"-n" {
            if tail.is_empty() {
                let _ = core::str::from_utf8(first);
                return Err(2);
            }
            limit = match parse_u64(tail[0]) {
                Some(n) => n,
                None => {
                    return Err(2);
                }
            };
            rest = &tail[1..];
        } else if first.starts_with(b"-") && first.len() > 1 {
            let _ = first;
            return Err(2);
        } else {
            break;
        }
    }

    Ok((limit, rest))
}

fn parse_u64(slice: &[u8]) -> Option<u64> {
    if slice.is_empty() {
        return None;
    }
    let mut out = 0u64;
    for &b in slice {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(out)
}

fn head_fd(io: &mut impl Io, fd: Fd, _path: Option<&[u8]>, limit: u64) -> i32 {
    if limit == 0 {
        return 0;
    }

    let mut buf = [0u8; BUFFER_SIZE];
    let mut printed_lines = 0u64;
    let mut retries = 0;

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) if n <= buf.len() => {
                let chunk = &buf[..n];
                let mut write_to = n;

                let mut newlines_in_chunk = 0u64;
                let mut last_nl_idx = None;
                for (i, &b) in chunk.iter().enumerate() {
                    if b == b'\n' {
                        newlines_in_chunk += 1;
                        if printed_lines + newlines_in_chunk >= limit {
                            write_to = i + 1;
                            last_nl_idx = Some(i);
                            break;
                        }
                        last_nl_idx = Some(i);
                    }
                }

                if let Some(_) = last_nl_idx {
                    let to_write = &chunk[..write_to];
                    let _ = io.write_stdout(to_write);
                    printed_lines += newlines_in_chunk;
                    if printed_lines >= limit {
                        break;
                    }
                } else {
                    // No newlines in this chunk, write everything
                    let _ = io.write_stdout(chunk);
                }
                retries = 0;
            }
            Ok(_) => {
                return 1;
            }
            Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                retries += 1;
                io.yield_now();
            }
            Err(_) => {
                let _ = io.write_stderr(b"head: read error\n");
                return 1;
            }
        }
    }

    0
}

pub struct NativeIo;

impl Io for NativeIo {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno> {
        sunlight_libc::open(path)
    }

    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
        sunlight_libc::read(fd, buf)
    }

    fn close(&mut self, fd: Fd) -> Result<(), Errno> {
        sunlight_libc::close(fd)
    }

    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(STDOUT, bytes)
    }

    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(STDERR, bytes)
    }

    fn yield_now(&mut self) {
        sunlight_libc::yield_now();
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    static EMPTY: [u8; 0] = [];
    static ONE_BYTE: [u8; 1] = [b'a'];
    static ONE_LINE_NL: [u8; 9] = *b"hello\n\0\0\0";
    static TEN_LINES: [u8; 100] = {
        let mut a = [0u8; 100];
        let mut i = 0;
        while i < 10 {
            a[i * 2] = b'A' + i as u8;
            a[i * 2 + 1] = b'\n';
            i += 1;
        }
        a
    };
    static NO_FINAL_NL: [u8; 5] = *b"hello";
    static LONG_LINE: [u8; 1024] = {
        let mut a = [0u8; 1024];
        let mut i = 0;
        while i < 1023 {
            a[i] = b'x';
            i += 1;
        }
        a[1023] = b'\n';
        a
    };
    static NUL_BYTES: [u8; 6] = [b'a', 0, b'b', b'\n', b'c', 0];
    static CONSECUTIVE_NLS: [u8; 4] = *b"\n\n\n\n";

    struct Mock {
        files: Vec<(&'static [u8], &'static [u8])>,
        output: Vec<u8>,
        errors: Vec<u8>,
        opens: usize,
        closes: usize,
        fail_read: bool,
        fail_write: bool,
        offsets: Vec<usize>,
        eagain_reads: usize,
        stdin_data: Option<&'static [u8]>,
        stdin_offset: usize,
    }

    impl Mock {
        fn new(files: Vec<(&'static [u8], &'static [u8])>) -> Self {
            let file_count = files.len();
            Self {
                files,
                output: Vec::new(),
                errors: Vec::new(),
                opens: 0,
                closes: 0,
                fail_read: false,
                fail_write: false,
                offsets: std::vec![0; file_count],
                eagain_reads: 0,
                stdin_data: None,
                stdin_offset: 0,
            }
        }
    }

    impl Io for Mock {
        fn open(&mut self, path: &[u8]) -> Result<Fd, Errno> {
            self.opens += 1;
            let Some(idx) = self.files.iter().position(|(name, _)| *name == path) else {
                return Err(Errno::NoEntry);
            };
            self.offsets[idx] = 0;
            Ok(Fd(idx as u32 + 3))
        }

        fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
            if self.fail_read {
                return Err(Errno::Failed);
            }
            if self.eagain_reads != 0 {
                self.eagain_reads -= 1;
                return Err(Errno::Again);
            }
            if fd == STDIN {
                let data = self.stdin_data.unwrap_or(b"");
                let offset = self.stdin_offset;
                if offset >= data.len() {
                    return Ok(0);
                }
                let n = (data.len() - offset).min(buf.len());
                buf[..n].copy_from_slice(&data[offset..offset + n]);
                self.stdin_offset += n;
                return Ok(n);
            }
            let index = (fd.0 - 3) as usize;
            if index >= self.files.len() {
                return Err(Errno::Failed);
            }
            let data = self.files[index].1;
            let offset = self.offsets[index];
            if offset >= data.len() {
                return Ok(0);
            }
            let n = (data.len() - offset).min(buf.len());
            buf[..n].copy_from_slice(&data[offset..offset + n]);
            self.offsets[index] += n;
            Ok(n)
        }

        fn close(&mut self, _fd: Fd) -> Result<(), Errno> {
            self.closes += 1;
            Ok(())
        }

        fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            if self.fail_write {
                Err(Errno::Failed)
            } else {
                self.output.extend_from_slice(bytes);
                Ok(())
            }
        }

        fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            if self.fail_write {
                Err(Errno::Failed)
            } else {
                self.errors.extend_from_slice(bytes);
                Ok(())
            }
        }

        fn yield_now(&mut self) {}
    }

    #[test]
    fn default_limit_is_10() {
        let data = &TEN_LINES[..20];
        let mut mock = Mock::new(std::vec![(b"f", data)]);
        assert_eq!(run(&[b"f"], &mut mock), 0);
        assert_eq!(mock.output, data);
        assert_eq!(mock.closes, 1);
    }

    #[test]
    fn explicit_line_count() {
        let data = b"a\nb\nc\nd\ne\nf\n";
        let mut mock = Mock::new(std::vec![(b"f", data.as_slice())]);
        assert_eq!(run(&[b"-n", b"3", b"f"], &mut mock), 0);
        assert_eq!(mock.output, b"a\nb\nc\n".to_vec());
    }

    #[test]
    fn zero_lines() {
        let mut mock = Mock::new(std::vec![(b"f", b"hello\n")]);
        assert_eq!(run(&[b"-n", b"0", b"f"], &mut mock), 0);
        assert_eq!(mock.output, b"".to_vec());
        assert_eq!(mock.closes, 1);
    }

    #[test]
    fn count_larger_than_file() {
        let mut mock = Mock::new(std::vec![(b"f", b"abc\n")]);
        assert_eq!(run(&[b"-n", b"100", b"f"], &mut mock), 0);
        assert_eq!(mock.output, b"abc\n".to_vec());
    }

    #[test]
    fn empty_file() {
        let mut mock = Mock::new(std::vec![(b"f", &EMPTY)]);
        assert_eq!(run(&[b"f"], &mut mock), 0);
        assert_eq!(mock.output, b"".to_vec());
    }

    #[test]
    fn file_without_final_newline() {
        let mut mock = Mock::new(std::vec![(b"f", &NO_FINAL_NL)]);
        assert_eq!(run(&[b"f"], &mut mock), 0);
        assert_eq!(mock.output, b"hello".to_vec());
    }

    #[test]
    fn file_with_nul_bytes() {
        let mut mock = Mock::new(std::vec![(b"f", &NUL_BYTES)]);
        assert_eq!(run(&[b"f"], &mut mock), 0);
        assert_eq!(mock.output[..], NUL_BYTES[..]);
    }

    #[test]
    fn long_line_across_buffers() {
        let mut mock = Mock::new(std::vec![(b"f", &LONG_LINE)]);
        assert_eq!(run(&[b"f"], &mut mock), 0);
        assert_eq!(mock.output[..], LONG_LINE[..]);
    }

    #[test]
    fn consecutive_newlines() {
        let mut mock = Mock::new(std::vec![(b"f", &CONSECUTIVE_NLS)]);
        assert_eq!(run(&[b"f"], &mut mock), 0);
        assert_eq!(mock.output[..], CONSECUTIVE_NLS[..]);
    }

    #[test]
    fn multiple_files_with_headers() {
        let mut mock = Mock::new(std::vec![
            (b"a", b"hello\n"),
            (b"b", b"world\n"),
        ]);
        assert_eq!(run(&[b"a", b"b"], &mut mock), 0);
        let expected = b"==> a <==\nhello\n==> b <==\nworld\n".to_vec();
        assert_eq!(mock.output, expected);
        assert_eq!(mock.closes, 2);
    }

    #[test]
    fn missing_file() {
        let mut mock = Mock::new(std::vec![(b"ok", b"ok\n")]);
        assert_eq!(run(&[b"missing"], &mut mock), 1);
        let s = std::str::from_utf8(&mock.errors).unwrap_or("");
        assert!(s.contains("cannot open"));
    }

    #[test]
    fn stdin_when_no_operand() {
        let mut mock = Mock::new(std::vec![]);
        mock.stdin_data = Some(b"alpha\nbeta\n");
        assert_eq!(run(&[], &mut mock), 0);
        assert_eq!(mock.output, b"alpha\nbeta\n".to_vec());
    }

    #[test]
    fn read_error_returns_1() {
        let mut mock = Mock::new(std::vec![(b"x", b"hello\n")]);
        mock.fail_read = true;
        assert_eq!(run(&[b"x"], &mut mock), 1);
        assert_eq!(mock.closes, 1);
    }

    #[test]
    fn stdout_failure_preserves_success() {
        let mut mock = Mock::new(std::vec![(b"x", b"hello\n")]);
        mock.fail_write = true;
        assert_eq!(run(&[b"x"], &mut mock), 0);
        assert_eq!(mock.closes, 1);
    }

    #[test]
    fn eagain_bounded() {
        let mut mock = Mock::new(std::vec![(b"x", b"hello\n")]);
        mock.eagain_reads = READ_RETRY_LIMIT + 1;
        assert_eq!(run(&[b"x"], &mut mock), 1);
        assert_eq!(mock.closes, 1);
    }
}
