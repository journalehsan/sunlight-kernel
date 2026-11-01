//! M3.2 representation: three independent pages, private cursors and private leases.
//!
//! All observed words are untrusted, including words on read-only mappings.
//! A receive copies bounded words once; the result may mix hostile writes but
//! cannot change afterward. No immutable publication or coherent snapshot is
//! promised. See docs/ipc/fastlane-m32-message-integrity.md.
//!
//! Three properties are kept distinct and are reported by [`ReadLease::integrity`]:
//!
//! 1. Memory safety — a hostile producer/consumer can never cause an out-of-bounds
//!    Rust access. Provided: no pointer, slice or reference indexes shared memory
//!    from peer data; every index is derived from validated local geometry.
//! 2. Snapshot stability — the bytes returned by a receive cannot change after
//!    delivery. Provided: each receive is a private initialized copy.
//! 3. Publication integrity — the returned bytes are one coherent committed
//!    message. **Not provided and not representable**: M2 grants the producer a
//!    writable alias to the payload frame, so a copy may interleave bytes written
//!    after commit or during acquisition.
//!
//! This module has no native attach, pointers, allocation, waiting or Doorbell.
//! A future native backend must use audited single-instruction accesses without
//! constructing Rust references to peer memory, and enforce mapping lifetime.
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU64, Ordering};

pub const PAGE_BYTES: usize = 4096;
pub const WORDS: usize = PAGE_BYTES / 8;
pub const CAPACITY: usize = 16;
pub const PAYLOAD_BYTES: usize = 256;
const PAYLOAD_WORDS: usize = PAYLOAD_BYTES / 8;
const DESCRIPTOR_WORDS: usize = 5;
pub const CURSOR_WORD: usize = 0;
// These compile-time bounds prove every fixed-width loop/index calculation
// below fits usize and stays in its page. Peer arithmetic is separately checked.
const _: () = assert!(CAPACITY * PAYLOAD_BYTES == PAGE_BYTES);
const _: () = assert!(CAPACITY * DESCRIPTOR_WORDS < WORDS);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Producer = 0,
    Consumer = 1,
    Payload = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Producer,
    Consumer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Configuration,
    Full,
    Empty,
    Busy,
    Length,
    OutputTooSmall,
    ProducerState,
    ConsumerState,
    Identity,
    Offset,
    Exhausted,
    StaleLease,
    Access,
    Closed,
}

/// Integrity properties of one returned private copy. Fields are private so a
/// caller cannot forge a stronger guarantee than the representation provides.
///
/// `coherent_publication` and `application_authenticated` are constants of the
/// representation: M2 leaves a producer-writable alias to every published byte,
/// so neither a coherent committed message nor message authenticity is provided.
/// A sequence counter, checksum or Acquire load held by a writable producer does
/// not remove that alias and is deliberately not modelled as a proof here. This
/// type therefore has no constructor that can set those fields true.
///
/// A stronger claim cannot be forged; the fields are private to this module:
///
/// ```compile_fail
/// use sunlight_fastlane_ring::hostile::Integrity;
/// let forged = Integrity {
///     memory_safe: true,
///     snapshot_stable: true,
///     coherent_publication: true,
///     application_authenticated: true,
/// };
/// assert!(forged.coherent_publication());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integrity {
    memory_safe: bool,
    snapshot_stable: bool,
    coherent_publication: bool,
    application_authenticated: bool,
}

impl Integrity {
    /// The only integrity value this representation can produce: bounded access
    /// is memory safe, the private copy is stable, but bytes may mix hostile
    /// writes and no authenticity is established. This is **transport** safety
    /// only; it is not application-level message authenticity.
    pub const PRIVATE_COPY: Self = Self {
        memory_safe: true,
        snapshot_stable: true,
        coherent_publication: false,
        application_authenticated: false,
    };

    /// Always `true` for this representation: peer metadata cannot cause an
    /// out-of-bounds access in the returned copy.
    pub const fn memory_safe(self) -> bool {
        self.memory_safe
    }

    /// Always `true` for this representation: the returned bytes are a private
    /// copy that cannot change after delivery.
    pub const fn snapshot_stable(self) -> bool {
        self.snapshot_stable
    }

    /// Always `false` here: the producer retains a writable alias, so the copy
    /// may interleave bytes written after commit or during acquisition. Callers
    /// must not treat the result as one coherent committed message.
    pub const fn coherent_publication(self) -> bool {
        self.coherent_publication
    }

    /// Always `false` here: transport integrity is not message authenticity. A
    /// well-formed hostile message is indistinguishable from an honest one.
    pub const fn application_authenticated(self) -> bool {
        self.application_authenticated
    }
}

