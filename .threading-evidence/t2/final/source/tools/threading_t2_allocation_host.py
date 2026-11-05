#!/usr/bin/env python3
"""Run real allocation failures with the host allocator returning null.

Includes the production fallible constructors and unmap ledger; fault controls
are per host thread so the test harness can still allocate to report failures.
"""
from pathlib import Path
import subprocess
import tempfile
ROOT = Path(__file__).resolve().parents[1]
source = r'''
#![allow(dead_code)]
extern crate alloc;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
std::thread_local! { static DENY: Cell<bool> = const { Cell::new(false) }; }
struct Allocator;
#[global_allocator] static ALLOCATOR: Allocator = Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if DENY.try_with(|v| v.get()).unwrap_or(false) { core::ptr::null_mut() }
        else { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if DENY.try_with(|v| v.get()).unwrap_or(false) { core::ptr::null_mut() }
        else { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if DENY.try_with(|v| v.get()).unwrap_or(false) { core::ptr::null_mut() }
        else { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) { System.dealloc(ptr, layout) }
}
fn denied<T>(operation: impl FnOnce() -> T) -> T {
    struct Guard;
    impl Drop for Guard { fn drop(&mut self) { DENY.with(|v| v.set(false)); } }
    DENY.with(|v| v.set(true));
    let _guard = Guard;
    operation()
}
// MODULES
use region::*;
fn anonymous(start: u64, end: u64) -> MappingRegion {
    MappingRegion::new(start, end, RegionProtection::READ_WRITE,
        MappingKind::Anonymous, RegionPolicy::MAY_UNMAP, RegionBacking::AnonymousOwner(1)).unwrap()
}
fn insert(ledger: &mut RegionLedger, region: MappingRegion) {
    let token = ledger.preflight(region).unwrap(); ledger.commit(token).unwrap();
}
#[test]
fn real_allocator_failure_returns_errors_for_owned_thread_resources() {
    assert!(matches!(denied(thread_admission::try_box_uninit::<[u8; 32768]>), Err(thread_admission::Error::NoMemory)));
    assert!(matches!(denied(|| thread_admission::try_string("owned")), Err(thread_admission::Error::NoMemory)));
    assert!(matches!(denied(|| thread_admission::try_copy(&[1u64, 2])), Err(thread_admission::Error::NoMemory)));
    let mut parent = env::EnvMap::new(); parent.set("A", "value");
    assert!(matches!(denied(|| env::EnvMap::try_inherit(&parent)), Err(thread_admission::Error::NoMemory)));
    assert_eq!(parent.get("A"), Some("value"));
}
#[test]
fn cold_task_and_queue_reservations_return_real_allocator_errors() {
    let mut tasks: Vec<[u8; 3152]> = Vec::new();
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    assert!(denied(|| tasks.try_reserve_exact(512)).is_err());
    assert!(denied(|| queue.try_reserve_exact(512)).is_err());
    assert_eq!((tasks.len(), tasks.capacity(), queue.len(), queue.capacity()), (0,0,0,0));
}
#[test]
fn unmap_all_shapes_without_any_heap_allocation() {
    for (start, end, remaining) in [(0x2000,0x6000,0), (0x2000,0x3000,1),
                                    (0x5000,0x6000,1), (0x3000,0x4000,2)] {
        let mut ledger = RegionLedger::new(); insert(&mut ledger, anonymous(0x2000,0x6000));
        let plan = denied(|| ledger.preflight_unmap(start, end)).unwrap();
        denied(|| ledger.commit_unmap(plan));
        assert_eq!(ledger.len(), remaining);
        assert_eq!(ledger.validate(), Ok(()));
    }
}
#[test]
fn full_ledger_middle_split_is_still_an_explicit_capacity_boundary() {
    let mut ledger = RegionLedger::new(); insert(&mut ledger, anonymous(0x1000,0x9000));
    for n in 1..MAX_REGIONS_PER_ADDRESS_SPACE {
        let start = 0x10000 + n as u64 * 0x2000;
        insert(&mut ledger, anonymous(start,start+0x1000));
    }
    let result = denied(|| ledger.preflight_unmap(0x3000,0x4000));
    assert!(matches!(result, Err(LedgerError::CapacityExhausted)));
    assert_eq!(ledger.len(), MAX_REGIONS_PER_ADDRESS_SPACE);
    let plan = denied(|| ledger.preflight_unmap(0x1000,0x9000)).unwrap();
    denied(|| ledger.commit_unmap(plan));
    assert_eq!(ledger.len(), MAX_REGIONS_PER_ADDRESS_SPACE-1);
}

#[test]
fn thread_resource_release_at_full_ledger_needs_neither_allocation_nor_capacity() {
    let mut ledger = RegionLedger::new();
    let resource = |start, end| {
        let mut region = anonymous(start, end);
        region.policy = region.policy.union(RegionPolicy::NO_MERGE);
        region
    };
    // Adjacent stacks and TLS must each retain an independent release boundary.
    insert(&mut ledger, resource(0x1000,0x5000));
    insert(&mut ledger, resource(0x5000,0x6000));
    insert(&mut ledger, resource(0x6000,0xa000));
    assert_eq!(ledger.len(), 3);
    for n in 3..MAX_REGIONS_PER_ADDRESS_SPACE {
        let start=0x10000+n as u64*0x2000;
        insert(&mut ledger, anonymous(start,start+0x1000));
    }
    for (start, end) in [(0x5000,0x6000),(0x1000,0x5000),(0x6000,0xa000)] {
        let plan=denied(|| ledger.preflight_unmap(start,end)).unwrap();
        assert_eq!(plan.effects().middle_splits,0);
        denied(|| ledger.commit_unmap(plan));
    }
    assert_eq!(ledger.len(),MAX_REGIONS_PER_ADDRESS_SPACE-3);
    assert_eq!(ledger.validate(),Ok(()));
}

#[test]
fn allocation_free_unmap_preserves_all_unaffected_page_coverage() {
    for pattern in [0xffff_ffffu32, 0x5555_5555, 0x0ff0_f0f0] {
        for first in 0..32 {
            for last in first+1..=32 {
                let mut ledger=RegionLedger::new();
                for page in 0..32 {
                    if pattern & (1 << page) != 0 {
                        let start=0x1000+page as u64*0x1000;
                        let mut region=anonymous(start,start+0x1000);
                        if page % 3 == 0 { region.policy=region.policy.union(RegionPolicy::NO_MERGE); }
                        insert(&mut ledger,region);
                    }
                }
                let plan=denied(|| ledger.preflight_unmap(0x1000+first*0x1000,0x1000+last*0x1000)).unwrap();
                denied(|| ledger.commit_unmap(plan));
                for page in 0..32 {
                    let expected=pattern & (1 << page) != 0 && !(first..last).contains(&(page as u64));
                    assert_eq!(ledger.lookup_address(0x1000+page as u64*0x1000).is_some(),expected);
                }
                assert_eq!(ledger.validate(),Ok(()));
            }
        }
    }
}
'''
modules = '\n'.join(f'#[path = "{ROOT / "kernel/src/process" / (name + ".rs")}"] mod {name};'
                    for name in ('thread_admission','env','region'))
with tempfile.TemporaryDirectory(prefix='t2-allocation-host-') as temporary:
    folder=Path(temporary)
    (folder/'test.rs').write_text(source.replace('// MODULES',modules))
    subprocess.run(['rustc','--edition=2021','--test',str(folder/'test.rs'),'-o',str(folder/'test')],check=True)
    subprocess.run([str(folder/'test'),'--test-threads=1'],check=True,timeout=60)
