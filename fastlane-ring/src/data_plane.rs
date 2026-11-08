//! Draft SHM layout and least-privilege mapping plan for bulk streams.
//!
//! Control messages (including setup and teardown) use synchronous kernel IPC.
//! This module only plans bounded storage; it grants no authority, allocates no
//! memory and never attaches Rust references to a peer-controlled mapping.
//! Cooperative zero-copy leases remain in [`crate::Ring`]; hostile peers must
//! use [`crate::hostile`] until the kernel can enforce slot ownership.

use crate::{Header, LayoutError, PAGE};

/// Draft per-stream ceiling, including metadata and both cursors (256 KiB).
/// The authority layer must also enforce aggregate per-client/service budgets.
pub const MAX_STREAM_PAGES: u64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    Layout(LayoutError),
    PageBudget,
    MappingSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Producer,
    Consumer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    Metadata,
    ProducerCursor,
    ConsumerCursor,
    Payload,
}

/// Required page permissions, not a capability or an installed kernel grant.
/// Every region is non-executable. Offsets are relative to the SHM object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub kind: RegionKind,
    pub offset: u64,
    pub bytes: u64,
    pub access: Access,
}

/// Trusted local geometry; never constructed from a shared header.
///
/// A future SHM adapter must bind the plan to the existing authenticated channel
/// identity/generation and enforce these permissions before exposing any data.
/// `generation` is layout validation input, not proof of authorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    header: Header,
}

impl Plan {
    /// Plan the existing cooperative ring ABI within a trusted service budget.
    /// The caller may lower the ceiling, but cannot exceed MAX_STREAM_PAGES.
    pub fn for_ring<const N: usize, const P: usize>(
        generation: u64,
        page_budget: u64,
    ) -> Result<Self, PlanError> {
        let header = Header::for_config::<N, P>(generation).map_err(PlanError::Layout)?;
        if page_budget == 0
            || page_budget > MAX_STREAM_PAGES
            || header.total_size / PAGE > page_budget
        {
            return Err(PlanError::PageBudget);
        }
        Ok(Self { header })
    }

    pub const fn header(&self) -> Header {
        self.header
    }

    pub const fn bytes(&self) -> u64 {
        self.header.total_size
    }

    pub const fn pages(&self) -> u64 {
        self.bytes() / PAGE
    }

    /// Compare a kernel-reported object size before attempting attachment.
    /// This does not validate a pointer, peer identity or page permissions.
    pub fn check_mapping_size(&self, actual_bytes: u64) -> Result<(), PlanError> {
        if actual_bytes != self.bytes() {
            return Err(PlanError::MappingSize);
        }
        Ok(())
    }

    /// Disjoint, page-aligned regions covering the complete bounded object.
    /// Initialize before sharing, then map immutable metadata read-only in both
    /// roles. Payload is producer-writable and consumer-read-only. These page
    /// permissions alone cannot prevent mutation of already published slots.
    pub const fn regions(&self, role: Role) -> [Region; 4] {
        let (producer, consumer) = match role {
            Role::Producer => (Access::ReadWrite, Access::ReadOnly),
            Role::Consumer => (Access::ReadOnly, Access::ReadWrite),
        };
        [
            Region {
                kind: RegionKind::Metadata,
                offset: 0,
                bytes: PAGE,
                access: Access::ReadOnly,
            },
            Region {
                kind: RegionKind::ProducerCursor,
                offset: self.header.producer_offset,
                bytes: PAGE,
                access: producer,
            },
            Region {
                kind: RegionKind::ConsumerCursor,
                offset: self.header.consumer_offset,
                bytes: PAGE,
                access: consumer,
            },
            Region {
                kind: RegionKind::Payload,
                offset: self.header.payload_offset,
                bytes: self.bytes() - self.header.payload_offset,
                access: producer,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ring;
    use core::mem::size_of;

    #[test]
    fn plans_match_existing_ring_storage() {
        fn check<const N: usize, const P: usize>() {
            let plan = Plan::for_ring::<N, P>(9, MAX_STREAM_PAGES).unwrap();
            assert_eq!(plan.bytes(), size_of::<Ring<N, P>>() as u64);
            assert_eq!(plan.header(), Header::for_config::<N, P>(9).unwrap());
            for role in [Role::Producer, Role::Consumer] {
                let mut end = 0;
                for region in plan.regions(role) {
                    assert_eq!(region.offset, end);
                    assert_eq!(region.offset % PAGE, 0);
                    assert_eq!(region.bytes % PAGE, 0);
                    assert!(region.bytes > 0);
                    end += region.bytes;
                }
                assert_eq!(end, plan.bytes());
            }
        }
        check::<1, 1>();
        check::<16, 256>();
        check::<32, 4096>();
    }

    #[test]
    fn rejects_invalid_geometry_and_over_budget_streams() {
        for result in [
            Plan::for_ring::<0, 256>(1, 64),
            Plan::for_ring::<3, 256>(1, 64),
            Plan::for_ring::<16, 0>(1, 64),
            Plan::for_ring::<16, 4097>(1, 64),
            Plan::for_ring::<16, 256>(0, 64),
        ] {
            assert_eq!(result, Err(PlanError::Layout(LayoutError::Configuration)));
        }
        for budget in [0, 4, 65, u64::MAX] {
            assert_eq!(
                Plan::for_ring::<16, 256>(1, budget),
                Err(PlanError::PageBudget)
            );
        }
        assert_eq!(
            Plan::for_ring::<64, 4096>(1, 64),
            Err(PlanError::PageBudget)
        );
        let plan = Plan::for_ring::<16, 256>(1, 5).unwrap();
        assert_eq!(plan.pages(), 5);
        assert_eq!(plan.check_mapping_size(plan.bytes()), Ok(()));
        for bytes in [0, plan.bytes() - 1, plan.bytes() + PAGE, u64::MAX] {
            assert_eq!(plan.check_mapping_size(bytes), Err(PlanError::MappingSize));
        }
    }

    #[test]
    fn each_role_writes_only_its_cursor_and_producer_payload() {
        let plan = Plan::for_ring::<16, 256>(1, 5).unwrap();
        assert_eq!(
            plan.regions(Role::Producer).map(|r| r.access),
            [
                Access::ReadOnly,
                Access::ReadWrite,
                Access::ReadOnly,
                Access::ReadWrite
            ]
        );
        assert_eq!(
            plan.regions(Role::Consumer).map(|r| r.access),
            [
                Access::ReadOnly,
                Access::ReadOnly,
                Access::ReadWrite,
                Access::ReadOnly
            ]
        );
    }
}
