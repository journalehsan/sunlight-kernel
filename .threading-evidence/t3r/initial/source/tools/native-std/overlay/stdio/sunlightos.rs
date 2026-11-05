use crate::io::{self, Read, Write};
use crate::sys::sunlightos::*;
pub struct Stdin;
pub struct Stdout;
pub struct Stderr;
impl Stdin {
    pub const fn new() -> Self {
        Self
    }
}
impl Stdout {
    pub const fn new() -> Self {
        Self
    }
}
impl Stderr {
    pub const fn new() -> Self {
        Self
    }
}
impl Read for Stdin {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        cvt(unsafe { sl_std_read(0, bytes.as_mut_ptr(), bytes.len()) })
    }
}
impl Write for Stdout {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        cvt(unsafe { sl_std_write(1, bytes.as_ptr(), bytes.len()) })
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Write for Stderr {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        cvt(unsafe { sl_std_write(2, bytes.as_ptr(), bytes.len()) })
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub const STDIN_BUF_SIZE: usize = 4096;
pub fn is_ebadf(error: &io::Error) -> bool {
    error.raw_os_error() == Some(9)
}
pub fn panic_output() -> Option<Stderr> {
    Some(Stderr)
}
