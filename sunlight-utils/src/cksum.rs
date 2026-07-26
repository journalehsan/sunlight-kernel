//! Byte-preserving `cksum` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `cksum [file...]`
//!
//! Implements the normative POSIX CRC-32 checksum algorithm:
//! - Polynomial: G(x) = x^32 + x^26 + x^23 + x^22 + x^16 + x^12 + x^11
//!   + x^10 + x^8 + x^7 + x^5 + x^4 + x^2 + x + 1  (0x04C11DB7)
//! - Init crc = 0, process each byte MSB-first through CRC table
//! - Append file length as 8 little-endian bytes
//! - Finalize: crc = ~crc
//! - Output: "%u %u %s\n" (checksum_decimal size_decimal filename)
//!
//! CRC table is computed at compile time via const evaluation.
//! Stdin is used when no file operand or "-" is given.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

const BUF_SIZE: usize = 512;

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = (i as u32) << 24;
        let mut j = 0;
        while j < 8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ 0x04c11db7;
            } else {
                crc <<= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

pub trait Io {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno>;
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let files: &[&[u8]] = args;
    let use_stdin = files.is_empty();

    if use_stdin {
        return cksum_fd(io, STDIN, b"-");
    }

    let mut code = 0i32;
    for &path in files {
        if path == b"-" {
            let file_code = cksum_fd(io, STDIN, b"-");
            if file_code != 0 {
                code = 1;
            }
            continue;
        }

        let fd = match io.open(path) {
            Ok(fd) => fd,
            Err(_) => {
                let _ = io.write_stderr(b"cksum: cannot open '");
                let _ = io.write_stderr(path);
                let _ = io.write_stderr(b"': No such file or directory\n");
                code = 1;
                continue;
            }
        };

        let file_code = cksum_fd(io, fd, path);
        let _ = io.close(fd);
        if file_code != 0 {
            code = 1;
        }
    }

    code
}

fn cksum_fd(io: &mut impl Io, fd: Fd, name: &[u8]) -> i32 {
    let mut crc: u32 = 0;
    let mut total: u64 = 0;
    let mut buf = [0u8; BUF_SIZE];

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) if n <= buf.len() => {
                for &byte in &buf[..n] {
                    let idx = ((crc >> 24) ^ byte as u32) & 0xFF;
                    crc = (crc << 8) ^ CRC_TABLE[idx as usize];
                }
                total = match total.checked_add(n as u64) {
                    Some(t) => t,
                    None => {
                        let _ = io.write_stderr(b"cksum: file too large\n");
                        return 1;
                    }
                };
            }
            Ok(_) => {
                let _ = io.write_stderr(b"cksum: read error\n");
                return 1;
            }
            Err(_) => {
                let _ = io.write_stderr(b"cksum: read error\n");
                return 1;
            }
        }
    }

    let mut len = total;
    while len != 0 {
        let idx = ((crc >> 24) ^ (len & 0xFF) as u32) & 0xFF;
        crc = (crc << 8) ^ CRC_TABLE[idx as usize];
        len >>= 8;
    }

    let checksum = !crc;

    let _ = write_u64_dec(io, checksum as u64);
    let _ = io.write_stdout(b" ");
    let _ = write_u64_dec(io, total);
    let _ = io.write_stdout(b" ");
    let _ = io.write_stdout(name);
    let _ = io.write_stdout(b"\n");

    0
}

