//! Anonymous page tracking for the ZRAM swap subsystem (Phase 6.6 Step 2).
//!
//! Tracks which physical frames back anonymous user mappings so a future
//! reclaim pass (Step 3+) has a candidate list to evict via `zram`.

use alloc::vec::Vec;
use spin::Mutex;
use x86_64::{PhysAddr, VirtAddr};

/// A tracked anonymous user frame: who owns it and where it's mapped.
#[derive(Debug, Clone, Copy)]
pub struct AnonFrame {
    pub pid: usize,
    pub vaddr: VirtAddr,
    pub frame: PhysAddr,
}

static CANDIDATES: Mutex<Vec<AnonFrame>> = Mutex::new(Vec::new());

/// Register a freshly mapped anonymous user page as a reclaim candidate.
pub fn track_anon(pid: usize, vaddr: VirtAddr, frame: PhysAddr) {
    CANDIDATES.lock().push(AnonFrame { pid, vaddr, frame });
}

/// Remove tracking for a frame (on unmap or process exit).
pub fn untrack(frame: PhysAddr) {
    CANDIDATES.lock().retain(|c| c.frame != frame);
}

/// Remove all tracked frames belonging to `pid` (on process exit).
pub fn untrack_process(pid: usize) {
    CANDIDATES.lock().retain(|c| c.pid != pid);
}

/// Number of frames currently registered as reclaim candidates.
pub fn candidate_count() -> usize {
    CANDIDATES.lock().len()
}
