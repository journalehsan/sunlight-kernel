//! Bounded cooperative SPSC ring with copying and zero-copy operations.
//!
//! Leases borrow the unique role handle until publication or reclamation. They
//! do not authenticate peers, attach mappings, revoke access or wake tasks. See
//! docs/ipc/fastlane-f12-spsc-ring.md and fastlane-zero-copy-leases.md.
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(not(all(
    target_arch = "x86_64",
    target_endian = "little",
    target_has_atomic = "64"
)))]
compile_error!("FastLane layout v1 supports little-endian x86_64 with 64-bit atomics only");

use core::{
    cell::{Cell, UnsafeCell},
    marker::PhantomData,
    mem::{align_of, size_of},
    sync::atomic::{AtomicU64, Ordering},
};

pub const PAGE: u64 = 4096;
pub const MAGIC: u64 = 0x5355_4e46_4c52_3031;
pub const VERSION: u32 = 1;
pub const MAX_CAPACITY: usize = 65536;
pub const MAX_PAYLOAD: usize = 4096;

mod leases;
pub use leases::{ReadLease, WriteLease};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    Configuration,
    Header,
    Generation,
    NotReady,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    Full,
    PayloadTooLarge,
    InvalidChannel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvError {
    Empty,
    OutputBufferTooSmall,
    InvalidChannel,
    InvalidFrame,
}

/// Immutable after trusted initialization. Native little-endian x86_64 ABI.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub magic: u64,
    pub version: u32,
    pub capacity: u32,
    pub payload_size: u32,
    pub slot_stride: u32,
    pub generation: u64,
    pub producer_offset: u64,
    pub consumer_offset: u64,
    pub payload_offset: u64,
    pub total_size: u64,
}

impl Header {
    pub fn for_config<const N: usize, const P: usize>(
        generation: u64,
    ) -> Result<Self, LayoutError> {
        if N == 0
            || !N.is_power_of_two()
            || N > MAX_CAPACITY
            || P == 0
            || P > MAX_PAYLOAD
            || generation == 0
        {
            return Err(LayoutError::Configuration);
        }
        let stride = (P as u64)
            .checked_add(7)
            .ok_or(LayoutError::Configuration)?
            & !3;
        let bytes = stride
            .checked_mul(N as u64)
            .and_then(|v| v.checked_add(3 * PAGE))
            .ok_or(LayoutError::Configuration)?;
        let total = bytes
            .checked_add(PAGE - 1)
            .ok_or(LayoutError::Configuration)?
            & !(PAGE - 1);
        Ok(Self {
            magic: MAGIC,
            version: VERSION,
            capacity: N as u32,
            payload_size: P as u32,
            slot_stride: stride as u32,
            generation,
            producer_offset: PAGE,
            consumer_offset: 2 * PAGE,
            payload_offset: 3 * PAGE,
            total_size: total,
        })
    }
    pub fn validate<const N: usize, const P: usize>(
        &self,
        generation: u64,
    ) -> Result<(), LayoutError> {
        if self.generation != generation {
            return Err(LayoutError::Generation);
        }
        if *self != Self::for_config::<N, P>(generation)? {
            return Err(LayoutError::Header);
        }
        Ok(())
    }
}

#[repr(C, align(4096))]
struct Metadata {
    header: Header,
    ready: AtomicU64,
}
#[repr(C, align(4096))]
struct Cursor {
    value: AtomicU64,
}
#[repr(C)]
struct Slot<const P: usize> {
    len: u32,
    bytes: [u8; P],
}
#[repr(C, align(4096))]
struct Payload<const N: usize, const P: usize> {
    slots: [UnsafeCell<Slot<P>>; N],
}

/// No pointers or heap metadata in this representation. Only `split` exposes access.
#[repr(C, align(4096))]
pub struct Ring<const N: usize, const P: usize> {
    metadata: Metadata,
    producer: Cursor,
    consumer: Cursor,
    payload: Payload<N, P>,
}

// SAFETY: private slots are accessed only through unique role handles; Release /
// Acquire transfers each slot between writer and reader. No shared &mut Slot is
// created. Exclusive Ring access is required for initialization, split and retire.
unsafe impl<const N: usize, const P: usize> Sync for Ring<N, P> {}

impl<const N: usize, const P: usize> Ring<N, P> {
    pub fn new(generation: u64) -> Result<Self, LayoutError> {
        let header = Header::for_config::<N, P>(generation)?;
        let ring = Self {
            metadata: Metadata {
                header,
                ready: AtomicU64::new(0),
            },
            producer: Cursor {
                value: AtomicU64::new(0),
            },
            consumer: Cursor {
                value: AtomicU64::new(0),
            },
            payload: Payload {
                slots: core::array::from_fn(|_| {
                    UnsafeCell::new(Slot {
                        len: 0,
                        bytes: [0; P],
                    })
                }),
            },
        };
        if size_of::<Self>() as u64 != header.total_size || align_of::<Self>() != PAGE as usize {
            return Err(LayoutError::Header);
        }
        ring.metadata.ready.store(generation, Ordering::Release);
        Ok(ring)
    }
    pub fn header(&self) -> Header {
        self.metadata.header
    }
    /// Retire only after handles and all operations have ended. No active revocation.
    pub fn retire(&mut self) {
        self.metadata.ready.store(0, Ordering::Release);
    }
    pub fn split(
        &mut self,
        generation: u64,
    ) -> Result<(Producer<'_, N, P>, Consumer<'_, N, P>), LayoutError> {
        if self.metadata.ready.load(Ordering::Acquire) != generation {
            return Err(LayoutError::NotReady);
        }
        self.metadata.header.validate::<N, P>(generation)?;
        Ok((
            Producer {
                ring: self,
                unique: PhantomData,
            },
            Consumer {
                ring: self,
                unique: PhantomData,
            },
        ))
    }
}

/// Unique, movable but not Sync; operations require &mut self. Not Clone.
pub struct Producer<'a, const N: usize, const P: usize> {
    ring: &'a Ring<N, P>,
    unique: PhantomData<Cell<()>>,
}
/// Unique, movable but not Sync; operations require &mut self. Not Clone.
pub struct Consumer<'a, const N: usize, const P: usize> {
    ring: &'a Ring<N, P>,
    unique: PhantomData<Cell<()>>,
}

impl<const N: usize, const P: usize> Producer<'_, N, P> {
    pub fn try_send(&mut self, bytes: &[u8]) -> Result<(), SendError> {
        // Every exposed byte is overwritten, so the copying path needs no clear.
        let mut lease = self.reserve_slot(bytes.len())?;
        lease.payload_mut().copy_from_slice(bytes);
        lease.commit();
        Ok(())
    }
}
impl<const N: usize, const P: usize> Consumer<'_, N, P> {
    pub fn try_recv(&mut self, out: &mut [u8]) -> Result<usize, RecvError> {
        let lease = self.try_read()?;
        let len = lease.payload().len();
        if len > out.len() {
            return Err(RecvError::OutputBufferTooSmall);
        }
        out[..len].copy_from_slice(lease.payload());
        lease.release();
        Ok(len)
    }
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod lease_tests;
#[cfg(test)]
mod tests;