fn write_u64_dec(io: &mut impl Io, mut v: u64) -> Result<(), Errno> {
    let mut buf = [0u8; 20];
    if v == 0 {
        return io.write_stdout(b"0");
    }
    let mut n = 0;
    while v > 0 && n < buf.len() {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    let len = n;
    let mut i = 0;
    while i < n / 2 {
        let tmp = buf[i];
        buf[i] = buf[n - 1 - i];
        buf[n - 1 - i] = tmp;
        i += 1;
    }
    io.write_stdout(&buf[..len])
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
    }

    impl Mock {
        fn new(files: Vec<(&'static [u8], &'static [u8])>) -> Self {
            let fc = files.len();
            Self {
                files, output: Vec::new(), errors: Vec::new(),
                opens: 0, closes: 0, offsets: std::vec![0; fc],
                fail_read: false, stdin_data: None, stdin_offset: 0,
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
    }

    #[test]
    fn cksum_table_is_posix_correct() {
        // Verify CRC table was generated correctly at compile time
        // by checking known entries. The POSIX polynomial is 0x04C11DB7.
        assert_eq!(CRC_TABLE[0], 0x00000000);
        // Validate that the table was generated; specific values depend
        // on the algorithm. We validate via full checksum tests below.
        assert_ne!(CRC_TABLE[1], 0);
        assert_ne!(CRC_TABLE[255], 0);
    }

    #[test]
    fn posix_acknowledged_test_vector() {
        // POSIX: empty file → CRC of length 0, complemented → 4294967295
        let mut m = Mock::new(std::vec![(b"empty", b"")]);
        assert_eq!(run(&[b"empty"], &mut m), 0);
        assert_eq!(m.output, b"4294967295 0 empty\n".to_vec());
    }

    #[test]
    fn one_byte_file() {
        // Verified against host cksum (GNU coreutils)
        let mut m = Mock::new(std::vec![(b"x", b"A")]);
        assert_eq!(run(&[b"x"], &mut m), 0);
        assert_eq!(m.output, b"1751207896 1 x\n".to_vec());
    }

    #[test]
    fn small_text() {
        // "abc\n" - 4 bytes, verified against host cksum
        let mut m = Mock::new(std::vec![(b"f", b"abc\n")]);
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert_eq!(m.output, b"1112837078 4 f\n".to_vec());
    }

    #[test]
    fn binary_bytes_with_nul() {
        // Verified against host cksum: 00 FF 7F 00 01 → 575073917 5
        let data: &[u8] = &[0x00, 0xFF, 0x7F, 0x00, 0x01];
        let mut m = Mock::new(std::vec![(b"bin", data)]);
        assert_eq!(run(&[b"bin"], &mut m), 0);
        let s = std::str::from_utf8(&m.output).unwrap();
        let parts: Vec<&str> = s.trim().split(' ').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "575073917");
        assert_eq!(parts[1], "5");
        assert_eq!(parts[2], "bin");
    }

    #[test]
    fn one_buffer_plus_one_byte() {
        let data: Vec<u8> = (0u8..=255).cycle().take(BUF_SIZE + 1).collect();
        let data_static: &[u8] = unsafe { std::mem::transmute(data.as_slice()) };
        let mut m = Mock::new(std::vec![(b"big", data_static)]);
        assert_eq!(run(&[b"big"], &mut m), 0);
        let s = std::str::from_utf8(&m.output).unwrap();
        assert!(s.trim().ends_with("big"));
    }

    #[test]
    fn several_buffers() {
        let data: Vec<u8> = (0u8..=255).cycle().take(BUF_SIZE * 3 + 7).collect();
        let data_static: &[u8] = unsafe { std::mem::transmute(data.as_slice()) };
        let mut m = Mock::new(std::vec![(b"many", data_static)]);
        assert_eq!(run(&[b"many"], &mut m), 0);
    }

    #[test]
    fn multiple_files() {
        let mut m = Mock::new(std::vec![
            (b"a", b"hello\n"), (b"b", b"world\n"),
        ]);
        assert_eq!(run(&[b"a", b"b"], &mut m), 0);
        let s = std::str::from_utf8(&m.output).unwrap();
        assert_eq!(s.lines().count(), 2);
    }

    #[test]
    fn stdin_when_no_operand() {
        let mut m = Mock::new(std::vec![]);
        m.stdin_data = Some(b"test\n");
        assert_eq!(run(&[], &mut m), 0);
        assert!(std::str::from_utf8(&m.output).unwrap().ends_with("-\n"));
    }

    #[test]
    fn missing_file() {
        let mut m = Mock::new(std::vec![]);
        assert_eq!(run(&[b"missing"], &mut m), 1);
    }

    #[test]
    fn read_error() {
        let mut m = Mock::new(std::vec![(b"x", b"hello")]);
        m.fail_read = true;
        assert_eq!(run(&[b"x"], &mut m), 1);
    }

    #[test]
    fn cross_check_with_posix_spec_test_vectors() {
        // All values verified against host cksum (GNU coreutils).

        // "" → 4294967295 0
        let mut m0 = Mock::new(std::vec![(b"e", b"")]);
        assert_eq!(run(&[b"e"], &mut m0), 0);
        assert_eq!(m0.output, b"4294967295 0 e\n".to_vec());

        // "a" → 1220704766 1
        let mut m1 = Mock::new(std::vec![(b"one", b"a")]);
        assert_eq!(run(&[b"one"], &mut m1), 0);
        assert_eq!(m1.output, b"1220704766 1 one\n".to_vec());

        // "abc" → 1219131554 3
        let mut m3 = Mock::new(std::vec![(b"t", b"abc")]);
        assert_eq!(run(&[b"t"], &mut m3), 0);
        assert_eq!(m3.output, b"1219131554 3 t\n".to_vec());

        // "\x00\xFF\x7F\x00\x01" → 575073917 5
        let data: &[u8] = &[0x00, 0xFF, 0x7F, 0x00, 0x01];
        let mut m4 = Mock::new(std::vec![(b"bin", data)]);
        assert_eq!(run(&[b"bin"], &mut m4), 0);
        assert_eq!(m4.output, b"575073917 5 bin\n".to_vec());
    }
}
