extern crate alloc;

#[allow(dead_code)]
#[path = "../../kernel/src/process/region.rs"]
mod region;

use region::{LedgerError, MappingKind, MappingRegion, RegionBacking, RegionLedger, RegionPolicy, RegionProtection};

#[test]
fn sparse_main_stack_keeps_both_guards_reserved() {
    const TOP: u64 = 0x0000_7fff_ffff_f000;
    const FLOOR: u64 = TOP - 2 * 1024 * 1024;
    const PAGE: u64 = 4096;
    let mut ledger = RegionLedger::new();
    let policy = RegionPolicy::SYSTEM.union(RegionPolicy::OWNER_MANAGED);
    let stack = MappingRegion::new(FLOOR, TOP, RegionProtection::READ_WRITE,
        MappingKind::UserStack, policy, RegionBacking::MainStack { owner: 42, generation: 7 }).unwrap();
    let guards = [FLOOR - PAGE, TOP].map(|start| MappingRegion::new(start, start + PAGE,
        RegionProtection::NONE, MappingKind::UserStackGuard, policy, RegionBacking::None).unwrap());
    let lower = ledger.preflight(guards[0]).unwrap();
    let upper = ledger.preflight(guards[1]).unwrap();
    let reservation = ledger.preflight(stack).unwrap();
    ledger.commit(reservation).unwrap();
    ledger.commit(lower).unwrap();
    ledger.commit(upper).unwrap();
    for guard in guards {
        assert_eq!(ledger.lookup_address(guard.start), Some(guard));
        assert_eq!(ledger.preflight(guard), Err(LedgerError::Overlap));
        assert!(ledger.preflight_unmap(guard.start, guard.end).is_err());
    }
    assert_eq!(ledger.lookup_address(FLOOR), Some(stack));
    assert_eq!(ledger.lookup_address(TOP - 1), Some(stack));
    assert!(ledger.preflight_unmap(FLOOR, TOP).is_err());
}
