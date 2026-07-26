//! Small, deterministic `od` implementation for the Sunlight C locale.
//!
//! It supports the byte/word formats most useful in shell scripts: `-b`,
//! `-c`, `-d`, `-o`, `-x`, `-t`, `-A`, `-j`, and `-N`.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

const BUF_SIZE: usize = 512;
const DATA_CAP: usize = 16 * 1024;
const RETRIES: usize = 8;

pub trait Io {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write_stdout(&mut self, bytes: &[u8]);
    fn write_stderr(&mut self, bytes: &[u8]);
    fn yield_now(&mut self);
}

#[derive(Clone, Copy)]
struct Options<'a> {
    format: Format,
    address: Address,
    skip: usize,
    count: Option<usize>,
    path: Option<&'a [u8]>,
}

#[derive(Clone, Copy)]
struct Format {
    kind: Kind,
    unit: usize,
    base: u32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    Char,
}
#[derive(Clone, Copy)]
enum Address {
    Octal,
    Decimal,
    Hex,
    None,
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let options = match parse_args(args, io) {
        Some(v) => v,
        None => return 2,
    };
    let fd = match options.path {
        None | Some(b"-") => STDIN,
        Some(path) => match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                io.write_stderr(b"od: cannot open ");
                io.write_stderr(path);
                io.write_stderr(b"\n");
                return 1;
            }
        },
    };
    let mut data = [0u8; DATA_CAP];
    let mut input = [0u8; BUF_SIZE];
    let mut total = 0usize;
    let mut skipped = 0usize;
    let mut retries = 0;
    let mut eof = false;
    while total < DATA_CAP && !eof {
        let n = match io.read(fd, &mut input) {
            Ok(n) if n <= input.len() => {
                retries = 0;
                n
            }
            Ok(_) => {
                io.write_stderr(b"od: invalid read count\n");
                if fd != STDIN {
                    let _ = io.close(fd);
                }
                return 1;
            }
            Err(Errno::Again) if retries < RETRIES => {
                retries += 1;
                io.yield_now();
                continue;
            }
            Err(_) => {
                io.write_stderr(b"od: read error\n");
                if fd != STDIN {
                    let _ = io.close(fd);
                }
                return 1;
            }
        };
        if n == 0 {
            eof = true;
            continue;
        }
        let mut at = 0;
        if skipped < options.skip {
            let drop = (options.skip - skipped).min(n);
            skipped += drop;
            at = drop;
        }
        if at < n {
            let wanted = options
                .count
                .map_or(n - at, |limit| limit.saturating_sub(total));
            let copy = (n - at).min(wanted).min(DATA_CAP - total);
            data[total..total + copy].copy_from_slice(&input[at..at + copy]);
            total += copy;
            if options.count.is_some_and(|limit| total >= limit) {
                eof = true;
            }
        }
    }
    if fd != STDIN {
        let _ = io.close(fd);
    }
    if total == DATA_CAP && options.count.is_none() {
        io.write_stderr(b"od: input too large\n");
        return 1;
    }
    print_data(
        &data[..total],
        options.format,
        options.address,
        options.skip,
        io,
    );
    0
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Option<Options<'a>> {
    let mut format = Format {
        kind: Kind::Number,
        unit: 2,
        base: 8,
    };
    let mut address = Address::Octal;
    let mut skip = 0;
    let mut count = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if arg == b"--" {
            i += 1;
            break;
        }
        if !arg.starts_with(b"-") || arg == b"-" {
            break;
        }
        if arg == b"-v" {
            i += 1;
            continue;
        }
        if arg.len() == 2 {
            match arg[1] {
                b'b' => {
                    format = Format {
                        kind: Kind::Number,
                        unit: 1,
                        base: 8,
                    };
                    i += 1;
                    continue;
                }
                b'c' => {
                    format = Format {
                        kind: Kind::Char,
                        unit: 1,
                        base: 0,
                    };
                    i += 1;
                    continue;
                }
                b'd' => {
                    format = Format {
                        kind: Kind::Number,
                        unit: 2,
                        base: 10,
                    };
                    i += 1;
                    continue;
                }
                b'o' => {
                    format = Format {
                        kind: Kind::Number,
                        unit: 2,
                        base: 8,
                    };
                    i += 1;
                    continue;
                }
                b'x' | b'X' => {
                    format = Format {
                        kind: Kind::Number,
                        unit: 2,
                        base: 16,
                    };
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        if arg.len() > 2
            && arg[1..]
                .iter()
                .all(|byte| matches!(*byte, b'b' | b'c' | b'd' | b'o' | b'x' | b'X'))
        {
            for &byte in &arg[1..] {
                format = match byte {
                    b'b' => Format {
                        kind: Kind::Number,
                        unit: 1,
                        base: 8,
                    },
                    b'c' => Format {
                        kind: Kind::Char,
                        unit: 1,
                        base: 0,
                    },
                    b'd' => Format {
                        kind: Kind::Number,
                        unit: 2,
                        base: 10,
                    },
                    b'o' => Format {
                        kind: Kind::Number,
                        unit: 2,
                        base: 8,
                    },
                    b'x' | b'X' => Format {
                        kind: Kind::Number,
                        unit: 2,
                        base: 16,
                    },
                    _ => unreachable!(),
                };
            }
            i += 1;
            continue;
        }
        let Some(kind) = arg.get(1).copied() else {
            i += 1;
            continue;
        };
        let tail = &arg[2..];
        let value = if !tail.is_empty() {
            tail
        } else if i + 1 < args.len() {
            i += 1;
            args[i]
        } else {
            io.write_stderr(b"od: option needs an argument\n");
            return None;
        };
        match kind {
            b'A' => {
                address = match value {
                    b"d" => Address::Decimal,
                    b"o" => Address::Octal,
                    b"x" => Address::Hex,
                    b"n" => Address::None,
                    _ => {
                        io.write_stderr(b"od: invalid address base\n");
                        return None;
                    }
                }
            }
            b'j' => skip = parse_number(value)?,
            b'N' => count = Some(parse_number(value)?),
            b't' => format = parse_type(value, io)?,
            b'v' => {}
            _ => {
                io.write_stderr(b"od: invalid option\n");
                return None;
            }
        }
        i += 1;
    }
    let rest = &args[i..];
    if rest.len() > 2 {
        io.write_stderr(b"od: too many operands\n");
        return None;
    }
    if rest.len() == 2 {
        skip = parse_number(rest[1])?;
    }
    Some(Options {
        format,
        address,
        skip,
        count,
        path: rest.first().copied(),
    })
}

fn parse_type(value: &[u8], io: &mut impl Io) -> Option<Format> {
    let kind = value.first().copied()?;
    if kind == b'c' {
        return Some(Format {
            kind: Kind::Char,
            unit: 1,
            base: 0,
        });
    }
    let base = match kind {
        b'o' => 8,
        b'd' | b'u' => 10,
        b'x' | b'X' => 16,
        _ => {
            io.write_stderr(b"od: unsupported type\n");
            return None;
        }
    };
    let unit = match value.get(1).copied().unwrap_or(b'2') {
        b'1' => 1,
        b'2' => 2,
        b'4' => 4,
        b'8' => 8,
        _ => {
            io.write_stderr(b"od: invalid type width\n");
            return None;
        }
    };
    Some(Format {
        kind: Kind::Number,
        unit,
        base,
    })
}
fn parse_number(value: &[u8]) -> Option<usize> {
    if value.is_empty() {
        return None;
    }
    let mut n = 0usize;
    for &b in value {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

fn print_data(data: &[u8], format: Format, address: Address, start: usize, io: &mut impl Io) {
    let per_line = 16 / format.unit.max(1);
    let mut at = 0;
    while at < data.len() {
        if !matches!(address, Address::None) {
            print_address(start + at, address, io);
            io.write_stdout(b" ");
        } else {
            io.write_stdout(b" ");
        }
        let line_end = (at + per_line * format.unit).min(data.len());
        let mut unit_at = at;
        let mut first = true;
        while unit_at < line_end {
            if !first {
                io.write_stdout(b" ");
            }
            first = false;
            let available = (line_end - unit_at).min(format.unit);
            if format.kind == Kind::Char {
                print_char(data[unit_at], io);
            } else {
                let mut value = 0u64;
                for j in 0..available {
                    value |= (data[unit_at + j] as u64) << (8 * j);
                }
                let width = match format.base {
                    8 => format.unit * 3,
                    10 => (format.unit * 8 + 2) / 3,
                    16 => format.unit * 2,
                    _ => 0,
                };
                print_number(value, format.base, width, io);
            }
            unit_at += format.unit;
        }
        io.write_stdout(b"\n");
        at = line_end;
    }
}
fn print_address(value: usize, address: Address, io: &mut impl Io) {
    let (base, width) = match address {
        Address::Octal => (8, 7),
        Address::Decimal => (10, 7),
        Address::Hex => (16, 7),
        Address::None => (10, 0),
    };
    print_number(value as u64, base, width, io);
}
fn print_number(mut value: u64, base: u32, width: usize, io: &mut impl Io) {
    let mut digits = [0u8; 32];
    let mut len = 0;
    if value == 0 {
        digits[0] = b'0';
        len = 1;
    }
    while value > 0 {
        let d = (value % base as u64) as u8;
        digits[len] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        len += 1;
        value /= base as u64;
    }
    for _ in len..width {
        io.write_stdout(b"0");
    }
    for i in (0..len).rev() {
        io.write_stdout(&[digits[i]]);
    }
}
fn print_char(byte: u8, io: &mut impl Io) {
    match byte {
        0 => io.write_stdout(b"\\0"),
        b'\n' => io.write_stdout(b"\\n"),
        b'\t' => io.write_stdout(b"\\t"),
        b'\r' => io.write_stdout(b"\\r"),
        0x20..=0x7e => io.write_stdout(&[byte]),
        _ => {
            io.write_stdout(b"\\");
            let mut digits = [0u8; 3];
            let mut n = byte;
            for i in (0..3).rev() {
                digits[i] = b'0' + n % 8;
                n /= 8;
            }
            io.write_stdout(&digits)
        }
    }
}

pub struct NativeIo;
impl Io for NativeIo {
    fn open(&mut self, p: &[u8]) -> Result<Fd, Errno> {
        sunlight_libc::open(p)
    }
    fn read(&mut self, f: Fd, b: &mut [u8]) -> Result<usize, Errno> {
        sunlight_libc::read(f, b)
    }
    fn close(&mut self, f: Fd) -> Result<(), Errno> {
        sunlight_libc::close(f)
    }
    fn write_stdout(&mut self, b: &[u8]) {
        let _ = sunlight_libc::write_all(STDOUT, b);
    }
    fn write_stderr(&mut self, b: &[u8]) {
        let _ = sunlight_libc::write_all(STDERR, b);
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
        fn new(i: &'static [u8]) -> Self {
            Self {
                input: i,
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
        fn write_stdout(&mut self, b: &[u8]) {
            self.out.extend_from_slice(b)
        }
        fn write_stderr(&mut self, b: &[u8]) {
            self.err.extend_from_slice(b)
        }
        fn yield_now(&mut self) {}
    }
    #[test]
    fn hex_bytes() {
        let mut m = Mock::new(b"abc");
        assert_eq!(run(&[b"-An", b"-tx1"], &mut m), 0);
        assert_eq!(m.out, b" 61 62 63\n");
    }
    #[test]
    fn skips_and_limits() {
        let mut m = Mock::new(b"abcdef");
        assert_eq!(run(&[b"-j2", b"-N2", b"-tx1"], &mut m), 0);
        assert_eq!(m.out, b"0000002 63 64\n");
    }
}
