//! Byte-preserving `uniq` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `uniq [-c|-d|-u] [-f fields] [-s chars] [input [output]]`
//!
//! Reads the input file (or stdin) and writes to output (or stdout),
//! suppressing adjacent duplicate lines.
//!
//! -c : prefix each output line with its occurrence count
//! -d : output only repeated lines (one copy per duplicate run)
//! -u : output only non-repeated (unique) lines
//! -f N : skip N fields before comparison
//! -s N : skip N characters before comparison
//!
//! Comparison uses precise byte equality (portable/C locale).
//! Adjacent-only: does not deduplicate globally.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

use crate::compare;

const READ_RETRY_LIMIT: usize = 8;
const BUF_SIZE: usize = 512;
const MAX_LINE_LEN: usize = 4096;

pub trait Io {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn yield_now(&mut self);
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    All,
    Count,
    Repeated,
    Unique,
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (mode, skip_fields, skip_chars, input_path, output_path) = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let in_fd = match open_input(io, input_path) {
        Ok(fd) => fd,
        Err(code) => return code,
    };
    let out_fd = match open_output(io, output_path) {
        Ok(fd) => fd,
        Err(code) => {
            let _ = io.close(in_fd);
            return code;
        }
    };

    let code = uniq_fd(io, in_fd, out_fd, mode, skip_fields, skip_chars);
    let _ = io.close(in_fd);
    if out_fd != STDOUT {
        let _ = io.close(out_fd);
    }
    code
}

fn open_input(io: &mut impl Io, path: Option<&[u8]>) -> Result<Fd, i32> {
    match path {
        None | Some(b"-") => Ok(STDIN),
        Some(p) => io.open(p).map_err(|_| {
            let _ = io.write_stderr(b"uniq: cannot open '");
            let _ = io.write_stderr(p);
            let _ = io.write_stderr(b"': No such file or directory\n");
            1
        }),
    }
}

fn open_output(io: &mut impl Io, path: Option<&[u8]>) -> Result<Fd, i32> {
    match path {
        None => Ok(STDOUT),
        Some(p) => {
            let fd = sunlight_libc::open_with_flags_mode(
                p,
                sunlight_libc::O_WRONLY | sunlight_libc::O_CREAT | sunlight_libc::O_TRUNC,
                0o644,
            )
            .map_err(|_| {
                let _ = io.write_stderr(b"uniq: cannot create '");
                let _ = io.write_stderr(p);
                let _ = io.write_stderr(b"'\n");
                1
            })?;
            Ok(fd)
        }
    }
}

fn parse_args<'a>(
    args: &'a [&'a [u8]],
    io: &mut impl Io,
) -> Result<(Mode, usize, usize, Option<&'a [u8]>, Option<&'a [u8]>), i32> {
    let mut mode = Mode::All;
    let mut skip_fields: usize = 0;
    let mut skip_chars: usize = 0;
    let mut rest = args;

    while let Some((first, tail)) = rest.split_first() {
        if *first == b"-c" {
            mode = Mode::Count;
            rest = tail;
        } else if *first == b"-d" {
            mode = Mode::Repeated;
            rest = tail;
        } else if *first == b"-u" {
            mode = Mode::Unique;
            rest = tail;
        } else if *first == b"-f" {
            if tail.is_empty() {
                let _ = io.write_stderr(b"uniq: option requires an argument -- 'f'\n");
                return Err(1);
            }
            skip_fields = parse_usize(tail[0]).map_err(|_| {
                let _ = io.write_stderr(b"uniq: invalid number of fields to skip\n");
                1i32
            })?;
            rest = &tail[1..];
        } else if *first == b"-s" {
            if tail.is_empty() {
                let _ = io.write_stderr(b"uniq: option requires an argument -- 's'\n");
                return Err(1);
            }
            skip_chars = parse_usize(tail[0]).map_err(|_| {
                let _ = io.write_stderr(b"uniq: invalid number of characters to skip\n");
                1i32
            })?;
            rest = &tail[1..];
        } else if first.starts_with(b"-") && first.len() > 1 {
            if *first == b"--" {
                rest = tail;
                break;
            }
            let _ = io.write_stderr(b"uniq: invalid option\n");
            return Err(1);
        } else {
            break;
        }
    }

    let (input, output) = match rest.len() {
        0 => (None, None),
        1 => (Some(rest[0]), None),
        _ => (Some(rest[0]), Some(rest[1])),
    };

    Ok((mode, skip_fields, skip_chars, input, output))
}

