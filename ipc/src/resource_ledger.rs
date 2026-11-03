//! Bounded, attributable resource accounting for a measurement channel
//! lifecycle (FastLane M5.4).
//!
//! The ledger deliberately keeps *separate* dimensions rather than a single
//! "memory used" number, because they mean different things:
//!
//! * **event counts** — how many allocation/free operations happened.
//! * **live object counts** — how many channel/role/receipt objects exist now.
//! * **physical frames** — backing frames allocated and retired.
//! * **virtual mappings** — page-table entries established and torn down.
//! * **page-table pages** — page-table allocations and reclamations.
//! * **retained vs reclaimed bytes** — allocator bookkeeping only.
//!
//! A ledger that reaches a balanced final state proves the *bookkeeping*
//! balanced. It does **not** prove physical memory was reclaimed: only the
//! remote-TLB acknowledgement and exactly-once retirement paths can show that.
//! [`Snapshot::bookkeeping_balanced`] returns that weaker claim and the name
//! says so.
//!
//! All arithmetic is checked; the first overflow latches [`Ledger::overflow`]
//! and every later derived value is reported as unreliable rather than wrapped.

/// Six independent dimensions plus peak and overflow state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ledger {
    pub object_alloc_events: u64,
    pub object_retire_events: u64,
    pub frame_alloc_events: u64,
    pub frame_retire_events: u64,
    pub map_events: u64,
    pub unmap_events: u64,
    pub page_table_alloc_events: u64,
    pub page_table_retire_events: u64,
    pub heap_alloc_events: u64,
    pub heap_free_events: u64,
    pub heap_bytes_retained: u64,
    pub heap_bytes_reclaimed: u64,
    pub live_objects: u64,
    pub live_frames: u64,
    pub live_mappings: u64,
    pub live_page_tables: u64,
    pub peak_live_objects: u64,
    pub peak_live_frames: u64,
    pub overflow: bool,
}

/// A derived view. `outstanding_*` are live counts, not event counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub object_alloc_events: u64,
    pub object_retire_events: u64,
    pub frame_alloc_events: u64,
    pub frame_retire_events: u64,
    pub map_events: u64,
    pub unmap_events: u64,
    pub page_table_alloc_events: u64,
    pub page_table_retire_events: u64,
    pub outstanding_objects: u64,
    pub outstanding_frames: u64,
    pub outstanding_mappings: u64,
    pub outstanding_page_tables: u64,
    pub peak_objects: u64,
    pub peak_frames: u64,
    pub heap_bytes_retained: u64,
    pub heap_bytes_reclaimed: u64,
    pub overflow: bool,
}

impl Ledger {
    pub const EMPTY: Self = Self {
        object_alloc_events: 0,
        object_retire_events: 0,
        frame_alloc_events: 0,
        frame_retire_events: 0,
        map_events: 0,
        unmap_events: 0,
        page_table_alloc_events: 0,
        page_table_retire_events: 0,
        heap_alloc_events: 0,
        heap_free_events: 0,
        heap_bytes_retained: 0,
        heap_bytes_reclaimed: 0,
        live_objects: 0,
        live_frames: 0,
        live_mappings: 0,
        live_page_tables: 0,
        peak_live_objects: 0,
        peak_live_frames: 0,
        overflow: false,
    };

    fn bump(counter: &mut u64, amount: u64, overflow: &mut bool) {
        match counter.checked_add(amount) {
            Some(v) => *counter = v,
            None => *overflow = true,
        }
    }

    fn live_up(counter: &mut u64, peak: &mut u64, amount: u64, overflow: &mut bool) {
        match counter.checked_add(amount) {
            Some(v) => {
                *counter = v;
                if v > *peak {
                    *peak = v;
                }
            }
            None => *overflow = true,
        }
    }

    fn live_down(counter: &mut u64, amount: u64, overflow: &mut bool) {
        match counter.checked_sub(amount) {
            Some(v) => *counter = v,
            None => *overflow = true, // retiring more than was allocated
        }
    }

    fn live_up_plain(counter: &mut u64, amount: u64, overflow: &mut bool) {
        Self::bump(counter, amount, overflow);
    }

    pub fn alloc_object(&mut self) {
        Self::bump(&mut self.object_alloc_events, 1, &mut self.overflow);
        Self::live_up(
            &mut self.live_objects,
            &mut self.peak_live_objects,
            1,
            &mut self.overflow,
        );
    }

    pub fn retire_object(&mut self) {
        Self::bump(&mut self.object_retire_events, 1, &mut self.overflow);
        Self::live_down(&mut self.live_objects, 1, &mut self.overflow);
    }

    pub fn alloc_frame(&mut self) {
        Self::bump(&mut self.frame_alloc_events, 1, &mut self.overflow);
        Self::live_up(
            &mut self.live_frames,
            &mut self.peak_live_frames,
            1,
            &mut self.overflow,
        );
    }

