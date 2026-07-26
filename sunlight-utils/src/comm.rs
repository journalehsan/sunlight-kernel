//! Byte-preserving `comm` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `comm [-123] file1 file2`
//!
//! Reads two sorted files and produces three-column output:
//!   column 1: lines unique to file1
//!   column 2: lines unique to file2
//!   column 3: lines common to both
//!
//! Columns are separated by a single tab.  The -1, -2, -3 options suppress
//! the corresponding column.
//!
//! Comparison uses precise byte ordering (portable/C locale).
//! Does not sort inputs; assumes they are pre-sorted.

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

struct LineReader {
    buf: [u8; BUF_SIZE],
    carry: [u8; MAX_LINE_LEN],
    carry_len: usize,
    eof: bool,
    error: bool,
    line_buf: [u8; MAX_LINE_LEN],
    line_len: usize,
    has_newline: bool,
    needs_read: bool,
}

impl LineReader {
    fn new() -> Self {
        Self {
            buf: [0; BUF_SIZE],
            carry: [0; MAX_LINE_LEN],
            carry_len: 0,
            eof: false,
            error: false,
            line_buf: [0; MAX_LINE_LEN],
            line_len: 0,
            has_newline: false,
            needs_read: true,
        }
    }

    /// Fill internal buffer from fd. Returns true if data is available.
    fn fill(&mut self, io: &mut impl Io, fd: Fd) -> bool {
        if self.eof || self.error {
            return self.carry_len > 0;
        }
        let mut retries = 0;
        loop {
            match io.read(fd, &mut self.buf) {
                Ok(0) => {
                    self.eof = true;
                    return self.carry_len > 0;
                }
                Ok(n) if n <= self.buf.len() => {
                    let end = self.carry_len + n;
                    if end > MAX_LINE_LEN {
                        self.error = true;
                        let _ = io.write_stderr(b"comm: line too long\n");
                        return false;
                    }
                    self.carry[self.carry_len..end].copy_from_slice(&self.buf[..n]);
                    self.carry_len = end;
                    return true;
                }
                Ok(_) => {
                    self.error = true;
                    return false;
                }
                Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                    retries += 1;
                    io.yield_now();
                }
                Err(_) => {
                    self.error = true;
                    let _ = io.write_stderr(b"comm: read error\n");
                    return false;
                }
            }
        }
    }

    /// Get next line. Returns None on EOF or error.
    fn next_line(&mut self, io: &mut impl Io, fd: Fd) -> Option<()> {
        if self.line_len > 0 && !self.needs_read {
            return Some(());
        }

        loop {
            let nl_pos = self.carry[..self.carry_len]
                .iter()
                .position(|&b| b == b'\n');

            match nl_pos {
                Some(nl) => {
                    let line = &self.carry[..nl];
                    let llen = line.len().min(MAX_LINE_LEN);
                    self.line_buf[..llen].copy_from_slice(line);
                    self.line_len = llen;
                    self.has_newline = true;

                    // Remove consumed line including newline
                    let consumed = nl + 1;
                    self.carry.copy_within(consumed..self.carry_len, 0);
                    self.carry_len -= consumed;
                    self.needs_read = false;
                    return Some(());
                }
                None => {
                    if self.eof {
                        if self.carry_len > 0 {
                            let llen = self.carry_len.min(MAX_LINE_LEN);
                            self.line_buf[..llen].copy_from_slice(&self.carry[..llen]);
                            self.line_len = llen;
                            self.has_newline = false;
                            self.carry_len = 0;
                            self.needs_read = false;
                            return Some(());
                        }
                        return None;
                    }
                    if !self.fill(io, fd) {
                        return None;
                    }
                }
            }
        }
    }

    fn current(&self) -> &[u8] {
        &self.line_buf[..self.line_len]
    }
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (suppress, file1, file2) = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let (fd1, fd2) = match open_files(io, file1, file2) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let code = comm_fds(io, fd1, fd2, suppress);
    let _ = io.close(fd1);
    let _ = io.close(fd2);
    code
}

