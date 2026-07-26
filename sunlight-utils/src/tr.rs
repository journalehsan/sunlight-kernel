//! Maintained `tr` implementation.
//!
//! The current Sunlight text contract is the POSIX/C single-byte locale.  The
//! parser therefore represents characters as bytes, which is lossless for C
//! locale input (including NUL and malformed UTF-8).  Locale character
//! classes are implemented for that locale only; equivalence and collating
//! symbols are accepted only when they name one byte.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

const READ_RETRIES: usize = 8;
const BUF_SIZE: usize = 512;
const ARRAY_MAX: usize = 256;

pub trait Io {
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn yield_now(&mut self);
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

#[derive(Clone, Copy)]
struct Array {
    bytes: [u8; ARRAY_MAX],
    len: usize,
}

impl Array {
    const fn empty() -> Self {
        Self { bytes: [0; ARRAY_MAX], len: 0 }
    }

    fn push(&mut self, byte: u8) -> Result<(), ()> {
        if self.len == self.bytes.len() { return Err(()); }
        self.bytes[self.len] = byte;
        self.len += 1;
        Ok(())
    }

    fn contains(&self, byte: u8) -> bool {
        self.bytes[..self.len].contains(&byte)
    }
}

#[derive(Clone, Copy)]
struct Options<'a> {
    complement: bool,
    delete: bool,
    squeeze: bool,
    first: &'a [u8],
    second: Option<&'a [u8]>,
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let Some(opts) = parse_args(args, io) else { return 2; };
    let mut first = match parse_array(opts.first, false, 0, io) {
        Ok(a) => a,
        Err(()) => { diagnostic(io, b"tr: invalid character array\n"); return 2; }
    };
    if first.len == 0 {
        diagnostic(io, b"tr: empty character array\n");
        return 2;
    }
    if opts.complement {
        let mut complement = Array::empty();
        for byte in 0..=u8::MAX {
            if !first.contains(byte) && complement.push(byte).is_err() {
                diagnostic(io, b"tr: character array too large\n");
                return 2;
            }
        }
        first = complement;
    }

    let second = match opts.second {
        Some(raw) => match parse_array(raw, true, first.len, io) {
            Ok(a) => Some(a),
            Err(()) => { diagnostic(io, b"tr: invalid character array\n"); return 2; }
        },
        None => None,
    };

    if opts.delete && opts.squeeze && second.is_none() {
        diagnostic(io, b"tr: -d and -s require two arrays\n");
        return 2;
    }
    if !opts.delete && !opts.squeeze && second.is_none() {
        diagnostic(io, b"tr: missing string2\n");
        return 2;
    }

    let mut translate = [0u8; ARRAY_MAX];
    for (i, slot) in translate.iter_mut().enumerate() { *slot = i as u8; }
    let mut delete_map = [false; ARRAY_MAX];
    let mut squeeze_map = [false; ARRAY_MAX];
    let second = second.unwrap_or(Array::empty());

    if opts.delete {
        for i in 0..first.len { delete_map[first.bytes[i] as usize] = true; }
    } else if !opts.squeeze {
        for i in 0..first.len {
            // POSIX leaves a short string2 unspecified.  The BSD-compatible
            // last-character extension is deterministic and useful for the
            // standard `tr 0-9 0` idiom.
            let target = if i < second.len { second.bytes[i] } else { second.bytes[second.len - 1] };
            translate[first.bytes[i] as usize] = target;
        }
    }
    if opts.squeeze {
        let source = if second.len == 0 { &first } else { &second };
        for i in 0..source.len { squeeze_map[source.bytes[i] as usize] = true; }
    }