fn parse_usize(s: &[u8]) -> Result<usize, ()> {
    if s.is_empty() {
        return Err(());
    }
    let mut v = 0usize;
    for &b in s {
        if !b.is_ascii_digit() {
            return Err(());
        }
        v = v.checked_mul(10).ok_or(())?;
        v = v.checked_add((b - b'0') as usize).ok_or(())?;
    }
    Ok(v)
}

fn uniq_fd(
    io: &mut impl Io,
    in_fd: Fd,
    out_fd: Fd,
    mode: Mode,
    skip_fields: usize,
    skip_chars: usize,
) -> i32 {
    let mut buf = [0u8; BUF_SIZE];
    let mut carry = [0u8; MAX_LINE_LEN];
    let mut carry_len: usize = 0;

    // Previous line (key portion for comparison, full for output)
    let mut prev_key: Option<[u8; MAX_LINE_LEN]> = None;
    let mut prev_key_len: usize = 0;
    let mut prev_full: [u8; MAX_LINE_LEN] = [0; MAX_LINE_LEN];
    let mut prev_full_len: usize = 0;
    let mut prev_has_newline: bool = false;
    let mut count: u64 = 0;

    let mut retries = 0;
    let mut write_buf = [0u8; 64];

    loop {
        match io.read(in_fd, &mut buf) {
            Ok(0) => break, // EOF
            Ok(n) if n <= buf.len() => {
                let end = carry_len + n;
                if end > MAX_LINE_LEN {
                    let _ = io.write_stderr(b"uniq: line too long\n");
                    return 1;
                }
                carry[carry_len..end].copy_from_slice(&buf[..n]);
                let mut pos = carry_len;
                carry_len = end;

                while pos < carry_len {
                    let nl = match carry[pos..carry_len].iter().position(|&b| b == b'\n') {
                        Some(off) => pos + off,
                        None => {
                            // No newline found; carry data forward
                            if pos > 0 {
                                let rem = carry_len - pos;
                                carry.copy_within(pos..carry_len, 0);
                                carry_len = rem;
                            } else if carry_len >= MAX_LINE_LEN {
                                let _ = io.write_stderr(b"uniq: line too long\n");
                                return 1;
                            }
                            break;
                        }
                    };

                    let line = &carry[pos..nl];
                    let has_nl = true;

                    // Compute comparison key
                    let key_start = if skip_fields > 0 || skip_chars > 0 {
                        let after_fields = if skip_fields > 0 {
                            compare::skip_fields(line, skip_fields, b" \t")
                        } else {
                            0
                        };
                        let key_off = if skip_chars > 0 {
                            after_fields + compare::skip_chars(&line[after_fields..], skip_chars)
                        } else {
                            after_fields
                        };
                        key_off.min(line.len())
                    } else {
                        0
                    };
                    let key = &line[key_start..];

                    let is_match = match &prev_key {
                        Some(pk) => {
                            let pk_slice = &pk[..prev_key_len];
                            key.len() == pk_slice.len() && key == pk_slice
                        }
                        None => false,
                    };

                    if is_match {
                        count += 1;
                    } else {
                        // Flush previous run
                        if count > 0 {
                            flush_run(
                                io,
                                out_fd,
                                mode,
                                count,
                                &prev_full[..prev_full_len],
                                prev_has_newline,
                                &mut write_buf,
                            );
                        }
                        // Start new run
                        prev_key_len = key.len().min(MAX_LINE_LEN);
                        let mut tmp = [0u8; MAX_LINE_LEN];
                        tmp[..prev_key_len].copy_from_slice(key);
                        prev_key = Some(tmp);
                        prev_full[..line.len()].copy_from_slice(line);
                        prev_full_len = line.len();
                        prev_has_newline = has_nl;
                        count = 1;
                    }

                    pos = nl + 1;
                }

                // After processing all complete lines, carry remainder
                if pos >= carry_len {
                    carry_len = 0;
                }
                retries = 0;
            }
            Ok(_) => return 1,
            Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                retries += 1;
                io.yield_now();
            }
            Err(_) => {
                let _ = io.write_stderr(b"uniq: read error\n");
                return 1;
            }
        }
    }

    // Handle final partial line (no newline)
    if carry_len > 0 {
        let line = &carry[..carry_len];
        let key_start = if skip_fields > 0 || skip_chars > 0 {
            let after_fields = if skip_fields > 0 {
                compare::skip_fields(line, skip_fields, b" \t")
            } else {
                0
            };
            let key_off = if skip_chars > 0 {
                after_fields + compare::skip_chars(&line[after_fields..], skip_chars)
            } else {
                after_fields
            };
            key_off.min(line.len())
        } else {
            0
        };
        let key = &line[key_start..];

        let is_match = match &prev_key {
            Some(pk) => {
                let pk_slice = &pk[..prev_key_len];
                key.len() == pk_slice.len() && key == pk_slice
            }
            None => false,
        };

        if is_match {
            count += 1;
            if count > prev_full_len as u64 {
                // Update previous full line to this one (unusual: different line same key)
                prev_full[..line.len()].copy_from_slice(line);
                prev_full_len = line.len();
                prev_has_newline = false;
            }
        } else {
            if count > 0 {
                flush_run(
                    io,
                    out_fd,
                    mode,
                    count,
                    &prev_full[..prev_full_len],
                    prev_has_newline,
                    &mut write_buf,
                );
            }
            let mut tmp = [0u8; MAX_LINE_LEN];
            let klen = key.len().min(MAX_LINE_LEN);
            tmp[..klen].copy_from_slice(key);
            prev_key = Some(tmp);
            prev_key_len = klen;
            prev_full[..line.len()].copy_from_slice(line);
            prev_full_len = line.len();
            prev_has_newline = false;
            count = 1;
        }
    }

    // Flush final run
    if count > 0 {
        flush_run(
            io,
            out_fd,
            mode,
            count,
            &prev_full[..prev_full_len],
            prev_has_newline,
            &mut write_buf,
        );
    }

    0
}