fn parse_args<'a>(
    args: &'a [&'a [u8]],
    io: &mut impl Io,
) -> Result<((bool, bool, bool), &'a [u8], &'a [u8]), i32> {
    let mut s1 = false;
    let mut s2 = false;
    let mut s3 = false;
    let mut rest = args;

    while let Some((first, tail)) = rest.split_first() {
        if *first == b"-1" {
            s1 = true;
            rest = tail;
        } else if *first == b"-2" {
            s2 = true;
            rest = tail;
        } else if *first == b"-3" {
            s3 = true;
            rest = tail;
        } else if first.starts_with(b"-") && first.len() > 1 {
            // Handle combined form like -12 or -123
            let opts = &first[1..];
            let mut valid = true;
            for &b in opts {
                match b {
                    b'1' => s1 = true,
                    b'2' => s2 = true,
                    b'3' => s3 = true,
                    _ => valid = false,
                }
            }
            if !valid {
                let _ = io.write_stderr(b"comm: invalid option\n");
                return Err(1);
            }
            rest = tail;
        } else if *first == b"--" {
            rest = tail;
            break;
        } else {
            break;
        }
    }

    if rest.len() < 2 {
        let _ = io.write_stderr(b"comm: missing operand\nusage: comm [-123] file1 file2\n");
        return Err(1);
    }

    Ok(((s1, s2, s3), rest[0], rest[1]))
}

fn open_files(io: &mut impl Io, path1: &[u8], path2: &[u8]) -> Result<(Fd, Fd), i32> {
    let fd1 = open_one(io, path1)?;
    let fd2 = match open_one(io, path2) {
        Ok(fd) => fd,
        Err(e) => {
            let _ = io.close(fd1);
            return Err(e);
        }
    };
    Ok((fd1, fd2))
}

fn open_one(io: &mut impl Io, path: &[u8]) -> Result<Fd, i32> {
    if path == b"-" {
        return Ok(STDIN);
    }
    io.open(path).map_err(|_| {
        let _ = io.write_stderr(b"comm: cannot open '");
        let _ = io.write_stderr(path);
        let _ = io.write_stderr(b"': No such file or directory\n");
        1
    })
}

