//! Bounded, streaming POSIX `join` for the maintained C-locale runtime.

use crate::compare;
use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

const BUF_SIZE: usize = 512;
pub const MAX_LINE_LEN: usize = 4096;
pub const MAX_GROUP: usize = 8;
const MAX_OUTPUT: usize = 32;
const RETRIES: usize = 8;

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

#[derive(Clone, Copy)]
struct Record {
    bytes: [u8; MAX_LINE_LEN],
    len: usize,
}
impl Record {
    const fn empty() -> Self {
        Self {
            bytes: [0; MAX_LINE_LEN],
            len: 0,
        }
    }
    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}
struct Reader {
    fd: Fd,
    buf: [u8; BUF_SIZE],
    carry: [u8; MAX_LINE_LEN + 1],
    carry_len: usize,
    eof: bool,
    error: bool,
}
impl Reader {
    fn new(fd: Fd) -> Self {
        Self {
            fd,
            buf: [0; BUF_SIZE],
            carry: [0; MAX_LINE_LEN + 1],
            carry_len: 0,
            eof: false,
            error: false,
        }
    }
    fn next(&mut self, io: &mut impl Io, out: &mut Record) -> bool {
        loop {
            if let Some(n) = self.carry[..self.carry_len]
                .iter()
                .position(|&b| b == b'\n')
            {
                if n > MAX_LINE_LEN {
                    self.error = true;
                    return false;
                }
                out.bytes[..n].copy_from_slice(&self.carry[..n]);
                out.len = n;
                let used = n + 1;
                self.carry.copy_within(used..self.carry_len, 0);
                self.carry_len -= used;
                return true;
            }
            if self.eof {
                if self.carry_len == 0 {
                    return false;
                }
                if self.carry_len > MAX_LINE_LEN {
                    self.error = true;
                    return false;
                }
                out.bytes[..self.carry_len].copy_from_slice(&self.carry[..self.carry_len]);
                out.len = self.carry_len;
                self.carry_len = 0;
                return true;
            }
            let mut retry = 0;
            let n = loop {
                match io.read(self.fd, &mut self.buf) {
                    Ok(n) if n <= self.buf.len() => break n,
                    Ok(_) => {
                        self.error = true;
                        return false;
                    }
                    Err(Errno::Again) if retry < RETRIES => {
                        retry += 1;
                        io.yield_now()
                    }
                    Err(_) => {
                        self.error = true;
                        return false;
                    }
                }
            };
            if n == 0 {
                self.eof = true;
                continue;
            }
            if self
                .carry_len
                .checked_add(n)
                .filter(|&n| n <= MAX_LINE_LEN + 1)
                .is_none()
            {
                self.error = true;
                return false;
            }
            self.carry[self.carry_len..self.carry_len + n].copy_from_slice(&self.buf[..n]);
            self.carry_len += n;
        }
    }
}

