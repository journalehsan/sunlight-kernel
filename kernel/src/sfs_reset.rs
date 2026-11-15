//! Terminal VirtIO reset coordinator. One step per scheduled invocation; no
//! scheduler lock, wait loop, allocation, DMA release or recovery authority.
use core::sync::atomic::{AtomicU64, Ordering};
use sunlight_block::contract::{DeviceIdentity, IoOutcome, RequestId};
use sunlight_virtio::{ResetObservation, ResetState, VirtioBlk};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Running,
    Quiescing,
    ResetOwnerEstablished,
    ResetPending,
    ResetAcknowledged,
    FenceUnqualified,
    TerminalQuarantine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub device: DeviceIdentity,
    pub queue: u64,
    // A task incarnation supplied by the trusted kernel, never a bare PID.
    pub actor: u64,
}

struct Inner {
    device: VirtioBlk,
    phase: Phase,
}

pub struct Coordinator {
    inner: spin::Mutex<Inner>,
    identity: DeviceIdentity,
    queue: u64,
    // Zero means admission open. CAS closes admission and selects one owner.
    owner: AtomicU64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe<T> {
    Busy,
    Closed,
    Stale,
    Value(T),
}

impl Coordinator {
    pub fn new(device: VirtioBlk) -> Self {
        let observation = device.reset_observation();
        Self {
            identity: observation.device,
            queue: observation.queue_incarnation,
            owner: AtomicU64::new(0),
            inner: spin::Mutex::new(Inner {
                device,
                phase: Phase::Running,
            }),
        }
    }

    pub fn ticket(&self, actor: u64) -> Ticket {
        Ticket {
            device: self.identity,
            queue: self.queue,
            actor,
        }
    }

    /// Generation-qualified, nonblocking admission closure and unique owner.
    pub fn request(&self, ticket: Ticket) -> Probe<()> {
        if ticket.actor == 0 || ticket.device != self.identity || ticket.queue != self.queue {
            return Probe::Stale;
        }
        match self
            .owner
            .compare_exchange(0, ticket.actor, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Probe::Value(()),
            Err(owner) if owner == ticket.actor => Probe::Value(()),
            Err(_) => Probe::Closed,
        }
    }

    pub fn inventory(&self) -> Probe<(Phase, ResetObservation, Option<IoOutcome>)> {
        let Some(inner) = self.inner.try_lock() else {
            return Probe::Busy;
        };
        Probe::Value((
            inner.phase,
            inner.device.reset_observation(),
            inner.device.first_uncertain(),
        ))
    }

    /// # Safety
    /// Own the entire PCI attachment, with its coherent DMA allocations pinned.
    /// No scheduler-critical lock may be held; every invocation is one step.
    pub unsafe fn step(&self, ticket: Ticket) -> Probe<Phase> {
        if ticket != self.ticket(self.owner.load(Ordering::Acquire)) || ticket.actor == 0 {
            return Probe::Stale;
        }
        let Some(mut inner) = self.inner.try_lock() else {
            return Probe::Busy;
        };
        inner.phase = match inner.phase {
            Phase::Running => Phase::Quiescing,
            Phase::Quiescing => Phase::ResetOwnerEstablished,
            Phase::ResetOwnerEstablished => {
                // Driver quarantines first. Outstanding metadata is retained.
                inner.device.begin_terminal_reset();
                Phase::ResetPending
            }
            Phase::ResetPending => match inner.device.poll_terminal_reset().state {
                ResetState::Waiting => Phase::ResetPending,
                ResetState::VerifiedRetained => Phase::ResetAcknowledged,
                _ => Phase::TerminalQuarantine,
            },
            Phase::ResetAcknowledged => Phase::FenceUnqualified,
            Phase::FenceUnqualified => Phase::TerminalQuarantine,
            Phase::TerminalQuarantine => Phase::TerminalQuarantine,
        };
        Probe::Value(inner.phase)
    }

    /// Actor cancellation closes admission and quarantines on the next available
    /// step, without pretending reset or quiescence completed. No owner transfer.
    #[allow(dead_code)] // Cancellation is fail-stop; native cancellation witness remains separate.
    pub fn cancel(&self, ticket: Ticket) -> Probe<Phase> {
        if ticket.actor == 0 || ticket != self.ticket(self.owner.load(Ordering::Acquire)) {
            return Probe::Stale;
        }
        let Some(mut inner) = self.inner.try_lock() else {
            return Probe::Busy;
        };
        inner.device.quarantine_terminal();
        // begin_terminal_reset is deliberately not called here. No port access,
        // no acknowledgment, and no release. Closed admission pins the owner.
        inner.phase = Phase::TerminalQuarantine;
        Probe::Value(inner.phase)
    }

    /// # Safety
    /// Same exclusive attachment and pinned DMA contract as `step`.
    pub unsafe fn start_write(
        &self,
        lba: u64,
        data: &[u8; 512],
    ) -> Probe<Result<RequestId, IoOutcome>> {
        if self.owner.load(Ordering::Acquire) != 0 {
            return Probe::Closed;
        }
        let Some(mut inner) = self.inner.try_lock() else {
            return Probe::Busy;
        };
        if self.owner.load(Ordering::Acquire) != 0 {
            return Probe::Closed;
        }
        Probe::Value(inner.device.start_write(lba, data))
    }

    /// # Safety
    /// Same exclusive attachment and pinned DMA contract as `step`.
    pub unsafe fn poll_write(&self, id: RequestId) -> Probe<Option<IoOutcome>> {
        let Some(mut inner) = self.inner.try_lock() else {
            return Probe::Busy;
        };
        if self.owner.load(Ordering::Acquire) != 0 {
            return Probe::Closed;
        }
        Probe::Value(inner.device.poll_write(id))
    }

    pub fn timeout_write(&self, id: RequestId) -> Probe<Option<IoOutcome>> {
        let Some(mut inner) = self.inner.try_lock() else {
            return Probe::Busy;
        };
        Probe::Value(inner.device.timeout_write(id))
    }
}
