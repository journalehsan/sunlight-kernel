//! Primary index: MemoryId -> location + latest visible revision.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use wiseowl_memory::MemoryId;

use crate::record::{LongTermMemoryRecord, LongTermRecordState};

/// On-disk / logical location of a record revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordLocation {
    pub segment_id: u64,
    /// Index within the segment's record list (or 0 if still only in WAL staging).
    pub record_index: u32,
    pub revision: u32,
}

/// One primary index entry (latest + history pointers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimaryEntry {
    pub id: MemoryId,
    pub latest_revision: u32,
    pub state: LongTermRecordState,
    pub location: RecordLocation,
    /// Older revisions (bounded; oldest dropped if exceeds 16).
    pub history: Vec<RecordLocation>,
}

const MAX_HISTORY: usize = 16;

#[derive(Debug, Default)]
pub struct PrimaryIndex {
    map: BTreeMap<u64, PrimaryEntry>,
}

impl PrimaryIndex {
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, id: MemoryId) -> Option<&PrimaryEntry> {
        self.map.get(&id.get())
    }

    pub fn contains(&self, id: MemoryId) -> bool {
        self.map.contains_key(&id.get())
    }

    pub fn upsert(&mut self, rec: &LongTermMemoryRecord, loc: RecordLocation) {
        let key = rec.id.get();
        match self.map.get_mut(&key) {
            None => {
                self.map.insert(
                    key,
                    PrimaryEntry {
                        id: rec.id,
                        latest_revision: rec.revision,
                        state: rec.state,
                        location: loc,
                        history: Vec::new(),
                    },
                );
            }
            Some(e) => {
                if rec.revision >= e.latest_revision {
                    e.history.push(e.location);
                    if e.history.len() > MAX_HISTORY {
                        e.history.remove(0);
                    }
                    e.latest_revision = rec.revision;
                    e.state = rec.state;
                    e.location = loc;
                } else {
                    e.history.push(loc);
                    if e.history.len() > MAX_HISTORY {
                        e.history.remove(0);
                    }
                }
            }
        }
    }

    pub fn set_state(&mut self, id: MemoryId, state: LongTermRecordState) {
        if let Some(e) = self.map.get_mut(&id.get()) {
            e.state = state;
        }
    }

    pub fn relocate_segment(&mut self, old_seg: u64, new_seg: u64, index_map: &BTreeMap<u32, u32>) {
        for e in self.map.values_mut() {
            if e.location.segment_id == old_seg {
                if let Some(&ni) = index_map.get(&e.location.record_index) {
                    e.location.segment_id = new_seg;
                    e.location.record_index = ni;
                }
            }
            for h in &mut e.history {
                if h.segment_id == old_seg {
                    if let Some(&ni) = index_map.get(&h.record_index) {
                        h.segment_id = new_seg;
                        h.record_index = ni;
                    }
                }
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &PrimaryEntry> {
        self.map.values()
    }

    pub fn ids(&self) -> Vec<MemoryId> {
        self.map.values().map(|e| e.id).collect()
    }
}
