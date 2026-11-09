//! Opt-in M3.1 observations only. No allocation, locks, retries or user bytes.
//! A reservation uniquely owns its slot, including when interrupted by an NMI.
//! Release publication makes the fixed atomic payload visible to the dumper.
//! Missing publication and exhausted storage invalidate the receipt explicitly.
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const CAPACITY: usize = 65536;
struct Slot {
    ready: AtomicBool,
    kind: AtomicU64,
    data: [AtomicU64; 8],
}
static EVENTS: [Slot; CAPACITY] = [const {
    Slot {
        ready: AtomicBool::new(false),
        kind: AtomicU64::new(0),
        data: [const { AtomicU64::new(0) }; 8],
    }
}; CAPACITY];
static WINDOW: AtomicBool = AtomicBool::new(!cfg!(feature = "quota_lifecycle_test"));
static NEXT: AtomicUsize = AtomicUsize::new(0);
static OBJECT: AtomicU64 = AtomicU64::new(1);

pub(crate) const CLAIM: u64 = 1;
pub(crate) const FREE: u64 = 2;
pub(crate) const RECLASS: u64 = 3;
pub(crate) const LIVE: u64 = 4;
pub(crate) const TABLE: u64 = 5;
pub(crate) const ROOT: u64 = 6;
pub(crate) const SHM_CREATE: u64 = 7;
pub(crate) const SHM_ADD: u64 = 8;
pub(crate) const SHM_RELEASE: u64 = 9;
pub(crate) const SHM_RETIRE: u64 = 10;
pub(crate) const SHM_LIVE: u64 = 11;
pub(crate) const VIEW: u64 = 12;
pub(crate) const TLB_REQUEST: u64 = 13;
pub(crate) const TLB_ACK: u64 = 14;
pub(crate) const TLB_COMPLETE: u64 = 15;
pub(crate) const TABLE_LIVE: u64 = 16;
pub(crate) const CHECKPOINT: u64 = 17;
pub(crate) const UNMAP: u64 = 18;
pub(crate) const VIEW_ADD: u64 = 19;
pub(crate) const TABLE_RETIRE: u64 = 20;

pub(crate) fn object_id() -> u64 {
    OBJECT.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn record(kind: u64, data: [u64; 8]) {
    if !WINDOW.load(Ordering::Relaxed) {
        return;
    }
    let index = NEXT.fetch_add(1, Ordering::Relaxed);
    if let Some(slot) = EVENTS.get(index) {
        slot.kind.store(kind, Ordering::Relaxed);
        for (destination, value) in slot.data.iter().zip(data) {
            destination.store(value, Ordering::Relaxed);
        }
        slot.ready.store(true, Ordering::Release);
    }
}

pub(crate) fn physical(kind: u64, frame: u64, class: u8, owner: u32, new_class: u8) {
    if cfg!(feature = "quota_lifecycle_test")
        || matches!(class, 6 | 8 | 15)
        || matches!(new_class, 6 | 8 | 15)
    {
        record(
            kind,
            [
                frame,
                class as u64,
                owner as u64,
                new_class as u64,
                0,
                0,
                0,
                0,
            ],
        );
    }
}

pub(crate) fn checkpoint(checkpoint: u64) {
    // A lifecycle image owns exactly one declared interval. Later independent
    // M2 checkpoints must not dump or relabel that already-closed evidence.
    if cfg!(feature = "quota_lifecycle_test") {
        return;
    }
    record(CHECKPOINT, [checkpoint, 3, 0, 0, 0, 0, 0, 0]);
    let count = NEXT.load(Ordering::Acquire);
    crate::serial_println!(
        "[FL-CENSUS-TRACE] checkpoint={} reserved={} capacity={} overflow={}",
        checkpoint,
        count,
        CAPACITY,
        count.saturating_sub(CAPACITY)
    );
    if checkpoint != 999 {
        return;
    }
    dump(false);
}

fn dump(lifecycle: bool) {
    let count = NEXT.load(Ordering::Acquire);
    let mut incomplete = 0;
    for (index, slot) in EVENTS.iter().take(count).enumerate() {
        if !slot.ready.load(Ordering::Acquire) {
            incomplete += 1;
            continue;
        }
        let d = &slot.data;
        crate::serial_println!(
            "[{}] seq={} kind={} x0={} x1={} x2={} x3={} x4={} x5={} x6={} x7={}",
            if lifecycle { "R51-OWN" } else { "FL-OWN" },
            index,
            slot.kind.load(Ordering::Relaxed),
            d[0].load(Ordering::Relaxed),
            d[1].load(Ordering::Relaxed),
            d[2].load(Ordering::Relaxed),
            d[3].load(Ordering::Relaxed),
            d[4].load(Ordering::Relaxed),
            d[5].load(Ordering::Relaxed),
            d[6].load(Ordering::Relaxed),
            d[7].load(Ordering::Relaxed)
        );
    }
    crate::serial_println!(
        "[{}] reserved={} emitted={} incomplete={} overflow={}",
        if lifecycle {
            "R51-CENSUS-TRACE-END"
        } else {
            "FL-CENSUS-TRACE-END"
        },
        count,
        count.min(CAPACITY) - incomplete,
        incomplete,
        count.saturating_sub(CAPACITY)
    );
}

/// Single bounded observation interval. The caller holds SCHEDULER and PMM.
/// Never reset or overwrite receipts. No allocator or accounting policy changes.
#[cfg(feature = "quota_lifecycle_test")]
pub(crate) fn lifecycle_begin() {
    assert_eq!(
        NEXT.load(Ordering::Acquire),
        0,
        "census window already used"
    );
    WINDOW.store(true, Ordering::Release);
    record(CHECKPOINT, [1, 0, 0, 0, 0, 0, 0, 0]);
}
#[cfg(feature = "quota_lifecycle_test")]
pub(crate) fn lifecycle_end() {
    record(CHECKPOINT, [2, 0, 0, 0, 0, 0, 0, 0]);
    WINDOW.store(false, Ordering::Release);
    dump(true);
}

/// Re-read the actual leaves created in this interval. No shadow mapping state.
/// Caller holds SCHEDULER and PMM, so VM identity/PTEs cannot change mid-walk.
#[cfg(feature = "quota_lifecycle_test")]
pub(crate) unsafe fn lifecycle_views(p: &crate::process::Process, hhdm: x86_64::VirtAddr) {
    let count = NEXT.load(Ordering::Acquire).min(CAPACITY);
    for slot in EVENTS.iter().take(count) {
        if !slot.ready.load(Ordering::Acquire)
            || slot.kind.load(Ordering::Relaxed) != 21
            || slot.data[1].load(Ordering::Relaxed) != p.address_space.identity().generation
        {
            continue;
        }
        let va = slot.data[3].load(Ordering::Relaxed);
        let page = x86_64::structures::paging::Page::<x86_64::structures::paging::Size4KiB>::from_start_address(x86_64::VirtAddr::new(va)).expect("recorded page");
        if let Some((phys, flags)) = p.address_space.lookup_entry(page, hhdm) {
            record(
                22,
                [
                    phys.as_u64(),
                    p.address_space.identity().generation,
                    p.address_space.identity().pml4_phys,
                    va,
                    flags.bits(),
                    p.pid as u64,
                    p.foreground.incarnation,
                    2,
                ],
            );
        }
    }
}
