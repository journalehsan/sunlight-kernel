//! Byte-preserving `cmp` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `cmp [-l|-s] file1 file2 [skip1 [skip2]]`
//!
//! Exit status: 0 if identical, 1 if different, >1 on error.
//! Default mode: "file1 file2 differ: char N, line M\n" (N=1-based byte pos).
//! -l: each difference as "%d %o %o\n" (decimal byte, octal byte1, octal byte2).
//! -s: silent, exit status only.
//! Compares incrementally with bounded buffers.

use sunlight_libc::{Errno, Fd, STDERR, STDOUT};

const READ_RETRY_LIMIT: usize = 8;
const BUF_SIZE: usize = 512;

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
    Default,
    Silent,
    Verbose,
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (mode, files, skip1, skip2) = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let fd1 = match io.open(files[0]) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = io.write_stderr(b"cmp: cannot open '");
            let _ = io.write_stderr(files[0]);
            let _ = io.write_stderr(b"': No such file or directory\n");
            return 2;
        }
    };

    let fd2 = match io.open(files[1]) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = io.close(fd1);
            let _ = io.write_stderr(b"cmp: cannot open '");
            let _ = io.write_stderr(files[1]);
            let _ = io.write_stderr(b"': No such file or directory\n");
            return 2;
        }
    };

    let code = cmp_fds(io, fd1, fd2, files[0], files[1], mode, skip1, skip2);
    let _ = io.close(fd1);
    let _ = io.close(fd2);
    code
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Result<(Mode, &'a [&'a [u8]], u64, u64), i32> {
    let mut mode = Mode::Default;
    let mut rest = args;

    while let Some((first, tail)) = rest.split_first() {
        if *first == b"-l" {
            mode = Mode::Verbose;
            rest = tail;
        } else if *first == b"-s" {
            mode = Mode::Silent;
            rest = tail;
        } else if first.starts_with(b"-") && first.len() > 1 {
            let mut consumed = false;
            for &b in &first[1..] {
                match b {
                    b'l' => { mode = Mode::Verbose; consumed = true; }
                    b's' => { mode = Mode::Silent; consumed = true; }
                    _ => {
                        let _ = io.write_stderr(b"cmp: invalid option -- '-");
                        let _ = io.write_stderr(&[b]);
                        let _ = io.write_stderr(b"'\n");
                        return Err(2);
                    }
                }
            }
            if consumed {
                rest = tail;
            } else {
                rest = tail;
            }
        } else {
            break;
        }
    }

    if rest.len() < 2 {
        let _ = io.write_stderr(b"cmp: missing operand\n");
        return Err(2);
    }

    let skip1 = rest.get(2).and_then(|s| parse_u64(s)).unwrap_or(0);
    let skip2 = rest.get(3).and_then(|s| parse_u64(s)).unwrap_or(0);

    Ok((mode, rest, skip1, skip2))
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

fn cmp_fds(
    io: &mut impl Io,
    fd1: Fd, fd2: Fd,
    name1: &[u8], name2: &[u8],
    mode: Mode,
    skip1: u64, skip2: u64,
) -> i32 {
    let mut buf1 = [0u8; BUF_SIZE];
    let mut buf2 = [0u8; BUF_SIZE];
    let mut off1 = 0usize;
    let mut len1 = 0usize;
    let mut off2 = 0usize;
    let mut len2 = 0usize;
    let mut eof1 = false;
    let mut eof2 = false;

    let mut byte_nr: u64 = 1;
    let mut line_nr: u64 = 1;
    let mut skipped1: u64 = 0;
    let mut skipped2: u64 = 0;

    let mut retries = 0;

    loop {
        if off1 >= len1 && !eof1 {
            len1 = match io.read(fd1, &mut buf1) {
                Ok(0) => { eof1 = true; 0 }
                Ok(n) => n,
                Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                    retries += 1;
                    io.yield_now();
                    continue;
                }
                Err(_) => {
                    let _ = io.write_stderr(b"cmp: read error\n");
                    return 2;
                }
            };
            off1 = 0;
            retries = 0;
        }
        if off2 >= len2 && !eof2 {
            len2 = match io.read(fd2, &mut buf2) {
                Ok(0) => { eof2 = true; 0 }
                Ok(n) => n,
                Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                    retries += 1;
                    io.yield_now();
                    continue;
                }
                Err(_) => {
                    let _ = io.write_stderr(b"cmp: read error\n");
                    return 2;
                }
            };
            off2 = 0;
            retries = 0;
        }

        let avail1 = if !eof1 { len1 - off1 } else { 0 };
        let avail2 = if !eof2 { len2 - off2 } else { 0 };

        if avail1 == 0 && avail2 == 0 {
            break;
        }

        let mut i1 = off1;
        let mut i2 = off2;

        while i1 < len1 || i2 < len2 {
            if i1 >= len1 && eof1 && i2 < len2 {
                match mode {
                    Mode::Silent => return 1,
                    Mode::Default => {
                        let _ = write_diff(io, name1, name2, byte_nr, line_nr);
                        return 1;
                    }
                    Mode::Verbose => {
                        let b2 = buf2[i2];
                        let _ = write_verbose_diff(io, byte_nr, 0, b2);
                        return 1;
                    }
                }
            }
            if i2 >= len2 && eof2 && i1 < len1 {
                match mode {
                    Mode::Silent => return 1,
                    Mode::Default => {
                        let _ = write_diff(io, name1, name2, byte_nr, line_nr);
                        return 1;
                    }
                    Mode::Verbose => {
                        let b1 = buf1[i1];
                        let _ = write_verbose_diff(io, byte_nr, b1, 0);
                        return 1;
                    }
                }
            }

            if skipped1 < skip1 {
                if i1 < len1 {
                    if buf1[i1] == b'\n' { line_nr += 1; }
                    i1 += 1;
                    skipped1 += 1;
                } else {
                    break;
                }
                continue;
            }
            if skipped2 < skip2 {
                if i2 < len2 {
                    if buf2[i2] == b'\n' { line_nr += 1; }
                    i2 += 1;
                    skipped2 += 1;
                } else {
                    break;
                }
                continue;
            }

            if i1 >= len1 || i2 >= len2 {
                break;
            }

            let b1 = buf1[i1];
            let b2 = buf2[i2];

            if b1 != b2 {
                match mode {
                    Mode::Silent => return 1,
                    Mode::Default => {
                        let _ = write_diff(io, name1, name2, byte_nr, line_nr);
                        return 1;
                    }
                    Mode::Verbose => {
                        let _ = write_verbose_diff(io, byte_nr, b1, b2);
                    }
                }
            }

            if b1 == b'\n' {
                line_nr += 1;
            }
            i1 += 1;
            i2 += 1;
            byte_nr += 1;
        }

        off1 = i1;
        off2 = i2;
    }

    if mode == Mode::Verbose { return 0; }
    0
}

