//! Bounded POSIX `paste` line composition.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

const BUF_SIZE: usize = 512;
pub const MAX_LINE_LEN: usize = 4096;
const MAX_INPUTS: usize = 8;
const RETRIES: usize = 8;

pub trait Io {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn yield_now(&mut self);
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] { argv.get(1..).unwrap_or(&[]) }

#[derive(Clone, Copy)]
struct Line { bytes: [u8; MAX_LINE_LEN], len: usize, newline: bool }
impl Line { const fn empty() -> Self { Self { bytes: [0; MAX_LINE_LEN], len: 0, newline: false } } }

struct Reader { fd: Fd, buf: [u8; BUF_SIZE], carry: [u8; MAX_LINE_LEN + 1], carry_len: usize, eof: bool, error: bool }
impl Reader {
    fn new(fd: Fd) -> Self { Self { fd, buf: [0; BUF_SIZE], carry: [0; MAX_LINE_LEN + 1], carry_len: 0, eof: false, error: false } }
    fn next(&mut self, io: &mut impl Io, line: &mut Line) -> bool {
        line.len = 0; line.newline = false;
        loop {
            if let Some(nl) = self.carry[..self.carry_len].iter().position(|&b| b == b'\n') {
                line.bytes[..nl].copy_from_slice(&self.carry[..nl]); line.len = nl; line.newline = true;
                let used = nl + 1; self.carry.copy_within(used..self.carry_len, 0); self.carry_len -= used; return true;
            }
            if self.eof {
                if self.carry_len == 0 { return false; }
                if self.carry_len > MAX_LINE_LEN { self.error = true; return false; }
                line.bytes[..self.carry_len].copy_from_slice(&self.carry[..self.carry_len]); line.len = self.carry_len; self.carry_len = 0; return true;
            }
            let mut retries = 0;
            let n = loop {
                match io.read(self.fd, &mut self.buf) {
                    Ok(n) if n <= self.buf.len() => break n,
                    Ok(_) => { self.error = true; return false; }
                    Err(Errno::Again) if retries < RETRIES => { retries += 1; io.yield_now(); }
                    Err(_) => { self.error = true; return false; }
                }
            };
            if n == 0 { self.eof = true; continue; }
            if self.carry_len.checked_add(n).filter(|&n| n <= MAX_LINE_LEN + 1).is_none() { self.error = true; return false; }
            self.carry[self.carry_len..self.carry_len+n].copy_from_slice(&self.buf[..n]); self.carry_len += n;
        }
    }
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (serial, delimiters, paths) = match parse_args(args, io) { Some(v) => v, None => return 2 };
    if paths.len() > MAX_INPUTS { diag(io, b"paste: too many input files (maximum 8)\n"); return 1; }
    if serial { return serial_mode(paths, &delimiters, io); }
    parallel_mode(paths, &delimiters, io)
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Option<(bool, Delimiters, &'a [&'a [u8]])> {
    let mut serial = false; let mut delimiters = Delimiters::tab(); let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == b"--" { i += 1; break; }
        if a == b"-s" { serial = true; i += 1; continue; }
        if a == b"-d" { i += 1; if i == args.len() { diag(io, b"paste: option -d needs an argument\n"); return None; } delimiters = match Delimiters::parse(args[i]) { Ok(d) => d, Err(()) => { diag(io, b"paste: invalid delimiter list\n"); return None; } }; i += 1; continue; }
        if a.starts_with(b"-d") && a.len() > 2 { delimiters = match Delimiters::parse(&a[2..]) { Ok(d) => d, Err(()) => { diag(io, b"paste: invalid delimiter list\n"); return None; } }; i += 1; continue; }
        if a.starts_with(b"-") && a.len() > 1 { diag(io, b"paste: invalid option\n"); return None; }
        break;
    }
    Some((serial, delimiters, &args[i..]))
}

