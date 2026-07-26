//! Bounded POSIX-oriented `xargs` for the native libc utility.
//!
//! Baseline: `xargs [options] [command [initial-arguments...]]`
//!
//! Options:
//! - `-n max-args` — max arguments appended per command invocation
//! - `-0` / `-d delim` — use NUL / single-byte delimiter instead of whitespace
//! - `-r` — do not run command if stdin yields no operands (GNU-compatible)
//!
//! Default command is `/bin/echo`. Command names without `/` are resolved as
//! `/bin/<name>` then `/usr/bin/<name>`.
//!
//! Each invocation is spawned and waited on. Exit status is the last non-zero
//! child status, or 1 on utility errors, or 2 on usage errors.

use sunlight_libc::{Errno, Fd, MAX_ARGS, STDERR, STDIN, STDOUT};

const READ_RETRIES: usize = 8;
const BUF_SIZE: usize = 512;
const MAX_TOKEN: usize = 255;
const MAX_PENDING: usize = 12;

pub trait Io {
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]);
    fn yield_now(&mut self);
    /// Resolve and spawn `argv[0]` with `argv`. Returns child pid.
    fn spawn(&mut self, path: &[u8], argv: &[&[u8]]) -> Result<u64, Errno>;
    /// Wait for child; returns exit status.
    fn waitpid(&mut self, pid: u64) -> Result<u64, Errno>;
    /// Optional existence probe used when resolving relative command names.
    fn path_exists(&mut self, path: &[u8]) -> bool;
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

struct Options {
    max_args: usize,
    delim: Option<u8>,
    no_run_if_empty: bool,
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (opts, command, initial) = match parse_args(args, io) {
        Some(v) => v,
        None => return 2,
    };

    let resolved = match resolve_command(command, io) {
        Some(p) => p,
        None => {
            io.write_stderr(b"xargs: cannot resolve command: ");
            io.write_stderr(command);
            io.write_stderr(b"\n");
            return 127;
        }
    };
    let path = resolved.as_bytes();

    // Cap batch size by remaining argv slots (command + initial + tokens + room).
    let reserved = 1 + initial.len(); // path in argv[0] slot + initials
    if reserved >= MAX_ARGS {
        io.write_stderr(b"xargs: too many initial arguments\n");
        return 1;
    }
    let room = (MAX_ARGS - reserved).min(MAX_PENDING).min(opts.max_args);
    if room == 0 {
        io.write_stderr(b"xargs: no room for operands\n");
        return 1;
    }

    let mut token_storage = [[0u8; MAX_TOKEN]; MAX_PENDING];
    let mut token_lens = [0usize; MAX_PENDING];
    let mut pending = 0usize;
    let mut had_any = false;
    let mut code = 0i32;

    let mut reader = TokenReader::new(opts.delim);
    loop {
        let mut tok = [0u8; MAX_TOKEN];
        match reader.next_token(io, &mut tok) {
            Ok(None) => break,
            Ok(Some(len)) => {
                had_any = true;
                token_storage[pending][..len].copy_from_slice(&tok[..len]);
                token_lens[pending] = len;
                pending += 1;
                if pending >= room {
                    let status =
                        run_batch(path, command, initial, &token_storage, &token_lens, pending, io);
                    if status != 0 {
                        code = status;
                    }
                    pending = 0;
                }
            }
            Err(status) => return status,
        }
    }

    if pending > 0 {
        let status = run_batch(path, command, initial, &token_storage, &token_lens, pending, io);
        if status != 0 {
            code = status;
        }
    } else if !opts.no_run_if_empty && !had_any {
        let status = run_batch(path, command, initial, &token_storage, &token_lens, 0, io);
        if status != 0 {
            code = status;
        }
    }
    code
}

fn run_batch(
    path: &[u8],
    command: &[u8],
    initial: &[&[u8]],
    tokens: &[[u8; MAX_TOKEN]; MAX_PENDING],
    lens: &[usize; MAX_PENDING],
    count: usize,
    io: &mut impl Io,
) -> i32 {
    // Build argv: [command_name, initial..., tokens...]
    let mut argv_bufs: [&[u8]; MAX_ARGS] = [&[]; MAX_ARGS];
    let mut n = 0usize;
    argv_bufs[n] = command;
    n += 1;
    for &arg in initial {
        if n >= MAX_ARGS {
            io.write_stderr(b"xargs: argument list too long\n");
            return 1;
        }
        argv_bufs[n] = arg;
        n += 1;
    }
    for i in 0..count {
        if n >= MAX_ARGS {
            io.write_stderr(b"xargs: argument list too long\n");
            return 1;
        }
        argv_bufs[n] = &tokens[i][..lens[i]];
        n += 1;
    }

    match io.spawn(path, &argv_bufs[..n]) {
        Ok(pid) => match io.waitpid(pid) {
            Ok(status) => {
                if status > 255 {
                    1
                } else {
                    status as i32
                }
            }
            Err(_) => {
                io.write_stderr(b"xargs: waitpid failed\n");
                1
            }
        },
        Err(_) => {
            io.write_stderr(b"xargs: failed to spawn command\n");
            1
        }
    }
}

