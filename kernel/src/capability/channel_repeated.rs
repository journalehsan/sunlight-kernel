//! Bounded private admission state for M5.1. Shared cursors never grant reuse.
//! The existing scheduler/broker transaction serializes every transition.
use super::channel::Error;
const CAPACITY: u64 = 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Available,
    Reserved,
    Published,
    Acquired,
    Copied,
}
#[derive(Clone, Copy)]
pub(super) struct Repeated {
    pub enabled: bool,
    head: u64,
    tail: u64,
    slots: [Slot; 16],
}
impl Repeated {
    pub const fn new() -> Self {
        Self {
            enabled: false,
            head: 0,
            tail: 0,
            slots: [Slot::Available; 16],
        }
    }
    pub fn cursor(&self, region: usize) -> u64 {
        if region == 0 {
            self.head
        } else {
            self.tail
        }
    }
    // Fixed control selectors: enable, reserve, acquire, copied, cancel reservation.
    pub fn control(&mut self, role: usize, op: u64, sequence: u64) -> Result<(), Error> {
        if op == 1536 {
            if role != 0 || sequence != 0 {
                return Err(Error::Denied);
            }
            if self.enabled {
                return Err(Error::Duplicate);
            }
            self.enabled = true;
            return Ok(());
        }
        if !self.enabled {
            return Err(Error::Incomplete);
        }
        let producer = matches!(op, 1537 | 1540);
        if role != if producer { 0 } else { 1 } {
            return Err(Error::Denied);
        }
        let expected = if producer { self.head } else { self.tail };
        if sequence != expected {
            return Err(Error::Stale);
        }
        sequence.checked_add(1).ok_or(Error::Exhausted)?;
        let slot = &mut self.slots[(sequence % CAPACITY) as usize];
        match op {
            1537 => {
                if self.head - self.tail == CAPACITY {
                    return Err(Error::Capacity);
                }
                if *slot != Slot::Available {
                    return Err(Error::Duplicate);
                }
                *slot = Slot::Reserved;
            }
            1538 => {
                if *slot != Slot::Published {
                    return Err(Error::Incomplete);
                }
                *slot = Slot::Acquired;
            }
            1539 => {
                if *slot != Slot::Acquired {
                    return Err(Error::Incomplete);
                }
                *slot = Slot::Copied;
            }
            1540 => {
                if *slot != Slot::Reserved {
                    return Err(Error::Incomplete);
                }
                *slot = Slot::Available;
            }
            _ => return Err(Error::Denied),
        }
        Ok(())
    }
    // Producer aliases remain mutable after publication (M3.2). Byte writes
    // never reserve capacity or advance a cursor; only control/advance can.
    pub fn writable(&self, slot: usize) -> bool {
        self.slots[slot] != Slot::Available
    }
    pub fn readable(&self, slot: usize) -> bool {
        self.slots[slot] == Slot::Acquired
    }
    pub fn advance(&mut self, region: usize, next: u64) -> Result<(), Error> {
        let cursor = self.cursor(region);
        if next != cursor.checked_add(1).ok_or(Error::Exhausted)? {
            return Err(Error::Stale);
        }
        let slot = &mut self.slots[(cursor % CAPACITY) as usize];
        if *slot
            != if region == 0 {
                Slot::Reserved
            } else {
                Slot::Copied
            }
        {
            return Err(Error::Duplicate);
        }
        *slot = if region == 0 {
            Slot::Published
        } else {
            Slot::Available
        };
        if region == 0 {
            self.head = next;
        } else {
            self.tail = next;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_reuse_and_stale_ack() {
        let mut s = Repeated::new();
        s.control(0, 1536, 0).unwrap();
        for base in 0..625u64 {
            for i in 0..16 {
                let n = base * 16 + i;
                s.control(0, 1537, n).unwrap();
                s.advance(0, n + 1).unwrap();
            }
            assert_eq!(s.control(0, 1537, (base + 1) * 16), Err(Error::Capacity));
            for i in 0..16 {
                let n = base * 16 + i;
                assert_eq!(s.advance(1, n + 1), Err(Error::Duplicate));
                s.control(1, 1538, n).unwrap();
                assert_eq!(s.advance(1, n + 1), Err(Error::Duplicate));
                s.control(1, 1539, n).unwrap();
                s.advance(1, n + 1).unwrap();
                assert_eq!(s.advance(1, n + 1), Err(Error::Stale));
            }
        }
        assert_eq!(s.head, 10000);
        assert_eq!(s.tail, 10000);
    }
    #[test]
    fn cancellation_identity_and_exhaustion() {
        let mut s = Repeated::new();
        assert_eq!(s.control(1, 1536, 0), Err(Error::Denied));
        s.control(0, 1536, 0).unwrap();
        s.control(0, 1537, 0).unwrap();
        assert_eq!(s.control(0, 1537, 0), Err(Error::Duplicate));
        s.control(0, 1540, 0).unwrap();
        assert!(!s.writable(0));
        s.control(0, 1537, 0).unwrap();
        s.advance(0, 1).unwrap();
        assert_eq!(s.control(1, 1538, 1), Err(Error::Stale));
        s.head = u64::MAX;
        s.tail = u64::MAX;
        assert_eq!(s.control(0, 1537, u64::MAX), Err(Error::Exhausted));
    }
}
