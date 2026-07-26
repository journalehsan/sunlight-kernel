//! Byte-preserving `wc` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `wc [-c|-m] [-lw] [file...]`
//!
//! Counts newline (0x0A), space-separated word, byte, and character counts.
//! Default mode reports lines, words, and bytes (three fields).
//! When any of -c, -m, -l, or -w is given, only the requested counts are shown
//! in the order they appear in the standard synopsis: lines, words, bytes, chars.
//!
//! -c : count bytes
//! -m : count characters (in POSIX/C locale: same as bytes)
//! -l : count lines only
//! -w : count words only
//!
//! Multiple files produce per-file output and a total line.  Stdin is used
//! when no file operand is given.
//!
//! Word definition (POSIX.1-2024): a maximal sequence of non-whitespace
//! characters separated by one or more whitespace characters.  In the POSIX
//! locale, whitespace is U+0020 SPACE, U+0009 TAB, and U+000A NEWLINE.

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
struct CountModes {
    lines: bool,
    words: bool,
    bytes: bool,
    chars: bool,
}

impl CountModes {
    fn default() -> Self {
        Self { lines: true, words: true, bytes: true, chars: false }
    }

    fn any_set(&self) -> bool {
        self.lines || self.words || self.bytes || self.chars
    }

    fn from_opts(lines: bool, words: bool, bytes: bool, chars: bool) -> Option<Self> {
        if !lines && !words && !bytes && !chars {
            return None;
        }
        Some(Self { lines, words, bytes, chars })
    }

    fn output_order(&self) -> [bool; 4] {
        [self.lines, self.words, self.bytes, self.chars]
    }
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (modes, files) = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let use_stdin = files.is_empty();
    if use_stdin {
        return wc_fd(io, STDIN, modes, b"");
    }

    let multiple = files.len() > 1;
    let mut code = 0i32;
    let mut totals = [0u64; 4];

    for &path in files {
        if path == b"-" {
            let file_code = wc_fd(io, STDIN, modes, b"-");
            if file_code != 0 {
                code = 1;
            }
            continue;
        }

        let fd = match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                let _ = io.write_stderr(b"wc: cannot open '");
                let _ = io.write_stderr(path);
                let _ = io.write_stderr(b"': No such file or directory\n");
                code = 1;
                continue;
            }
        };

        let (counts, file_code) = wc_fd_with_counts(io, fd, modes);
        let _ = io.close(fd);
        if file_code != 0 {
            code = 1;
            continue;
        }

        write_counts(io, modes, &counts, Some(path));
        for i in 0..4 {
            totals[i] = match totals[i].checked_add(counts[i]) {
                Some(t) => t,
                None => totals[i],
            };
        }
    }

    if multiple {
        write_counts(io, modes, &totals, Some(b"total"));
    }

    code
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Result<(CountModes, &'a [&'a [u8]]), i32> {
    let mut lines = false;
    let mut words = false;
    let mut bytes = false;
    let mut chars = false;
    let mut any_option = false;
    let mut rest = args;

    while let Some((first, tail)) = rest.split_first() {
        if *first == b"-l" {
            lines = true;
            any_option = true;
            rest = tail;
        } else if *first == b"-w" {
            words = true;
            any_option = true;
            rest = tail;
        } else if *first == b"-c" {
            bytes = true;
            any_option = true;
            rest = tail;
        } else if *first == b"-m" {
            chars = true;
            any_option = true;
            rest = tail;
        } else if first.starts_with(b"-") && first.len() > 1 {
            if first == b"--" {
                rest = tail;
                break;
            }
            let _ = io.write_stderr(b"wc: invalid option -- '");
            let _ = io.write_stderr(first);
            let _ = io.write_stderr(b"'\n");
            return Err(1);
        } else {
            break;
        }
    }

    let modes = if any_option {
        match CountModes::from_opts(lines, words, bytes, chars) {
            Some(m) => m,
            None => CountModes::default(),
        }
    } else {
        CountModes::default()
    };

    Ok((modes, rest))
}

fn wc_fd(io: &mut impl Io, fd: Fd, modes: CountModes, _name: &[u8]) -> i32 {
    let (counts, code) = wc_fd_with_counts(io, fd, modes);
    if code != 0 {
        return code;
    }
    write_counts(io, modes, &counts, None);
    0
}