#[derive(Clone, Copy)]
struct Delimiters { bytes: [u8; 16], len: usize }
impl Delimiters {
    const fn tab() -> Self { Self { bytes: [b'\t'; 16], len: 1 } }
    fn parse(input: &[u8]) -> Result<Self, ()> {
        let mut d = Self { bytes: [0; 16], len: 0 }; let mut i = 0;
        while i < input.len() { let b = if input[i] != b'\\' { let v = input[i]; i += 1; v } else { let (v, n) = escape(input, i)?; i = n; v }; if d.len == d.bytes.len() { return Err(()); } d.bytes[d.len] = b; d.len += 1; }
        Ok(d)
    }
    fn get(&self, index: usize) -> Option<u8> { if self.len == 0 { None } else { Some(self.bytes[index % self.len]) } }
}
fn escape(input: &[u8], at: usize) -> Result<(u8, usize), ()> {
    if at + 1 >= input.len() { return Err(()); }
    match input[at + 1] { b'n' => Ok((b'\n', at+2)), b't' => Ok((b'\t', at+2)), b'b' => Ok((8, at+2)), b'f' => Ok((12, at+2)), b'r' => Ok((b'\r', at+2)), b'v' => Ok((11, at+2)), b'\\' => Ok((b'\\', at+2)), b'0'..=b'7' => { let mut n=0u16; let mut j=at+1; let mut count=0; while j<input.len() && count<3 && (b'0'..=b'7').contains(&input[j]) { n=n*8+(input[j]-b'0') as u16; j+=1; count+=1; } Ok((n as u8,j)) }, _ => Err(()) }
}

fn open_readers<'a>(paths: &'a [&'a [u8]], io: &mut impl Io) -> Result<([Option<Reader>; MAX_INPUTS], usize), i32> {
    let count = if paths.is_empty() { 1 } else { paths.len() }; let mut readers = [const { None }; MAX_INPUTS];
    for i in 0..count { let fd = if paths.is_empty() || paths[i] == b"-" { STDIN } else { match io.open(paths[i]) { Ok(fd) => fd, Err(_) => { diag_path(io, b"paste: cannot open ", paths[i]); close_readers(&mut readers, io); return Err(1); } } }; readers[i] = Some(Reader::new(fd)); }
    Ok((readers, count))
}
fn close_readers(readers: &mut [Option<Reader>; MAX_INPUTS], io: &mut impl Io) { for reader in readers.iter_mut() { if let Some(r) = reader.take() { if r.fd != STDIN { let _ = io.close(r.fd); } } } }

fn parallel_mode(paths: &[&[u8]], delimiters: &Delimiters, io: &mut impl Io) -> i32 {
    let (mut readers, count) = match open_readers(paths, io) { Ok(v) => v, Err(e) => return e };
    let mut lines = [Line::empty(); MAX_INPUTS]; let mut active = [false; MAX_INPUTS];
    loop {
        let mut any = false; for i in 0..count { let ok = readers[i].as_mut().unwrap().next(io, &mut lines[i]); active[i] = ok; any |= ok; }
        if !any { break; }
        for i in 0..count { if i != 0 { if let Some(d) = delimiters.get(i-1) { if io.write_stdout(&[d]).is_err() { close_readers(&mut readers, io); diag(io,b"paste: write error\n"); return 1; } } } if active[i] && io.write_stdout(&lines[i].bytes[..lines[i].len]).is_err() { close_readers(&mut readers,io); diag(io,b"paste: write error\n"); return 1; } }
        if io.write_stdout(b"\n").is_err() { close_readers(&mut readers,io); diag(io,b"paste: write error\n"); return 1; }
        if readers.iter().take(count).any(|r| r.as_ref().map_or(false, |r| r.error)) { close_readers(&mut readers,io); diag(io,b"paste: read error\n"); return 1; }
    }
    close_readers(&mut readers, io); 0
}