/// Resolved absolute path storage for relative command names.
struct ResolvedPath {
    buf: [u8; 64],
    len: usize,
}

impl ResolvedPath {
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

fn resolve_command(command: &[u8], io: &mut impl Io) -> Option<ResolvedPath> {
    if command.is_empty() {
        return None;
    }
    if command.contains(&b'/') {
        if command.len() > 64 {
            return None;
        }
        let mut r = ResolvedPath {
            buf: [0; 64],
            len: command.len(),
        };
        r.buf[..command.len()].copy_from_slice(command);
        return Some(r);
    }
    // Try /bin/<cmd> then /usr/bin/<cmd>
    for prefix in [b"/bin/" as &[u8], b"/usr/bin/"] {
        let total = prefix.len() + command.len();
        if total > 64 {
            continue;
        }
        let mut r = ResolvedPath {
            buf: [0; 64],
            len: total,
        };
        r.buf[..prefix.len()].copy_from_slice(prefix);
        r.buf[prefix.len()..total].copy_from_slice(command);
        if io.path_exists(r.as_bytes()) {
            return Some(r);
        }
    }
    // Fall back to /bin/ even if probe fails — spawn will report error.
    let prefix = b"/bin/";
    let total = prefix.len() + command.len();
    if total > 64 {
        return None;
    }
    let mut r = ResolvedPath {
        buf: [0; 64],
        len: total,
    };
    r.buf[..prefix.len()].copy_from_slice(prefix);
    r.buf[prefix.len()..total].copy_from_slice(command);
    Some(r)
}

struct TokenReader {
    buf: [u8; BUF_SIZE],
    pos: usize,
    end: usize,
    eof: bool,
    delim: Option<u8>,
}

impl TokenReader {
    fn new(delim: Option<u8>) -> Self {
        Self {
            buf: [0; BUF_SIZE],
            pos: 0,
            end: 0,
            eof: false,
            delim,
        }
    }

    fn next_token(&mut self, io: &mut impl Io, out: &mut [u8; MAX_TOKEN]) -> Result<Option<usize>, i32> {
        // Skip leading delimiters / whitespace.
        loop {
            if self.pos >= self.end {
                if !self.fill(io)? {
                    return Ok(None);
                }
            }
            let b = self.buf[self.pos];
            if self.is_delim(b) {
                self.pos += 1;
                continue;
            }
            break;
        }

        let mut len = 0usize;
        loop {
            if self.pos >= self.end {
                if !self.fill(io)? {
                    break;
                }
            }
            let b = self.buf[self.pos];
            if self.is_delim(b) {
                self.pos += 1;
                break;
            }
            if len >= MAX_TOKEN {
                io.write_stderr(b"xargs: token too long\n");
                return Err(1);
            }
            out[len] = b;
            len += 1;
            self.pos += 1;
        }
        if len == 0 {
            Ok(None)
        } else {
            Ok(Some(len))
        }
    }

    fn is_delim(&self, b: u8) -> bool {
        match self.delim {
            Some(d) => b == d,
            None => b == b' ' || b == b'\t' || b == b'\n' || b == b'\r',
        }
    }

