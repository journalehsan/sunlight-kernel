//! Exclusive admission and settlement for a whole device. The caller is the
//! trusted coordinator and supplies authenticated registration identities.
use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
use spin::Mutex;
#[cfg(not(target_os = "none"))]
use alloc::{boxed::Box, vec};
use sunlight_fs::BLOCK_SIZE;
use sunlight_ipc::vfs_rpc::{lifecycle::RequestIds, registration::Registration};

#[cfg_attr(target_os = "none", path = "device_lease/admission.rs")]
mod admission;
pub use admission::*;

#[cfg(test)]
mod settlement_tests;
#[cfg(test)]
mod tests;

const BLOCKS: usize = 4;
const DATA_CREDITS: usize = 4;
#[cfg(not(target_os = "none"))]
const TRANSPORT_SLOTS: usize = 4;
#[cfg(target_os = "none")]
const TRANSPORT_SLOTS: usize = 1;

pub type Block = [u8; BLOCK_SIZE];

#[derive(Clone, Debug)]
struct Key {
    scope: Weak<()>,
    id: u64,
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.scope.ptr_eq(&other.scope)
    }
}

impl Eq for Key {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    owner: Registration,
    generation: Key,
    media: Option<Key>,
    grant: Option<Grant>,
}

impl Lease {
    pub fn owner(&self) -> Registration {
        self.owner
    }

    pub fn generation(&self) -> u64 {
        self.generation.id
    }
}

#[derive(Debug)]
struct Ticket {
    key: Key,
    lease: Key,
    media: Option<Key>,
}

#[derive(Debug)]
pub struct Operation(Ticket);

#[derive(Debug)]
pub struct Transition(Key);

#[derive(Debug)]
pub struct Flush(Ticket);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Occupied,
    Stale,
    AdmissionClosed,
    Busy,
    OutOfRange,
    NoCredit,
    Exhausted,
    InvalidSuccessor,
    Fenced,
    Unauthorized,
    InvalidGeometry,
    NoMedia,
}

/// An ambiguous completion settles the transport credit but leaves the
/// effect unknown. A lost transport can also be reported before settlement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failed,
    Ambiguous { applied: bool },
}

impl Outcome {
    fn applied(self) -> bool {
        matches!(self, Self::Success | Self::Ambiguous { applied: true })
    }