fn flush_run(
    io: &mut impl Io,
    out_fd: Fd,
    mode: Mode,
    count: u64,
    line: &[u8],
    has_newline: bool,
    write_buf: &mut [u8; 64],
) {
    match mode {
        Mode::All => {
            let _ = write_fd(io, out_fd, line);
            if has_newline {
                let _ = write_fd(io, out_fd, b"\n");
            }
        }
        Mode::Count => {
            let prefix = format_count(count, write_buf);
            let _ = write_fd(io, out_fd, prefix);
            let _ = write_fd(io, out_fd, line);
            if has_newline {
                let _ = write_fd(io, out_fd, b"\n");
            }
        }
        Mode::Repeated => {
            if count > 1 {
                let _ = write_fd(io, out_fd, line);
                if has_newline {
                    let _ = write_fd(io, out_fd, b"\n");
                }
            }
        }
        Mode::Unique => {
            if count == 1 {
                let _ = write_fd(io, out_fd, line);
                if has_newline {
                    let _ = write_fd(io, out_fd, b"\n");
                }
            }
        }
    }
}

fn format_count<'a>(count: u64, buf: &'a mut [u8; 64]) -> &'a [u8] {
    let mut n = 0usize;
    let mut v = count;
    loop {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    // reverse
    let end = n;
    for i in 0..end / 2 {
        buf.swap(i, end - 1 - i);
    }
    buf[end] = b' ';
    &buf[..end + 1]
}

fn write_fd(io: &mut impl Io, fd: Fd, bytes: &[u8]) -> Result<(), Errno> {
    if fd == STDOUT {
        io.write_stdout(bytes)
    } else {
        write_fd_raw(fd, bytes)
    }
}