    transform(io, &translate, &delete_map, &squeeze_map, opts.delete, opts.squeeze)
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Option<Options<'a>> {
    let mut complement = false;
    let mut delete = false;
    let mut squeeze = false;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if arg == b"--" { index += 1; break; }
        if arg.len() < 2 || arg[0] != b'-' { break; }
        let mut valid = true;
        for &c in &arg[1..] {
            match c {
                b'c' | b'C' => {
                    if complement && ((c == b'c') || (c == b'C')) { valid = false; }
                    complement = true;
                }
                b'd' => delete = true,
                b's' => squeeze = true,
                _ => valid = false,
            }
        }
        if !valid {
            diagnostic(io, b"tr: invalid option\n");
            return None;
        }
        index += 1;
    }
    if complement && args[..index].iter().any(|a| a.contains(&b'c') && a.contains(&b'C')) {
        diagnostic(io, b"tr: conflicting complement options\n");
        return None;
    }
    let remaining = &args[index..];
    if remaining.len() != 1 && remaining.len() != 2 {
        diagnostic(io, b"tr: usage: tr [-Ccs] string1 [string2]\n");
        return None;
    }
    if !delete && !squeeze && remaining.len() != 2 {
        diagnostic(io, b"tr: string2 is required\n");
        return None;
    }
    Some(Options { complement, delete, squeeze, first: remaining[0], second: remaining.get(1).copied() })
}

fn parse_array(input: &[u8], allow_repeat: bool, extend_to: usize, io: &mut impl Io) -> Result<Array, ()> {
    let mut out = Array::empty();
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'[' {
            if let Some((name, next, marker)) = bracket_name(input, i) {
                if marker == b':' {
                    expand_class(name, &mut out).map_err(|_| ())?;
                } else if name.len() == 1 {
                    out.push(name[0]).map_err(|_| ())?;
                } else {
                    diagnostic(io, b"tr: unsupported collating expression\n");
                    return Err(());
                }
                i = next;
                continue;
            }
            let (atom, next, repeat) = parse_bracket(input, i, allow_repeat)?;
            for _ in 0..repeat { out.push(atom).map_err(|_| ())?; }
            i = next;
            continue;
        }
        let (start, next) = parse_atom(input, i)?;
        if next < input.len() && input[next] == b'-' && next + 1 < input.len() {
            let (end, after) = parse_atom(input, next + 1)?;
            if start > end {
                diagnostic(io, b"tr: descending range\n");
                return Err(());
            }
            let mut c = start;
            loop {
                out.push(c).map_err(|_| ())?;
                if c == end { break; }
                c = c.wrapping_add(1);
            }
            i = after;
        } else {
            out.push(start).map_err(|_| ())?;
            i = next;
        }
    }
    if out.len == 0 { diagnostic(io, b"tr: invalid empty array\n"); return Err(()); }
    // [x*] and [x*0] mean enough copies to extend string2 to string1.
    if extend_to > out.len && allow_repeat && input.windows(2).any(|w| w == b"*]") {
        let last = out.bytes[out.len - 1];
        while out.len < extend_to { out.push(last).map_err(|_| ())?; }
    }
    Ok(out)
}

fn bracket_name<'a>(input: &'a [u8], at: usize) -> Option<(&'a [u8], usize, u8)> {
    let marker = *input.get(at + 1)?;
    if marker != b':' && marker != b'=' && marker != b'.' { return None; }
    let rest = input.get(at + 2..)?;
    let end = rest.windows(2).position(|w| w[0] == marker && w[1] == b']')?;
    Some((&rest[..end], at + 2 + end + 2, marker))
}

fn parse_atom(input: &[u8], at: usize) -> Result<(u8, usize), ()> {
    if at >= input.len() { return Err(()); }
    if input[at] != b'\\' { return Ok((input[at], at + 1)); }
    if at + 1 >= input.len() { return Err(()); }
    let c = input[at + 1];
    let value = match c {
        b'a' => 7, b'b' => 8, b'f' => 12, b'n' => 10, b'r' => 13,
        b't' => 9, b'v' => 11, b'\\' => 92,
        b'0'..=b'7' => {
            let mut n = 0u16;
            let mut j = at + 1;
            let mut count = 0;
            while j < input.len() && count < 3 && (b'0'..=b'7').contains(&input[j]) {
                n = n * 8 + (input[j] - b'0') as u16;
                j += 1; count += 1;
            }
            return Ok((n as u8, j));
        }
        _ => return Err(()),
    };
    Ok((value, at + 2))
}