/// Word indices are page-relative and never come from the peer. Implementations
/// must bounds-check them and use Acquire loads / Release stores (or stronger).
/// An access failure terminates the local endpoint; no failed publication retries.
/// This safe trait conveys no authority or mapping-lifetime proof. Implementing
/// it for native memory requires its own audit; casting a page to AtomicPage is
/// not supported. The host backend uses uniform AtomicU64 accesses exclusively.
pub trait Memory {
    fn load(&self, region: Region, word: usize) -> Result<u64, Error>;
    fn store(&self, region: Region, word: usize, value: u64) -> Result<(), Error>;
}

/// Host backing only. Every bit, including unused storage, starts initialized.
#[repr(C, align(4096))]
struct AtomicPage([AtomicU64; WORDS]);

pub struct AtomicPages([AtomicPage; 3]);

impl Default for AtomicPages {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicPages {
    pub fn new() -> Self {
        Self(core::array::from_fn(|_| {
            AtomicPage(core::array::from_fn(|_| AtomicU64::new(0)))
        }))
    }

    /// Mirrors M2 page permissions, including adversarial writes by each owner.
    pub fn view(&self, role: Role) -> View<'_> {
        View { pages: self, role }
    }
}

#[derive(Clone, Copy)]
pub struct View<'a> {
    pages: &'a AtomicPages,
    role: Role,
}

impl Memory for View<'_> {
    fn load(&self, region: Region, word: usize) -> Result<u64, Error> {
        self.pages.0[region as usize]
            .0
            .get(word)
            .map(|v| v.load(Ordering::Acquire))
            .ok_or(Error::Access)
    }

    fn store(&self, region: Region, word: usize, value: u64) -> Result<(), Error> {
        let owner = if region == Region::Consumer {
            Role::Consumer
        } else {
            Role::Producer
        };
        if self.role != owner {
            return Err(Error::Access);
        }
        self.pages.0[region as usize]
            .0
            .get(word)
            .ok_or(Error::Access)?
            .store(value, Ordering::Release);
        Ok(())
    }
}

/// Non-authorizing identity of one slot use. Only checked local geometry chooses
/// an address. Sequence numbers never wrap; generation increments on slot reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub channel: u64,
    pub sequence: u64,
    pub slot: usize,
    pub generation: u64,
}

impl Identity {
    fn new(channel: u64, sequence: u64) -> Result<Self, Error> {
        if channel == 0 {
            return Err(Error::Configuration);
        }
        // A publication needs a representable next cursor, even for an empty frame.
        sequence.checked_add(1).ok_or(Error::Exhausted)?;
        Ok(Self {
            channel,
            sequence,
            slot: (sequence % CAPACITY as u64) as usize,
            generation: (sequence / CAPACITY as u64)
                .checked_add(1)
                .ok_or(Error::Exhausted)?,
        })
    }

    fn offset(self) -> usize {
        // slot was reduced modulo the fixed capacity; 16 * 256 == one page.
        self.slot * PAYLOAD_BYTES
    }

    fn descriptor(self) -> usize {
        1 + self.slot * DESCRIPTOR_WORDS
    }

    fn validate(self, fields: [u64; DESCRIPTOR_WORDS]) -> Result<usize, Error> {
        if fields[..3] != [self.channel, self.sequence, self.generation] {
            return Err(Error::Identity);
        }
        // Check addition before conversion, and require the canonical slot offset.
        let end = fields[3].checked_add(fields[4]).ok_or(Error::Offset)?;
        if fields[3] != self.offset() as u64 || end > PAGE_BYTES as u64 {
            return Err(Error::Offset);
        }
        if fields[4] > PAYLOAD_BYTES as u64 {
            return Err(Error::Length);
        }
        Ok(fields[4] as usize)
    }
}

/// Trusted local state. None of this structure is stored in shared memory.
struct Local {
    channel: u64,
    next: u64,
    observed: u64,
    serial: u64,
    pending: Option<(Identity, u64)>,
    terminal: Option<Error>,
}

impl Local {
    fn new(channel: u64) -> Result<Self, Error> {
        Identity::new(channel, 0)?;
        Ok(Self {
            channel,
            next: 0,
            observed: 0,
            serial: 0,
            pending: None,
            terminal: None,
        })
    }

    fn ready(&self) -> Result<Identity, Error> {
        if let Some(error) = self.terminal {
            return Err(error);
        }
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        Identity::new(self.channel, self.next)
    }

    fn fail<T>(&mut self, error: Error) -> Result<T, Error> {
        // First terminal error wins, including explicit local retirement.
        let first = *self.terminal.get_or_insert(error);
        self.pending = None;
        Err(first)
    }

    fn begin(&mut self, id: Identity) -> Result<u64, Error> {
        let Some(serial) = self.serial.checked_add(1) else {
            return self.fail(Error::Exhausted);
        };
        self.serial = serial;
        self.pending = Some((id, serial));
        Ok(serial)
    }