fn comm_fds(
    io: &mut impl Io,
    fd1: Fd,
    fd2: Fd,
    suppress: (bool, bool, bool),
) -> i32 {
    let (s1, s2, s3) = suppress;
    let mut r1 = LineReader::new();
    let mut r2 = LineReader::new();

    let mut l1 = r1.next_line(io, fd1);
    let mut l2 = r2.next_line(io, fd2);

    if r1.error || r2.error {
        return 1;
    }

    loop {
        match (l1.is_some(), l2.is_some()) {
            (true, true) => {
                let a = r1.current();
                let b = r2.current();
                match compare::byte_cmp(a, b) {
                    core::cmp::Ordering::Less => {
                        if !s1 {
                            let _ = io.write_stdout(a);
                            if r1.has_newline {
                                let _ = io.write_stdout(b"\n");
                            }
                        }
                        l1 = r1.next_line(io, fd1);
                    }
                    core::cmp::Ordering::Greater => {
                        if !s2 {
                            if !s1 {
                                let _ = io.write_stdout(b"\t");
                            }
                            let _ = io.write_stdout(b);
                            if r2.has_newline {
                                let _ = io.write_stdout(b"\n");
                            }
                        }
                        l2 = r2.next_line(io, fd2);
                    }
                    core::cmp::Ordering::Equal => {
                        if !s3 {
                            if !s1 {
                                let _ = io.write_stdout(b"\t");
                            }
                            if !s2 {
                                let _ = io.write_stdout(b"\t");
                            }
                            let _ = io.write_stdout(a);
                            if r1.has_newline {
                                let _ = io.write_stdout(b"\n");
                            }
                        }
                        l1 = r1.next_line(io, fd1);
                        l2 = r2.next_line(io, fd2);
                    }
                }
            }
            (true, false) => {
                while l1.is_some() {
                    let a = r1.current();
                    if !s1 {
                        let _ = io.write_stdout(a);
                        if r1.has_newline {
                            let _ = io.write_stdout(b"\n");
                        }
                    }
                    l1 = r1.next_line(io, fd1);
                }
                break;
            }
            (false, true) => {
                while l2.is_some() {
                    let b = r2.current();
                    if !s2 {
                        if !s1 {
                            let _ = io.write_stdout(b"\t");
                        }
                        let _ = io.write_stdout(b);
                        if r2.has_newline {
                            let _ = io.write_stdout(b"\n");
                        }
                    }
                    l2 = r2.next_line(io, fd2);
                }
                break;
            }
            (false, false) => break,
        }

        if r1.error || r2.error {
            return 1;
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

    struct Mock {
        files: std::collections::HashMap<Vec<u8>, (Vec<u8>, usize)>,
        output: Vec<u8>,
        errors: Vec<u8>,
        fail_read: bool,
        eagain_count: usize,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                files: std::collections::HashMap::new(),
                output: Vec::new(),
                errors: Vec::new(),
                fail_read: false,
                eagain_count: 0,
            }
        }

        fn add_file(&mut self, path: &[u8], data: &[u8]) {
            self.files.insert(path.to_vec(), (data.to_vec(), 0));
        }
    }

    impl Io for Mock {
        fn open(&mut self, path: &[u8]) -> Result<Fd, Errno> {
            if self.files.contains_key(path) {
                Ok(Fd(1))
            } else if path == b"-" {
                Ok(Fd(0))
            } else {
                Err(Errno::NoEntry)
            }
        }
        fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
            if self.fail_read {
                return Err(Errno::Failed);
            }
            if self.eagain_count > 0 {
                self.eagain_count -= 1;
                return Err(Errno::Again);
            }
            // Find the file by fd (simplified: fd 1 = first, fd 2 = second)
            let idx = fd.0 as usize;
            let keys: Vec<Vec<u8>> = self.files.keys().cloned().collect();
            if idx >= keys.len() {
                return Ok(0);
            }
            let key = &keys[idx];
            let (data, offset) = self.files.get_mut(key).unwrap();
            if *offset >= data.len() {
                return Ok(0);
            }
            let end = (*offset + buf.len()).min(data.len());
            let n = end - *offset;
            buf[..n].copy_from_slice(&data[*offset..end]);
            *offset = end;
            Ok(n)
        }
        fn close(&mut self, _fd: Fd) -> Result<(), Errno> {
            Ok(())
        }
        fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            self.output.extend_from_slice(bytes);
            Ok(())
        }
        fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            self.errors.extend_from_slice(bytes);
            Ok(())
        }
        fn yield_now(&mut self) {}
    }

    #[test]
    fn empty_files() {
        let mut m = Mock::new();
        m.add_file(b"a", b"");
        m.add_file(b"b", b"");
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        assert!(m.output.is_empty());
    }

    #[test]
    fn identical_files() {
        let mut m = Mock::new();
        m.add_file(b"a", b"apple\nbanana\n");
        m.add_file(b"b", b"apple\nbanana\n");
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        // Column 3 only (no suppression)
        assert_eq!(m.output, b"\t\tapple\n\t\tbanana\n");
    }

    #[test]
    fn disjoint_files() {
        let mut m = Mock::new();
        m.add_file(b"a", b"apple\n");
        m.add_file(b"b", b"banana\n");
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        assert_eq!(m.output, b"apple\n\tbanana\n");
    }

    #[test]
    fn suppress_col1() {
        let mut m = Mock::new();
        m.add_file(b"a", b"a\nb\nc\n");
        m.add_file(b"b", b"b\nc\nd\n");
        assert_eq!(run(&[b"-1", b"a", b"b"], &mut m), 0);
        // Column 1 suppressed: "a" hidden, "b" and "c" show as col2, "d" as col2
        assert_eq!(m.output, b"\t\tb\n\t\tc\n\td\n");
    }

    #[test]
    fn suppress_col2() {
        let mut m = Mock::new();
        m.add_file(b"a", b"a\nb\nc\n");
        m.add_file(b"b", b"b\nc\nd\n");
        assert_eq!(run(&[b"-2", b"a", b"b"], &mut m), 0);
        assert_eq!(m.output, b"a\n\tb\n\tc\n");
    }

    #[test]
    fn suppress_col3() {
        let mut m = Mock::new();
        m.add_file(b"a", b"a\nb\nc\n");
        m.add_file(b"b", b"b\nc\nd\n");
        assert_eq!(run(&[b"-3", b"a", b"b"], &mut m), 0);
        assert_eq!(m.output, b"a\n\td\n");
    }

    #[test]
    fn suppress_combined() {
        let mut m = Mock::new();
        m.add_file(b"a", b"a\nb\nc\n");
        m.add_file(b"b", b"b\nc\nd\n");
        // -13: show only column 2
        assert_eq!(run(&[b"-13", b"a", b"b"], &mut m), 0);
        assert_eq!(m.output, b"d\n");
    }

    #[test]
    fn no_final_newline() {
        let mut m = Mock::new();
        m.add_file(b"a", b"x");
        m.add_file(b"b", b"x");
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        assert_eq!(m.output, b"\t\tx");
    }

    #[test]
    fn one_empty_file() {
        let mut m = Mock::new();
        m.add_file(b"a", b"a\nb\n");
        m.add_file(b"b", b"");
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        assert_eq!(m.output, b"a\nb\n");
    }
}
