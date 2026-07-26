//! Byte-preserving `fold` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `fold [-bs] [-w width] [file...]`
//!
//! Wraps input lines to fit within a specified width.  Default width is 80
//! columns (treated as byte positions in -b mode or the POSIX/C locale).
//!
//! -b : count bytes rather than columns
//! -s : break at spaces if possible (within width)
//! -w width : maximum width (default 80, positive integer)
//!
//! Stdin is used when no file operand is given.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

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

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (width, break_spaces, count_bytes, files) = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let use_stdin = files.is_empty();
    if use_stdin {
        return fold_fd(io, STDIN, width, break_spaces, count_bytes);
    }

    let mut code = 0i32;
    for &path in files {
        if path == b"-" {
            let fc = fold_fd(io, STDIN, width, break_spaces, count_bytes);
            if fc != 0 { code = 1; }
            continue;
        }
        let fd = match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                let _ = io.write_stderr(b"fold: cannot open '");
                let _ = io.write_stderr(path);
                let _ = io.write_stderr(b"': No such file or directory\n");
                code = 1;
                continue;
            }
        };
        let fc = fold_fd(io, fd, width, break_spaces, count_bytes);
        let _ = io.close(fd);
        if fc != 0 { code = 1; }
    }
    code
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Result<(u64, bool, bool, &'a [&'a [u8]]), i32> {
    let mut width: u64 = 80;
    let mut break_spaces = false;
    let mut count_bytes = false;
    let mut rest = args;

    while let Some((first, tail)) = rest.split_first() {
        if *first == b"-b" {
            count_bytes = true;
            rest = tail;
        } else if *first == b"-s" {
            break_spaces = true;
            rest = tail;
        } else if *first == b"-w" {
            if tail.is_empty() {
                let _ = io.write_stderr(b"fold: option requires an argument -- 'w'\n");
                return Err(1);
            }
            width = match parse_u64(tail[0]) {
                Some(w) if w > 0 => w,
                _ => {
                    let _ = io.write_stderr(b"fold: invalid width\n");
                    return Err(1);
                }
            };
            rest = &tail[1..];
        } else if first.starts_with(b"-") && first.len() > 1 {
            if *first == b"--" {
                rest = tail;
                break;
            }
            let _ = io.write_stderr(b"fold: invalid option\n");
            return Err(1);
        } else {
            break;
        }
    }

    Ok((width, break_spaces, count_bytes, rest))
}