    fn check(&self, id: Identity, serial: u64) -> Result<(), Error> {
        if let Some(error) = self.terminal {
            return Err(error);
        }
        if self.pending != Some((id, serial)) || Identity::new(self.channel, self.next)? != id {
            return Err(Error::StaleLease);
        }
        Ok(())
    }
}

/// One fresh role per fresh channel, constructed with a trusted M1 generation.
/// Constructors do not attach, resume, clear peer pages or authenticate a caller.
/// Duplicating roles/restarting cursors violates the SPSC protocol, but cannot
/// create Rust memory unsafety with a sound Memory backend.
pub struct Producer<M> {
    memory: M,
    local: Local,
}

pub struct Consumer<M> {
    memory: M,
    local: Local,
}

impl<M: Memory> Producer<M> {
    pub fn new(memory: M, channel: u64) -> Result<Self, Error> {
        Ok(Self {
            memory,
            local: Local::new(channel)?,
        })
    }

    pub fn try_reserve(&mut self, len: usize) -> Result<WriteLease<'_, M>, Error> {
        let id = match self.local.ready() {
            Ok(id) => id,
            Err(Error::Exhausted) => return self.local.fail(Error::Exhausted),
            Err(error) => return Err(error),
        };
        if len > PAYLOAD_BYTES {
            return Err(Error::Length);
        }
        let tail = match self.memory.load(Region::Consumer, CURSOR_WORD) {
            Ok(tail) => tail,
            Err(error) => return self.local.fail(error),
        };
        if tail < self.local.observed || tail > self.local.next {
            return self.local.fail(Error::ConsumerState);
        }
        self.local.observed = tail;
        let used = self
            .local
            .next
            .checked_sub(tail)
            .ok_or(Error::ConsumerState)?;
        if used > CAPACITY as u64 {
            return self.local.fail(Error::ConsumerState);
        }
        if used == CAPACITY as u64 {
            return Err(Error::Full);
        }
        let serial = self.local.begin(id)?;
        Ok(WriteLease {
            owner: self,
            id,
            serial,
            len,
            bytes: [0; PAYLOAD_BYTES],
        })
    }

    pub fn try_send(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut lease = self.try_reserve(bytes.len())?;
        lease.payload_mut().copy_from_slice(bytes);
        lease.commit()
    }

    /// Recover a forgotten local reservation. No publication, cursor advance or
    /// peer reclamation occurs. The next lease has a different local serial.
    pub fn cancel_pending(&mut self) {
        self.local.pending = None;
    }

    pub fn retire(&mut self) {
        let _ = self.local.fail::<()>(Error::Closed);
    }
}

impl<M: Memory> Consumer<M> {
    pub fn new(memory: M, channel: u64) -> Result<Self, Error> {
        Ok(Self {
            memory,
            local: Local::new(channel)?,
        })
    }

    pub fn try_read(&mut self) -> Result<ReadLease<'_, M>, Error> {
        let id = match self.local.ready() {
            Ok(id) => id,
            Err(Error::Exhausted) => return self.local.fail(Error::Exhausted),
            Err(error) => return Err(error),
        };
        let read = || {
            let head = self.memory.load(Region::Producer, CURSOR_WORD)?;
            let used = head
                .checked_sub(self.local.next)
                .ok_or(Error::ProducerState)?;
            if head < self.local.observed || used > CAPACITY as u64 {
                return Err(Error::ProducerState);
            }
            if used == 0 {
                return Ok((head, None));
            }
            let mut fields = [0; DESCRIPTOR_WORDS];
            for (i, field) in fields.iter_mut().enumerate() {
                *field = self.memory.load(Region::Producer, id.descriptor() + i)?;
            }
            let len = id.validate(fields)?;
            let mut bytes = [0; PAYLOAD_BYTES];
            // Use the LOCAL canonical offset, never a peer offset. Always copy
            // complete u64 words into initialized private storage; expose len only.
            for i in 0..len.div_ceil(8) {
                let value = self.memory.load(Region::Payload, id.offset() / 8 + i)?;
                bytes[i * 8..i * 8 + 8].copy_from_slice(&value.to_le_bytes());
            }
            bytes[len..].fill(0);
            Ok((head, Some((len, bytes))))
        };
        let (head, frame) = match read() {
            Ok(frame) => frame,
            Err(error) => return self.local.fail(error),
        };
        self.local.observed = head;
        let Some((len, bytes)) = frame else {
            return Err(Error::Empty);
        };
        let serial = self.local.begin(id)?;
        Ok(ReadLease {
            owner: self,
            id,
            serial,
            len,
            bytes,
        })
    }

    pub fn try_recv(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        let lease = self.try_read()?;
        let len = lease.payload().len();
        if out.len() < len {
            return Err(Error::OutputTooSmall);
        }
        out[..len].copy_from_slice(lease.payload());
        lease.release()?;
        Ok(len)
    }

    /// Abandon a forgotten private copy; leave the message queued. Re-reading
    /// can observe different adversarial bytes. It is not immutable redelivery.
    pub fn cancel_pending(&mut self) {
        self.local.pending = None;
    }

    pub fn retire(&mut self) {
        let _ = self.local.fail::<()>(Error::Closed);
    }
}

