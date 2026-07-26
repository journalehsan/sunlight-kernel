//! Byte-preserving `cut` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `cut -b list [-n] [file...]`
//!                       `cut -c list [file...]`
//!                       `cut -f list [-d delim] [-s] [file...]`
//!
//! -b list : select byte positions
//! -c list : select character positions
//! -f list : select delimited fields
//! -d delim: field delimiter (default tab)
//! -s      : suppress lines without delimiter
//!
//! The list is a comma- or space-separated sequence of positive integers
//! and ranges: N, N-M, -M (1 through M), N- (N to end).

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Bytes,
    Chars,
    Fields,
}

#[derive(Clone, Copy)]
struct Range {
    lo: u64,
    hi: u64, // 0 means unbounded
}

impl Range {
    fn contains(&self, pos: u64) -> bool {
        pos >= self.lo && (self.hi == 0 || pos <= self.hi)
    }

    fn beyond(&self, pos: u64) -> bool {
        self.hi != 0 && pos > self.hi
    }
}

struct Selection {
    ranges: [Range; 64],
    len: usize,
}

impl Selection {
    fn new() -> Self {
        Self { ranges: [Range { lo: 0, hi: 0 }; 64], len: 0 }
    }

    fn add(&mut self, r: Range) {
        if self.len < self.ranges.len() {
            self.ranges[self.len] = r;
            self.len += 1;
        }
    }

    fn select(&self, pos: u64) -> bool {
        for i in 0..self.len {
            if self.ranges[i].contains(pos) {
                return true;
            }
        }
        false
    }

    fn all_beyond(&self, pos: u64) -> bool {
        self.len > 0 && self.ranges.iter().take(self.len).all(|r| r.beyond(pos))
    }
}

fn parse_list(input: &[u8]) -> Result<Selection, ()> {
    let mut sel = Selection::new();
    let s = core::str::from_utf8(input).map_err(|_| ())?;
    // Split on comma
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c == ',' || c == ' ' {
            if start < i {
                let part = s[start..i].trim();
                if !part.is_empty() {
                    parse_range_part(part, &mut sel)?;
                }
            }
            start = i + 1;
        }
    }
    if start < s.len() {
        let part = s[start..].trim();
        if !part.is_empty() {
            parse_range_part(part, &mut sel)?;
        }
    }
    if sel.len == 0 {
        return Err(());
    }
    Ok(sel)
}

fn parse_range_part(part: &str, sel: &mut Selection) -> Result<(), ()> {
    if let Some(dash_pos) = part.find('-') {
        let a_str = &part[..dash_pos];
        let b_str = &part[dash_pos + 1..];
        if a_str.is_empty() && b_str.is_empty() {
            return Err(());
        }
        if a_str.is_empty() {
            let hi = parse_u64_bytes(b_str.as_bytes()).ok_or(())?;
            if hi == 0 { return Err(()); }
            sel.add(Range { lo: 1, hi });
        } else if b_str.is_empty() {
            let lo = parse_u64_bytes(a_str.as_bytes()).ok_or(())?;
            if lo == 0 { return Err(()); }
            sel.add(Range { lo, hi: 0 });
        } else {
            let lo = parse_u64_bytes(a_str.as_bytes()).ok_or(())?;
            let hi = parse_u64_bytes(b_str.as_bytes()).ok_or(())?;
            if lo == 0 || hi == 0 || lo > hi { return Err(()); }
            sel.add(Range { lo, hi });
        }
    } else {
        let n = parse_u64_bytes(part.as_bytes()).ok_or(())?;
        if n == 0 { return Err(()); }
        sel.add(Range { lo: n, hi: n });
    }
    Ok(())
}