    fn ambiguous(self) -> bool {
        matches!(self, Self::Ambiguous { .. })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Completion {
    Read(Block),
    Written,
    Failed,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fence {
    AmbiguousIo,
    AmbiguousFlush,
    TransportLost,
    MediaRemoved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Vacant,
    Active,
    Quiescing,
    Flushing,
    Cancelling,
    Synchronized,
    Retiring,
    Fenced(Fence),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub lease: Option<Lease>,
    pub status: Status,
    pub outstanding: usize,
    pub retained_data: usize,
    pub flush_pending: bool,
}

#[derive(Clone)]
enum Destination {
    #[cfg(test)]
    Transfer(Registration),
    AuthenticatedTransfer(Grant),
    Release,
    Retire,
    Resume,
    ClientFlush,
}

#[derive(Clone)]
struct Change {
    key: Key,
    destination: Destination,
}

enum Phase {
    Vacant,
    Active,
    Quiescing(Change),
    Flushing(Change),
    Synchronized(Change),
    Fenced(Fence),
}

impl Phase {
    fn status(&self) -> Status {
        match self {
            Self::Quiescing(change) | Self::Flushing(change)
                if matches!(change.destination, Destination::Retire) =>
            {
                Status::Retiring
            }
            Self::Vacant => Status::Vacant,
            Self::Active => Status::Active,
            Self::Quiescing(_) => Status::Quiescing,
            Self::Flushing(change) if matches!(change.destination, Destination::Resume) => {
                Status::Cancelling
            }
            Self::Flushing(_) => Status::Flushing,
            Self::Synchronized(_) => Status::Synchronized,
            Self::Fenced(reason) => Status::Fenced(*reason),
        }
    }
}

enum Command {
    Read(usize),
    Write(usize, Block),
}

struct Pending {
    block: usize,
    buffer: Block,
    write: bool,
    credit: bool,
}

struct Settlement {
    #[cfg(not(target_os = "none"))]
    visible: Box<[Block]>,
    #[cfg(not(target_os = "none"))]
    durable: Box<[Block]>,
    pending: BTreeMap<u64, Pending>,
    flush: Option<Key>,
}

impl Settlement {
    fn credits(&self) -> usize {
        self.pending
            .values()
            .filter(|request| request.credit)
            .count()
    }

    #[cfg(not(target_os = "none"))]
    fn complete(&mut self, id: u64, outcome: Outcome) -> Result<Completion, Error> {
        let mut request = self.pending.remove(&id).ok_or(Error::Stale)?;
        let completion = if outcome.applied() {
            if request.write {
                self.visible[request.block] = request.buffer;
                Completion::Written
            } else {
                request.buffer = self.visible[request.block];
                Completion::Read(request.buffer)
            }
        } else {
            Completion::Failed
        };
        Ok(if outcome.ambiguous() {
            Completion::Ambiguous
        } else {
            completion
        })
    }
}

struct State {
    ids: RequestIds,
    scope: Arc<()>,
    lease: Option<Lease>,
    phase: Phase,
    transport: Settlement,
    admission: Admission,
}

impl State {
    fn key(&mut self) -> Result<Key, Error> {
        Ok(Key {
            scope: Arc::downgrade(&self.scope),
            id: self.ids.allocate().ok_or(Error::Exhausted)?,
        })
    }

    fn check_lease(&self, lease: &Lease) -> Result<(), Error> {
        if self.lease.as_ref() == Some(lease) {
            Ok(())
        } else {
            Err(Error::Stale)
        }
    }

    fn ticket(&mut self) -> Result<Ticket, Error> {
        let key = self.key()?;
        let lease = self.lease.as_ref().expect("submission requires a lease");
        Ok(Ticket {
            key,
            lease: lease.generation.clone(),
            media: lease.media.clone(),
        })
    }

    fn check_ticket(&self, ticket: &Ticket) -> Result<(), Error> {
        let lease = self.lease.as_ref().ok_or(Error::Stale)?;
        if lease.generation != ticket.lease || lease.media != ticket.media {
            return Err(Error::Stale);
        }
        Ok(())
    }

    #[cfg(test)]
    fn activate(&mut self, owner: Registration, generation: Key) -> Lease {
        self.activate_grant(owner, generation, None)
    }

    fn activate_grant(
        &mut self,
        owner: Registration,
        generation: Key,
        grant: Option<Grant>,
    ) -> Lease {
        let lease = Lease {
            owner,
            generation,
            media: self.admission.media.as_ref().map(|media| media.key.clone()),
            grant,
        };
        self.lease = Some(lease.clone());
        self.phase = Phase::Active;
        lease
    }

    fn release(&mut self) {
        self.lease = None;
        self.phase = Phase::Vacant;
    }

    fn fence(&mut self, reason: Fence) {
        if !matches!(self.phase, Phase::Fenced(_)) {
            self.phase = Phase::Fenced(reason);
        }
    }
}

/// All admitted I/O for this device goes through one authority. Its lock covers
/// bounded admission and completion steps. There are no waits for drain or
/// flush, callbacks, or filesystem/namespace lock acquisitions inside it.
pub struct DeviceLease {
    state: Mutex<State>,
}

impl Default for DeviceLease {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceLease {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                ids: RequestIds::new(),
                scope: Arc::new(()),
                lease: None,
                phase: Phase::Vacant,
                transport: Settlement {
                    #[cfg(not(target_os = "none"))]
                    visible: vec![[0; BLOCK_SIZE]; BLOCKS].into_boxed_slice(),
                    #[cfg(not(target_os = "none"))]
                    durable: vec![[0; BLOCK_SIZE]; BLOCKS].into_boxed_slice(),
                    pending: BTreeMap::new(),
                    flush: None,
                },
                admission: Admission::default(),
            }),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock();
        Snapshot {
            lease: state.lease.clone(),
            status: state.phase.status(),
            outstanding: state.transport.credits(),
            retained_data: state.transport.pending.len(),
            flush_pending: state.transport.flush.is_some(),
        }
    }

    #[cfg(test)]
    fn acquire(&self, owner: Registration) -> Result<Lease, Error> {
        let mut state = self.state.lock();
        match state.phase {
            Phase::Vacant => {}
            Phase::Fenced(_) => return Err(Error::Fenced),
            _ => return Err(Error::Occupied),
        }
        let key = state.key()?;
        Ok(state.activate(owner, key))
    }

    #[cfg(test)]
    fn read(&self, lease: &Lease, block: usize) -> Result<Operation, Error> {
        self.submit(lease, block, Command::Read(block))
    }

    #[cfg(test)]
    fn write(&self, lease: &Lease, block: usize, bytes: Block) -> Result<Operation, Error> {
        self.submit(lease, block, Command::Write(block, bytes))
    }

    #[cfg(test)]
    fn submit(&self, lease: &Lease, block: usize, command: Command) -> Result<Operation, Error> {
        let mut state = self.state.lock();
        state.check_lease(lease)?;
        if !matches!(state.phase, Phase::Active) {
            return Err(Error::AdmissionClosed);
        }
        if block >= BLOCKS {
            return Err(Error::OutOfRange);
        }
        state.enqueue(command)
    }

    /// Retire the caller's credit after cancellation or timeout. The trusted
    /// coordinator keeps the operation ticket for the transport acknowledgement.
    pub fn abandon(&self, operation: &Operation) -> Result<(), Error> {
        let mut state = self.state.lock();
        state.check_ticket(&operation.0)?;
        let pending = state
            .transport
            .pending
            .get_mut(&operation.0.key.id)
            .ok_or(Error::Stale)?;
        if !core::mem::replace(&mut pending.credit, false) {
            return Err(Error::Stale);
        }
        Ok(())
    }

    /// Deliver a terminal fake transport acknowledgement. Even an ambiguous
    /// effect here means the transport has relinquished the request and buffer.
    #[cfg(not(target_os = "none"))]
    pub fn complete(&self, operation: &Operation, outcome: Outcome) -> Result<Completion, Error> {
        let mut state = self.state.lock();
        state.check_ticket(&operation.0)?;
        let completion = state.transport.complete(operation.0.key.id, outcome)?;
        if outcome.ambiguous() {
            state.fence(Fence::AmbiguousIo);
        }
        Ok(completion)
    }

    #[cfg(target_os = "none")]
    pub fn command(&self, operation: &Operation) -> Result<(u64, Option<Block>), Error> {
        let state = self.state.lock();
        state.check_ticket(&operation.0)?;
        let pending = state
            .transport
            .pending
            .get(&operation.0.key.id)
            .ok_or(Error::Stale)?;
        Ok((
            pending.block as u64,
            pending.write.then_some(pending.buffer),
        ))
    }

    #[cfg(target_os = "none")]
    pub fn complete_live(&self, operation: &Operation, data: Option<Block>) -> Result<(), Error> {
        let mut state = self.state.lock();
        state.check_ticket(&operation.0)?;
        let pending = state
            .transport
            .pending
            .get_mut(&operation.0.key.id)
            .ok_or(Error::Stale)?;
        match (pending.write, data) {
            (true, None) => {}
            (false, Some(data)) => pending.buffer = data,
            _ => return Err(Error::Stale),
        }
        state.transport.pending.remove(&operation.0.key.id);
        Ok(())
    }

    #[cfg(test)]
    fn begin_transfer(&self, lease: &Lease, next: Registration) -> Result<Transition, Error> {
        if next == lease.owner {
            return Err(Error::InvalidSuccessor);
        }
        self.begin(lease, Destination::Transfer(next))
    }

    /// Call after namespace withdrawal dependencies have been settled.
    pub fn begin_release(&self, lease: &Lease) -> Result<Transition, Error> {
        self.begin(lease, Destination::Release)
    }

    fn begin(&self, lease: &Lease, destination: Destination) -> Result<Transition, Error> {
        let mut state = self.state.lock();
        state.begin(lease, destination)
    }
}

impl State {
    fn enqueue(&mut self, command: Command) -> Result<Operation, Error> {
        if self.transport.credits() == DATA_CREDITS
            || self.transport.pending.len() == TRANSPORT_SLOTS
        {
            return Err(Error::NoCredit);
        }
        let ticket = self.ticket()?;
        let (block, buffer, write) = match command {
            Command::Read(block) => (block, [0; BLOCK_SIZE], false),
            Command::Write(block, buffer) => (block, buffer, true),
        };
        self.transport.pending.insert(
            ticket.key.id,
            Pending {
                block,
                buffer,
                write,
                credit: true,
            },
        );
        Ok(Operation(ticket))
    }

