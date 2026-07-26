//! Byte-preserving `cat` behavior for the native libc utility.

use sunlight_libc::{Errno, Fd, STDERR, STDOUT};

const READ_RETRY_LIMIT: usize = 8;

/// The small filesystem/output seam keeps command behavior testable without
/// linking target-only syscall instructions into host tests.
pub trait Io {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn yield_now(&mut self);
}

/// Preserve the existing utility semantics: no options, no stdin shorthand,
/// process files in order, stop at the first open/read failure, preserve raw
/// file bytes, and ignore stdout/close failures in the command status.
pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    if args.is_empty() {
        let _ = io.write_stdout(b"cat: missing file operand\n");
        return 1;
    }

    for &path in args {
        let fd = match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                report_stderr(io, b"cat: cannot open ", path);
                return 1;
            }
        };

        let mut buf = [0u8; 512];
        let mut retries = 0;
        let result = loop {
            match io.read(fd, &mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) if n <= buf.len() => {
                    let _ = io.write_stdout(&buf[..n]);
                    retries = 0;
                }
                Ok(_) => break Err(()),
                Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                    retries += 1;
                    io.yield_now();
                }
                Err(_) => break Err(()),
            }
        };

        let _ = io.close(fd);
        if result.is_err() {
            report_stderr(io, b"cat: read error on ", path);
            return 1;
        }
    }

    0
}

fn report_stderr(io: &mut impl Io, prefix: &[u8], path: &[u8]) {
    let _ = io.write_stderr(prefix);
    let _ = io.write_stderr(path);
    let _ = io.write_stderr(b"\n");
}

/// Remove argv[0] from the raw native argument vector.
pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

/// Adapter used only by the target binary. All kernel-facing operations stay
/// in sunlight-libc, including the shared partial-write retry loop.
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

    static ONE_BYTE: [u8; 1] = [b'a'];
    static BELOW_BUFFER: [u8; 511] = [b'b'; 511];
    static EXACT_BUFFER: [u8; 512] = [b'c'; 512];
    static ONE_OVER_BUFFER: [u8; 513] = [b'd'; 513];
    static SEVERAL_BUFFERS: [u8; 1537] = [b'e'; 1537];

    struct Mock {
        files: Vec<(&'static [u8], &'static [u8])>,
        output: Vec<u8>,
        errors: Vec<u8>,
        opens: usize,
        closes: usize,
        reads: usize,
        eagain_reads: usize,
        fail_write: bool,
        fail_read: bool,
        offsets: Vec<usize>,
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
                reads: 0,
                eagain_reads: 0,
                fail_write: false,
                fail_read: false,
                offsets: std::vec![0; file_count],
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
            self.reads += 1;
            if self.fail_read {
                return Err(Errno::Failed);
            }
            if self.eagain_reads != 0 {
                self.eagain_reads -= 1;
                return Err(Errno::Again);
            }
            let index = (fd.0 - 3) as usize;
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
            if !self.fail_write {
                self.output.extend_from_slice(bytes);
            }
            if self.fail_write {
                Err(Errno::Failed)
            } else {
                Ok(())
            }
        }

        fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            if !self.fail_write {
                self.errors.extend_from_slice(bytes);
            }
            if self.fail_write {
                Err(Errno::Failed)
            } else {
                Ok(())
            }
        }

        fn yield_now(&mut self) {}
    }

    #[test]
    fn missing_operand_matches_legacy_output() {
        let mut mock = Mock::new(std::vec![]);
        assert_eq!(run(&[], &mut mock), 1);
        assert_eq!(mock.output, b"cat: missing file operand\n".to_vec());
        assert_eq!(mock.opens, 0);
    }

    #[test]
    fn streams_multiple_raw_files_without_adding_newlines() {
        let mut mock = Mock::new(std::vec![(b"a", b"A\0"), (b"b", b"B")]);
        assert_eq!(run(&[b"a", b"b", b"a"], &mut mock), 0);
        assert_eq!(mock.output, b"A\0BA\0".to_vec());
        assert_eq!(mock.closes, 3);
    }

    #[test]
    fn streams_files_larger_than_the_fixed_buffer() {
        let mut mock = Mock::new(std::vec![(b"large", &ONE_OVER_BUFFER)]);
        assert_eq!(run(&[b"large"], &mut mock), 0);
        assert_eq!(mock.output.len(), ONE_OVER_BUFFER.len());
        assert_eq!(mock.output.as_slice(), ONE_OVER_BUFFER);
        assert_eq!(mock.closes, 1);
    }

    #[test]
    fn preserves_empty_single_and_exact_boundary_sizes() {
        let cases = [
            (b"empty".as_slice(), b"".as_slice()),
            (b"one".as_slice(), ONE_BYTE.as_slice()),
            (b"below".as_slice(), BELOW_BUFFER.as_slice()),
            (b"exact".as_slice(), EXACT_BUFFER.as_slice()),
            (b"several".as_slice(), SEVERAL_BUFFERS.as_slice()),
        ];

        for (name, data) in cases {
            let mut mock = Mock::new(std::vec![(name, data)]);
            assert_eq!(run(&[name], &mut mock), 0);
            assert_eq!(mock.output.as_slice(), data);
            assert_eq!(mock.closes, 1);
        }
    }

    #[test]
    fn missing_file_stops_before_later_files() {
        let mut mock = Mock::new(std::vec![(b"ok", b"ok")]);
        assert_eq!(run(&[b"ok", b"missing"], &mut mock), 1);
        assert_eq!(mock.opens, 2);
        assert_eq!(mock.closes, 1);
        assert_eq!(mock.output, b"ok".to_vec());
        assert_eq!(mock.errors, b"cat: cannot open missing\n".to_vec());
    }

    #[test]
    fn read_failure_closes_the_file() {
        let mut mock = Mock::new(std::vec![(b"x", b"x")]);
        mock.fail_read = true;
        assert_eq!(run(&[b"x"], &mut mock), 1);
        assert_eq!(mock.closes, 1);
        assert_eq!(mock.output, b"".to_vec());
        assert_eq!(mock.errors, b"cat: read error on x\n".to_vec());
    }

    #[test]
    fn eagain_is_bounded_and_does_not_spin_forever() {
        let mut mock = Mock::new(std::vec![(b"x", b"x")]);
        mock.eagain_reads = READ_RETRY_LIMIT + 1;
        assert_eq!(run(&[b"x"], &mut mock), 1);
        assert_eq!(mock.closes, 1);
    }

    #[test]
    fn stdout_failure_keeps_historical_success_status() {
        let mut mock = Mock::new(std::vec![(b"x", b"x")]);
        mock.fail_write = true;
        assert_eq!(run(&[b"x"], &mut mock), 0);
        assert_eq!(mock.closes, 1);
    }
}