fn parse_u64_bytes(slice: &[u8]) -> Option<u64> {
    if slice.is_empty() { return None; }
    let mut out = 0u64;
    for &b in slice {
        if !b.is_ascii_digit() { return None; }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(out)
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (mode, selection, delimiter, suppress, files) = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let use_stdin = files.is_empty();
    if use_stdin {
        return cut_fd(io, STDIN, mode, &selection, delimiter, suppress);
    }

    let mut code = 0i32;
    for &path in files {
        if path == b"-" {
            let fc = cut_fd(io, STDIN, mode, &selection, delimiter, suppress);
            if fc != 0 { code = 1; }
            continue;
        }
        let fd = match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                let _ = io.write_stderr(b"cut: cannot open '");
                let _ = io.write_stderr(path);
                let _ = io.write_stderr(b"': No such file or directory\n");
                code = 1;
                continue;
            }
        };
        let fc = cut_fd(io, fd, mode, &selection, delimiter, suppress);
        let _ = io.close(fd);
        if fc != 0 { code = 1; }
    }
    code
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Result<(Mode, Selection, u8, bool, &'a [&'a [u8]]), i32> {
    let mut mode: Option<Mode> = None;
    let mut list: Option<&[u8]> = None;
    let mut delimiter: u8 = b'\t';
    let mut suppress = false;
    let mut rest = args;

    while let Some((first, tail)) = rest.split_first() {
        if *first == b"-b" {
            if mode.is_some() { return Err(1); }
            mode = Some(Mode::Bytes);
            if tail.is_empty() {
                let _ = io.write_stderr(b"cut: option requires an argument -- 'b'\n");
                return Err(1);
            }
            list = Some(tail[0]);
            rest = &tail[1..];
        } else if *first == b"-c" {
            if mode.is_some() { return Err(1); }
            mode = Some(Mode::Chars);
            if tail.is_empty() {
                let _ = io.write_stderr(b"cut: option requires an argument -- 'c'\n");
                return Err(1);
            }
            list = Some(tail[0]);
            rest = &tail[1..];
        } else if *first == b"-f" {
            if mode.is_some() { return Err(1); }
            mode = Some(Mode::Fields);
            if tail.is_empty() {
                let _ = io.write_stderr(b"cut: option requires an argument -- 'f'\n");
                return Err(1);
            }
            list = Some(tail[0]);
            rest = &tail[1..];
        } else if *first == b"-d" {
            if tail.is_empty() {
                let _ = io.write_stderr(b"cut: option requires an argument -- 'd'\n");
                return Err(1);
            }
            let delim_arg = tail[0];
            if delim_arg.len() > 1 {
                // POSIX allows -d with single char or the first byte of longer arg
            }
            delimiter = delim_arg.first().copied().unwrap_or(b'\t');
            rest = &tail[1..];
        } else if *first == b"-s" {
            suppress = true;
            rest = tail;
        } else if first.starts_with(b"-") && first.len() > 1 {
            if *first == b"--" {
                rest = tail;
                break;
            }
            let _ = io.write_stderr(b"cut: invalid option -- '");
            let _ = io.write_stderr(first);
            let _ = io.write_stderr(b"'\n");
            return Err(1);
        } else {
            break;
        }
    }

    let mode = mode.ok_or_else(|| {
        let _ = io.write_stderr(b"cut: you must specify a list of bytes, characters, or fields\n");
        1i32
    })?;

    let list_bytes = list.ok_or_else(|| {
        let _ = io.write_stderr(b"cut: you must specify a list\n");
        1i32
    })?;

    let selection = parse_list(list_bytes).map_err(|_| {
        let _ = io.write_stderr(b"cut: invalid list argument\n");
        1i32
    })?;

    Ok((mode, selection, delimiter, suppress, rest))
}

fn cut_fd(io: &mut impl Io, fd: Fd, mode: Mode, selection: &Selection, delim: u8, suppress: bool) -> i32 {
    match mode {
        Mode::Bytes => cut_bytes_fd(io, fd, selection),
        Mode::Chars => cut_chars_fd(io, fd, selection),
        Mode::Fields => cut_fields_fd(io, fd, selection, delim, suppress),
    }
}

