//! POSIX `grep` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `grep [-E|-F] [-c|-l|-q] [-insvx] [-e pattern] [-f file] [file...]`
//!
//! Searches input files for lines matching a pattern.
//!
//! Exit status: 0 = one or more lines selected, 1 = no lines selected, >1 = error.
//!
//! Mandatory options:
//! -E : extended regular expressions (recorded conformance gap — treated as BRE)
//! -F : fixed strings (substring matching)
//! -c : print only count of matching lines
//! -l : print only names of files with matches
//! -q : quiet — exit status only, suppress output
//! -i : case-insensitive matching (ASCII only)
//! -n : prefix each line with its line number
//! -s : suppress error messages about nonexistent/unreadable files
//! -v : select non-matching lines
//! -x : exact whole-line match
//! -e pattern : specify pattern (useful for patterns starting with -)
//! -f file : read patterns from file
//!
//! Streaming: uses bounded 64KB buffer with carry for partial lines.
//! Does not load entire input files into memory.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

use crate::regex::Regex;

const READ_RETRY_LIMIT: usize = 8;
const BUF_SIZE: usize = 4096;
const MAX_LINE_LEN: usize = 8192;
const MAX_PATTERNS: usize = 8;
const MAX_PATTERN_LEN: usize = 512;

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

struct Options {
    fixed: bool,
    extended: bool,
    count_only: bool,
    files_with_matches: bool,
    files_without_matches: bool,
    quiet: bool,
    icase: bool,
    line_number: bool,
    silent_err: bool,
    invert: bool,
    whole_line: bool,
}

impl Options {
    fn new() -> Self {
        Self {
            fixed: false, extended: false, count_only: false,
            files_with_matches: false, files_without_matches: false,
            quiet: false, icase: false, line_number: false,
            silent_err: false, invert: false, whole_line: false,
        }
    }
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (opts, mut pattern_data, pattern_lens_in, pattern_count, files) = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    if pattern_count == 0 {
        let _ = io.write_stderr(b"grep: missing pattern\n");
        return 2;
    }

    let use_stdin = files.is_empty();
    let show_filename = files.len() > 1;

    let mut found_any = false;
    let mut overall_code = 0i32;

    // Use the pattern data and lengths directly
    let pattern_slices: &[[u8; MAX_PATTERN_LEN]; MAX_PATTERNS] = &pattern_data;
    let pattern_lens: &[usize; MAX_PATTERNS] = &pattern_lens_in;

    let mut found_any = false;
    let mut overall_code = 0i32;

    if use_stdin {
        match grep_fd(io, STDIN, b"-", false, &opts, &pattern_slices, &pattern_lens, pattern_count) {
            Ok(found) => {
                if !found && !opts.quiet {
                    found_any = true;
                }
                found_any = found_any || found;
            }
            Err(_) => {
                overall_code = 1;
            }
        }
        return exit_code(found_any, overall_code, opts.quiet);
    }

    for &path in files {
        if path == b"-" {
            match grep_fd(io, STDIN, b"-", show_filename, &opts, &pattern_slices, &pattern_lens, pattern_count) {
                Ok(found) => found_any = found_any || found,
                Err(_) => overall_code = 1,
            }
            continue;
        }

        let fd = match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                if !opts.silent_err {
                    let _ = io.write_stderr(b"grep: cannot open '");
                    let _ = io.write_stderr(path);
                    let _ = io.write_stderr(b"': No such file or directory\n");
                }
                overall_code = 1;
                continue;
            }
        };

        let file_code = match grep_fd(io, fd, path, show_filename, &opts, &pattern_slices, &pattern_lens, pattern_count) {
            Ok(found) => {
                found_any = found_any || found;
                0
            }
            Err(_) => 1,
        };

        let _ = io.close(fd);
        if file_code != 0 {
            overall_code = 1;
        }
    }

    exit_code(found_any, overall_code, opts.quiet)
}

fn exit_code(found_any: bool, error_code: i32, quiet: bool) -> i32 {
    if error_code != 0 {
        2
    } else if found_any || quiet {
        0
    } else {
        1
    }
}