    fn begin(&mut self, lease: &Lease, destination: Destination) -> Result<Transition, Error> {
        self.check_lease(lease)?;
        if !matches!(self.phase, Phase::Active) {
            return Err(Error::AdmissionClosed);
        }
        let key = self.key()?;
        self.phase = Phase::Quiescing(Change {
            key: key.clone(),
            destination,
        });
        Ok(Transition(key))
    }
}

impl DeviceLease {
    /// Begin the transport durability barrier after all accepted data I/O
    /// has settled. Backend writeback must already have entered the queue.
    pub fn start_flush(&self, transition: &Transition) -> Result<Flush, Error> {
        let mut state = self.state.lock();
        let change = match &state.phase {
            Phase::Quiescing(change) if change.key == transition.0 => change.clone(),
            _ => return Err(Error::Stale),
        };
        if !state.transport.pending.is_empty() {
            return Err(Error::Busy);
        }
        let ticket = state.ticket()?;
        state.transport.flush = Some(ticket.key.clone());
        state.phase = Phase::Flushing(change);
        Ok(Flush(ticket))
    }

    pub fn complete_flush(&self, flush: &Flush, outcome: Outcome) -> Result<(), Error> {
        let mut state = self.state.lock();
        state.check_ticket(&flush.0)?;
        if state.transport.flush.as_ref() != Some(&flush.0.key) {
            return Err(Error::Stale);
        }
        state.transport.flush = None;
        #[cfg(not(target_os = "none"))]
        if outcome.applied() {
            {
                let transport = &mut state.transport;
                transport.durable.copy_from_slice(&transport.visible);
            }
        }
        if outcome.ambiguous() {
            state.fence(Fence::AmbiguousFlush);
            return Ok(());
        }
        let Phase::Flushing(change) = &state.phase else {
            // Transport loss already fenced this device.
            return Ok(());
        };
        state.phase = match (&change.destination, outcome) {
            (Destination::Retire, Outcome::Success) => {
                state.release();
                return Ok(());
            }
            (Destination::Retire, Outcome::Failed) => Phase::Quiescing(change.clone()),
            (Destination::Resume | Destination::ClientFlush, _) | (_, Outcome::Failed) => {
                Phase::Active
            }
            (_, Outcome::Success) => Phase::Synchronized(change.clone()),
            (_, Outcome::Ambiguous { .. }) => unreachable!(),
        };
        Ok(())
    }

