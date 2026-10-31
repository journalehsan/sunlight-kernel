//! Slot ownership is transferred only by cursor Release stores, never by Drop.
use super::{Consumer, Producer, RecvError, Ring, SendError};
use core::{cell::Cell, marker::PhantomData, sync::atomic::Ordering};

/// Exclusive write access to one unpublished slot. Dropping cancels publication.
/// A subsequent reservation clears its own exposed bytes before returning them.
#[must_use = "commit the lease to publish; dropping it cancels the reservation"]
pub struct WriteLease<'a, const N: usize, const P: usize> {
    ring: &'a Ring<N, P>,
    head: u64,
    len: usize,
    unique: PhantomData<Cell<()>>,
}

/// Read access pins one published slot until explicit release. Dropping keeps
/// the message queued, including during unwinding. No slot reference may survive
/// release, so a producer cannot race safe readers during slot reuse.
#[must_use = "release the lease to consume; dropping it keeps the message queued"]
pub struct ReadLease<'a, const N: usize, const P: usize> {
    ring: &'a Ring<N, P>,
    tail: u64,
    len: usize,
    unique: PhantomData<Cell<()>>,
}

impl<const N: usize, const P: usize> Producer<'_, N, P> {
    /// Reserve exactly `len` initialized bytes for writing directly in the ring.
    /// Those bytes are cleared even on reuse or after a cancelled reservation.
    /// There is no publication until `WriteLease::commit` consumes the lease.
    ///
    /// ```
    /// use sunlight_fastlane_ring::Ring;
    /// let mut ring = Ring::<2, 64>::new(1).unwrap();
    /// let (mut producer, mut consumer) = ring.split(1).unwrap();
    /// let mut write = producer.try_reserve(4).unwrap();
    /// write.payload_mut().copy_from_slice(b"ping");
    /// write.commit();
    /// let read = consumer.try_read().unwrap();
    /// assert_eq!(read.payload(), b"ping");
    /// read.release();
    /// ```
    ///
    /// A role cannot advance while its slot is borrowed:
    ///
    /// ```compile_fail
    /// use sunlight_fastlane_ring::Ring;
    /// let mut ring = Ring::<2, 64>::new(1).unwrap();
    /// let (mut producer, _) = ring.split(1).unwrap();
    /// let mut write = producer.try_reserve(4).unwrap();
    /// producer.try_send(b"next").unwrap();
    /// write.payload_mut()[0] = 1;
    /// ```
    pub fn try_reserve(&mut self, len: usize) -> Result<WriteLease<'_, N, P>, SendError> {
        let mut lease = self.reserve_slot(len)?;
        lease.payload_mut().fill(0);
        Ok(lease)
    }

    pub(super) fn reserve_slot(&mut self, len: usize) -> Result<WriteLease<'_, N, P>, SendError> {
        if len > P {
            return Err(SendError::PayloadTooLarge);
        }
        let head = self.ring.producer.value.load(Ordering::Relaxed);
        let tail = self.ring.consumer.value.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail);
        if used > N as u64 {
            return Err(SendError::InvalidChannel);
        }
        if used == N as u64 {
            return Err(SendError::Full);
        }
        Ok(WriteLease {
            ring: self.ring,
            head,
            len,
            unique: PhantomData,
        })
    }
}

impl<const N: usize, const P: usize> WriteLease<'_, N, P> {
    pub fn payload_mut(&mut self) -> &mut [u8] {
        let slot = self.ring.payload.slots[self.head as usize & (N - 1)].get();
        // SAFETY: the unique producer is borrowed for this lease's lifetime.
        // Acquire tail in reserve_slot observed completion of all previous reads.
        // No head publication occurs until commit consumes this lease, which
        // cannot happen with a live returned borrow. len <= P was checked once
        // before constructing the lease; geometry is process-local and fixed.
        unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!((*slot).bytes).cast::<u8>(),
                self.len,
            )
        }
    }

    /// Publish the complete message, including a valid zero-length message.
    /// Notify any platform wait channel only after this returns.
    pub fn commit(self) {
        let slot = self.ring.payload.slots[self.head as usize & (N - 1)].get();
        // SAFETY: the unpublished slot remains exclusively producer-owned.
        // len is bounded by P <= MAX_PAYLOAD <= u32::MAX.
        unsafe { core::ptr::addr_of_mut!((*slot).len).write(self.len as u32) };
        self.ring
            .producer
            .value
            .store(self.head.wrapping_add(1), Ordering::Release);
    }
}

impl<const N: usize, const P: usize> Consumer<'_, N, P> {
    /// Borrow the next message directly from ring storage, without copying it.
    /// Neither dropping nor forgetting the lease consumes the message.
    ///
    /// Slot references cannot escape release:
    ///
    /// ```compile_fail
    /// use sunlight_fastlane_ring::Ring;
    /// let mut ring = Ring::<1, 8>::new(1).unwrap();
    /// let (mut producer, mut consumer) = ring.split(1).unwrap();
    /// producer.try_send(b"payload").unwrap();
    /// let read = consumer.try_read().unwrap();
    /// let bytes = read.payload();
    /// read.release();
    /// assert_eq!(bytes, b"payload");
    /// ```
    pub fn try_read(&mut self) -> Result<ReadLease<'_, N, P>, RecvError> {
        let tail = self.ring.consumer.value.load(Ordering::Relaxed);
        let head = self.ring.producer.value.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail);
        if used > N as u64 {
            return Err(RecvError::InvalidChannel);
        }
        if used == 0 {
            return Err(RecvError::Empty);
        }
        let slot = self.ring.payload.slots[tail as usize & (N - 1)].get();
        // SAFETY: Acquire head observed length and payload publication. The
        // producer cannot reuse this slot until this lease releases the tail.
        let len = unsafe { core::ptr::addr_of!((*slot).len).read() } as usize;
        if len > P {
            return Err(RecvError::InvalidFrame);
        }
        Ok(ReadLease {
            ring: self.ring,
            tail,
            len,
            unique: PhantomData,
        })
    }
}

impl<const N: usize, const P: usize> ReadLease<'_, N, P> {
    pub fn payload(&self) -> &[u8] {
        let slot = self.ring.payload.slots[self.tail as usize & (N - 1)].get();
        // SAFETY: length was checked after Acquire publication; this lease
        // borrows the consumer, keeping tail unchanged while any slice lives.
        // Payload is initialized, and the producer cannot mutate this slot.
        unsafe {
            core::slice::from_raw_parts(core::ptr::addr_of!((*slot).bytes).cast::<u8>(), self.len)
        }
    }

    /// Consume the message and permit producer reuse after its Acquire tail.
    /// A platform capacity notification must follow this reclamation.
    pub fn release(self) {
        self.ring
            .consumer
            .value
            .store(self.tail.wrapping_add(1), Ordering::Release);
    }
}
