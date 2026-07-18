#![no_std]

#[path = "process/mm2a_plan.rs"]
pub mod mm2a_plan;

#[path = "process/mm2b_state.rs"]
pub mod mm2b_state;

#[path = "process/region.rs"]
pub mod region;

#[cfg(test)]
mod tests {
    use super::mm2a_plan::{checked_page_layout, DeferredCursor, PlanError};
    use super::region::{
        LedgerError, MappingKind, MappingRegion, RangeLookup, RegionBacking, RegionLedger,
        RegionPolicy, RegionProtection, MAX_REGIONS_PER_ADDRESS_SPACE,
    };

    fn anonymous(start: u64, end: u64) -> MappingRegion {
        MappingRegion::new(
            start,
            end,
            RegionProtection::READ_WRITE,
            MappingKind::Anonymous,
            RegionPolicy::MAY_UNMAP
                .union(RegionPolicy::MAY_CHANGE_PROTECTION)
                .union(RegionPolicy::OWNER_MANAGED),
            RegionBacking::None,
        )
        .unwrap()
    }

    fn insert(ledger: &mut RegionLedger, region: MappingRegion) {
        let reservation = ledger.preflight(region).unwrap();
        ledger.commit(reservation).unwrap();
    }

    #[test]
    fn checked_page_layout_rejects_zero_and_overflow() {
        assert_eq!(checked_page_layout(0), Err(PlanError::ZeroLength));
        assert_eq!(checked_page_layout(u64::MAX), Err(PlanError::Overflow));
        assert_eq!(checked_page_layout(1), Ok((1, 4096)));
        assert_eq!(checked_page_layout(4097), Ok((2, 8192)));
    }

    #[test]
    fn cursor_changes_only_at_commit() {
        let mut cursor = 0;
        let transaction = DeferredCursor::new(cursor, 0x10_0000_0000, 8192).unwrap();
        assert_eq!(transaction.base(), 0x10_0000_0000);

        // A failed allocation/map path drops the transaction without publishing.
        assert_eq!(cursor, 0);

        transaction.commit(&mut cursor);
        assert_eq!(cursor, 0x10_0000_2000);
    }

    #[test]
    fn cursor_overflow_fails_without_mutation() {
        let cursor = u64::MAX - 4095;
        assert_eq!(
            DeferredCursor::new(cursor, 0, 8192),
            Err(PlanError::Overflow)
        );
        assert_eq!(cursor, u64::MAX - 4095);
    }

    #[test]
    fn ledger_insert_overlap_adjacency_and_merge() {
        let mut ledger = RegionLedger::new();
        insert(&mut ledger, anonymous(0x1000, 0x2000));
        assert_eq!(ledger.len(), 1);
        assert_eq!(
            ledger.preflight(anonymous(0x1000, 0x3000)),
            Err(LedgerError::Overlap)
        );

        let distinct = MappingRegion::new(
            0x2000,
            0x3000,
            RegionProtection::READ_ONLY,
            MappingKind::Anonymous,
            RegionPolicy::MAY_UNMAP,
            RegionBacking::None,
        )
        .unwrap();
        insert(&mut ledger, distinct);
        assert_eq!(ledger.len(), 2);

        insert(&mut ledger, anonymous(0x3000, 0x4000));
        insert(&mut ledger, anonymous(0x4000, 0x5000));
        assert_eq!(ledger.len(), 3);
        assert_eq!(ledger.record_at(2).unwrap().end, 0x5000);
        assert_eq!(ledger.validate(), Ok(()));
    }

    #[test]
    fn distinct_shm_objects_never_merge() {
        let policy = RegionPolicy::SHARED.union(RegionPolicy::OWNER_MANAGED);
        let mut ledger = RegionLedger::new();
        for (start, token) in [(0x1000, 11), (0x2000, 12)] {
            insert(
                &mut ledger,
                MappingRegion::new(
                    start,
                    start + 0x1000,
                    RegionProtection::READ_WRITE,
                    MappingKind::SharedMemory,
                    policy,
                    RegionBacking::SharedMemory(token),
                )
                .unwrap(),
            );
        }
        assert_eq!(ledger.len(), 2);
    }

    #[test]
    fn address_and_range_lookup_detect_containment_and_holes() {
        let mut ledger = RegionLedger::new();
        insert(&mut ledger, anonymous(0x1000, 0x3000));
        insert(&mut ledger, anonymous(0x4000, 0x5000));
        assert!(ledger.lookup_address(0x1800).is_some());
        assert!(matches!(
            ledger.lookup_range(0x1000, 0x2000),
            Ok(RangeLookup::Contained(_))
        ));
        assert_eq!(
            ledger.lookup_range(0x2000, 0x5000),
            Ok(RangeLookup::Hole)
        );
    }

    #[test]
    fn capacity_is_reserved_before_commit() {
        let mut ledger = RegionLedger::new();
        for index in 0..MAX_REGIONS_PER_ADDRESS_SPACE {
            let start = 0x1000 + index as u64 * 0x2000;
            let region = MappingRegion::new(
                start,
                start + 0x1000,
                RegionProtection::READ_ONLY,
                MappingKind::InternalUserMapping,
                RegionPolicy::SYSTEM,
                RegionBacking::Internal(index as u64 + 1),
            )
            .unwrap();
            insert(&mut ledger, region);
        }
        assert_eq!(
            ledger.preflight(anonymous(0x20_0000, 0x20_1000)),
            Err(LedgerError::CapacityExhausted)
        );
        assert_eq!(ledger.len(), MAX_REGIONS_PER_ADDRESS_SPACE);
    }

    #[test]
    fn rollback_removal_is_exact() {
        let mut ledger = RegionLedger::new();
        let region = anonymous(0x1000, 0x2000);
        insert(&mut ledger, region);
        assert_eq!(
            ledger.remove_exact(anonymous(0x1000, 0x3000)),
            Err(LedgerError::ExactRecordNotFound)
        );
        ledger.remove_exact(region).unwrap();
        assert!(ledger.is_empty());
    }
}