fn parse_args<'a>(
    args: &'a [&'a [u8]],
    io: &mut impl Io,
) -> Result<(Options, [[u8; MAX_PATTERN_LEN]; MAX_PATTERNS], [usize; MAX_PATTERNS], usize, &'a [&'a [u8]]), i32> {
    let mut opts = Options::new();
    let mut patterns: [[u8; MAX_PATTERN_LEN]; MAX_PATTERNS] = [[0; MAX_PATTERN_LEN]; MAX_PATTERNS];
    let mut pattern_lens: [usize; MAX_PATTERNS] = [0; MAX_PATTERNS];
    let mut pcount: usize = 0;
    let mut i = 0;
    let mut explicit_pattern = false;

    while i < args.len() {
        let a = args[i];
        match a {
            b"-F" => { opts.fixed = true; i += 1; }
            b"-E" => { opts.extended = true; i += 1; }
            b"-c" => { opts.count_only = true; i += 1; }
            b"-l" => { opts.files_with_matches = true; i += 1; }
            b"-L" => { opts.files_without_matches = true; i += 1; }
            b"-q" => { opts.quiet = true; i += 1; }
            b"-i" => { opts.icase = true; i += 1; }
            b"-n" => { opts.line_number = true; i += 1; }
            b"-s" => { opts.silent_err = true; i += 1; }
            b"-v" => { opts.invert = true; i += 1; }
            b"-x" => { opts.whole_line = true; i += 1; }
            b"-e" => {
                if i + 1 >= args.len() {
                    let _ = io.write_stderr(b"grep: option requires an argument -- 'e'\n");
                    return Err(2);
                }
                i += 1;
                let pat = args[i];
                if pcount < MAX_PATTERNS && !pat.is_empty() {
                    let plen = pat.len().min(MAX_PATTERN_LEN);
                    patterns[pcount][..plen].copy_from_slice(&pat[..plen]);
                    pattern_lens[pcount] = plen;
                    pcount += 1;
                }
                explicit_pattern = true;
                i += 1;
            }
            b"-f" => {
                if i + 1 >= args.len() {
                    let _ = io.write_stderr(b"grep: option requires an argument -- 'f'\n");
                    return Err(2);
                }
                i += 1;
                let pat_file = args[i];
                pcount = read_pattern_file(io, pat_file, &mut patterns, &mut pattern_lens, pcount)?;
                explicit_pattern = true;
                i += 1;
            }
            b"--" => { i += 1; break; }
            _a if _a.starts_with(b"-") && _a.len() > 1 => {
                let _ = io.write_stderr(b"grep: invalid option -- '");
                let _ = io.write_stderr(_a);
                let _ = io.write_stderr(b"'\n");
                return Err(2);
            }
            _ => break,
        }
    }

    if !explicit_pattern && i < args.len() {
        let pat = args[i];
        if pcount < MAX_PATTERNS && !pat.is_empty() {
            let plen = pat.len().min(MAX_PATTERN_LEN);
            patterns[pcount][..plen].copy_from_slice(&pat[..plen]);
            pattern_lens[pcount] = plen;
            pcount += 1;
        }
        i += 1;
    }

    Ok((opts, patterns, pattern_lens, pcount, &args[i..]))
}