    fn fill(&mut self, io: &mut impl Io) -> Result<bool, i32> {
        if self.eof {
            return Ok(false);
        }
        let mut retries = 0;
        loop {
            match io.read(STDIN, &mut self.buf) {
                Ok(0) => {
                    self.eof = true;
                    self.pos = 0;
                    self.end = 0;
                    return Ok(false);
                }
                Ok(n) => {
                    // Treat TTY ^D (0x04) as EOF for interactive use.
                    if let Some(i) = self.buf[..n].iter().position(|&b| b == 0x04) {
                        self.end = i;
                        self.pos = 0;
                        self.eof = true;
                        return Ok(i > 0);
                    }
                    self.pos = 0;
                    self.end = n;
                    return Ok(true);
                }
                Err(Errno::Again) if retries < READ_RETRIES => {
                    retries += 1;
                    io.yield_now();
                }
                Err(_) => {
                    io.write_stderr(b"xargs: read error\n");
                    return Err(1);
                }
            }
        }
    }
}

fn parse_args<'a>(
    args: &'a [&'a [u8]],
    io: &mut impl Io,
) -> Option<(Options, &'a [u8], &'a [&'a [u8]])> {
    let mut opts = Options {
        max_args: MAX_PENDING,
        delim: None,
        no_run_if_empty: false,
    };
    let mut i = 0usize;
    while i < args.len() {
        let a = args[i];
        if a == b"--" {
            i += 1;
            break;
        }
        if !a.starts_with(b"-") || a == b"-" {
            break;
        }
        if a == b"-0" {
            opts.delim = Some(0);
            i += 1;
            continue;
        }
        if a == b"-r" {
            opts.no_run_if_empty = true;
            i += 1;
            continue;
        }
        if a == b"-n" || a.starts_with(b"-n") {
            let value = if a == b"-n" {
                i += 1;
                args.get(i).copied()?
            } else {
                &a[2..]
            };
            let n = parse_positive(value)?;
            if n == 0 {
                io.write_stderr(b"xargs: -n must be positive\n");
                return None;
            }
            opts.max_args = n.min(MAX_PENDING);
            i += 1;
            continue;
        }
        if a == b"-d" || a.starts_with(b"-d") {
            let value = if a == b"-d" {
                i += 1;
                args.get(i).copied()?
            } else {
                &a[2..]
            };
            if value.len() != 1 {
                io.write_stderr(b"xargs: -d requires a single-byte delimiter\n");
                return None;
            }
            opts.delim = Some(value[0]);
            i += 1;
            continue;
        }
        io.write_stderr(b"xargs: invalid option\n");
        return None;
    }

    let (command, initial) = if i >= args.len() {
        (b"/bin/echo" as &[u8], &[][..])
    } else {
        (args[i], &args[i + 1..])
    };
    Some((opts, command, initial))
}

fn parse_positive(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() {
        return None;
    }
    let mut n = 0usize;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

pub struct NativeIo;

impl Io for NativeIo {
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
        sunlight_libc::read(fd, buf)
    }
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(STDOUT, bytes)
    }
    fn write_stderr(&mut self, bytes: &[u8]) {
        let _ = sunlight_libc::write_all(STDERR, bytes);
    }
    fn yield_now(&mut self) {
        sunlight_libc::yield_now();
    }
    fn spawn(&mut self, path: &[u8], argv: &[&[u8]]) -> Result<u64, Errno> {
        sunlight_libc::spawn(path, argv, None)
    }
    fn waitpid(&mut self, pid: u64) -> Result<u64, Errno> {
        sunlight_libc::waitpid(pid)
    }
    fn path_exists(&mut self, path: &[u8]) -> bool {
        sunlight_libc::stat(path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    struct Mock {
        input: &'static [u8],
        at: usize,
        spawns: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
        err: Vec<u8>,
        existing: Vec<Vec<u8>>,
    }

    impl Mock {
        fn new(input: &'static [u8]) -> Self {
            Self {
                input,
                at: 0,
                spawns: Vec::new(),
                err: Vec::new(),
                existing: vec![b"/bin/echo".to_vec(), b"/bin/grep".to_vec()],
            }
        }
    }

    impl Io for Mock {
        fn read(&mut self, _: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
            let n = (self.input.len() - self.at).min(buf.len());
            buf[..n].copy_from_slice(&self.input[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
        fn write_stdout(&mut self, _: &[u8]) -> Result<(), Errno> {
            Ok(())
        }
        fn write_stderr(&mut self, b: &[u8]) {
            self.err.extend_from_slice(b);
        }
        fn yield_now(&mut self) {}
        fn spawn(&mut self, path: &[u8], argv: &[&[u8]]) -> Result<u64, Errno> {
            let args = argv.iter().map(|a| a.to_vec()).collect();
            self.spawns.push((path.to_vec(), args));
            Ok(self.spawns.len() as u64)
        }
        fn waitpid(&mut self, _: u64) -> Result<u64, Errno> {
            Ok(0)
        }
        fn path_exists(&mut self, path: &[u8]) -> bool {
            self.existing.iter().any(|p| p.as_slice() == path)
        }
    }

    #[test]
    fn batches_whitespace_tokens_to_echo() {
        let mut m = Mock::new(b"a b  c\nd\n");
        assert_eq!(run(&[], &mut m), 0);
        assert_eq!(m.spawns.len(), 1);
        assert_eq!(m.spawns[0].0, b"/bin/echo");
        assert_eq!(
            m.spawns[0].1,
            vec![
                b"/bin/echo".to_vec(),
                b"a".to_vec(),
                b"b".to_vec(),
                b"c".to_vec(),
                b"d".to_vec()
            ]
        );
    }

    #[test]
    fn respects_n_and_command() {
        let mut m = Mock::new(b"1 2 3 4\n");
        assert_eq!(run(&[b"-n", b"2", b"grep", b"-F"], &mut m), 0);
        assert_eq!(m.spawns.len(), 2);
        assert_eq!(m.spawns[0].0, b"/bin/grep");
        assert_eq!(
            m.spawns[0].1,
            vec![
                b"grep".to_vec(),
                b"-F".to_vec(),
                b"1".to_vec(),
                b"2".to_vec()
            ]
        );
        assert_eq!(
            m.spawns[1].1,
            vec![
                b"grep".to_vec(),
                b"-F".to_vec(),
                b"3".to_vec(),
                b"4".to_vec()
            ]
        );
    }

    #[test]
    fn r_skips_empty_stdin() {
        let mut m = Mock::new(b"");
        assert_eq!(run(&[b"-r", b"echo"], &mut m), 0);
        assert!(m.spawns.is_empty());
    }

    #[test]
    fn empty_stdin_runs_once_by_default() {
        let mut m = Mock::new(b"");
        assert_eq!(run(&[b"echo"], &mut m), 0);
        assert_eq!(m.spawns.len(), 1);
        assert_eq!(m.spawns[0].1, vec![b"echo".to_vec()]);
    }
}
