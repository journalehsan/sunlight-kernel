//! Relationship indexes: outgoing and incoming edges.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use wiseowl_memory::MemoryId;

use crate::relationship::{MemoryRelationship, RelDirection, RelationshipKind, RelationshipQuery};

#[derive(Debug, Default)]
pub struct RelationshipIndex {
    outgoing: BTreeMap<u64, Vec<MemoryRelationship>>,
    incoming: BTreeMap<u64, Vec<MemoryRelationship>>,
    count: u64,
}

impl RelationshipIndex {
    pub fn len(&self) -> u64 {
        self.count
    }

    pub fn insert(&mut self, rel: MemoryRelationship) {
        let s = rel.source.get();
        let t = rel.target.get();
        self.outgoing.entry(s).or_default().push(rel.clone());
        self.incoming.entry(t).or_default().push(rel);
        self.count = self.count.saturating_add(1);
    }

    pub fn outgoing(&self, id: MemoryId) -> &[MemoryRelationship] {
        self.outgoing
            .get(&id.get())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn incoming(&self, id: MemoryId) -> &[MemoryRelationship] {
        self.incoming
            .get(&id.get())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Bounded relationship query (no unrestricted graph walk).
    pub fn query(&self, q: &RelationshipQuery) -> Vec<MemoryRelationship> {
        let mut out = Vec::new();
        let max_edges = q.max_edges as usize;
        let max_depth = q.max_depth.max(1);

        // Depth-1 only unless max_depth > 1, then BFS with work budget.
        let mut frontier = vec![q.of.get()];
        let mut visited = alloc::collections::BTreeSet::new();
        visited.insert(q.of.get());
        let mut depth = 0u32;

        while depth < max_depth && !frontier.is_empty() && out.len() < max_edges {
            let mut next = Vec::new();
            for id in frontier {
                let edges: Vec<&MemoryRelationship> = match q.direction {
                    RelDirection::Outgoing => self
                        .outgoing(MemoryId::from_raw_unchecked(id))
                        .iter()
                        .collect(),
                    RelDirection::Incoming => self
                        .incoming(MemoryId::from_raw_unchecked(id))
                        .iter()
                        .collect(),
                    RelDirection::Both => {
                        let mut v: Vec<&MemoryRelationship> = self
                            .outgoing(MemoryId::from_raw_unchecked(id))
                            .iter()
                            .collect();
                        v.extend(self.incoming(MemoryId::from_raw_unchecked(id)).iter());
                        v
                    }
                };
                for e in edges {
                    if e.tombstoned {
                        continue;
                    }
                    if let Some(k) = q.kind {
                        if e.kind != k {
                            continue;
                        }
                    }
                    if out.len() >= max_edges {
                        break;
                    }
                    out.push(e.clone());
                    let other = if e.source.get() == id {
                        e.target.get()
                    } else {
                        e.source.get()
                    };
                    if visited.insert(other) {
                        next.push(other);
                    }
                }
            }
            frontier = next;
            depth += 1;
        }
        out
    }

    /// Detect simple Supersedes loop: following Supersedes from `start` returns to start within budget.
    pub fn supersedes_loop(&self, start: MemoryId, budget: u32) -> bool {
        let mut cur = start.get();
        let mut seen = alloc::collections::BTreeSet::new();
        for _ in 0..budget {
            if !seen.insert(cur) {
                return true;
            }
            let outs = self.outgoing(MemoryId::from_raw_unchecked(cur));
            let mut next = None;
            for e in outs {
                if e.kind == RelationshipKind::Supersedes && !e.tombstoned {
                    next = Some(e.target.get());
                    break;
                }
            }
            match next {
                Some(n) => cur = n,
                None => return false,
            }
        }
        false
    }
}
