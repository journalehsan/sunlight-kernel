//! Exact I/O with explicit completion and durability provenance.
use crate::{Error, Result};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub capacity: u64,
    pub logical_block: u32,
    pub physical_block: Option<u32>,
    pub alignment: u32,
    pub max_transfer: u32,
    pub identity: Option<[u8; 16]>,
    pub incarnation: u64,
    pub read_only: bool,
    pub flush: bool,
    pub fua: bool,
}
impl DeviceInfo {
    pub fn validate(&self) -> Result<()> {
        if !self.logical_block.is_power_of_two()
            || !(512..=4096).contains(&self.logical_block)
            || !self.alignment.is_power_of_two()
            || self.alignment > 4096
            || self.max_transfer < self.logical_block
            || !self.max_transfer.is_multiple_of(self.logical_block)
            || self.capacity == 0
            || !self.capacity.is_multiple_of(self.logical_block as u64)
            || self.incarnation == 0
            || self
                .physical_block
                .is_some_and(|v| !v.is_power_of_two() || v < self.logical_block)
        {
            return Err(Error::Geometry);
        }
        Ok(())
    }
    pub fn range(&self, offset: u64, len: usize) -> Result<core::ops::Range<usize>> {
        self.validate()?;
        let length = u64::try_from(len).map_err(|_| Error::Overflow)?;
        let end = offset.checked_add(length).ok_or(Error::Overflow)?;
        if length == 0 || length > self.max_transfer as u64 {
            return Err(Error::Bounds);
        }
        if !offset.is_multiple_of(self.alignment as u64)
            || !length.is_multiple_of(self.logical_block as u64)
            || !offset.is_multiple_of(self.logical_block as u64)
        {
            return Err(Error::Alignment);
        }
        if end > self.capacity {
            return Err(Error::Bounds);
        }
        Ok(usize::try_from(offset).map_err(|_| Error::Overflow)?
            ..usize::try_from(end).map_err(|_| Error::Overflow)?)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoKind {
    Invalid(Error),
    ReadOnly,
    UnsupportedDurability,
    Read,
    Write,
    Flush,
    Offline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Certainty {
    Known,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoFailure {
    pub kind: IoKind,
    pub completed: Option<u64>,
    pub certainty: Certainty,
}
impl IoFailure {
    pub fn known(kind: IoKind, completed: u64) -> Self {
        Self {
            kind,
            completed: Some(completed),
            certainty: Certainty::Known,
        }
    }
    pub fn uncertain(kind: IoKind) -> Self {
        Self {
            kind,
            completed: None,
            certainty: Certainty::Uncertain,
        }
    }
}
impl From<Error> for IoFailure {
    fn from(e: Error) -> Self {
        Self::known(IoKind::Invalid(e), 0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteCompletion {
    Completed,
    Durable,
}
pub type IoResult<T> = core::result::Result<T, IoFailure>;
/// Synchronous exclusive submission. Failed reads must not be exposed by consumers.
/// Flush success drains all prior accepted writes. No default successful barrier.
/// Failed writes/flushes require caller fail-stop; no implicit ambiguous retry.
pub trait BlockDevice {
    fn info(&self) -> DeviceInfo;
    fn read(&mut self, offset: u64, output: &mut [u8]) -> IoResult<()>;
    fn write(&mut self, offset: u64, input: &[u8]) -> IoResult<WriteCompletion>;
    fn flush(&mut self) -> IoResult<()>;
    fn write_fua(&mut self, _offset: u64, _input: &[u8]) -> IoResult<WriteCompletion> {
        Err(IoFailure::known(IoKind::UnsupportedDurability, 0))
    }
}

/// Checked eight-sector aggregation model for the future native 512-byte path.
/// The inner adapter must already qualify uncertainty and reset fencing; this does
/// not turn the existing native Io error into a proven cancellation contract.
pub struct Sector512Adapter<D> {
    pub inner: D,
}
impl<D: BlockDevice> Sector512Adapter<D> {
    pub fn new(inner: D) -> Result<Self> {
        let info = inner.info();
        info.validate()?;
        if info.logical_block != 512 || info.alignment > 512 || info.max_transfer < 512 {
            return Err(Error::Geometry);
        }
        Ok(Self { inner })
    }
}
impl<D: BlockDevice> BlockDevice for Sector512Adapter<D> {
    fn info(&self) -> DeviceInfo {
        let mut info = self.inner.info();
        info.max_transfer = 4096;
        info.fua = false;
        info
    }
    fn read(&mut self, offset: u64, output: &mut [u8]) -> IoResult<()> {
        self.info().range(offset, output.len())?;
        let incarnation = self.inner.info().incarnation;
        let mut scratch = [0; 4096];
        for (i, chunk) in scratch[..output.len()].chunks_mut(512).enumerate() {
            if self.inner.info().incarnation != incarnation {
                return Err(IoFailure::uncertain(IoKind::Offline));
            }
            self.inner.read(offset + i as u64 * 512, chunk)?;
        }
        if self.inner.info().incarnation != incarnation {
            return Err(IoFailure::uncertain(IoKind::Offline));
        }
        output.copy_from_slice(&scratch[..output.len()]);
        Ok(())
    }
    fn write(&mut self, offset: u64, input: &[u8]) -> IoResult<WriteCompletion> {
        self.info().range(offset, input.len())?;
        let incarnation = self.inner.info().incarnation;
        let mut durable = true;
        for (i, chunk) in input.chunks(512).enumerate() {
            if self.inner.info().incarnation != incarnation {
                return Err(IoFailure::uncertain(IoKind::Offline));
            }
            match self.inner.write(offset + i as u64 * 512, chunk) {
                Ok(result) => durable &= result == WriteCompletion::Durable,
                Err(mut e) => {
                    if let Some(n) = e.completed {
                        e.completed = n.checked_add(i as u64 * 512);
                    }
                    return Err(e);
                }
            }
        }
        if self.inner.info().incarnation != incarnation {
            return Err(IoFailure::uncertain(IoKind::Offline));
        }
        Ok(if durable {
            WriteCompletion::Durable
        } else {
            WriteCompletion::Completed
        })
    }
    fn flush(&mut self) -> IoResult<()> {
        self.inner.flush()
    }
}