fn serial_mode(paths: &[&[u8]], delimiters: &Delimiters, io: &mut impl Io) -> i32 {
    let list: [&[u8]; MAX_INPUTS] = { let mut x=[b"-".as_slice(); MAX_INPUTS]; for (i,p) in paths.iter().enumerate() { x[i]=p; } x };
    let count = if paths.is_empty() { 1 } else { paths.len() };
    for path in list.iter().take(count) {
        let fd = if *path == b"-" { STDIN } else { match io.open(path) { Ok(fd)=>fd, Err(_)=>{diag_path(io,b"paste: cannot open ",path);return 1;} } };
        let mut reader=Reader::new(fd); let mut line=Line::empty(); let mut pending=Line::empty(); let mut have_pending=false; let mut index=0;
        loop { let ok=reader.next(io,&mut line); if !ok { break; }
            if have_pending { if io.write_stdout(&pending.bytes[..pending.len]).is_err(){if fd!=STDIN{let _=io.close(fd);}diag(io,b"paste: write error\n");return 1;} if let Some(d)=delimiters.get(index){if io.write_stdout(&[d]).is_err(){if fd!=STDIN{let _=io.close(fd);}diag(io,b"paste: write error\n");return 1;}} index+=1; }
            pending=line; have_pending=true;
        }
        if have_pending { if io.write_stdout(&pending.bytes[..pending.len]).is_err(){if fd!=STDIN{let _=io.close(fd);}diag(io,b"paste: write error\n");return 1;} if pending.newline && io.write_stdout(b"\n").is_err(){if fd!=STDIN{let _=io.close(fd);}diag(io,b"paste: write error\n");return 1;} }
        if reader.error { diag(io,b"paste: read error\n"); if fd!=STDIN{let _=io.close(fd);} return 1; }
        if fd!=STDIN { let _=io.close(fd); }
    }
    0
}

fn diag(io: &mut impl Io, msg: &[u8]) { let _=io.write_stderr(msg); }
fn diag_path(io: &mut impl Io, prefix: &[u8], path: &[u8]) { let _=io.write_stderr(prefix); let _=io.write_stderr(path); let _=io.write_stderr(b"\n"); }

pub struct NativeIo;
impl Io for NativeIo { fn open(&mut self,p:&[u8])->Result<Fd,Errno>{sunlight_libc::open(p)} fn read(&mut self,f:Fd,b:&mut[u8])->Result<usize,Errno>{sunlight_libc::read(f,b)} fn close(&mut self,f:Fd)->Result<(),Errno>{sunlight_libc::close(f)} fn write_stdout(&mut self,b:&[u8])->Result<(),Errno>{sunlight_libc::write_all(STDOUT,b)} fn write_stderr(&mut self,b:&[u8])->Result<(),Errno>{sunlight_libc::write_all(STDERR,b)} fn yield_now(&mut self){sunlight_libc::yield_now()} }

#[cfg(test)]
mod tests { extern crate std; use super::*; use std::vec::Vec;
    struct Mock { files: Vec<(Vec<u8>,Vec<u8>,usize)>, out: Vec<u8>, err: Vec<u8> }
    impl Mock { fn new(a:&[u8],b:&[u8])->Self{Self{files:std::vec![(b"a".to_vec(),a.to_vec(),0),(b"b".to_vec(),b.to_vec(),0)],out:Vec::new(),err:Vec::new()}} }
    impl Io for Mock { fn open(&mut self,p:&[u8])->Result<Fd,Errno>{self.files.iter().position(|x|x.0==p).map(|i|Fd(i as u32+3)).ok_or(Errno::NoEntry)} fn read(&mut self,f:Fd,b:&mut[u8])->Result<usize,Errno>{let x=&mut self.files[(f.0-3)as usize];if x.2==x.1.len(){return Ok(0)}let n=(x.1.len()-x.2).min(b.len());b[..n].copy_from_slice(&x.1[x.2..x.2+n]);x.2+=n;Ok(n)} fn close(&mut self,_:Fd)->Result<(),Errno>{Ok(())} fn write_stdout(&mut self,b:&[u8])->Result<(),Errno>{self.out.extend_from_slice(b);Ok(())} fn write_stderr(&mut self,b:&[u8])->Result<(),Errno>{self.err.extend_from_slice(b);Ok(())} fn yield_now(&mut self){} }
    #[test] fn parallel_unequal_lines(){let mut m=Mock::new(b"a1\na2\n",b"b1\n");assert_eq!(run(&[b"a",b"b"],&mut m),0);assert_eq!(m.out,b"a1\tb1\na2\t\n");}
    #[test] fn serial_preserves_final_missing_newline(){let mut m=Mock::new(b"a\nb",b"");assert_eq!(run(&[b"-s",b"a"],&mut m),0);assert_eq!(m.out,b"a\tb");}
    #[test] fn delimiters_cycle(){let d=Delimiters::parse(b":\\t,").unwrap();assert_eq!(d.get(0),Some(b':'));assert_eq!(d.get(1),Some(b'\t'));assert_eq!(d.get(2),Some(b','));assert_eq!(d.get(3),Some(b':'));}
}
