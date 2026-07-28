//! Byte-preserving `expand` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `expand [-t tablist] [file...]`
//!
//! Replaces tab characters with the appropriate number of spaces to reach the
//! next tab stop.  Default tab stops are every 8 columns (8, 16, 24, ...).
//! -t tablist: comma-separated list of positive integers.
//!   A single integer N sets tab stops every N columns.
//!   Multiple integers N1,N2,N3,... set tab stops at those columns.
//!   After the last explicit stop, the implicit interval from the last two
//!   stops is used (or the default 8 if fewer than 2 stops).
//!
//! Stdin is used when no file operand is given.  "-" can be used for stdin.

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

#[derive(Clone)]
enum TabStops {
    Single(u64),
    List { stops: [u64; 64], len: usize },
}

impl TabStops {
    fn next_stop(&self, current_column: u64) -> u64 {
        match self {
            TabStops::Single(interval) => {
                let interval = *interval;
                if interval == 0 {
                    return current_column + 1;
                }
                ((current_column / interval) + 1) * interval
            }
            TabStops::List { stops, len } => {
                for i in 0..*len {
                    if stops[i] > current_column {
                        return stops[i];
                    }
                }
                // After last explicit stop, use the interval between last two
                if *len >= 2 {
                    let last = stops[*len - 1];
                    let second_last = stops[*len - 2];
                    let interval = last - second_last;
                    if interval == 0 {
                        return last + 8;
                    }
                    ((current_column / interval) + 1) * interval
                } else if *len == 1 {
                    current_column + 8
                } else {
                    current_column + 8
                }
            }
        }
    }
}

fn parse_tablist(input: &[u8]) -> Result<TabStops, ()> {
    let s = core::str::from_utf8(input).map_err(|_| ())?;
    let bytes = s.as_bytes();

    // Count parts to detect single-element case
    let mut part_count = 0;
    let mut has_comma = false;
    for &b in bytes {
        if b == b',' {
            part_count += 1;
            has_comma = true;
        }
    }
    if part_count == 0 && !has_comma {
        part_count = if bytes.is_empty() { 0 } else { 1 };
    } else {
        part_count += 1;
    }

    if part_count == 0 {
        return Err(());
    }

    if part_count == 1 && !has_comma {
        let n = parse_u64(bytes).ok_or(())?;
        if n == 0 {
            return Err(());
        }
        return Ok(TabStops::Single(n));
    }

    let mut stops = [0u64; 64];
    let mut len = 0;
    let mut prev: u64 = 0;
    let mut start = 0;

    for (i, &b) in bytes.iter().enumerate() {
        if b == b',' {
            if start < i {
                let part = &bytes[start..i];
                let part_trimmed = trim_bytes(part);
                if !part_trimmed.is_empty() {
                    let n = parse_u64(part_trimmed).ok_or(())?;
                    if n == 0 {
                        return Err(());
                    }
                    if n <= prev {
                        return Err(());
                    }
                    if len < 64 {
                        stops[len] = n;
                        len += 1;
                        prev = n;
                    }
                }
            }
            start = i + 1;
        }
    }
    if start < bytes.len() {
        let part = &bytes[start..];
        let part_trimmed = trim_bytes(part);
        if !part_trimmed.is_empty() {
            let n = parse_u64(part_trimmed).ok_or(())?;
            if n == 0 {
                return Err(());
            }
            if n <= prev {
                return Err(());
            }
            if len < 64 {
                stops[len] = n;
                len += 1;
            }
        }
    }

    if len == 0 {
        return Err(());
    }

    Ok(TabStops::List { stops, len })
}

fn trim_bytes(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|&b| b != b' ').unwrap_or(bytes.len());
    let end = bytes[start..]
        .iter()
        .rposition(|&b| b != b' ')
        .map_or(bytes.len(), |p| start + p + 1);
    &bytes[start..end]
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

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (tab_stops, files) = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let use_stdin = files.is_empty();
    if use_stdin {
        return expand_fd(io, STDIN, &tab_stops);
    }

    let mut code = 0i32;
    for &path in files {
        if path == b"-" {
            let fc = expand_fd(io, STDIN, &tab_stops);
            if fc != 0 {
                code = 1;
            }
            continue;
        }
        let fd = match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                let _ = io.write_stderr(b"expand: cannot open '");
                let _ = io.write_stderr(path);
                let _ = io.write_stderr(b"': No such file or directory\n");
                code = 1;
                continue;
            }
        };
        let fc = expand_fd(io, fd, &tab_stops);
        let _ = io.close(fd);
        if fc != 0 {
            code = 1;
        }
    }
    code
}