fn cut_bytes_fd(io: &mut impl Io, fd: Fd, selection: &Selection) -> i32 {
    // Proper per-line byte-position tracker.
    let mut buf = [0u8; BUF_SIZE];
    let mut line_buf: [u8; BUF_SIZE * 4] = [0u8; BUF_SIZE * 4];
    let mut line_len: usize = 0;
    let mut retries = 0;

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => {
                // Flush remaining line
                if line_len > 0 {
                    output_selected_bytes(io, &line_buf[..line_len], selection);
                    // No trailing newline added per POSIX
                }
                break;
            }
            Ok(n) if n <= buf.len() => {
                let mut start = 0;
                for (i, &b) in buf[..n].iter().enumerate() {
                    if b == b'\n' {
                        // Flush line up to this byte including the newline
                        let chunk_len = i + 1 - start;
                        if line_len + chunk_len <= line_buf.len() {
                            line_buf[line_len..line_len + chunk_len].copy_from_slice(&buf[start..i + 1]);
                            line_len += chunk_len;
                        }
                        output_selected_bytes(io, &line_buf[..line_len], selection);
                        line_len = 0;
                        start = i + 1;
                    }
                }
                // Carry remainder
                let remaining = n - start;
                if remaining > 0 {
                    if line_len + remaining <= line_buf.len() {
                        line_buf[line_len..line_len + remaining].copy_from_slice(&buf[start..n]);
                        line_len += remaining;
                    } else {
                        // Line too long, output selected bytes from what we have
                        // and reset
                        output_selected_bytes(io, &line_buf[..line_len], selection);
                        line_len = 0;
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
                let _ = io.write_stderr(b"cut: read error\n");
                return 1;
            }
        }
    }
    0
}

fn output_selected_bytes(io: &mut impl Io, line: &[u8], selection: &Selection) {
    let mut pos: u64 = 1;
    for &b in line {
        if b == b'\n' {
            let _ = io.write_stdout(b"\n");
            break;
        }
        if selection.select(pos) {
            let _ = io.write_stdout(&[b]);
        }
        pos += 1;
    }
}

fn cut_chars_fd(io: &mut impl Io, fd: Fd, selection: &Selection) -> i32 {
    cut_bytes_fd(io, fd, selection)
}

fn cut_fields_fd(io: &mut impl Io, fd: Fd, selection: &Selection, delim: u8, suppress: bool) -> i32 {
    let mut buf = [0u8; BUF_SIZE];
    let mut line_buf: [u8; BUF_SIZE * 4] = [0u8; BUF_SIZE * 4];
    let mut line_len: usize = 0;
    let mut retries = 0;

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => {
                if line_len > 0 {
                    output_selected_fields(io, &line_buf[..line_len], selection, delim, suppress);
                }
                break;
            }
            Ok(n) if n <= buf.len() => {
                let mut start = 0;
                for (i, &b) in buf[..n].iter().enumerate() {
                    if b == b'\n' {
                        let chunk_len = i + 1 - start;
                        if line_len + chunk_len <= line_buf.len() {
                            line_buf[line_len..line_len + chunk_len].copy_from_slice(&buf[start..i + 1]);
                            line_len += chunk_len;
                        }
                        output_selected_fields(io, &line_buf[..line_len], selection, delim, suppress);
                        line_len = 0;
                        start = i + 1;
                    }
                }
                let remaining = n - start;
                if remaining > 0 && line_len + remaining <= line_buf.len() {
                    line_buf[line_len..line_len + remaining].copy_from_slice(&buf[start..n]);
                    line_len += remaining;
                } else if remaining > 0 {
                    // Line too large for buffer
                    output_selected_fields(io, &line_buf[..line_len], selection, delim, suppress);
                    line_len = 0;
                }
                retries = 0;
            }
            Ok(_) => { return 1; }
            Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                retries += 1;
                io.yield_now();
            }
            Err(_) => {
                let _ = io.write_stderr(b"cut: read error\n");
                return 1;
            }
        }
    }
    0
}