fn parse_bracket(input: &[u8], at: usize, allow_repeat: bool) -> Result<(u8, usize, usize), ()> {
    // Character classes, equivalence classes, and collating symbols are
    // single-byte in the C locale.  Expand classes in parse_array's caller by
    // using the first byte here; classes are handled by parse_class below.
    if input.get(at + 1) == Some(&b':') || input.get(at + 1) == Some(&b'=') || input.get(at + 1) == Some(&b'.') {
        let marker = input[at + 1];
        let end = input[at + 2..].windows(2).position(|w| w == [marker, b']']).ok_or(())? + at + 2;
        let name = &input[at + 2..end];
        let class = if marker == b':' { parse_class(name) } else if name.len() == 1 { Some([name[0]; 1]) } else { None };
        let Some(bytes) = class else { return Err(()); };
        let mut count = 0;
        // Classes can contain many bytes.  This compact parser returns the
        // first byte and lets the class expansion path below handle the full
        // set through a special encoded marker.
        let _ = bytes;
        if marker == b':' && name == b"lower" { count = 0; }
        let next = end + 2;
        if count == 0 && marker == b':' {
            // A class is represented by a sentinel; parse_array expands it.
            return Err(());
        }
        return Ok((name[0], next, 1));
    }
    let (byte, mut next) = if input.get(at + 1) == Some(&b']') {
        (b']', at + 2)
    } else {
        parse_atom(input, at + 1)?
    };
    if input.get(next) != Some(&b'*') { 
        if input.get(next) != Some(&b']') { return Err(()); }
        return Ok((byte, next + 1, 1));
    }
    if !allow_repeat { return Err(()); }
    next += 1;
    let start = next;
    while next < input.len() && input[next].is_ascii_digit() { next += 1; }
    if input.get(next) != Some(&b']') { return Err(()); }
    let count = if start == next { 0 } else { parse_count(&input[start..next])? };
    Ok((byte, next + 1, if count == 0 { 1 } else { count }))
}

fn parse_count(bytes: &[u8]) -> Result<usize, ()> {
    let base = if bytes.len() > 1 && bytes[0] == b'0' { 8 } else { 10 };
    let mut n = 0usize;
    for &b in bytes {
        let d = if b.is_ascii_digit() { (b - b'0') as usize } else { return Err(()); };
        if d >= base || n > (ARRAY_MAX - d) / base { return Err(()); }
        n = n * base + d;
    }
    Ok(n)
}

fn parse_class(name: &[u8]) -> Option<[u8; 1]> {
    // Kept as a validation hook.  Full expansion is performed by
    // expand_class, which is intentionally explicit about C-locale bytes.
    match name {
        b"alnum" | b"blank" | b"digit" | b"lower" | b"punct" | b"upper"
        | b"alpha" | b"cntrl" | b"graph" | b"print" | b"space" | b"xdigit" => Some([0; 1]),
        _ => None,
    }
}

fn expand_class(name: &[u8], out: &mut Array) -> Result<(), ()> {
    if parse_class(name).is_none() {
        return Err(());
    }
    let mut add = |b: u8| out.push(b);
    for b in 0..=u8::MAX {
        let yes = match name {
            b"alnum" => b.is_ascii_alphanumeric(),
            b"alpha" => b.is_ascii_alphabetic(),
            b"blank" => b == b' ' || b == b'\t',
            b"cntrl" => b < 0x20 || b == 0x7f,
            b"digit" => b.is_ascii_digit(),
            b"graph" => (0x21..=0x7e).contains(&b),
            b"lower" => b.is_ascii_lowercase(),
            b"print" => (0x20..=0x7e).contains(&b),
            b"punct" => b.is_ascii_punctuation(),
            b"space" => b.is_ascii_whitespace(),
            b"upper" => b.is_ascii_uppercase(),
            b"xdigit" => b.is_ascii_hexdigit(),
            _ => unreachable!(),
        };
        if yes { add(b)?; }
    }
    Ok(())
}