    pub fn retire_frame(&mut self) {
        Self::bump(&mut self.frame_retire_events, 1, &mut self.overflow);
        Self::live_down(&mut self.live_frames, 1, &mut self.overflow);
    }

    pub fn map(&mut self) {
        Self::bump(&mut self.map_events, 1, &mut self.overflow);
        Self::live_up_plain(&mut self.live_mappings, 1, &mut self.overflow);
    }

    pub fn unmap(&mut self) {
        Self::bump(&mut self.unmap_events, 1, &mut self.overflow);
        Self::live_down(&mut self.live_mappings, 1, &mut self.overflow);
    }

    pub fn alloc_page_table(&mut self) {
        Self::bump(&mut self.page_table_alloc_events, 1, &mut self.overflow);
        Self::live_up_plain(&mut self.live_page_tables, 1, &mut self.overflow);
    }

    pub fn retire_page_table(&mut self) {
        Self::bump(&mut self.page_table_retire_events, 1, &mut self.overflow);
        Self::live_down(&mut self.live_page_tables, 1, &mut self.overflow);
    }

    /// Allocator bookkeeping only: `bytes` retained by the allocator for the
    /// scope, not physical frames and not proof of residency.
    pub fn heap_alloc(&mut self, bytes: u64) {
        Self::bump(&mut self.heap_alloc_events, 1, &mut self.overflow);
        Self::bump(&mut self.heap_bytes_retained, bytes, &mut self.overflow);
    }

    /// Allocator bookkeeping only: the allocator reports `bytes` returned to
    /// its free pools. This is **not** physical reclamation.
    pub fn heap_free(&mut self, bytes: u64) {
        Self::bump(&mut self.heap_free_events, 1, &mut self.overflow);
        Self::live_down(&mut self.heap_bytes_retained, bytes, &mut self.overflow);
        Self::bump(&mut self.heap_bytes_reclaimed, bytes, &mut self.overflow);
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            object_alloc_events: self.object_alloc_events,
            object_retire_events: self.object_retire_events,
            frame_alloc_events: self.frame_alloc_events,
            frame_retire_events: self.frame_retire_events,
            map_events: self.map_events,
            unmap_events: self.unmap_events,
            page_table_alloc_events: self.page_table_alloc_events,
            page_table_retire_events: self.page_table_retire_events,
            outstanding_objects: self.live_objects,
            outstanding_frames: self.live_frames,
            outstanding_mappings: self.live_mappings,
            outstanding_page_tables: self.live_page_tables,
            peak_objects: self.peak_live_objects,
            peak_frames: self.peak_live_frames,
            heap_bytes_retained: self.heap_bytes_retained,
            heap_bytes_reclaimed: self.heap_bytes_reclaimed,
            overflow: self.overflow,
        }
    }
}

impl Snapshot {
    /// Weak claim: every scoped object/frame/mapping/page-table was retired and
    /// no counter overflowed. It says nothing about physical reclamation.
    pub fn bookkeeping_balanced(&self) -> bool {
        !self.overflow
            && self.outstanding_objects == 0
            && self.outstanding_frames == 0
            && self.outstanding_mappings == 0
            && self.outstanding_page_tables == 0
            && self.object_alloc_events == self.object_retire_events
            && self.frame_alloc_events == self.frame_retire_events
            && self.map_events == self.unmap_events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_lifecycle_reports_balanced() {
        let mut l = Ledger::EMPTY;
        l.alloc_object();
        l.alloc_object();
        l.alloc_frame();
        l.alloc_page_table();
        l.map();
        l.map();
        l.heap_alloc(4096);
        assert_eq!(l.snapshot().peak_objects, 2);
        l.retire_object();
        l.retire_object();
        l.retire_frame();
        l.retire_page_table();
        l.unmap();
        l.unmap();
        l.heap_free(4096);
        let s = l.snapshot();
        assert!(s.bookkeeping_balanced());
        assert_eq!(s.heap_bytes_retained, 0);
        assert_eq!(s.heap_bytes_reclaimed, 4096);
    }

    #[test]
    fn outstanding_resources_are_detected() {
        let mut l = Ledger::EMPTY;
        l.alloc_object();
        l.alloc_frame();
        let s = l.snapshot();
        assert_eq!(s.outstanding_objects, 1);
        assert_eq!(s.outstanding_frames, 1);
        assert!(!s.bookkeeping_balanced());
    }

    #[test]
    fn over_retirement_latches_overflow_instead_of_wrapping() {
        let mut l = Ledger::EMPTY;
        l.retire_object();
        assert!(l.overflow);
        assert!(!l.snapshot().bookkeeping_balanced());
    }

    #[test]
    fn heap_reclaimed_is_not_a_physical_claim() {
        let mut l = Ledger::EMPTY;
        l.heap_alloc(8192);
        l.heap_free(8192);
        let s = l.snapshot();
        // Balanced bookkeeping, but the type offers no physical-reclamation bit.
        assert!(s.bookkeeping_balanced());
        assert_eq!(s.heap_bytes_reclaimed, 8192);
    }
}
