//! POSIX-oriented `split` with line and byte chunking.

use sunlight_libc::{Errno, Fd, O_CREAT, O_TRUNC, O_WRONLY, STDERR, STDIN};

const BUF_SIZE: usize = 512;
const MAX_SUFFIX: usize = 8;
const RETRIES: usize = 8;

pub trait Io {
    fn open_input(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn create_output(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write(&mut self, fd: Fd, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]);
    fn yield_now(&mut self);
}

#[derive(Clone, Copy)]
enum Mode {
    Lines(u64),
    Bytes(u64),
}

#[derive(Clone, Copy)]
struct Options<'a> {
    mode: Mode,
    suffix_len: usize,
    input: Option<&'a [u8]>,
    prefix: &'a [u8],
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let options = match parse_args(args, io) {
        Some(v) => v,
        None => return 2,
    };
    let input = match options.input {
        Some(b"-") | None => STDIN,
        Some(path) => match io.open_input(path) {
            Ok(fd) => fd,
            Err(_) => {
                diag(io, b"split: cannot open ");
                io.write_stderr(path);
                diag(io, b"\n");
                return 1;
            }
        },
    };
    let result = split_stream(input, &options, io);
    if input != STDIN {
        let _ = io.close(input);
    }
    result
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Option<Options<'a>> {
    let mut mode = Mode::Lines(1000);
    let mut suffix_len = 2;
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
        let (kind, value) = match arg.get(1) {
            Some(&kind) => {
                let tail = &arg[2..];
                if !tail.is_empty() {
                    (kind, Some(tail))
                } else if i + 1 < args.len() {
                    i += 1;
                    (kind, Some(args[i]))
                } else {
                    diag(io, b"split: option needs an argument\n");
                    return None;
                }
            }
            None => {
                i += 1;
                continue;
            }
        };
        match kind {
            b'l' => mode = Mode::Lines(parse_positive(value?)?),
            b'b' => mode = Mode::Bytes(parse_positive(value?)?),
            b'a' => suffix_len = parse_suffix_len(value?)?,
            _ => {
                diag(io, b"split: invalid option\n");
                return None;
            }
        }
        i += 1;
    }
    let rest = &args[i..];
    if rest.len() > 2 {
        diag(io, b"split: too many operands\n");
        return None;
    }
    Some(Options {
        mode,
        suffix_len,
        input: rest.first().copied(),
        prefix: rest.get(1).copied().unwrap_or(b"x"),
    })
}