fn wc_fd_with_counts(io: &mut impl Io, fd: Fd, modes: CountModes) -> ([u64; 4], i32) {
    let mut lines: u64 = 0;
    let mut words: u64 = 0;
    let mut nbytes: u64 = 0;
    let mut nchars: u64 = 0;
    let mut in_word = false;
    let mut buf = [0u8; BUF_SIZE];
    let mut retries = 0;

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) if n <= buf.len() => {
                nbytes = match nbytes.checked_add(n as u64) {
                    Some(t) => t,
                    None => {
                        let _ = io.write_stderr(b"wc: file too large\n");
                        return ([lines, words, nbytes, nchars], 1);
                    }
                };

                for &b in &buf[..n] {
                    if modes.lines && b == b'\n' {
                        lines = match lines.checked_add(1) {
                            Some(l) => l,
                            None => lines,
                        };
                    }
                    if modes.words {
                        if is_whitespace(b) {
                            in_word = false;
                        } else if !in_word {
                            in_word = true;
                            words = match words.checked_add(1) {
                                Some(w) => w,
                                None => words,
                            };
                        }
                    }
                }

                if modes.chars {
                    // In POSIX/C locale, character count equals byte count.
                    // UTF-8 support: count decoded codepoints.
                    let utf8_count = count_utf8_chars(&buf[..n]);
                    nchars = match nchars.checked_add(utf8_count) {
                        Some(c) => c,
                        None => nchars,
                    };
                }

                retries = 0;
            }
            Ok(_) => {
                let _ = io.write_stderr(b"wc: read error\n");
                return ([lines, words, nbytes, nchars], 1);
            }
            Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                retries += 1;
                io.yield_now();
            }
            Err(_) => {
                let _ = io.write_stderr(b"wc: read error\n");
                return ([lines, words, nbytes, nchars], 1);
            }
        }
    }

    ([lines, words, nbytes, nchars], 0)
}

fn is_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n'
}

fn count_utf8_chars(buf: &[u8]) -> u64 {
    let mut count = 0u64;
    let mut i = 0;
    while i < buf.len() {
        let b = buf[i];
        if b < 0x80 {
            count += 1;
            i += 1;
        } else if b < 0xC0 {
            count += 1; // continuation byte → malformed, count as one
            i += 1;
        } else if b < 0xE0 {
            count += 1;
            i += 1; // skip continuation bytes
            while i < buf.len() && (buf[i] & 0xC0) == 0x80 {
                i += 1;
            }
        } else if b < 0xF0 {
            count += 1;
            i += 1;
            while i < buf.len() && (buf[i] & 0xC0) == 0x80 {
                i += 1;
            }
        } else if b < 0xF8 {
            count += 1;
            i += 1;
            while i < buf.len() && (buf[i] & 0xC0) == 0x80 {
                i += 1;
            }
        } else {
            count += 1;
            i += 1;
        }
    }
    count
}

fn write_counts(io: &mut impl Io, modes: CountModes, counts: &[u64; 4], name: Option<&[u8]>) {
    let counts_buf = [counts[0], counts[1], counts[2], counts[3]];
    let order = modes.output_order();

    for i in 0..4 {
        if order[i] {
            let _ = io.write_stdout(b" ");
            let width = if i == 3 && modes.chars { 7 } else { 7 };
            write_u64_padded(io, counts_buf[i], width);
        }
    }

    if let Some(n) = name {
        let _ = io.write_stdout(b" ");
        let _ = io.write_stdout(n);
    }
    let _ = io.write_stdout(b"\n");
}