fn transform(io: &mut impl Io, translate: &[u8; ARRAY_MAX], delete: &[bool; ARRAY_MAX], squeeze: &[bool; ARRAY_MAX], deleting: bool, squeezing: bool) -> i32 {
    let mut input = [0u8; BUF_SIZE];
    let mut output = [0u8; BUF_SIZE];
    let mut out_len = 0;
    let mut retries = 0;
    let mut previous = None;
    loop {
        let n = match io.read(STDIN, &mut input) {
            Ok(n) if n <= input.len() => { retries = 0; n }
            Ok(_) => { diagnostic(io, b"tr: invalid read count\n"); return 1; }
            Err(Errno::Again) if retries < READ_RETRIES => { retries += 1; io.yield_now(); continue; }
            Err(_) => { diagnostic(io, b"tr: read error\n"); return 1; }
        };
        if n == 0 { break; }
        for &raw in &input[..n] {
            if deleting && delete[raw as usize] { continue; }
            let mapped = if deleting { raw } else { translate[raw as usize] };
            if squeezing && squeeze[mapped as usize] && previous == Some(mapped) { continue; }
            if out_len == output.len() {
                if io.write_stdout(&output).is_err() { diagnostic(io, b"tr: write error\n"); return 1; }
                out_len = 0;
            }
            output[out_len] = mapped;
            out_len += 1;
            previous = Some(mapped);
        }
    }
    if out_len != 0 && io.write_stdout(&output[..out_len]).is_err() { diagnostic(io, b"tr: write error\n"); return 1; }
    0
}

fn diagnostic(io: &mut impl Io, message: &[u8]) { let _ = io.write_stderr(message); }

pub struct NativeIo;
impl Io for NativeIo {
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> { sunlight_libc::read(fd, buf) }
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> { sunlight_libc::write_all(STDOUT, bytes) }
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno> { sunlight_libc::write_all(STDERR, bytes) }
    fn yield_now(&mut self) { sunlight_libc::yield_now(); }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    struct Mock { input: Vec<u8>, at: usize, output: Vec<u8>, error: Vec<u8> }
    impl Mock { fn new(input: &[u8]) -> Self { Self { input: input.to_vec(), at: 0, output: Vec::new(), error: Vec::new() } } }
    impl Io for Mock {
        fn read(&mut self, _fd: Fd, b: &mut [u8]) -> Result<usize, Errno> { if self.at == self.input.len() { return Ok(0); } let n = (self.input.len()-self.at).min(b.len()); b[..n].copy_from_slice(&self.input[self.at..self.at+n]); self.at += n; Ok(n) }
        fn write_stdout(&mut self, b: &[u8]) -> Result<(), Errno> { self.output.extend_from_slice(b); Ok(()) }
        fn write_stderr(&mut self, b: &[u8]) -> Result<(), Errno> { self.error.extend_from_slice(b); Ok(()) }
        fn yield_now(&mut self) {}
    }

    #[test]
    fn translates_and_preserves_binary_input() {
        let mut io = Mock::new(b"a\0b\n");
        assert_eq!(run(&[b"a", b"b"], &mut io), 0);
        assert_eq!(io.output, b"b\0b\n");
    }

    #[test]
    fn deletes_and_squeezes() {
        let mut io = Mock::new(b"aabb  c");
        assert_eq!(run(&[b"-ds", b"a", b" "], &mut io), 0);
        assert_eq!(io.output, b"bb c");
    }

    #[test]
    fn rejects_bad_range_and_escape() {
        let mut io = Mock::new(b"");
        assert_eq!(run(&[b"z-a", b"x"], &mut io), 2);
        assert_eq!(run(&[b"\\" , b"x"], &mut io), 2);
    }
}
