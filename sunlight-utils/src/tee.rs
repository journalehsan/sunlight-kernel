//! POSIX `tee`: copy standard input to standard output and named files.

use sunlight_libc::{Errno, Fd, O_APPEND, O_CREAT, O_TRUNC, O_WRONLY, STDERR, STDIN, STDOUT};

const BUF_SIZE: usize = 512;
const MAX_OUTPUTS: usize = 8;
const RETRIES: usize = 8;

pub trait Io {
    fn open_output(&mut self, path: &[u8], flags: u64) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write(&mut self, fd: Fd, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]);
    fn yield_now(&mut self);
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (append, paths) = match parse_args(args, io) {
        Some(v) => v,
        None => return 2,
    };
    if paths.len() > MAX_OUTPUTS {
        diag(io, b"tee: too many output files\n");
        return 1;
    }
    let flags = O_WRONLY | O_CREAT | if append { O_APPEND } else { O_TRUNC };
    let mut outputs = [Fd(0); MAX_OUTPUTS];
    let mut count = 0;
    for &path in paths {
        match io.open_output(path, flags) {
            Ok(fd) => {
                outputs[count] = fd;
                count += 1;
            }
            Err(_) => {
                diag(io, b"tee: cannot open ");
                io.write_stderr(path);
                diag(io, b"\n");
                for fd in outputs.iter().take(count) {
                    let _ = io.close(*fd);
                }
                return 1;
            }
        }
    }

    let mut input = [0u8; BUF_SIZE];
    let mut retries = 0;
    let mut code = 0;
    loop {
        let n = match io.read(STDIN, &mut input) {
            Ok(n) if n <= input.len() => {
                retries = 0;
                n
            }
            Ok(_) => {
                diag(io, b"tee: invalid read count\n");
                code = 1;
                break;
            }
            Err(Errno::Again) if retries < RETRIES => {
                retries += 1;
                io.yield_now();
                continue;
            }
            Err(_) => {
                diag(io, b"tee: read error\n");
                code = 1;
                break;
            }
        };
        if n == 0 {
            break;
        }
        // Sunlight's current raw TTY forwards ^D as byte 0x04 rather than
        // synthesizing an EOF return. Treat it as EOF here so interactive
        // `tee` behaves like the POSIX utility while the TTY line discipline
        // remains intentionally outside this small native command.
        let eof = input[..n].iter().position(|&byte| byte == 0x04);
        let bytes = &input[..eof.unwrap_or(n)];
        if !bytes.is_empty() {
            if io.write(STDOUT, bytes).is_err() {
                diag(io, b"tee: write error\n");
                code = 1;
                break;
            }
            for fd in outputs.iter().take(count) {
                if io.write(*fd, bytes).is_err() {
                    diag(io, b"tee: write error\n");
                    code = 1;
                    break;
                }
            }
        }
        if code != 0 {
            break;
        }
        if eof.is_some() {
            break;
        }
    }
    for fd in outputs.iter().take(count) {
        if io.close(*fd).is_err() {
            code = 1;
        }
    }
    code
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Option<(bool, &'a [&'a [u8]])> {
    let mut append = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if arg == b"--" {
            i += 1;
            break;
        }
        if arg == b"-a" {
            append = true;
            i += 1;
            continue;
        }
        if arg == b"-i" {
            i += 1;
            continue;
        }
        if arg.starts_with(b"-") && arg != b"-" {
            let mut valid = true;
            for &byte in &arg[1..] {
                if byte == b'a' {
                    append = true;
                } else if byte != b'i' {
                    valid = false;
                }
            }
            if !valid {
                diag(io, b"tee: invalid option\n");
                return None;
            }
            i += 1;
            continue;
        }
        break;
    }
    Some((append, &args[i..]))
}

fn diag(io: &mut impl Io, message: &[u8]) {
    io.write_stderr(message);
}

pub struct NativeIo;
impl Io for NativeIo {
    fn open_output(&mut self, path: &[u8], flags: u64) -> Result<Fd, Errno> {
        sunlight_libc::open_with_flags_mode(path, flags, 0o644)
    }
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
        sunlight_libc::read(fd, buf)
    }
    fn close(&mut self, fd: Fd) -> Result<(), Errno> {
        sunlight_libc::close(fd)
    }
    fn write(&mut self, fd: Fd, bytes: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(fd, bytes)
    }
    fn write_stderr(&mut self, bytes: &[u8]) {
        let _ = sunlight_libc::write_all(STDERR, bytes);
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
        input: &'static [u8],
        at: usize,
        files: [Vec<u8>; 2],
        opens: usize,
        out: Vec<u8>,
        err: Vec<u8>,
    }
    impl Mock {
        fn new(input: &'static [u8]) -> Self {
            Self {
                input,
                at: 0,
                files: [Vec::new(), Vec::new()],
                opens: 0,
                out: Vec::new(),
                err: Vec::new(),
            }
        }
    }
    impl Io for Mock {
        fn open_output(&mut self, _: &[u8], _: u64) -> Result<Fd, Errno> {
            let fd = Fd(3 + self.opens as u32);
            self.opens += 1;
            Ok(fd)
        }
        fn read(&mut self, _: Fd, b: &mut [u8]) -> Result<usize, Errno> {
            let n = (self.input.len() - self.at).min(b.len());
            b[..n].copy_from_slice(&self.input[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
        fn close(&mut self, _: Fd) -> Result<(), Errno> {
            Ok(())
        }
        fn write(&mut self, fd: Fd, b: &[u8]) -> Result<(), Errno> {
            if fd == STDOUT {
                self.out.extend_from_slice(b);
            } else {
                self.files[(fd.0 - 3) as usize].extend_from_slice(b);
            }
            Ok(())
        }
        fn write_stderr(&mut self, b: &[u8]) {
            self.err.extend_from_slice(b);
        }
        fn yield_now(&mut self) {}
    }
    #[test]
    fn copies_to_stdout_and_files() {
        let mut m = Mock::new(b"hello\n");
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        assert_eq!(m.out, b"hello\n");
        assert_eq!(m.files[0], b"hello\n");
    }
    #[test]
    fn accepts_combined_append_flags() {
        let mut m = Mock::new(b"x");
        assert_eq!(run(&[b"-ai", b"a"], &mut m), 0);
        assert_eq!(m.files[0], b"x");
    }

    #[test]
    fn treats_raw_tty_ctrl_d_as_eof() {
        let mut m = Mock::new(b"hello\x04");
        assert_eq!(run(&[b"a"], &mut m), 0);
        assert_eq!(m.out, b"hello");
        assert_eq!(m.files[0], b"hello");
    }
}