fn parse_positive(value: &[u8]) -> Option<u64> {
    let mut n = 0u64;
    if value.is_empty() {
        return None;
    }
    for &b in value {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    (n > 0).then_some(n)
}
fn parse_suffix_len(value: &[u8]) -> Option<usize> {
    let n = parse_positive(value)?;
    (n <= MAX_SUFFIX as u64).then_some(n as usize)
}

fn split_stream(fd: Fd, options: &Options<'_>, io: &mut impl Io) -> i32 {
    let mut input = [0u8; BUF_SIZE];
    let mut retries = 0;
    let mut output = None;
    let mut suffix = 0u64;
    let mut used = 0u64;
    let mut code = 0;
    loop {
        let n = match io.read(fd, &mut input) {
            Ok(n) if n <= input.len() => {
                retries = 0;
                n
            }
            Ok(_) => {
                diag(io, b"split: invalid read count\n");
                code = 1;
                break;
            }
            Err(Errno::Again) if retries < RETRIES => {
                retries += 1;
                io.yield_now();
                continue;
            }
            Err(_) => {
                diag(io, b"split: read error\n");
                code = 1;
                break;
            }
        };
        if n == 0 {
            break;
        }
        let mut at = 0;
        while at < n {
            if output.is_none() || chunk_full(used, &options.mode) {
                if let Some(old) = output.take() {
                    let _ = io.close(old);
                }
                let mut name = [0u8; 256];
                let name_len = make_name(options.prefix, suffix, options.suffix_len, &mut name);
                let Some(name_len) = name_len else {
                    diag(io, b"split: suffix exhausted\n");
                    code = 1;
                    break;
                };
                output = match io.create_output(&name[..name_len]) {
                    Ok(fd) => Some(fd),
                    Err(_) => {
                        diag(io, b"split: cannot create ");
                        io.write_stderr(&name[..name_len]);
                        diag(io, b"\n");
                        code = 1;
                        break;
                    }
                };
                suffix = suffix.saturating_add(1);
                used = 0;
            }
            let remaining = match options.mode {
                Mode::Bytes(limit) => (limit - used).min((n - at) as u64) as usize,
                Mode::Lines(limit) => {
                    let lines_left = limit - used;
                    let mut seen = 0u64;
                    let mut end = n - at;
                    for (index, &byte) in input[at..n].iter().enumerate() {
                        if byte == b'\n' {
                            seen += 1;
                            if seen == lines_left {
                                end = index + 1;
                                break;
                            }
                        }
                    }
                    end
                }
            };
            if remaining == 0 {
                continue;
            }
            let bytes = &input[at..at + remaining];
            let Some(outfd) = output else {
                code = 1;
                break;
            };
            if io.write(outfd, bytes).is_err() {
                diag(io, b"split: write error\n");
                code = 1;
                break;
            }
            at += remaining;
            match options.mode {
                Mode::Bytes(_) => used += remaining as u64,
                Mode::Lines(_) => {
                    used += bytes.iter().filter(|&&b| b == b'\n').count() as u64;
                }
            }
        }
        if code != 0 {
            break;
        }
    }
    if let Some(fd) = output {
        let _ = io.close(fd);
    }
    code
}

fn chunk_full(used: u64, mode: &Mode) -> bool {
    match mode {
        Mode::Lines(limit) | Mode::Bytes(limit) => used >= *limit,
    }
}

fn make_name(prefix: &[u8], mut number: u64, width: usize, out: &mut [u8; 256]) -> Option<usize> {
    if prefix.len() + width > out.len() {
        return None;
    }
    out[..prefix.len()].copy_from_slice(prefix);
    for slot in out[prefix.len()..prefix.len() + width].iter_mut().rev() {
        *slot = b'a' + (number % 26) as u8;
        number /= 26;
    }
    (number == 0).then_some(prefix.len() + width)
}

fn diag(io: &mut impl Io, message: &[u8]) {
    io.write_stderr(message);
}

pub struct NativeIo;
impl Io for NativeIo {
    fn open_input(&mut self, path: &[u8]) -> Result<Fd, Errno> {
        sunlight_libc::open(path)
    }
    fn create_output(&mut self, path: &[u8]) -> Result<Fd, Errno> {
        sunlight_libc::open_with_flags_mode(path, O_WRONLY | O_CREAT | O_TRUNC, 0o644)
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
        files: Vec<(Vec<u8>, Vec<u8>)>,
        err: Vec<u8>,
    }
    impl Mock {
        fn new(input: &'static [u8]) -> Self {
            Self {
                input,
                at: 0,
                files: Vec::new(),
                err: Vec::new(),
            }
        }
    }
    impl Io for Mock {
        fn open_input(&mut self, _: &[u8]) -> Result<Fd, Errno> {
            Ok(STDIN)
        }
        fn create_output(&mut self, p: &[u8]) -> Result<Fd, Errno> {
            self.files.push((p.to_vec(), Vec::new()));
            Ok(Fd(self.files.len() as u32 + 2))
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
        fn write(&mut self, f: Fd, b: &[u8]) -> Result<(), Errno> {
            self.files[(f.0 - 3) as usize].1.extend_from_slice(b);
            Ok(())
        }
        fn write_stderr(&mut self, b: &[u8]) {
            self.err.extend_from_slice(b)
        }
        fn yield_now(&mut self) {}
    }
    #[test]
    fn splits_lines_and_names_files() {
        let mut m = Mock::new(b"a\nb\nc\n");
        assert_eq!(run(&[b"-l", b"2", b"-"], &mut m), 0);
        assert_eq!(m.files[0].0, b"xaa".to_vec());
        assert_eq!(m.files[0].1, b"a\nb\n".to_vec());
        assert_eq!(m.files[1].1, b"c\n".to_vec());
    }
    #[test]
    fn splits_bytes_without_losing_data() {
        let mut m = Mock::new(b"abcdef");
        assert_eq!(run(&[b"-b3", b"-", b"out"], &mut m), 0);
        assert_eq!(m.files[0].1, b"abc".to_vec());
        assert_eq!(m.files[1].1, b"def".to_vec());
    }
}
