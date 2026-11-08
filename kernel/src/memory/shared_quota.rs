//! Aggregate accounting for legacy bearer SHM.
//!
//! This is an allocation budget, not an isolation boundary. Legacy
//! `shm_alloc`/`shm_create` tokens remain bearer capabilities; restricted
//! channel mappings carry the two-endpoint isolation contract.

use spin::Mutex;

pub const LEGACY_SHM_QUOTA_BYTES: usize = 16 * 1024 * 1024;
const MAX_IDENTITIES: usize = 256;

#[derive(Clone, Copy)]
struct Entry {
    identity: u64,
    bytes: usize,
}

impl Entry {
    const EMPTY: Self = Self {
        identity: 0,
        bytes: 0,
    };
}

static LEDGER: Mutex<[Entry; MAX_IDENTITIES]> = Mutex::new([Entry::EMPTY; MAX_IDENTITIES]);

pub struct Reservation {
    identity: u64,
    bytes: usize,
    active: bool,
}

pub fn reserve(identity: u64, bytes: usize) -> Option<Reservation> {
    if identity == 0 || bytes == 0 || bytes > LEGACY_SHM_QUOTA_BYTES {
        return None;
    }
    let mut ledger = LEDGER.lock();
    let slot = ledger
        .iter_mut()
        .find(|entry| entry.identity == identity || entry.identity == 0)?;
    if slot.identity == 0 {
        slot.identity = identity;
    }
    let next = slot.bytes.checked_add(bytes)?;
    if next > LEGACY_SHM_QUOTA_BYTES {
        return None;
    }
    slot.bytes = next;
    Some(Reservation {
        identity,
        bytes,
        active: true,
    })
}

impl Reservation {
    /// Keep the charge until the backing object is actually reclaimed.
    pub fn commit(mut self) {
        self.active = false;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.active {
            release(self.identity, self.bytes);
        }
    }
}

pub fn release(identity: u64, bytes: usize) {
    if identity == 0 || bytes == 0 {
        return;
    }
    let mut ledger = LEDGER.lock();
    if let Some(entry) = ledger.iter_mut().find(|entry| entry.identity == identity) {
        assert!(entry.bytes >= bytes, "legacy SHM quota underflow");
        entry.bytes -= bytes;
        if entry.bytes == 0 {
            *entry = Entry::EMPTY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::vec::Vec;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn clear() {
        *LEDGER.lock() = [Entry::EMPTY; MAX_IDENTITIES];
    }

    #[test]
    fn cumulative_allocation_is_bounded() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear();
        let a = reserve(1, LEGACY_SHM_QUOTA_BYTES / 2).unwrap();
        let b = reserve(1, LEGACY_SHM_QUOTA_BYTES / 2).unwrap();
        assert!(reserve(1, 4096).is_none());
        drop(a);
        drop(b);
        assert!(reserve(1, LEGACY_SHM_QUOTA_BYTES).is_some());
    }

    #[test]
    fn concurrent_reservation_is_atomic() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear();
        let wins = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut threads = Vec::new();
        for _ in 0..32 {
            let wins = Arc::clone(&wins);
            threads.push(thread::spawn(move || {
                for _ in 0..(LEGACY_SHM_QUOTA_BYTES / PAGE / 32) {
                    if let Some(r) = reserve(2, PAGE) {
                        wins.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        r.commit();
                    }
                }
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(
            wins.load(std::sync::atomic::Ordering::Relaxed),
            LEGACY_SHM_QUOTA_BYTES / PAGE
        );
        release(2, LEGACY_SHM_QUOTA_BYTES);
    }

    const PAGE: usize = 4096;

    #[test]
    fn failed_reservation_rolls_back() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear();
        {
            let _r = reserve(3, PAGE).unwrap();
        }
        assert!(reserve(3, LEGACY_SHM_QUOTA_BYTES).is_some());
    }

    #[test]
    fn shared_references_release_once() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear();
        let r = reserve(4, PAGE * 2).unwrap();
        r.commit();
        release(4, PAGE * 2);
        release(4, PAGE * 2);
        assert!(reserve(4, LEGACY_SHM_QUOTA_BYTES).is_some());
    }

    #[test]
    fn process_exit_cleanup_releases_identity_charge() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear();
        let r = reserve(5, PAGE).unwrap();
        r.commit();
        release(5, PAGE);
        assert!(reserve(5, LEGACY_SHM_QUOTA_BYTES).is_some());
    }
}
