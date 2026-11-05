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
    if matches!(class, 6 | 8 | 15) || matches!(new_class, 6 | 8 | 15) {
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
    let mut incomplete = 0;
    for (index, slot) in EVENTS.iter().take(count).enumerate() {
        if !slot.ready.load(Ordering::Acquire) {
            incomplete += 1;
            continue;
        }
        let d = &slot.data;
        crate::serial_println!(
            "[FL-OWN] seq={} kind={} x0={} x1={} x2={} x3={} x4={} x5={} x6={} x7={}",
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
        "[FL-CENSUS-TRACE-END] reserved={} emitted={} incomplete={} overflow={}",
        count,
        count.min(CAPACITY) - incomplete,
        incomplete,
        count.saturating_sub(CAPACITY)
    );
}