fn parse_args<'a>(
    args: &'a [&'a [u8]],
    io: &mut impl Io,
) -> Result<(TabStops, &'a [&'a [u8]]), i32> {
    let mut tab_stops: TabStops = TabStops::Single(8);
    let mut rest = args;

    while let Some((first, tail)) = rest.split_first() {
        if *first == b"-t" {
            if tail.is_empty() {
                let _ = io.write_stderr(b"expand: option requires an argument -- 't'\n");
                return Err(1);
            }
            tab_stops = parse_tablist(tail[0]).map_err(|_| {
                let _ = io.write_stderr(b"expand: invalid tab stop list\n");
                1i32
            })?;
            rest = &tail[1..];
        } else if first.starts_with(b"-") && first.len() > 1 {
            if *first == b"--" {
                rest = tail;
                break;
            }
            let _ = io.write_stderr(b"expand: invalid option\n");
            return Err(1);
        } else {
            break;
        }
    }

    Ok((tab_stops, rest))
}

fn expand_fd(io: &mut impl Io, fd: Fd, tab_stops: &TabStops) -> i32 {
    let mut buf = [0u8; BUF_SIZE];
    let mut col: u64 = 0;
    let mut retries = 0;

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) if n <= buf.len() => {
                for &b in &buf[..n] {
                    match b {
                        b'\t' => {
                            let next = tab_stops.next_stop(col);
                            let spaces = next - col;
                            for _ in 0..spaces {
                                let _ = io.write_stdout(b" ");
                            }
                            col = next;
                        }
                        b'\n' => {
                            let _ = io.write_stdout(b"\n");
                            col = 0;
                        }
                        0x08 => {
                            let _ = io.write_stdout(&[b]);
                            if col > 0 {
                                col -= 1;
                            }
                        }
                        b'\r' => {
                            let _ = io.write_stdout(&[b]);
                            col = 0;
                        }
                        _ => {
                            let _ = io.write_stdout(&[b]);
                            if b < 0x80 || b >= 0xC0 {
                                col += 1;
                            }
                        }
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
                let _ = io.write_stderr(b"expand: read error\n");
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
                files,
                output: Vec::new(),
                errors: Vec::new(),
                opens: 0,
                closes: 0,
                offsets: std::vec![0; fc],
                fail_read: false,
                stdin_data: None,
                stdin_offset: 0,
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
            if self.fail_read {
                return Err(Errno::Failed);
            }
            if self.eagain_reads != 0 {
                self.eagain_reads -= 1;
                return Err(Errno::Again);
            }
            if fd == STDIN {
                let data = self.stdin_data.unwrap_or(b"");
                let off = self.stdin_offset;
                if off >= data.len() {
                    return Ok(0);
                }
                let n = (data.len() - off).min(buf.len());
                buf[..n].copy_from_slice(&data[off..off + n]);
                self.stdin_offset += n;
                return Ok(n);
            }
            let idx = (fd.0 - 3) as usize;
            if idx >= self.files.len() {
                return Ok(0);
            }
            let data = self.files[idx].1;
            let off = self.offsets[idx];
            if off >= data.len() {
                return Ok(0);
            }
            let n = (data.len() - off).min(buf.len());
            buf[..n].copy_from_slice(&data[off..off + n]);
            self.offsets[idx] += n;
            Ok(n)
        }
        fn close(&mut self, _fd: Fd) -> Result<(), Errno> {
            self.closes += 1;
            Ok(())
        }
        fn write_stdout(&mut self, b: &[u8]) -> Result<(), Errno> {
            self.output.extend_from_slice(b);
            Ok(())
        }
        fn write_stderr(&mut self, b: &[u8]) -> Result<(), Errno> {
            self.errors.extend_from_slice(b);
            Ok(())
        }
        fn yield_now(&mut self) {}
    }

    fn output_as_str(output: &[u8]) -> &str {
        std::str::from_utf8(output).unwrap_or("")
    }

    #[test]
    fn no_tabs() {
        let mut m = Mock::new(std::vec![(b"f", b"hello world\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert_eq!(m.output, b"hello world\n".to_vec());
    }

    #[test]
    fn tab_default_stops() {
        let mut m = Mock::new(std::vec![(b"f", b"a\tb\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        // 'a' is at col 1, next tab stop at col 8, so 7 spaces
        assert_eq!(m.output, b"a       b\n".to_vec());
    }

    #[test]
    fn consecutive_tabs() {
        let mut m = Mock::new(std::vec![(b"f", b"\t\tx\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        // First tab at col 0 -> col 8 (8 spaces)
        // Second tab at col 8 -> col 16 (8 spaces)
        // 'x' -> col 17
        // newline
        let expected = format!("{}{}x\n", " ".repeat(8), " ".repeat(8));
        assert_eq!(m.output, expected.as_bytes().to_vec());
    }

    #[test]
    fn tab_at_column_zero() {
        let mut m = Mock::new(std::vec![(b"f", b"\tx\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        // tab at col 0 -> col 8 (8 spaces), then x
        assert_eq!(m.output, b"        x\n".to_vec());
    }

    #[test]
    fn tab_before_stop() {
        let mut m = Mock::new(std::vec![(b"f", b"abcd\tx\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        // abcd at cols 1-4, next tab stop at col 8 -> 4 spaces
        assert_eq!(m.output, b"abcd    x\n".to_vec());
    }

    #[test]
    fn tab_exactly_on_stop() {
        let mut m = Mock::new(std::vec![(b"f", b"abcdefgh\tx\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        // abcdefgh at cols 1-8, next tab stop at col 16 -> 8 spaces
        assert_eq!(m.output, b"abcdefgh        x\n".to_vec());
    }

    #[test]
    fn custom_single_interval() {
        let mut m = Mock::new(std::vec![(b"f", b"a\tb\n")]);
        assert_eq!(run(&[b"-t", b"4", b"f"], &mut m), 0);
        // 'a' at col 1, next stop at 4 -> 3 spaces
        assert_eq!(m.output, b"a   b\n".to_vec());
    }

    #[test]
    fn custom_tab_list() {
        let mut m = Mock::new(std::vec![(b"f", b"a\tb\tc\n")]);
        assert_eq!(run(&[b"-t", b"3,5,10", b"f"], &mut m), 0);
        // 'a' at col 1, next stop at 3 -> 2 spaces
        // 'b' at col 4, next stop at 5 -> 1 space
        // 'c' at col 6
        assert_eq!(m.output, b"a  b c\n".to_vec());
    }

    #[test]
    fn multiple_lines() {
        let mut m = Mock::new(std::vec![(b"f", b"x\ty\nz\tw\n")]);
        assert_eq!(run(&[b"-t", b"5", b"f"], &mut m), 0);
        // line 1: x at col 1, stop at 5 -> 4 spaces, y
        // line 2: z at col 1, stop at 5 -> 4 spaces, w
        assert_eq!(m.output, b"x    y\nz    w\n".to_vec());
    }

    #[test]
    fn no_final_newline() {
        let mut m = Mock::new(std::vec![(b"f", b"a\tb")]);
        assert_eq!(run(&[b"-t", b"5", b"f"], &mut m), 0);
        assert_eq!(m.output, b"a    b".to_vec());
    }

    #[test]
    fn empty_input() {
        let mut m = Mock::new(std::vec![(b"f", b"")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert_eq!(m.output, b"".to_vec());
    }

    #[test]
    fn invalid_tab_stop_zero() {
        assert!(parse_tablist(b"0").is_err());
        assert!(parse_tablist(b"1,0,3").is_err());
    }

    #[test]
    fn invalid_descending() {
        assert!(parse_tablist(b"5,3").is_err());
    }

    #[test]
    fn invalid_malformed() {
        assert!(parse_tablist(b"abc").is_err());
    }

    #[test]
    fn parse_tab_single() {
        let ts = parse_tablist(b"4").unwrap();
        assert_eq!(ts.next_stop(0), 4);
        assert_eq!(ts.next_stop(4), 8);
    }

    #[test]
    fn parse_tab_list() {
        let ts = parse_tablist(b"3,7,12").unwrap();
        assert_eq!(ts.next_stop(0), 3);
        assert_eq!(ts.next_stop(4), 7);
        assert_eq!(ts.next_stop(8), 12);
    }

    #[test]
    fn tab_beyond_last_explicit() {
        let ts = parse_tablist(b"5,9").unwrap();
        // Last two: 5,9 interval=4
        // At col 10, next should be 13
        assert_eq!(ts.next_stop(10), 13);
    }

    #[test]
    fn missing_file() {
        let mut m = Mock::new(std::vec![]);
        assert_eq!(run(&[b"missing"], &mut m), 1);
    }

    #[test]
    fn read_error() {
        let mut m = Mock::new(std::vec![(b"x", b"x")]);
        m.fail_read = true;
        assert_eq!(run(&[b"x"], &mut m), 1);
    }
}
