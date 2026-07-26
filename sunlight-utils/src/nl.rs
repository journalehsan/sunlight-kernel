//! A bounded, byte-preserving implementation of the POSIX `nl` utility.
//!
//! The C locale is intentional: a line is a byte stream terminated by LF and
//! numbering does not attempt to interpret UTF-8 characters.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

const READ_RETRIES: usize = 8;
const BUF_SIZE: usize = 512;
const MAX_FILES: usize = 8;

pub trait Io {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]);
    fn yield_now(&mut self);
}

#[derive(Clone, Copy)]
struct Options<'a> {
    all: bool,
    number_nonempty: bool,
    width: usize,
    separator: &'a [u8],
    number: u64,
    increment: u64,
    format: NumberFormat,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NumberFormat {
    Right,
    Left,
    Zero,
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (options, paths) = match parse_args(args, io) {
        Some(v) => v,
        None => return 2,
    };
    if paths.len() > MAX_FILES {
        io.write_stderr(b"nl: too many input files\n");
        return 1;
    }
    let mut number = options.number;
    let mut code = 0;
    if paths.is_empty() {
        if number_stream(STDIN, &options, &mut number, io) != 0 {
            code = 1;
        }
    } else {
        for &path in paths {
            let fd = if path == b"-" {
                STDIN
            } else {
                match io.open(path) {
                    Ok(fd) => fd,
                    Err(_) => {
                        io.write_stderr(b"nl: cannot open ");
                        io.write_stderr(path);
                        io.write_stderr(b"\n");
                        code = 1;
                        continue;
                    }
                }
            };
            if number_stream(fd, &options, &mut number, io) != 0 {
                code = 1;
            }
            if fd != STDIN {
                let _ = io.close(fd);
            }
        }
    }
    code
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Option<(Options<'a>, &'a [&'a [u8]])> {
    let mut options = Options {
        all: false,
        number_nonempty: true,
        width: 6,
        separator: b"\t",
        number: 1,
        increment: 1,
        format: NumberFormat::Right,
    };
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if arg == b"--" {
            i += 1;
            break;
        }
        if arg == b"-ba" {
            options.all = true;
            options.number_nonempty = false;
            i += 1;
            continue;
        }
        if arg == b"-bt" {
            options.all = false;
            options.number_nonempty = true;
            i += 1;
            continue;
        }
        if arg == b"-bp" {
            options.all = false;
            options.number_nonempty = true;
            i += 1;
            continue;
        }
        if arg == b"-p" {
            i += 1;
            continue;
        }
        if !arg.starts_with(b"-") || arg == b"-" {
            break;
        }
        let (letter, value) = match arg.get(1) {
            Some(b'-') => {
                diag(io, b"nl: invalid option\n");
                return None;
            }
            Some(&letter) => {
                let tail = &arg[2..];
                if !tail.is_empty() {
                    (letter, Some(tail))
                } else if i + 1 < args.len() {
                    i += 1;
                    (letter, Some(args[i]))
                } else {
                    diag(io, b"nl: option needs an argument\n");
                    return None;
                }
            }
            None => {
                i += 1;
                continue;
            }
        };
        match letter {
            b'b' => match value {
                Some(b"a") => {
                    options.all = true;
                    options.number_nonempty = false;
                }
                Some(b"t") | Some(b"p") => {
                    options.all = false;
                    options.number_nonempty = true;
                }
                _ => {
                    diag(io, b"nl: invalid body type\n");
                    return None;
                }
            },
            b'n' => {
                options.format = match value {
                    Some(b"ln") => NumberFormat::Left,
                    Some(b"rn") => NumberFormat::Right,
                    Some(b"rz") => NumberFormat::Zero,
                    _ => {
                        diag(io, b"nl: invalid number format\n");
                        return None;
                    }
                }
            }
            b's' => options.separator = value?,
            b'w' => options.width = parse_positive(value?)?,
            b'v' => options.number = parse_u64(value?)?,
            b'i' => options.increment = parse_u64(value?)?,
            b'l' => {
                let _ = parse_positive(value?)?;
            }
            _ => {
                diag(io, b"nl: invalid option\n");
                return None;
            }
        }
        i += 1;
    }
    Some((options, &args[i..]))
}

fn parse_positive(value: &[u8]) -> Option<usize> {
    let n = parse_u64(value)?;
    (n > 0 && n <= 64).then_some(n as usize)
}
fn parse_u64(value: &[u8]) -> Option<u64> {
    if value.is_empty() {
        return None;
    }
    let mut n = 0u64;
    for &b in value {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(n)
}

fn number_stream(fd: Fd, options: &Options<'_>, number: &mut u64, io: &mut impl Io) -> i32 {
    let mut input = [0u8; BUF_SIZE];
    let mut retries = 0;
    let mut at_start = true;
    let mut has_text = false;
    let mut numbered = false;
    loop {
        let n = match io.read(fd, &mut input) {
            Ok(n) if n <= input.len() => {
                retries = 0;
                n
            }
            Ok(_) => {
                diag(io, b"nl: invalid read count\n");
                return 1;
            }
            Err(Errno::Again) if retries < READ_RETRIES => {
                retries += 1;
                io.yield_now();
                continue;
            }
            Err(_) => {
                diag(io, b"nl: read error\n");
                return 1;
            }
        };
        if n == 0 {
            break;
        }
        for &byte in &input[..n] {
            if at_start && (options.all || byte != b'\n') {
                write_number(io, *number, options);
                numbered = true;
                has_text = false;
                at_start = false;
            }
            if byte != b'\n' {
                has_text = true;
            }
            if byte == b'\n' {
                if at_start && options.all {
                    write_number(io, *number, options);
                }
                if io.write_stdout(&[byte]).is_err() {
                    diag(io, b"nl: write error\n");
                    return 1;
                }
                if numbered && (options.all || has_text) {
                    *number = (*number).saturating_add(options.increment);
                }
                at_start = true;
                numbered = false;
                has_text = false;
            } else {
                if io.write_stdout(&[byte]).is_err() {
                    diag(io, b"nl: write error\n");
                    return 1;
                }
            }
        }
    }
    // A final unterminated line still receives its number, but retains the
    // input's missing newline exactly as POSIX utilities generally do.
    if !at_start && !numbered {
        write_number(io, *number, options);
    }
    0
}

fn write_number(io: &mut impl Io, number: u64, options: &Options<'_>) {
    let mut digits = [0u8; 20];
    let mut n = number;
    let mut len = 0;
    if n == 0 {
        digits[0] = b'0';
        len = 1;
    }
    while n != 0 {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    let padding = options.width.saturating_sub(len);
    match options.format {
        NumberFormat::Right => {
            for _ in 0..padding {
                let _ = io.write_stdout(b" ");
            }
        }
        NumberFormat::Zero => {
            for _ in 0..padding {
                let _ = io.write_stdout(b"0");
            }
        }
        NumberFormat::Left => {}
    }
    for i in (0..len).rev() {
        let _ = io.write_stdout(&[digits[i]]);
    }
    if options.format == NumberFormat::Left {
        for _ in len..options.width {
            let _ = io.write_stdout(b" ");
        }
    }
    let _ = io.write_stdout(options.separator);
}

fn diag(io: &mut impl Io, message: &[u8]) {
    io.write_stderr(message);
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
        out: Vec<u8>,
        err: Vec<u8>,
    }
    impl Mock {
        fn new(input: &'static [u8]) -> Self {
            Self {
                input,
                at: 0,
                out: Vec::new(),
                err: Vec::new(),
            }
        }
    }
    impl Io for Mock {
        fn open(&mut self, _: &[u8]) -> Result<Fd, Errno> {
            Err(Errno::NoEntry)
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
        fn write_stdout(&mut self, b: &[u8]) -> Result<(), Errno> {
            self.out.extend_from_slice(b);
            Ok(())
        }
        fn write_stderr(&mut self, b: &[u8]) {
            self.err.extend_from_slice(b);
        }
        fn yield_now(&mut self) {}
    }
    #[test]
    fn numbers_nonempty_lines() {
        let mut m = Mock::new(b"a\n\nxy\n");
        assert_eq!(run(&[], &mut m), 0);
        assert_eq!(m.out, b"     1\ta\n\n     2\txy\n");
    }
    #[test]
    fn numbers_all_lines_left_and_start() {
        let mut m = Mock::new(b"\nq\n");
        assert_eq!(
            run(&[b"-ba", b"-n", b"rz", b"-v", b"9", b"-w", b"3"], &mut m),
            0
        );
        assert_eq!(m.out, b"009\t\n010\tq\n");
    }
}