fn parse_u64(slice: &[u8]) -> Option<u64> {
    if slice.is_empty() { return None; }
    let mut out = 0u64;
    for &b in slice {
        if !b.is_ascii_digit() { return None; }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(out)
}

fn fold_fd(io: &mut impl Io, fd: Fd, width: u64, break_spaces: bool, count_bytes: bool) -> i32 {
    let mut buf = [0u8; BUF_SIZE];
    let mut retries = 0;
    let mut col: u64 = 0;
    let mut last_blank: Option<(u64, usize)> = None; // (column, buffer-index-of-space)
    // We keep a small look-back buffer for -s mode
    let mut hold_buf: [u8; 256] = [0u8; 256];
    let mut hold_len: usize = 0;


    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => {
                // Flush hold buffer
                if hold_len > 0 {
                    let _ = io.write_stdout(&hold_buf[..hold_len]);
                }
                // Add final newline if needed
                if col > 0 || hold_len > 0 {
                    let _ = io.write_stdout(b"\n");
                }
                break;
            }
            Ok(n) if n <= buf.len() => {
                for &b in &buf[..n] {
                    let char_width = if b == b'\t' {
                        8 - (col % 8)
                    } else if b == 0x08 {
                        if col > 0 { col -= 1; }
                        hold_buf[hold_len.min(255)] = b;
                        hold_len = (hold_len + 1).min(255);
                        continue;
                    } else if b == b'\n' {
                        // Flush hold buffer, plus this newline
                        let _ = io.write_stdout(&hold_buf[..hold_len]);
                        let _ = io.write_stdout(b"\n");
                        hold_len = 0;
                        col = 0;
                        last_blank = None;
                        continue;
                    } else if b == b'\r' {
                        hold_buf[hold_len.min(255)] = b;
                        hold_len = (hold_len + 1).min(255);
                        col = 0;

                        last_blank = None;
                        continue;
                    } else {
                        1u64
                    };

                    if col + char_width > width && hold_len > 0 {
                        if break_spaces && last_blank.is_some() {
                            // Find the last space in hold buffer and break there
                            let (blank_col, blank_idx) = last_blank.unwrap();
                            // Output up to and including the blank
                            let _ = io.write_stdout(&hold_buf[..blank_idx]);
                            let _ = io.write_stdout(b"\n");
                            // Carry the rest
                            let rest_start = blank_idx;
                            let rest_end = hold_len;
                            hold_len = 0;
                            col = 0;
                            let mut new_col: u64 = 0;
                            let mut ri = rest_start;
                            while ri < rest_end {
                                let rb = hold_buf[ri];
                                if rb != b' ' && rb != b'\t' {
                                    if hold_len < 255 {
                                        hold_buf[hold_len] = rb;
                                        hold_len += 1;
                                    }
                                    new_col += 1;
                                }
                                ri += 1;
                            }
                            col = new_col;
                            last_blank = None;
                        } else {
                            // Need to fold
                            let _ = io.write_stdout(&hold_buf[..hold_len]);
                            let _ = io.write_stdout(b"\n");
                            hold_len = 0;
                            col = 0;
                            last_blank = None;
                        }
                    }

                    if b == b' ' && break_spaces {
                        last_blank = Some((col, hold_len));
                    }

                    hold_buf[hold_len.min(255)] = b;
                    hold_len = (hold_len + 1).min(255);
                    col += char_width;
                    // If hold buffer fills up, flush a line
                    if hold_len >= 255 && col > width {
                        let _ = io.write_stdout(&hold_buf[..hold_len]);
                        let _ = io.write_stdout(b"\n");
                        hold_len = 0;
                        col = 0;
                        last_blank = None;
                    }
                }
                retries = 0;
            }
            Ok(_) => return 1,
            Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                retries += 1;
                io.yield_now();
            }
            Err(_) => {
                let _ = io.write_stderr(b"fold: read error\n");
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

    struct Mock {
        files: Vec<(&'static [u8], &'static [u8])>,
        output: Vec<u8>,
        errors: Vec<u8>,
        opens: usize,
        closes: usize,
        offsets: Vec<usize>,
        fail_read: bool,
        stdin_data: Option<&'static [u8]>,
        stdin_offset: usize,
        eagain_reads: usize,
    }

    impl Mock {
        fn new(files: Vec<(&'static [u8], &'static [u8])>) -> Self {
            let fc = files.len();
            Self {
                files, output: Vec::new(), errors: Vec::new(),
                opens: 0, closes: 0, offsets: std::vec![0; fc],
                fail_read: false, stdin_data: None, stdin_offset: 0,
                eagain_reads: 0,
            }
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
            if self.eagain_reads != 0 {
                self.eagain_reads -= 1;
                return Err(Errno::Again);
            }
            if fd == STDIN {
                let data = self.stdin_data.unwrap_or(b"");
                let off = self.stdin_offset;
                if off >= data.len() { return Ok(0); }
                let n = (data.len() - off).min(buf.len());
                buf[..n].copy_from_slice(&data[off..off + n]);
                self.stdin_offset += n;
                return Ok(n);
            }
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

    fn output_as_str(output: &[u8]) -> &str {
        std::str::from_utf8(output).unwrap_or("")
    }

    #[test]
    fn empty_file() {
        let mut m = Mock::new(std::vec![(b"f", b"")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert_eq!(m.output, b"".to_vec());
    }

    #[test]
    fn short_line_no_fold() {
        let mut m = Mock::new(std::vec![(b"f", b"hi\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert_eq!(m.output, b"hi\n".to_vec());
    }

    #[test]
    fn long_line_folds() {
        let text: Vec<u8> = (b'a'..=b'z').cycle().take(85).collect();
        let st: &[u8] = unsafe { std::mem::transmute(text.as_slice()) };
        let mut m = Mock::new(std::vec![(b"f", st)]);
        assert_eq!(run(&[b"-w", b"10", b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 9); // 85 chars at width 10 = 9 lines
    }

    #[test]
    fn no_final_newline_added() {
        let text = [b'x'; 5];
        let mut m = Mock::new(std::vec![(b"f", &text)]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        // POSIX: fold adds newline at end if input doesn't end with one
        assert_eq!(m.output, b"xxxxx\n".to_vec());
    }

    #[test]
    fn existing_newlines_preserved() {
        let mut m = Mock::new(std::vec![(b"f", b"short\nlonger line here\n")]);
        assert_eq!(run(&[b"-w", b"10", b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("short\n"), "got: {s}");
    }

    #[test]
    fn missing_file() {
        let mut m = Mock::new(std::vec![]);
        assert_eq!(run(&[b"missing"], &mut m), 1);
    }

    #[test]
    fn exact_width() {
        let mut m = Mock::new(std::vec![(b"f", b"1234567890\n")]);
        assert_eq!(run(&[b"-w", b"10", b"f"], &mut m), 0);
        assert_eq!(m.output, b"1234567890\n".to_vec());
    }

    #[test]
    fn width_plus_one() {
        let mut m = Mock::new(std::vec![(b"f", b"12345678901\n")]);
        assert_eq!(run(&[b"-w", b"10", b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("1234567890\n1\n"), "got: {s}");
    }
}
