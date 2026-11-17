//! Bounded deterministic breadth-first search on a uniform-cost grid.
//! Scratch is retained across searches; no per-expanded-cell allocation.
use crate::{Tile, CELLS, MAP_SIZE};
use alloc::{vec, vec::Vec};
#[derive(Clone, Debug)]
pub struct Search {
    parent: Vec<u16>,
    queue: Vec<u16>,
}
// Scratch is not authoritative world state.
impl PartialEq for Search {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Search {}
impl Default for Search {
    fn default() -> Self {
        Self::new()
    }
}
impl Search {
    pub fn new() -> Self {
        Self {
            parent: vec![u16::MAX; CELLS],
            queue: Vec::with_capacity(CELLS),
        }
    }
    /// Reversed route: last element is the next tile, first is destination.
    pub fn find(&mut self, start: Tile, end: Tile, blocked: &[bool]) -> Option<Vec<Tile>> {
        let a = start.index()?;
        let b = end.index()?;
        if blocked.len() != CELLS || blocked[b] && a != b {
            return None;
        }
        self.parent.fill(u16::MAX);
        self.queue.clear();
        self.queue.push(a as u16);
        self.parent[a] = a as u16;
        let mut head = 0;
        while head < self.queue.len() {
            let n = self.queue[head] as usize;
            head += 1;
            if n == b {
                let mut path = Vec::new();
                let mut p = b;
                while p != a {
                    path.push(Tile::new(p as i32 % MAP_SIZE, p as i32 / MAP_SIZE));
                    p = self.parent[p] as usize;
                }
                return Some(path);
            }
            let t = Tile::new(n as i32 % MAP_SIZE, n as i32 / MAP_SIZE);
            for next in t.neighbors() {
                if let Some(k) = next.index() {
                    if !blocked[k] && self.parent[k] == u16::MAX {
                        self.parent[k] = n as u16;
                        self.queue.push(k as u16);
                    }
                }
            }
        }
        None
    }
}