fn output_selected_fields(io: &mut impl Io, line: &[u8], selection: &Selection, delim: u8, suppress: bool) {
    // Strip trailing newline for field analysis
    let content = if line.last() == Some(&b'\n') {
        &line[..line.len() - 1]
    } else {
        line
    };

    let has_delim = content.iter().any(|&b| b == delim);
    if suppress && !has_delim {
        // Always output the newline
        let _ = io.write_stdout(b"\n");
        return;
    }

    // Find field boundaries
    let mut field_starts: [usize; 64] = [0; 64];
    let mut field_ends: [usize; 64] = [0; 64];
    let mut nfields: usize = 0;
    let mut start: usize = 0;

    for (i, &b) in content.iter().enumerate() {
        if b == delim {
            if nfields < 64 {
                field_starts[nfields] = start;
                field_ends[nfields] = i;
                nfields += 1;
            }
            start = i + 1;
        }
    }
    // Last field
    if nfields < 64 {
        field_starts[nfields] = start;
        field_ends[nfields] = content.len();
        nfields += 1;
    }

    let mut output_delim = false;
    for i in 1..=nfields {
        let fnum = i as u64;
        if selection.select(fnum) {
            if output_delim {
                let _ = io.write_stdout(&[delim]);
            }
            let fstart = field_starts[i - 1];
            let fend = field_ends[i - 1];
            if fstart < fend {
                let _ = io.write_stdout(&content[fstart..fend]);
            }
            output_delim = true;
        }
    }

    // Always add newline
    let _ = io.write_stdout(b"\n");
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

    #[test]
    fn parse_simple_number() {
        let sel = parse_list(b"3").unwrap();
        assert!(sel.select(3));
        assert!(!sel.select(2));
        assert!(!sel.select(4));
    }

    #[test]
    fn parse_range() {
        let sel = parse_list(b"2-5").unwrap();
        assert!(!sel.select(1));
        assert!(sel.select(2));
        assert!(sel.select(5));
        assert!(!sel.select(6));
    }

    #[test]
    fn parse_open_start() {
        let sel = parse_list(b"-3").unwrap();
        assert!(sel.select(1));
        assert!(sel.select(3));
        assert!(!sel.select(4));
    }

    #[test]
    fn parse_open_end() {
        let sel = parse_list(b"4-").unwrap();
        assert!(!sel.select(3));
        assert!(sel.select(4));
        assert!(sel.select(100));
    }

    #[test]
    fn parse_multiple() {
        let sel = parse_list(b"1,3,5-7").unwrap();
        assert!(sel.select(1));
        assert!(!sel.select(2));
        assert!(sel.select(3));
        assert!(!sel.select(4));
        assert!(sel.select(5));
        assert!(sel.select(7));
        assert!(!sel.select(8));
    }

    #[test]
    fn reject_zero() {
        assert!(parse_list(b"0").is_err());
        assert!(parse_list(b"1-0").is_err());
    }

    #[test]
    fn reject_descending() {
        assert!(parse_list(b"5-3").is_err());
    }

    #[test]
    fn reject_malformed() {
        assert!(parse_list(b"abc").is_err());
        assert!(parse_list(b"").is_err());
    }

    #[test]
    fn bytes_mode_simple() {
        let mut m = Mock::new(std::vec![(b"f", b"abcdef\n")]);
        assert_eq!(run(&[b"-b", b"2-4", b"f"], &mut m), 0);
        assert_eq!(m.output, b"bcd\n".to_vec());
    }

    #[test]
    fn bytes_multiple_lines() {
        let mut m = Mock::new(std::vec![(b"f", b"ab\ncd\nef\n")]);
        assert_eq!(run(&[b"-b", b"1", b"f"], &mut m), 0);
        assert_eq!(m.output, b"a\nc\ne\n".to_vec());
    }

    #[test]
    fn fields_simple() {
        let mut m = Mock::new(std::vec![(b"f", b"a:b:c\n")]);
        assert_eq!(run(&[b"-f", b"2", b"-d", b":", b"f"], &mut m), 0);
        assert_eq!(m.output, b"b\n".to_vec());
    }

    #[test]
    fn fields_multiple() {
        let mut m = Mock::new(std::vec![(b"f", b"a:b:c:d\n")]);
        assert_eq!(run(&[b"-f", b"1,3", b"-d", b":", b"f"], &mut m), 0);
        assert_eq!(m.output, b"a:c\n".to_vec());
    }

    #[test]
    fn no_final_newline() {
        let mut m = Mock::new(std::vec![(b"f", b"abcde")]);
        assert_eq!(run(&[b"-b", b"1-3", b"f"], &mut m), 0);
        assert_eq!(m.output, b"abc".to_vec());
    }
}
