use std::collections::BTreeMap;
use sunlight_sunfs::{block::*, codec::crc64, Error};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    ReadFailure,
    WriteFailure,
    Partial(usize),
    Torn {
        start: usize,
        len: usize,
    },
    UncertainWrite {
        persist: bool,
    },
    FlushFailure {
        persist_writes: usize,
        uncertain: bool,
    },
    PowerLoss,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub id: u64,
    pub seed: u64,
    pub operation: &'static str,
    pub offset: u64,
    pub length: usize,
    pub digest: u64,
    pub outcome: String,
    pub persistence: Vec<(u64, usize)>,
    pub incarnation: u64,
}
#[derive(Clone)]
struct Pending {
    offset: usize,
    bytes: Vec<u8>,
    persisted: Vec<bool>,
}
#[derive(Clone)]
pub struct Simulator {
    info: DeviceInfo,
    pub seed: u64,
    durable: Vec<u8>,
    visible: Vec<u8>,
    pending: Vec<Pending>,
    pub faults: BTreeMap<u64, Fault>,
    pub trace: Vec<Event>,
    next_event: u64,
    online: bool,
    /// Deliberately violates advertised flush. Never part of conforming verdicts.
    pub dishonest_flush: bool,
    pub crash_after: Option<u64>,
}
impl Simulator {
    pub fn new(capacity: usize, seed: u64) -> Result<Self, Error> {
        if capacity > 128 * 1024 * 1024 {
            return Err(Error::Budget);
        }
        let info = DeviceInfo {
            capacity: capacity as u64,
            logical_block: 512,
            physical_block: Some(512),
            alignment: 512,
            max_transfer: 4096,
            identity: Some([0x53; 16]),
            incarnation: 1,
            read_only: false,
            flush: true,
            fua: false,
        };
        info.validate()?;
        Ok(Self {
            info,
            seed,
            durable: vec![0; capacity],
            visible: vec![0; capacity],
            pending: Vec::new(),
            faults: BTreeMap::new(),
            trace: Vec::new(),
            next_event: 0,
            online: true,
            dishonest_flush: false,
            crash_after: None,
        })
    }
    pub fn from_durable(bytes: Vec<u8>, seed: u64) -> Result<Self, Error> {
        let mut s = Self::new(bytes.len(), seed)?;
        s.visible.clone_from(&bytes);
        s.durable = bytes;
        Ok(s)
    }
    pub fn durable(&self) -> &[u8] {
        &self.durable
    }
    pub fn memory_bytes(&self) -> usize {
        self.durable.capacity()
            + self.visible.capacity()
            + self
                .pending
                .iter()
                .map(|p| p.bytes.capacity() + p.persisted.capacity())
                .sum::<usize>()
    }
    pub fn set_flush_support(&mut self, value: bool) {
        self.info.flush = value;
    }
    pub fn set_read_only(&mut self, value: bool) {
        self.info.read_only = value;
    }
    fn event(
        &mut self,
        op: &'static str,
        offset: u64,
        length: usize,
        digest: u64,
    ) -> (usize, Option<Fault>) {
        self.next_event += 1;
        let id = self.next_event;
        self.trace.push(Event {
            id,
            seed: self.seed,
            operation: op,
            offset,
            length,
            digest,
            outcome: String::new(),
            persistence: Vec::new(),
            incarnation: self.info.incarnation,
        });
        (self.trace.len() - 1, self.faults.get(&id).cloned())
    }
    fn after(&mut self, event: usize) {
        if self.crash_after == Some(self.trace[event].id) {
            self.power_loss();
        }
    }
    fn loss(&mut self) {
        self.visible.clone_from(&self.durable);
        self.pending.clear();
        self.online = false;
    }
    pub fn power_loss(&mut self) {
        let (e, _) = self.event("power-loss", 0, 0, 0);
        self.loss();
        self.trace[e].outcome = "volatile discarded".into();
    }
    pub fn reset(&mut self) -> IoResult<()> {
        let next = self
            .info
            .incarnation
            .checked_add(1)
            .ok_or_else(|| IoFailure::known(IoKind::Offline, 0))?;
        let (e, _) = self.event("reset", 0, 0, 0);
        self.loss();
        self.info.incarnation = next;
        self.online = true;
        self.trace[e].outcome = "new session; pending fenced".into();
        Ok(())
    }
    fn enqueue(&mut self, offset: usize, bytes: &[u8]) {
        self.visible[offset..offset + bytes.len()].copy_from_slice(bytes);
        self.pending.push(Pending {
            offset,
            bytes: bytes.to_vec(),
            persisted: vec![false; bytes.len()],
        });
    }
    fn fragment(&mut self, index: usize, start: usize, len: usize) -> IoResult<(u64, usize)> {
        let p = self
            .pending
            .get(index)
            .ok_or_else(|| IoFailure::from(Error::Bounds))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| IoFailure::from(Error::Overflow))?;
        if end > p.bytes.len() {
            return Err(Error::Bounds.into());
        }
        let low = p.offset + start;
        let high = p.offset + end;
        // Same-byte ordering is preserved even when disjoint writes reorder.
        for older in &self.pending[..index] {
            let a = low.max(older.offset);
            let b = high.min(older.offset + older.bytes.len());
            if a < b
                && older.persisted[a - older.offset..b - older.offset]
                    .iter()
                    .any(|&v| !v)
            {
                return Err(Error::Structure.into());
            }
        }
        let p = &mut self.pending[index];
        for i in start..end {
            if !p.persisted[i] {
                self.durable[p.offset + i] = p.bytes[i];
                p.persisted[i] = true;
            }
        }
        Ok((low as u64, len))
    }
    /// Explicit persistence cut, including byte tears. Pending indexes are stable
    /// until a flush/reset. Independent writes may be persisted in any permutation.
    pub fn persist(&mut self, index: usize, start: usize, len: usize) -> IoResult<()> {
        let (e, _) = self.event("persist-fragment", index as u64, len, 0);
        let result = self.fragment(index, start, len);
        if let Ok(range) = result {
            self.trace[e].persistence.push(range);
        }
        self.trace[e].outcome = format!("{result:?}");
        self.after(e);
        result.map(|_| ())
    }
    fn drain(&mut self, count: usize, event: usize) -> IoResult<()> {
        for i in 0..count.min(self.pending.len()) {
            let len = self.pending[i].bytes.len();
            let range = self.fragment(i, 0, len)?;
            self.trace[event].persistence.push(range);
        }
        Ok(())
    }
}
impl BlockDevice for Simulator {
    fn info(&self) -> DeviceInfo {
        self.info
    }
    fn read(&mut self, offset: u64, output: &mut [u8]) -> IoResult<()> {
        let (e, fault) = self.event("read", offset, output.len(), 0);
        let result = (|| {
            if !self.online {
                return Err(IoFailure::known(IoKind::Offline, 0));
            }
            let r = self.info.range(offset, output.len())?;
            if fault == Some(Fault::PowerLoss) {
                self.loss();
                return Err(IoFailure::uncertain(IoKind::Read));
            }
            if fault == Some(Fault::ReadFailure) {
                return Err(IoFailure::known(IoKind::Read, 0));
            }
            output.copy_from_slice(&self.visible[r]);
            Ok(())
        })();
        self.trace[e].outcome = format!("{result:?}");
        self.after(e);
        result
    }
    fn write(&mut self, offset: u64, input: &[u8]) -> IoResult<WriteCompletion> {
        let (e, fault) = self.event("write", offset, input.len(), crc64(input));
        let result = (|| {
            if !self.online {
                return Err(IoFailure::known(IoKind::Offline, 0));
            }
            if self.info.read_only {
                return Err(IoFailure::known(IoKind::ReadOnly, 0));
            }
            let range = self.info.range(offset, input.len())?;
            match fault {
                Some(Fault::PowerLoss) => {
                    self.loss();
                    Err(IoFailure::uncertain(IoKind::Write))
                }
                Some(Fault::WriteFailure) => Err(IoFailure::known(IoKind::Write, 0)),
                Some(Fault::Partial(n)) => {
                    let n = n.min(input.len());
                    self.enqueue(range.start, &input[..n]);
                    Err(IoFailure::known(IoKind::Write, n as u64))
                }
                Some(Fault::Torn { start, len }) => {
                    if start.checked_add(len).is_none_or(|end| end > input.len()) {
                        return Err(Error::Bounds.into());
                    }
                    self.enqueue(range.start, input);
                    let index = self.pending.len() - 1;
                    let r = self.fragment(index, start, len)?;
                    self.trace[e].persistence.push(r);
                    self.loss();
                    Err(IoFailure::uncertain(IoKind::Write))
                }
                Some(Fault::UncertainWrite { persist }) => {
                    self.enqueue(range.start, input);
                    if persist {
                        self.drain(self.pending.len(), e)?;
                    }
                    Err(IoFailure::uncertain(IoKind::Write))
                }
                _ => {
                    self.enqueue(range.start, input);
                    Ok(WriteCompletion::Completed)
                }
            }
        })();
        self.trace[e].outcome = format!("{result:?}");
        self.after(e);
        result
    }
    fn flush(&mut self) -> IoResult<()> {
        let (e, fault) = self.event("flush", 0, 0, 0);
        let result = (|| {
            if !self.online {
                return Err(IoFailure::known(IoKind::Offline, 0));
            }
            if !self.info.flush {
                return Err(IoFailure::known(IoKind::UnsupportedDurability, 0));
            }
            match fault {
                Some(Fault::PowerLoss) => {
                    self.loss();
                    Err(IoFailure::uncertain(IoKind::Flush))
                }
                Some(Fault::FlushFailure {
                    persist_writes,
                    uncertain,
                }) => {
                    self.drain(persist_writes, e)?;
                    Err(if uncertain {
                        IoFailure::uncertain(IoKind::Flush)
                    } else {
                        IoFailure::known(IoKind::Flush, 0)
                    })
                }
                _ => {
                    if !self.dishonest_flush {
                        self.drain(self.pending.len(), e)?;
                    }
                    self.pending.clear();
                    Ok(())
                }
            }
        })();
        self.trace[e].outcome = format!("{result:?}");
        self.after(e);
        result
    }
}