fn write_diff(io: &mut impl Io, name1: &[u8], name2: &[u8], byte: u64, line: u64) -> Result<(), Errno> {
    io.write_stdout(name1)?;
    io.write_stdout(b" ")?;
    io.write_stdout(name2)?;
    io.write_stdout(b" differ: char ")?;
    write_u64_dec(io, byte)?;
    io.write_stdout(b", line ")?;
    write_u64_dec(io, line)?;
    io.write_stdout(b"\n")
}

fn write_verbose_diff(io: &mut impl Io, byte: u64, b1: u8, b2: u8) -> Result<(), Errno> {
    write_u64_dec(io, byte)?;
    io.write_stdout(b" ")?;
    write_octal(io, b1)?;
    io.write_stdout(b" ")?;
    write_octal(io, b2)?;
    io.write_stdout(b"\n")
}

fn write_u64_dec(io: &mut impl Io, mut v: u64) -> Result<(), Errno> {
    let mut buf = [0u8; 20];
    if v == 0 {
        return io.write_stdout(b"0");
    }
    let mut n = 0;
    while v > 0 && n < buf.len() {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    for i in 0..n / 2 {
        buf.swap(i, n - 1 - i);
    }
    io.write_stdout(&buf[..n])
}

fn write_octal(io: &mut impl Io, v: u8) -> Result<(), Errno> {
    let d2 = b'0' + (v / 64);
    let d1 = b'0' + ((v / 8) % 8);
    let d0 = b'0' + (v % 8);
    io.write_stdout(&[d2, d1, d0])
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
        files: Vec<(&'static [u8], &'static [u8])>,
        output: Vec<u8>,
        errors: Vec<u8>,
        opens: usize,
        closes: usize,
        offsets: Vec<usize>,
        fail_read: bool,
        eagain_reads: usize,
    }

    impl Mock {
        fn new(files: Vec<(&'static [u8], &'static [u8])>) -> Self {
            let fc = files.len();
            Self { files, output: Vec::new(), errors: Vec::new(), opens: 0, closes: 0,
                   offsets: std::vec![0; fc], fail_read: false, eagain_reads: 0 }
        }
    }

    impl Io for Mock {
        fn open(&mut self, path: &[u8]) -> Result<Fd, Errno> {
            self.opens += 1;
            let Some(idx) = self.files.iter().position(|(n, _)| *n == path) else {
                return Err(Errno::NoEntry);
            };
            self.offsets[idx] = 0;
            Ok(Fd(idx as u32 + 3))
        }
        fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
            if self.fail_read { return Err(Errno::Failed); }
            if self.eagain_reads != 0 { self.eagain_reads -= 1; return Err(Errno::Again); }
            let idx = (fd.0 - 3) as usize;
            if idx >= self.files.len() { return Ok(0); }
            let data = self.files[idx].1;
            let off = self.offsets[idx];
            if off >= data.len() { return Ok(0); }
            let n = (data.len() - off).min(buf.len());
            buf[..n].copy_from_slice(&data[off..off + n]);
            self.offsets[idx] += n;
            Ok(n)
        }
        fn close(&mut self, _fd: Fd) -> Result<(), Errno> { self.closes += 1; Ok(()) }
        fn write_stdout(&mut self, b: &[u8]) -> Result<(), Errno> { self.output.extend_from_slice(b); Ok(()) }
        fn write_stderr(&mut self, b: &[u8]) -> Result<(), Errno> { self.errors.extend_from_slice(b); Ok(()) }
        fn yield_now(&mut self) {}
    }

    #[test]
    fn equal_files_exit_0() {
        let mut m = Mock::new(std::vec![(b"a", b"hello\n"), (b"b", b"hello\n")]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        assert_eq!(m.output, b"");
    }

    #[test]
    fn equal_empty_files() {
        let mut m = Mock::new(std::vec![(b"a", b""), (b"b", b"")]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
    }

    #[test]
    fn first_byte_differs() {
        let mut m = Mock::new(std::vec![(b"a", b"abc\n"), (b"b", b"xbc\n")]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 1);
        assert!(m.output.starts_with(b"a b differ: char 1, line 1\n"));
    }

    #[test]
    fn middle_diff() {
        let mut m = Mock::new(std::vec![(b"a", b"hello\nworld\n"), (b"b", b"hello\nWorLd\n")]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 1);
        let s = std::str::from_utf8(&m.output).unwrap();
        assert!(s.contains("differ: char"));
    }

    #[test]
    fn prefix_differ() {
        let mut m = Mock::new(std::vec![(b"a", b"hello"), (b"b", b"hello world")]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 1);
    }

    #[test]
    fn silent_mode() {
        let mut m = Mock::new(std::vec![(b"a", b"x"), (b"b", b"x")]);
        assert_eq!(run(&[b"-s", b"a", b"b"], &mut m), 0);
        let mut m2 = Mock::new(std::vec![(b"a", b"a"), (b"b", b"x")]);
        assert_eq!(run(&[b"-s", b"a", b"b"], &mut m2), 1);
        assert_eq!(m2.output, b"");
    }

    #[test]
    fn missing_file_exit_2() {
        let mut m = Mock::new(std::vec![(b"a", b"x")]);
        assert_eq!(run(&[b"a", b"missing"], &mut m), 2);
    }

    #[test]
    fn large_equal_files() {
        let data: Vec<u8> = (0u8..=255).cycle().take(BUF_SIZE * 3 + 7).collect();
        let data_static: &[u8] = unsafe { std::mem::transmute(data.as_slice()) };
        let mut m = Mock::new(std::vec![(b"a", data_static), (b"b", data_static)]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
    }

    #[test]
    fn large_different_files() {
        let data1: Vec<u8> = (0u8..=255).cycle().take(BUF_SIZE * 3 + 7).collect();
        let mut data2 = data1.clone();
        data2[BUF_SIZE * 2 + 5] ^= 1;
        let s1: &[u8] = unsafe { std::mem::transmute(data1.as_slice()) };
        let s2: &[u8] = unsafe { std::mem::transmute(data2.as_slice()) };
        let mut m = Mock::new(std::vec![(b"a", s1), (b"b", s2)]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 1);
    }

    #[test]
    fn read_error() {
        let mut m = Mock::new(std::vec![(b"a", b"x"), (b"b", b"x")]);
        m.fail_read = true;
        assert_eq!(run(&[b"a", b"b"], &mut m), 2);
    }

    #[test]
    fn eagain_bounded() {
        let mut m = Mock::new(std::vec![(b"a", b"x"), (b"b", b"x")]);
        m.eagain_reads = READ_RETRY_LIMIT + 1;
        assert_eq!(run(&[b"a", b"b"], &mut m), 2);
    }
}