fn write_fd_raw(fd: Fd, mut data: &[u8]) -> Result<(), Errno> {
    while !data.is_empty() {
        match sunlight_libc::write(fd, data) {
            Ok(n) => data = &data[n.min(data.len())..],
            Err(Errno::Again) => sunlight_libc::yield_now(),
            Err(e) => return Err(e),
        }
    }
    Ok(())
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

    struct Mock {
        files: Vec<(Vec<u8>, Vec<u8>)>,
        output: Vec<u8>,
        errors: Vec<u8>,
        created: Vec<Vec<u8>>,
        offsets: Vec<usize>,
        fail_read: bool,
        write_fail: bool,
        eagain_count: usize,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                files: Vec::new(),
                output: Vec::new(),
                errors: Vec::new(),
                created: Vec::new(),
                offsets: Vec::new(),
                fail_read: false,
                write_fail: false,
                eagain_count: 0,
            }
        }

        fn add_stdin(&mut self, data: &[u8]) {
            self.files.push((b"".to_vec(), data.to_vec()));
            self.offsets.push(0);
        }

        fn add_file(&mut self, path: &[u8], data: &[u8]) {
            self.files.push((path.to_vec(), data.to_vec()));
            self.offsets.push(0);
        }
    }

    impl Io for Mock {
        fn open(&mut self, path: &[u8]) -> Result<Fd, Errno> {
            for (i, (p, _)) in self.files.iter().enumerate() {
                if p == path {
                    return Ok(Fd(i as u32));
                }
            }
            Ok(Fd(self.files.len() as u32 + 10))
        }
        fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
            if self.fail_read {
                return Err(Errno::Failed);
            }
            if self.eagain_count > 0 {
                self.eagain_count -= 1;
                return Err(Errno::Again);
            }
            let idx = fd.0 as usize;
            if idx >= self.files.len() {
                return Ok(0);
            }
            let offset = self.offsets.get(idx).copied().unwrap_or(0);
            let data = &self.files[idx].1;
            if offset >= data.len() {
                return Ok(0);
            }
            let end = (offset + buf.len()).min(data.len());
            let n = end - offset;
            buf[..n].copy_from_slice(&data[offset..end]);
            self.offsets[idx] = end;
            Ok(n)
        }
        fn close(&mut self, _fd: Fd) -> Result<(), Errno> {
            Ok(())
        }
        fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            if self.write_fail {
                return Err(Errno::Failed);
            }
            self.output.extend_from_slice(bytes);
            Ok(())
        }
        fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            self.errors.extend_from_slice(bytes);
            Ok(())
        }
        fn yield_now(&mut self) {}
    }

    // Override open_output for mock tests
    fn run_mock(args: &[&[u8]], mock: &mut Mock) -> i32 {
        let (mode, skip_fields, skip_chars, input_path, _output_path) = match parse_args(args, mock)
        {
            Ok(v) => v,
            Err(code) => return code,
        };

        let in_fd = match open_input(mock, input_path) {
            Ok(fd) => fd,
            Err(code) => return code,
        };

        uniq_fd(mock, in_fd, STDOUT, mode, skip_fields, skip_chars)
    }

    #[test]
    fn empty_input() {
        let mut m = Mock::new();
        m.add_stdin(b"");
        assert_eq!(run_mock(&[], &mut m), 0);
        assert!(m.output.is_empty());
    }

    #[test]
    fn single_line() {
        let mut m = Mock::new();
        m.add_stdin(b"hello\n");
        assert_eq!(run_mock(&[], &mut m), 0);
        assert_eq!(m.output, b"hello\n");
    }

    #[test]
    fn two_identical() {
        let mut m = Mock::new();
        m.add_stdin(b"hello\nhello\n");
        assert_eq!(run_mock(&[], &mut m), 0);
        assert_eq!(m.output, b"hello\n");
    }

    #[test]
    fn non_adjacent_not_merged() {
        let mut m = Mock::new();
        m.add_stdin(b"a\nb\na\n");
        assert_eq!(run_mock(&[], &mut m), 0);
        assert_eq!(m.output, b"a\nb\na\n");
    }

    #[test]
    fn count_option() {
        let mut m = Mock::new();
        m.add_stdin(b"a\na\nb\n");
        assert_eq!(run_mock(&[b"-c"], &mut m), 0);
        assert_eq!(m.output, b"   2 a\n   1 b\n");
    }

    #[test]
    fn repeated_only() {
        let mut m = Mock::new();
        m.add_stdin(b"a\na\nb\nc\nc\nc\nd\n");
        assert_eq!(run_mock(&[b"-d"], &mut m), 0);
        assert_eq!(m.output, b"a\nc\n");
    }

    #[test]
    fn unique_only() {
        let mut m = Mock::new();
        m.add_stdin(b"a\na\nb\nc\nc\n");
        assert_eq!(run_mock(&[b"-u"], &mut m), 0);
        assert_eq!(m.output, b"b\n");
    }

    #[test]
    fn skip_fields() {
        let mut m = Mock::new();
        m.add_stdin(b"1 a\n2 a\n3 b\n");
        // skip first field (the number), compare by second field
        assert_eq!(run_mock(&[b"-f", b"1"], &mut m), 0);
        assert_eq!(m.output, b"1 a\n3 b\n");
    }

    #[test]
    fn no_final_newline() {
        let mut m = Mock::new();
        m.add_stdin(b"a\na");
        assert_eq!(run_mock(&[], &mut m), 0);
        assert_eq!(m.output, b"a");
    }

    #[test]
    fn empty_lines() {
        let mut m = Mock::new();
        m.add_stdin(b"\n\n\nb\n");
        assert_eq!(run_mock(&[], &mut m), 0);
        assert_eq!(m.output, b"\nb\n");
    }
}