#[derive(Clone, Copy)]
struct FieldMode {
    delimiter: Option<u8>,
}
#[derive(Clone, Copy)]
struct OutputField {
    file: u8,
    field: usize,
}
#[derive(Clone, Copy)]
struct Options<'a> {
    f1: usize,
    f2: usize,
    a1: bool,
    a2: bool,
    v1: bool,
    v2: bool,
    delimiter: Option<u8>,
    replacement: Option<&'a [u8]>,
    output: [OutputField; MAX_OUTPUT],
    output_len: usize,
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (opts, paths) = match parse_args(args, io) {
        Some(v) => v,
        None => return 2,
    };
    if paths[0] == b"-" && paths[1] == b"-" {
        diag(io, b"join: standard input specified twice\n");
        return 2;
    }
    let fd1 = open_one(paths[0], io);
    let Ok(fd1) = fd1 else { return 1 };
    let fd2 = match open_one(paths[1], io) {
        Ok(f) => f,
        Err(_) => {
            if fd1 != STDIN {
                let _ = io.close(fd1);
            }
            return 1;
        }
    };
    let result = join_files(fd1, fd2, &opts, io);
    if fd1 != STDIN {
        let _ = io.close(fd1);
    }
    if fd2 != STDIN {
        let _ = io.close(fd2);
    }
    result
}
fn open_one(path: &[u8], io: &mut impl Io) -> Result<Fd, i32> {
    if path == b"-" {
        Ok(STDIN)
    } else {
        io.open(path).map_err(|_| {
            diag_path(io, b"join: cannot open ", path);
            1
        })
    }
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Option<(Options<'a>, &'a [&'a [u8]])> {
    let mut o = Options {
        f1: 1,
        f2: 1,
        a1: false,
        a2: false,
        v1: false,
        v2: false,
        delimiter: None,
        replacement: None,
        output: [OutputField { file: 0, field: 0 }; MAX_OUTPUT],
        output_len: 0,
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == b"--" {
            i += 1;
            break;
        }
        if a == b"-a" || a == b"-v" || a == b"-e" || a == b"-o" || a == b"-t" {
            if i + 1 == args.len() {
                diag(io, b"join: option needs an argument\n");
                return None;
            }
            let val = args[i + 1];
            match a {
                b"-a" => match one(val) {
                    Some(1) => o.a1 = true,
                    Some(2) => o.a2 = true,
                    _ => {
                        diag(io, b"join: invalid file number\n");
                        return None;
                    }
                },
                b"-v" => match one(val) {
                    Some(1) => o.v1 = true,
                    Some(2) => o.v2 = true,
                    _ => {
                        diag(io, b"join: invalid file number\n");
                        return None;
                    }
                },
                b"-e" => o.replacement = Some(val),
                b"-o" => {
                    if parse_output(val, &mut o).is_err() {
                        diag(io, b"join: invalid output list\n");
                        return None;
                    }
                }
                b"-t" => {
                    if val.len() != 1 {
                        diag(io, b"join: delimiter must be one byte\n");
                        return None;
                    } else {
                        o.delimiter = Some(val[0])
                    }
                }
                _ => {}
            }
            i += 2;
            continue;
        }
        if a.starts_with(b"-a") && a.len() > 2 {
            match one(&a[2..]) {
                Some(1) => o.a1 = true,
                Some(2) => o.a2 = true,
                _ => {
                    diag(io, b"join: invalid file number\n");
                    return None;
                }
            }
            i += 1;
            continue;
        }
        if a.starts_with(b"-v") && a.len() > 2 {
            match one(&a[2..]) {
                Some(1) => o.v1 = true,
                Some(2) => o.v2 = true,
                _ => {
                    diag(io, b"join: invalid file number\n");
                    return None;
                }
            }
            i += 1;
            continue;
        }
        if a.starts_with(b"-e") && a.len() > 2 {
            o.replacement = Some(&a[2..]);
            i += 1;
            continue;
        }
        if a.starts_with(b"-o") && a.len() > 2 {
            if parse_output(&a[2..], &mut o).is_err() {
                diag(io, b"join: invalid output list\n");
                return None;
            }
            i += 1;
            continue;
        }
        if a.starts_with(b"-t") && a.len() > 2 {
            if a[2..].len() != 1 {
                diag(io, b"join: delimiter must be one byte\n");
                return None;
            }
            o.delimiter = Some(a[2]);
            i += 1;
            continue;
        }
        if a == b"-1" || a == b"-2" {
            if i + 1 == args.len() {
                diag(io, b"join: option needs an argument\n");
                return None;
            }
            let n = parse_positive(args[i + 1]);
            if n == 0 {
                diag(io, b"join: invalid field number\n");
                return None;
            }
            if a == b"-1" {
                o.f1 = n
            } else {
                o.f2 = n
            }
            i += 2;
            continue;
        }
        if a.starts_with(b"-1") && a.len() > 2 {
            let n = parse_positive(&a[2..]);
            if n == 0 {
                diag(io, b"join: invalid field number\n");
                return None;
            }
            o.f1 = n;
            i += 1;
            continue;
        }
        if a.starts_with(b"-2") && a.len() > 2 {
            let n = parse_positive(&a[2..]);
            if n == 0 {
                diag(io, b"join: invalid field number\n");
                return None;
            }
            o.f2 = n;
            i += 1;
            continue;
        }
        if a.starts_with(b"-") && a.len() > 1 {
            diag(io, b"join: invalid option\n");
            return None;
        }
        break;
    }
    let paths = &args[i..];
    if paths.len() != 2 {
        diag(io, b"join: usage: join [options] file1 file2\n");
        return None;
    }
    Some((o, paths))
}
fn one(b: &[u8]) -> Option<u8> {
    if b.len() == 1 && b[0] == b'1' {
        Some(1)
    } else if b.len() == 1 && b[0] == b'2' {
        Some(2)
    } else {
        None
    }
}
fn parse_positive(b: &[u8]) -> usize {
    if b.is_empty() {
        return 0;
    }
    let mut n = 0usize;
    for &c in b {
        if !c.is_ascii_digit() {
            return 0;
        }
        let d = (c - b'0') as usize;
        if n > (usize::MAX - d) / 10 {
            return 0;
        }
        n = n * 10 + d
    }
    if n == 0 {
        0
    } else {
        n
    }
}
fn parse_output(input: &[u8], o: &mut Options) -> Result<(), ()> {
    let mut pos = 0;
    while pos < input.len() {
        while pos < input.len() && (input[pos] == b',' || input[pos] == b' ' || input[pos] == b'\t')
        {
            pos += 1
        }
        if pos == input.len() {
            break;
        }
        let file = input[pos];
        let n = if file == b'0' {
            pos += 1;
            0
        } else if file == b'1' || file == b'2' {
            pos += 1;
            if input.get(pos) != Some(&b'.') {
                return Err(());
            }
            pos += 1;
            let start = pos;
            while pos < input.len() && input[pos].is_ascii_digit() {
                pos += 1
            }
            if start == pos {
                return Err(());
            }
            parse_positive(&input[start..pos])
        } else {
            return Err(());
        };
        if o.output_len == MAX_OUTPUT || (file != b'0' && n == 0) {
            return Err(());
        }
        o.output[o.output_len] = OutputField { file, field: n };
        o.output_len += 1;
        if pos < input.len() && input[pos] != b',' && input[pos] != b' ' && input[pos] != b'\t' {
            return Err(());
        }
    }
    if o.output_len == 0 {
        Err(())
    } else {
        Ok(())
    }
}

fn join_files<'a>(fd1: Fd, fd2: Fd, opts: &Options<'a>, io: &mut impl Io) -> i32 {
    let mut r1 = Reader::new(fd1);
    let mut r2 = Reader::new(fd2);
    let mut c1 = Record::empty();
    let mut c2 = Record::empty();
    let mut have1 = r1.next(io, &mut c1);
    let mut have2 = r2.next(io, &mut c2);
    let mut g1 = [Record::empty(); MAX_GROUP];
    let mut g2 = [Record::empty(); MAX_GROUP];
    if r1.error || r2.error {
        diag(io, b"join: read error\n");
        return 1;
    }

    loop {
        match (have1, have2) {
            (false, false) => break,
            (true, false) => {
                if want_unmatched(1, opts) && emit_unmatched(&c1, 1, opts, io).is_err() {
                    return 1;
                }
                have1 = r1.next(io, &mut c1);
            }
            (false, true) => {
                if want_unmatched(2, opts) && emit_unmatched(&c2, 2, opts, io).is_err() {
                    return 1;
                }
                have2 = r2.next(io, &mut c2);
            }
            (true, true) => {
                let mode = FieldMode {
                    delimiter: opts.delimiter,
                };
                let k1 = field(c1.as_bytes(), opts.f1, mode).unwrap_or(&[]);
                let k2 = field(c2.as_bytes(), opts.f2, mode).unwrap_or(&[]);
                match compare::byte_cmp(k1, k2) {
                    core::cmp::Ordering::Less => {
                        if want_unmatched(1, opts) && emit_unmatched(&c1, 1, opts, io).is_err() {
                            return 1;
                        }
                        have1 = r1.next(io, &mut c1);
                    }
                    core::cmp::Ordering::Greater => {
                        if want_unmatched(2, opts) && emit_unmatched(&c2, 2, opts, io).is_err() {
                            return 1;
                        }
                        have2 = r2.next(io, &mut c2);
                    }
                    core::cmp::Ordering::Equal => {
                        let mut n1 = 0;
                        g1[n1] = c1;
                        n1 += 1;
                        loop {
                            let mut next = Record::empty();
                            if !r1.next(io, &mut next) {
                                have1 = false;
                                break;
                            }
                            let nk = field(next.as_bytes(), opts.f1, mode).unwrap_or(&[]);
                            if compare::byte_cmp(k1, nk) == core::cmp::Ordering::Equal {
                                if n1 == MAX_GROUP {
                                    diag(io, b"join: duplicate-key group exceeds 8 records\n");
                                    return 1;
                                }
                                g1[n1] = next;
                                n1 += 1;
                            } else {
                                c1 = next;
                                have1 = true;
                                break;
                            }
                        }
                        let mut n2 = 0;
                        g2[n2] = c2;
                        n2 += 1;
                        loop {
                            let mut next = Record::empty();
                            if !r2.next(io, &mut next) {
                                have2 = false;
                                break;
                            }
                            let nk = field(next.as_bytes(), opts.f2, mode).unwrap_or(&[]);
                            if compare::byte_cmp(k2, nk) == core::cmp::Ordering::Equal {
                                if n2 == MAX_GROUP {
                                    diag(io, b"join: duplicate-key group exceeds 8 records\n");
                                    return 1;
                                }
                                g2[n2] = next;
                                n2 += 1;
                            } else {
                                c2 = next;
                                have2 = true;
                                break;
                            }
                        }
                        if !opts.v1 && !opts.v2 {
                            for x in 0..n1 {
                                for y in 0..n2 {
                                    if emit_pair(&g1[x], &g2[y], opts, io).is_err() {
                                        return 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if r1.error || r2.error {
            diag(io, b"join: read error\n");
            return 1;
        }
    }
    0
}
fn want_unmatched(file: u8, o: &Options) -> bool {
    if o.v1 || o.v2 {
        (file == 1 && o.v1) || (file == 2 && o.v2)
    } else {
        (file == 1 && o.a1) || (file == 2 && o.a2)
    }
}

fn field<'a>(line: &'a [u8], number: usize, mode: FieldMode) -> Option<&'a [u8]> {
    if number == 0 {
        return None;
    }
    match mode.delimiter {
        Some(d) => {
            let mut start = 0;
            let mut n = 1;
            for i in 0..=line.len() {
                if i == line.len() || line[i] == d {
                    if n == number {
                        return Some(&line[start..i]);
                    }
                    n += 1;
                    start = i + 1
                }
            }
            None
        }
        None => {
            let mut i = 0;
            let mut n = 0;
            while i < line.len() {
                while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
                    i += 1
                }
                if i == line.len() {
                    break;
                }
                let start = i;
                while i < line.len() && line[i] != b' ' && line[i] != b'\t' {
                    i += 1
                }
                n += 1;
                if n == number {
                    return Some(&line[start..i]);
                }
            }
            None
        }
    }
}
fn field_count(line: &[u8], mode: FieldMode) -> usize {
    match mode.delimiter {
        Some(d) => line.iter().filter(|&&b| b == d).count() + 1,
        None => {
            let mut n = 0;
            let mut in_f = false;
            for &b in line {
                if b == b' ' || b == b'\t' {
                    in_f = false
                } else if !in_f {
                    n += 1;
                    in_f = true
                }
            }
            n
        }
    }
}
fn emit_unmatched<'a>(r: &Record, file: u8, o: &Options<'a>, io: &mut impl Io) -> Result<(), ()> {
    if o.output_len > 0 {
        emit_selected(r, None, file, o, io)
    } else {
        let count = field_count(
            r.as_bytes(),
            FieldMode {
                delimiter: o.delimiter,
            },
        );
        for n in 1..=count {
            if n > 1 {
                write_sep(o, io)?
            }
            write_field(
                field(
                    r.as_bytes(),
                    n,
                    FieldMode {
                        delimiter: o.delimiter,
                    },
                )
                .unwrap_or(&[]),
                io,
            )?
        }
        newline(io)
    }
}
fn emit_pair<'a>(a: &Record, b: &Record, o: &Options<'a>, io: &mut impl Io) -> Result<(), ()> {
    if o.output_len > 0 {
        emit_selected(a, Some(b), 0, o, io)
    } else {
        let mode = FieldMode {
            delimiter: o.delimiter,
        };
        let ka = field(a.as_bytes(), o.f1, mode).unwrap_or(&[]);
        write_field(ka, io)?;
        let ca = field_count(a.as_bytes(), mode);
        for n in 1..=ca {
            if n != o.f1 {
                write_sep(o, io)?;
                write_field(field(a.as_bytes(), n, mode).unwrap_or(&[]), io)?
            }
        }
        let cb = field_count(b.as_bytes(), mode);
        for n in 1..=cb {
            if n != o.f2 {
                write_sep(o, io)?;
                write_field(field(b.as_bytes(), n, mode).unwrap_or(&[]), io)?
            }
        }
        newline(io)
    }
}
fn emit_selected<'a>(
    a: &Record,
    b: Option<&Record>,
    unmatched_file: u8,
    o: &Options<'a>,
    io: &mut impl Io,
) -> Result<(), ()> {
    for i in 0..o.output_len {
        if i > 0 {
            write_sep(o, io)?
        }
        let spec = o.output[i];
        let bytes = if spec.file == 0 {
            field(
                a.as_bytes(),
                if unmatched_file == 2 { o.f2 } else { o.f1 },
                FieldMode {
                    delimiter: o.delimiter,
                },
            )
        } else if spec.file == 1 {
            if unmatched_file == 2 {
                None
            } else {
                field(
                    a.as_bytes(),
                    spec.field,
                    FieldMode {
                        delimiter: o.delimiter,
                    },
                )
            }
        } else if unmatched_file == 1 {
            None
        } else if unmatched_file == 2 {
            field(
                a.as_bytes(),
                spec.field,
                FieldMode {
                    delimiter: o.delimiter,
                },
            )
        } else {
            b.and_then(|r| {
                field(
                    r.as_bytes(),
                    spec.field,
                    FieldMode {
                        delimiter: o.delimiter,
                    },
                )
            })
        };
        match bytes {
            Some(v) if !v.is_empty() => write_field(v, io)?,
            Some(v) => {
                if let Some(e) = o.replacement {
                    write_field(e, io)?
                } else {
                    write_field(v, io)?
                }
            }
            None => {
                if let Some(e) = o.replacement {
                    write_field(e, io)?
                }
            }
        }
    }
    newline(io)
}
fn write_field(b: &[u8], io: &mut impl Io) -> Result<(), ()> {
    io.write_stdout(b).map_err(|_| ())
}
fn write_sep<'a>(o: &Options<'a>, io: &mut impl Io) -> Result<(), ()> {
    io.write_stdout(&[o.delimiter.unwrap_or(b' ')])
        .map_err(|_| ())
}
fn newline(io: &mut impl Io) -> Result<(), ()> {
    io.write_stdout(b"\n").map_err(|_| ())
}
fn diag(io: &mut impl Io, m: &[u8]) {
    let _ = io.write_stderr(m);
}
fn diag_path(io: &mut impl Io, p: &[u8], x: &[u8]) {
    let _ = io.write_stderr(p);
    let _ = io.write_stderr(x);
    let _ = io.write_stderr(b"\n");
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
    fn write_stdout(&mut self, b: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(STDOUT, b)
    }
    fn write_stderr(&mut self, b: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(STDERR, b)
    }
    fn yield_now(&mut self) {
        sunlight_libc::yield_now()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::HashMap;
    use std::vec::Vec;
    struct M {
        f: HashMap<Vec<u8>, (Vec<u8>, usize)>,
        o: Vec<u8>,
        e: Vec<u8>,
    }
    impl M {
        fn new() -> Self {
            Self {
                f: HashMap::new(),
                o: Vec::new(),
                e: Vec::new(),
            }
        }
        fn add(&mut self, n: &[u8], v: &[u8]) {
            self.f.insert(n.to_vec(), (v.to_vec(), 0));
        }
    }
    impl Io for M {
        fn open(&mut self, p: &[u8]) -> Result<Fd, Errno> {
            self.f
                .contains_key(p)
                .then_some(Fd(if p == b"a" { 3 } else { 4 }))
                .ok_or(Errno::NoEntry)
        }
        fn read(&mut self, f: Fd, b: &mut [u8]) -> Result<usize, Errno> {
            let n = if f.0 == 3 {
                b"a x\na x2\nb z\n"
            } else {
                b"a y\na y2\nc q\n"
            };
            static mut A: [usize; 2] = [0, 0];
            let slot = if f.0 == 3 { 0 } else { 1 };
            let at = unsafe { A[slot] };
            if at == n.len() {
                return Ok(0);
            }
            let x = (n.len() - at).min(b.len());
            b[..x].copy_from_slice(&n[at..at + x]);
            unsafe { A[slot] += x }
            Ok(x)
        }
        fn close(&mut self, _: Fd) -> Result<(), Errno> {
            Ok(())
        }
        fn write_stdout(&mut self, b: &[u8]) -> Result<(), Errno> {
            self.o.extend_from_slice(b);
            Ok(())
        }
        fn write_stderr(&mut self, b: &[u8]) -> Result<(), Errno> {
            self.e.extend_from_slice(b);
            Ok(())
        }
        fn yield_now(&mut self) {}
    }
    #[test]
    fn duplicate_groups_emit_cartesian() {
        let mut m = M::new();
        m.add(b"a", b"");
        m.add(b"b", b"");
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        assert_eq!(m.o, b"a x y\na x y2\na x2 y\na x2 y2\n");
    }
    #[test]
    fn output_list_is_structural() {
        let mut o = Options {
            f1: 1,
            f2: 1,
            a1: false,
            a2: false,
            v1: false,
            v2: false,
            delimiter: None,
            replacement: None,
            output: [OutputField { file: 0, field: 0 }; MAX_OUTPUT],
            output_len: 0,
        };
        assert!(parse_output(b"0,1.2,2.1", &mut o).is_ok());
        assert_eq!(o.output_len, 3);
    }
}