    /// Serialize old-generation revocation and new-generation activation.
    pub fn finish(&self, transition: &Transition) -> Result<Option<Lease>, Error> {
        let mut state = self.state.lock();
        let change = match &state.phase {
            Phase::Synchronized(change) if change.key == transition.0 => change.clone(),
            Phase::Quiescing(change) | Phase::Flushing(change) if change.key == transition.0 => {
                return Err(Error::Busy);
            }
            Phase::Fenced(_) => return Err(Error::Fenced),
            _ => return Err(Error::Stale),
        };
        match change.destination {
            #[cfg(test)]
            Destination::Transfer(owner) => Ok(Some(state.activate(owner, change.key))),
            Destination::AuthenticatedTransfer(grant) => {
                let incumbent = state.lease.as_ref().and_then(|lease| lease.grant.as_ref());
                state
                    .admission
                    .check_session(&incumbent.ok_or(Error::Stale)?.session)?;
                state.admission.check_session(&grant.session)?;
                Ok(Some(state.activate_grant(
                    grant.session.registration,
                    change.key,
                    Some(grant),
                )))
            }
            Destination::Release => {
                state.release();
                Ok(None)
            }
            _ => unreachable!("only transfer and release reach synchronized"),
        }
    }

    /// An already submitted flush must settle before admission reopens.
    pub fn cancel(&self, transition: &Transition) -> Result<(), Error> {
        let mut state = self.state.lock();
        match &mut state.phase {
            Phase::Quiescing(change) | Phase::Synchronized(change)
                if change.key == transition.0
                    && !matches!(change.destination, Destination::Retire) =>
            {
                state.phase = Phase::Active;
            }
            Phase::Flushing(change)
                if change.key == transition.0
                    && !matches!(change.destination, Destination::Retire) =>
            {
                change.destination = Destination::Resume;
            }
            _ => return Err(Error::Stale),
        }
        Ok(())
    }

    /// Cancel any prospective successor and permanently close incumbent I/O
    /// admission. The coordinator can finish draining using the returned
    /// transition. A barrier already in flight can satisfy retirement.
    pub fn retire(&self, lease: &Lease) -> Result<Transition, Error> {
        let mut state = self.state.lock();
        state.check_lease(lease)?;
        let key = lease.generation.clone();
        let change = Change {
            key: key.clone(),
            destination: Destination::Retire,
        };
        state.phase = match state.phase {
            Phase::Flushing(_) => Phase::Flushing(change),
            Phase::Synchronized(_) => {
                state.release();
                return Ok(Transition(key));
            }
            Phase::Fenced(_) => return Err(Error::Fenced),
            _ => Phase::Quiescing(change),
        };
        Ok(Transition(key))
    }

    /// A loss notification is scoped to the lease that observed it. Its
    /// queued operations retain their transport slots until terminal settlement.
    pub fn transport_lost(&self, lease: &Lease) -> Result<(), Error> {
        let mut state = self.state.lock();
        state.check_lease(lease)?;
        state.fence(Fence::TransportLost);
        Ok(())
    }
}