#[must_use = "commit publishes; dropping cancels the private staging reservation"]
pub struct WriteLease<'a, M: Memory> {
    owner: &'a mut Producer<M>,
    id: Identity,
    serial: u64,
    len: usize,
    bytes: [u8; PAYLOAD_BYTES],
}

impl<M: Memory> WriteLease<'_, M> {
    pub fn identity(&self) -> Identity {
        self.id
    }

    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.bytes[..self.len]
    }

    pub fn commit(self) -> Result<(), Error> {
        self.owner.local.check(self.id, self.serial)?;
        let publish = || {
            // Clear the WHOLE reused slot, including all bytes beyond len, by
            // copying the initialized private staging array in bounded words.
            for i in 0..PAYLOAD_WORDS {
                let mut word = [0; 8];
                word.copy_from_slice(&self.bytes[i * 8..i * 8 + 8]);
                self.owner.memory.store(
                    Region::Payload,
                    self.id.offset() / 8 + i,
                    u64::from_le_bytes(word),
                )?;
            }
            let fields = [
                self.id.channel,
                self.id.sequence,
                self.id.generation,
                self.id.offset() as u64,
                self.len as u64,
            ];
            for (i, value) in fields.into_iter().enumerate() {
                self.owner
                    .memory
                    .store(Region::Producer, self.id.descriptor() + i, value)?;
            }
            let next = self.id.sequence.checked_add(1).ok_or(Error::Exhausted)?;
            self.owner
                .memory
                .store(Region::Producer, CURSOR_WORD, next)?;
            Ok(next)
        };
        match publish() {
            Ok(next) => {
                self.owner.local.next = next;
                self.owner.local.pending = None;
                Ok(())
            }
            Err(error) => self.owner.local.fail(error),
        }
    }
}

impl<M: Memory> Drop for WriteLease<'_, M> {
    fn drop(&mut self) {
        if self.owner.local.pending == Some((self.id, self.serial)) {
            self.owner.local.pending = None;
        }
    }
}

/// A bounded PRIVATE copy, not a borrowed view of the shared payload. Drop keeps
/// the frame queued; release advances only the trusted local consumer cursor.
///
/// The returned bytes carry [`Integrity::PRIVATE_COPY`]: memory safe and stable
/// after return, but never a coherent committed message and never authenticated.
/// Application code must validate this private copy itself and must not treat it
/// as one coherent transaction, even when it parses cleanly.
///
/// A private payload borrow cannot survive consuming its lease:
///
/// ```compile_fail
/// use sunlight_fastlane_ring::hostile::{AtomicPages, Producer, Consumer, Role};
/// let pages = AtomicPages::new();
/// let mut p = Producer::new(pages.view(Role::Producer), 7).unwrap();
/// let mut c = Consumer::new(pages.view(Role::Consumer), 7).unwrap();
/// p.try_send(b"private").unwrap();
/// let r = c.try_read().unwrap();
/// let bytes = r.payload();
/// r.release().unwrap();
/// assert_eq!(bytes, b"private");
/// ```
#[must_use = "release consumes; dropping keeps the message queued"]
pub struct ReadLease<'a, M: Memory> {
    owner: &'a mut Consumer<M>,
    id: Identity,
    serial: u64,
    len: usize,
    bytes: [u8; PAYLOAD_BYTES],
}

impl<M: Memory> ReadLease<'_, M> {
    pub fn identity(&self) -> Identity {
        self.id
    }

    pub fn payload(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Integrity of exactly the bytes returned by [`Self::payload`]. This is the
    /// only integrity value the representation can produce; there is no path
    /// that yields coherent publication or application authenticity.
    pub const fn integrity(&self) -> Integrity {
        Integrity::PRIVATE_COPY
    }

    pub fn release(self) -> Result<(), Error> {
        self.owner.local.check(self.id, self.serial)?;
        let next = self.id.sequence.checked_add(1).ok_or(Error::Exhausted)?;
        if let Err(error) = self.owner.memory.store(Region::Consumer, CURSOR_WORD, next) {
            return self.owner.local.fail(error);
        }
        self.owner.local.next = next;
        self.owner.local.pending = None;
        Ok(())
    }
}

impl<M: Memory> Drop for ReadLease<'_, M> {
    fn drop(&mut self) {
        if self.owner.local.pending == Some((self.id, self.serial)) {
            self.owner.local.pending = None;
        }
    }
}

#[cfg(test)]
mod tests;
