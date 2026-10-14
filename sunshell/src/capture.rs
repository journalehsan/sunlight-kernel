//! Bounded, stack-scoped interpreter output sinks. Negative handles never
//! cross the process ABI: platform adapters materialize them as anonymous pipes.
use crate::script::Error;
use alloc::vec::Vec;

pub const MAX_CAPTURE_BYTES: usize = 128 * 1024;
pub const MAX_CAPTURE_TOTAL: usize = 256 * 1024;
pub const MAX_CAPTURE_DEPTH: usize = 16;
#[derive(Default)]
pub struct Captures {
    buffers: Vec<Vec<u8>>,
}
impl Captures {
    pub fn begin(&mut self) -> Result<i32, Error> {
        if self.buffers.len() >= MAX_CAPTURE_DEPTH {
            return Err(Error::fatal("substitution nesting exceeds 16 levels"));
        }
        self.buffers
            .try_reserve(1)
            .map_err(|_| Error::fatal("out of memory starting capture"))?;
        self.buffers.push(Vec::new());
        Ok(-(self.buffers.len() as i32))
    }
    pub fn append(&mut self, fd: i32, bytes: &[u8]) -> Result<(), Error> {
        let total = self.buffers.iter().map(Vec::len).sum::<usize>();
        let index = fd
            .checked_neg()
            .and_then(|n| n.checked_sub(1))
            .ok_or_else(|| Error::fatal("invalid capture handle"))? as usize;
        let buffer = self
            .buffers
            .get_mut(index)
            .ok_or_else(|| Error::fatal("expired capture handle"))?;
        if buffer.len() + bytes.len() > MAX_CAPTURE_BYTES || total + bytes.len() > MAX_CAPTURE_TOTAL
        {
            return Err(Error::fatal(
                "command substitution capture limit exceeded (128 KiB per capture, 256 KiB total)",
            ));
        }
        buffer
            .try_reserve(bytes.len())
            .map_err(|_| Error::fatal("out of memory capturing command substitution"))?;
        buffer.extend_from_slice(bytes);
        Ok(())
    }
    pub fn end(&mut self, fd: i32) -> Result<Vec<u8>, Error> {
        if fd != -(self.buffers.len() as i32) {
            return Err(Error::fatal("capture scope mismatch"));
        }
        self.buffers
            .pop()
            .ok_or_else(|| Error::fatal("missing capture scope"))
    }
}