fn read_pattern_file(
    io: &mut impl Io,
    path: &[u8],
    patterns: &mut [[u8; MAX_PATTERN_LEN]; MAX_PATTERNS],
    pattern_lens: &mut [usize; MAX_PATTERNS],
    mut pcount: usize,
) -> Result<usize, i32> {
    let fd = io.open(path).map_err(|_| {
        let _ = io.write_stderr(b"grep: cannot open pattern file\n");
        2i32
    })?;
    let mut buf = [0u8; BUF_SIZE];
    let mut carry = [0u8; MAX_PATTERN_LEN];
    let mut carry_len = 0usize;
    let mut retries = 0;

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) if n <= buf.len() => {
                let end = carry_len + n;
                if end > MAX_PATTERN_LEN {
                    let _ = io.close(fd);
                    return Err(2);
                }
                carry[carry_len..end].copy_from_slice(&buf[..n]);
                let mut pos = carry_len;
                carry_len = end;

                while pos < carry_len {
                    let nl = match carry[pos..carry_len].iter().position(|&b| b == b'\n') {
                        Some(off) => pos + off,
                        None => {
                            if pos > 0 {
                                let rem = carry_len - pos;
                                carry.copy_within(pos..carry_len, 0);
                                carry_len = rem;
                            }
                            break;
                        }
                    };

                    let line = &carry[pos..nl];
                    if pcount < MAX_PATTERNS {
                        let llen = line.len().min(MAX_PATTERN_LEN);
                        patterns[pcount][..llen].copy_from_slice(&line[..llen]);
                        pattern_lens[pcount] = llen;
                        pcount += 1;
                    } else {
                        let _ = io.close(fd);
                        return Err(2);
                    }
                    pos = nl + 1;
                }

                if pos >= carry_len {
                    carry_len = 0;
                }
                retries = 0;
            }
            Ok(_) => { let _ = io.close(fd); return Err(2); }
            Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                retries += 1;
                io.yield_now();
            }
            Err(_) => { let _ = io.close(fd); return Err(2); }
        }
    }

    // Final pattern without newline
    if carry_len > 0 && pcount < MAX_PATTERNS {
        let llen = carry_len.min(MAX_PATTERN_LEN);
        patterns[pcount][..llen].copy_from_slice(&carry[..llen]);
        pattern_lens[pcount] = llen;
        pcount += 1;
    }

    let _ = io.close(fd);
    Ok(pcount)
}