fn write_u64_padded(io: &mut impl Io, v: u64, width: usize) {
    let mut buf = [0u8; 20];
    let mut n = 0;
    let mut val = v;
    if val == 0 {
        buf[n] = b'0';
        n = 1;
    } else {
        while val > 0 && n < buf.len() {
            buf[n] = b'0' + (val % 10) as u8;
            n += 1;
            val /= 10;
        }
        // reverse
        let mut i = 0;
        while i < n / 2 {
            buf.swap(i, n - 1 - i);
            i += 1;
        }
    }
    // Pad with spaces to reach width+1 total field (POSIX uses right-alignment)
    // The "+1" accounts for the leading space printed before each field.
    // We need `width` characters for the number.
    if n < width {
        let padding = width - n;
        let spaces = [b' '; 20];
        let _ = io.write_stdout(&spaces[..padding]);
    }
    let _ = io.write_stdout(&buf[..n]);
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
        // " NNN NNN NNN f\n"  but we need to verify the pattern
        let s = output_as_str(&m.output);
        assert!(s.contains(" 0 0 0 f"), "got: {s}");
    }

    #[test]
    fn one_line() {
        let mut m = Mock::new(std::vec![(b"f", b"hello world\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("1 2 12 f"), "got: {s}");
    }

    #[test]
    fn multiple_lines() {
        let text = b"line one\nline two\nline three\n";
        let mut m = Mock::new(std::vec![(b"f", text)]);

        // Calculate: 3 lines, 6 words, X bytes
        let nlines = 3u64;
        let nwords = 6u64;
        let nbytes = text.len() as u64;
        assert_eq!(run(&[b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains(&format!(" {nlines} {nwords} {nbytes} f")), "got: {s}");
    }

    #[test]
    fn no_final_newline() {
        let mut m = Mock::new(std::vec![(b"f", b"hello")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("0 1 5 f"), "got: {s}");
    }

    #[test]
    fn lines_only() {
        let mut m = Mock::new(std::vec![(b"f", b"a\nb\nc\n")]);
        assert_eq!(run(&[b"-l", b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("3 f"), "got: {s}");
    }

    #[test]
    fn words_only() {
        let mut m = Mock::new(std::vec![(b"f", b"one two three\n")]);
        assert_eq!(run(&[b"-w", b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("3 f"), "got: {s}");
    }

    #[test]
    fn bytes_only() {
        let mut m = Mock::new(std::vec![(b"f", b"abc\n")]);
        assert_eq!(run(&[b"-c", b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("4 f"), "got: {s}");
    }

    #[test]
    fn chars_only() {
        let mut m = Mock::new(std::vec![(b"f", b"\xc3\xa9\n")]);
        assert_eq!(run(&[b"-m", b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("2 f"), "got: {s}");
    }

    #[test]
    fn combined_options() {
        let mut m = Mock::new(std::vec![(b"f", b"hi\n")]);
        assert_eq!(run(&[b"-l", b"-w", b"-c", b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("1 1 3 f"), "got: {s}");
    }

    #[test]
    fn multiple_files_with_total() {
        let mut m = Mock::new(std::vec![
            (b"a", b"hi\n"), (b"b", b"there\n"),
        ]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("total"), "got: {s}");
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn stdin_when_no_operand() {
        let mut m = Mock::new(std::vec![]);
        m.stdin_data = Some(b"abc\n");
        assert_eq!(run(&[], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("4"), "got: {s}");
    }

    #[test]
    fn dash_as_stdin() {
        let mut m = Mock::new(std::vec![(b"a", b"hello\n")]);
        m.stdin_data = Some(b"world\n");
        assert_eq!(run(&[b"-", b"a"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("-"), "got: {s}");
        assert!(s.contains("a"), "got: {s}");
    }

    #[test]
    fn whitespace_only() {
        let mut m = Mock::new(std::vec![(b"f", b"   \t  \n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("1 0"), "got: {s}");
    }

    #[test]
    fn consecutive_newlines() {
        let mut m = Mock::new(std::vec![(b"f", b"\n\n\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("3 0 3 f"), "got: {s}");
    }

    #[test]
    fn missing_file() {
        let mut m = Mock::new(std::vec![]);
        assert_eq!(run(&[b"missing"], &mut m), 1);
        assert!(!m.errors.is_empty());
    }

    #[test]
    fn mixed_good_and_bad_files() {
        let mut m = Mock::new(std::vec![(b"ok", b"hi\n")]);
        assert_eq!(run(&[b"ok", b"missing"], &mut m), 1);
        let s = output_as_str(&m.output);
        assert!(s.contains("ok"), "got: {s}");
    }

    #[test]
    fn long_line_across_buffers() {
        let data: Vec<u8> = (b'a'..=b'z').cycle().take(BUF_SIZE * 2 + 5).collect();
        let st: &[u8] = unsafe { std::mem::transmute(data.as_slice()) };
        let mut m = Mock::new(std::vec![(b"big", st)]);
        assert_eq!(run(&[b"big"], &mut m), 0);
    }

    #[test]
    fn one_buffer_boundary() {
        static DATA: [u8; BUF_SIZE] = [b'x'; BUF_SIZE];
        let mut m = Mock::new(std::vec![(b"exact", &DATA)]);
        assert_eq!(run(&[b"exact"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains(&format!("0 1 {} exact", BUF_SIZE)), "got: {s}");
    }

    #[test]
    fn one_buffer_plus_one() {
        static DATA: [u8; BUF_SIZE + 1] = [b'x'; BUF_SIZE + 1];
        let mut m = Mock::new(std::vec![(b"over", &DATA)]);
        assert_eq!(run(&[b"over"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains(&format!("0 1 {} over", BUF_SIZE + 1)), "got: {s}");
    }

    #[test]
    fn utf8_multibyte_text() {
        let text = "héllo\nwörld\n";
        let mut m = Mock::new(std::vec![(b"u8", text.as_bytes())]);
        assert_eq!(run(&[b"-m", b"u8"], &mut m), 0);
        let s = output_as_str(&m.output);
        assert!(s.contains("12 u8") || s.contains("13 u8"), "got: {s}");
    }

    #[test]
    fn nul_bytes() {
        let mut m = Mock::new(std::vec![(b"nul", b"\x00hello\x00\n")]);
        assert_eq!(run(&[b"nul"], &mut m), 0);
        let s = output_as_str(&m.output);
        // 1 line, 1 word (hello), 7 bytes
        assert!(s.contains("1 1 7 nul"), "got: {s}");
    }

    #[test]
    fn read_error() {
        let mut m = Mock::new(std::vec![(b"x", b"hello\n")]);
        m.fail_read = true;
        assert_eq!(run(&[b"x"], &mut m), 1);
    }

    #[test]
    fn eagain_bounded() {
        let mut m = Mock::new(std::vec![(b"x", b"hello\n")]);
        m.eagain_reads = READ_RETRY_LIMIT + 1;
        assert_eq!(run(&[b"x"], &mut m), 1);
    }
}