fn grep_fd(
    io: &mut impl Io,
    fd: Fd,
    filename: &[u8],
    show_filename: bool,
    opts: &Options,
    pattern_data: &[[u8; MAX_PATTERN_LEN]; MAX_PATTERNS],
    pattern_lens: &[usize; MAX_PATTERNS],
    pattern_count: usize,
) -> Result<bool, ()> {
    // Compile patterns
    let mut compiled: [Option<Regex>; MAX_PATTERNS] = core::array::from_fn(|_| None);
    let mut num_compiled = 0usize;

    for i in 0..pattern_count {
        let pat = &pattern_data[i][..pattern_lens[i]];
        if opts.fixed {
            compiled[num_compiled] = None;
        } else {
            match Regex::compile(pat, opts.icase, opts.whole_line) {
                Ok(re) => {
                    compiled[num_compiled] = Some(re);
                }
                Err(_) => {
                    let _ = io.write_stderr(b"grep: invalid regular expression\n");
                    return Err(());
                }
            }
        }
        num_compiled += 1;
    }

    let mut buf = [0u8; BUF_SIZE];
    let mut carry = [0u8; MAX_LINE_LEN];
    let mut carry_len: usize = 0;
    let mut line_number: u64 = 0;
    let mut match_count: u64 = 0;
    let mut matched_any: bool = false;
    let mut retries = 0;
    let mut num_buf = [0u8; 24];

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) if n <= buf.len() => {
                let end = carry_len + n;
                if end > MAX_LINE_LEN {
                    let _ = io.write_stderr(b"grep: line too long\n");
                    return Err(());
                }
                carry[carry_len..end].copy_from_slice(&buf[..n]);
                let mut pos = carry_len;
                carry_len = end;

                while pos < carry_len {
                    let nl = match carry[pos..carry_len].iter().position(|&b| b == b'\n') {
                        Some(off) => pos + off,
                        None => {
                            if pos > 0 {
                                let rem = carry_len - pos;
                                carry.copy_within(pos..carry_len, 0);
                                carry_len = rem;
                            }
                            break;
                        }
                    };

                    line_number += 1;
                    let line = &carry[pos..nl];

                    let is_match = if opts.fixed {
                        fixed_match(pattern_data, pattern_lens, pattern_count, line, opts)
                    } else {
                        regex_match(&compiled[..num_compiled], line)
                    };

                    let selected = if opts.invert { !is_match } else { is_match };

                    if selected {
                        matched_any = true;
                        match_count += 1;

                        if !opts.quiet && !opts.count_only
                            && !opts.files_with_matches && !opts.files_without_matches
                        {
                            if show_filename && !opts.files_with_matches {
                                let _ = io.write_stdout(filename);
                                let _ = io.write_stdout(b":");
                            }
                            if opts.line_number {
                                let s = format_u64(line_number, &mut num_buf);
                                let _ = io.write_stdout(s);
                                let _ = io.write_stdout(b":");
                            }
                            let _ = io.write_stdout(line);
                            let _ = io.write_stdout(b"\n");
                        }
                    }

                    pos = nl + 1;
                }

                if pos >= carry_len {
                    carry_len = 0;
                }
                retries = 0;
            }
            Ok(_) => return Err(()),
            Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                retries += 1;
                io.yield_now();
            }
            Err(_) => {
                if !opts.silent_err {
                    let _ = io.write_stderr(b"grep: read error\n");
                }
                return Err(());
            }
        }
    }

    // Final partial line
    if carry_len > 0 {
        line_number += 1;
        let line = &carry[..carry_len];

        let is_match = if opts.fixed {
            fixed_match(pattern_data, pattern_lens, pattern_count, line, opts)
        } else {
            regex_match(&compiled[..num_compiled], line)
        };

        let selected = if opts.invert { !is_match } else { is_match };

        if selected {
            matched_any = true;
            match_count += 1;

            if !opts.quiet && !opts.count_only
                && !opts.files_with_matches && !opts.files_without_matches
            {
                if show_filename && !opts.files_with_matches {
                    let _ = io.write_stdout(filename);
                    let _ = io.write_stdout(b":");
                }
                if opts.line_number {
                    let s = format_u64(line_number, &mut num_buf);
                    let _ = io.write_stdout(s);
                    let _ = io.write_stdout(b":");
                }
                let _ = io.write_stdout(line);
                let _ = io.write_stdout(b"\n");
            }
        }
    }

    // Handle -c, -l, -L
    if opts.count_only && !opts.quiet {
        if show_filename {
            let _ = io.write_stdout(filename);
            let _ = io.write_stdout(b":");
        }
        let s = format_u64(match_count, &mut num_buf);
        let _ = io.write_stdout(s);
        let _ = io.write_stdout(b"\n");
    }

    if opts.files_with_matches && matched_any {
        let _ = io.write_stdout(filename);
        let _ = io.write_stdout(b"\n");
    }

    if opts.files_without_matches && !matched_any {
        let _ = io.write_stdout(filename);
        let _ = io.write_stdout(b"\n");
    }

    Ok(matched_any)
}

fn fixed_match(
    data: &[[u8; MAX_PATTERN_LEN]; MAX_PATTERNS],
    lens: &[usize; MAX_PATTERNS],
    count: usize,
    line: &[u8],
    opts: &Options,
) -> bool {
    for i in 0..count {
        let pat = &data[i][..lens[i]];
        if opts.whole_line {
            if opts.icase {
                if pat.len() == line.len() && pat.iter().zip(line.iter()).all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase()) {
                    return true;
                }
            } else if pat == line {
                return true;
            }
        } else {
            if opts.icase {
                if line.windows(pat.len()).any(|w| w.iter().zip(pat.iter()).all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())) {
                    return true;
                }
            } else if contains_substring(line, pat) {
                return true;
            }
        }
    }
    false
}

fn contains_substring(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn regex_match(compiled: &[Option<Regex>], line: &[u8]) -> bool {
    for re_opt in compiled {
        if let Some(re) = re_opt {
            if re.is_match(line) {
                return true;
            }
        }
    }
    false
}

fn format_u64<'a>(v: u64, buf: &'a mut [u8; 24]) -> &'a [u8] {
    let mut n = 0;
    let mut v = v;
    loop {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 { break; }
    }
    let end = n;
    for i in 0..end / 2 { buf.swap(i, end - 1 - i); }
    &buf[..end]
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
