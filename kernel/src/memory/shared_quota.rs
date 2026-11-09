//! Backing lifetime budgets. Keys are authenticated values, never bearer tokens.
//! The ledger is accounting, not an identity registry. Lock order is scheduler
//! -> PMM -> broker -> ledger; nothing calls back out while holding the ledger.
use spin::Mutex;

// Display permits 256 MiB surfaces and allocates replacement before release.
// Two maximum surfaces also cover docserv's 64 MiB + page response contract.
pub const PROCESS_LIMIT: usize = 512 * 1024 * 1024;
pub const SESSION_LIMIT: usize = 2 * PROCESS_LIMIT;
const MAX_ENTRIES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProcessIdentity {
    pub pid: usize,
    pub incarnation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionIdentity {
    Pty {
        broker: u64,
        id: u64,
        generation: u64,
    },
    // Restricted channels already authenticate their Account session.
    Account {
        id: u64,
        epoch: u64,
    },
}
impl SessionIdentity {
    fn valid(self) -> bool {
        match self {
            Self::Pty {
                broker,
                id,
                generation,
            } => broker != 0 && id != 0 && generation != 0,
            Self::Account { id, epoch } => id != 0 && epoch != 0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Process(ProcessIdentity),
    Session(SessionIdentity),
}
#[derive(Clone, Copy)]
struct Entry {
    key: Option<Key>,
    bytes: usize,
}
impl Entry {
    const EMPTY: Self = Self {
        key: None,
        bytes: 0,
    };
}
struct Ledger {
    entries: [Entry; MAX_ENTRIES],
}
impl Ledger {
    const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; MAX_ENTRIES],
        }
    }
    fn bytes(&self, key: Key) -> usize {
        self.entries
            .iter()
            .find(|e| e.key == Some(key))
            .map_or(0, |e| e.bytes)
    }
    fn reserve(
        &mut self,
        owner: ProcessIdentity,
        session: Option<SessionIdentity>,
        bytes: usize,
    ) -> bool {
        if owner.pid == 0
            || owner.incarnation == 0
            || bytes == 0
            || bytes > PROCESS_LIMIT
            || session.is_some_and(|s| !s.valid())
        {
            return false;
        }
        let keys = [Some(Key::Process(owner)), session.map(Key::Session)];
        // Preflight both limits AND all metadata slots before any mutation.
        let mut missing = 0;
        for key in keys.into_iter().flatten() {
            let limit = match key {
                Key::Process(_) => PROCESS_LIMIT,
                Key::Session(_) => SESSION_LIMIT,
            };
            if !self
                .bytes(key)
                .checked_add(bytes)
                .is_some_and(|n| n <= limit)
            {
                return false;
            }
            missing += usize::from(!self.entries.iter().any(|e| e.key == Some(key)));
        }
        if self.entries.iter().filter(|e| e.key.is_none()).count() < missing {
            return false;
        }
        for key in keys.into_iter().flatten() {
            // Search existing keys BEFORE empty slots: a hole must not split a budget.
            let index = self
                .entries
                .iter()
                .position(|e| e.key == Some(key))
                .or_else(|| self.entries.iter().position(|e| e.key.is_none()))
                .unwrap();
            self.entries[index].key = Some(key);
            self.entries[index].bytes += bytes;
        }
        true
    }
    fn release(
        &mut self,
        owner: ProcessIdentity,
        session: Option<SessionIdentity>,
        bytes: usize,
    ) -> bool {
        let keys = [Some(Key::Process(owner)), session.map(Key::Session)];
        if bytes == 0 || keys.into_iter().flatten().any(|k| self.bytes(k) < bytes) {
            return false;
        }
        for key in keys.into_iter().flatten() {
            let entry = self
                .entries
                .iter_mut()
                .find(|e| e.key == Some(key))
                .unwrap();
            entry.bytes -= bytes;
            if entry.bytes == 0 {
                *entry = Entry::EMPTY;
            }
        }
        true
    }
}
static LEDGER: Mutex<Ledger> = Mutex::new(Ledger::new());

/// Move-only receipt. Held by an unpublished allocation, then the backing
/// object, then the PMM reclamation result. No PID/byte-based release API exists.
/// Exec never rewrites this identity; surviving peer views retain this receipt.
pub(crate) struct Reservation {
    owner: ProcessIdentity,
    session: Option<SessionIdentity>,
    bytes: usize,
    #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
    trace_id: u64,
}
pub(crate) fn reserve(
    owner: ProcessIdentity,
    session: Option<SessionIdentity>,
    bytes: usize,
) -> Option<Reservation> {
    #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
    super::quota_lifecycle::reservation_attempt(owner, session, bytes);
    LEDGER
        .lock()
        .reserve(owner, session, bytes)
        .then(|| Reservation {
            owner,
            session,
            bytes,
            #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
            trace_id: super::quota_lifecycle::reserved(owner, session, bytes),
        })
}
impl Drop for Reservation {
    fn drop(&mut self) {
        assert!(
            LEDGER.lock().release(self.owner, self.session, self.bytes),
            "SHM quota receipt corrupted"
        );
        #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
        super::quota_lifecycle::consumed(self.trace_id, self.owner, self.session, self.bytes);
    }
}
#[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
impl Reservation {
    pub(crate) fn lifecycle_id(&self) -> u64 {
        self.trace_id
    }
    pub(crate) fn lifecycle_charged(&self) {
        super::quota_lifecycle::charged(self.trace_id, self.owner, self.session, self.bytes);
    }
}
#[cfg(all(
    debug_assertions,
    feature = "fastlane_mapping_test",
    feature = "fastlane_census_trace"
))]
impl Reservation {
    pub(crate) fn restricted_trace(&self) -> Option<(ProcessIdentity, SessionIdentity, usize)> {
        match self.session {
            Some(session @ SessionIdentity::Account { .. }) => {
                Some((self.owner, session, self.bytes))
            }
            _ => None,
        }
    }
}
#[cfg(all(
    debug_assertions,
    feature = "fastlane_mapping_test",
    feature = "fastlane_census_trace"
))]
pub(crate) fn account_observation() -> (usize, usize) {
    let ledger = LEDGER.lock();
    let mut bytes = 0;
    let mut entries = 0;
    for entry in &ledger.entries {
        if matches!(
            entry.key,
            Some(Key::Session(SessionIdentity::Account { .. }))
        ) {
            bytes += entry.bytes;
            entries += 1;
        }
    }
    (bytes, entries)
}
#[cfg(any(
    test,
    all(
        debug_assertions,
        any(
            feature = "shared_quota_test",
            feature = "quota_lifecycle_test",
            all(feature = "fastlane_mapping_test", feature = "fastlane_census_trace")
        )
    )
))]
pub(crate) fn observation(
    owner: ProcessIdentity,
    session: Option<SessionIdentity>,
) -> (usize, usize) {
    let ledger = LEDGER.lock();
    (
        ledger.bytes(Key::Process(owner)),
        session.map_or(0, |s| ledger.bytes(Key::Session(s))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Mutex as HostMutex},
        thread,
        vec::Vec,
    };
    const PAGE: usize = 4096;
    fn owner(pid: usize, incarnation: u64) -> ProcessIdentity {
        ProcessIdentity { pid, incarnation }
    }
    fn session(generation: u64) -> Option<SessionIdentity> {
        Some(SessionIdentity::Pty {
            broker: 7,
            id: 1,
            generation,
        })
    }
    #[test]
    fn holes_do_not_split_an_existing_budget() {
        let mut l = Ledger::new();
        assert!(l.reserve(owner(1, 1), None, PAGE));
        assert!(l.reserve(owner(2, 2), None, PROCESS_LIMIT - PAGE));
        assert!(l.release(owner(1, 1), None, PAGE));
        assert!(l.reserve(owner(2, 2), None, PAGE));
        assert!(!l.reserve(owner(2, 2), None, PAGE));
        assert_eq!(l.bytes(Key::Process(owner(2, 2))), PROCESS_LIMIT);
    }
    #[test]
    fn limits_overflow_and_invalid_identities() {
        let mut l = Ledger::new();
        for bytes in [0, usize::MAX, PROCESS_LIMIT + 1] {
            assert!(!l.reserve(owner(1, 1), None, bytes));
        }
        assert!(!l.reserve(owner(1, 0), None, PAGE));
        assert!(!l.reserve(owner(0, 1), None, PAGE));
        assert!(!l.reserve(owner(1, 1), session(0), PAGE));
        assert!(l.reserve(owner(1, 1), None, PROCESS_LIMIT));
        assert!(!l.reserve(owner(1, 1), None, PAGE));
    }
    #[test]
    fn session_aggregate_and_independence() {
        let mut l = Ledger::new();
        for pid in 1..=2 {
            assert!(l.reserve(owner(pid, 1), session(1), PROCESS_LIMIT));
        }
        assert!(!l.reserve(owner(3, 1), session(1), PAGE));
        assert_eq!(l.bytes(Key::Process(owner(3, 1))), 0);
        assert!(l.reserve(owner(3, 1), session(2), PROCESS_LIMIT));
        assert!(!l.release(owner(1, 1), session(2), PROCESS_LIMIT + 1));
        assert_eq!(l.bytes(Key::Session(session(1).unwrap())), SESSION_LIMIT);
    }
    #[test]
    fn incarnation_reuse_and_stale_release() {
        let mut l = Ledger::new();
        assert!(l.reserve(owner(1, 10), session(1), PAGE));
        assert!(l.reserve(owner(1, 11), session(2), PROCESS_LIMIT));
        assert!(!l.release(owner(1, 12), session(2), PAGE));
        assert!(!l.release(owner(1, 10), session(3), PAGE));
        assert!(l.release(owner(1, 10), session(1), PAGE));
        assert!(!l.release(owner(1, 10), session(1), PAGE));
        assert_eq!(l.bytes(Key::Process(owner(1, 11))), PROCESS_LIMIT);
    }
    #[test]
    fn metadata_exhaustion_is_atomic() {
        let mut l = Ledger::new();
        for pid in 1..MAX_ENTRIES {
            assert!(l.reserve(owner(pid, 1), None, PAGE));
        }
        assert!(!l.reserve(owner(MAX_ENTRIES, 1), session(1), PAGE));
        assert_eq!(l.bytes(Key::Process(owner(MAX_ENTRIES, 1))), 0);
    }
    #[test]
    fn concurrent_allocations_and_receipt_release() {
        let ledger = Arc::new(HostMutex::new(Ledger::new()));
        let mut threads = Vec::new();
        for _ in 0..32 {
            let l = ledger.clone();
            threads.push(thread::spawn(move || {
                l.lock()
                    .unwrap()
                    .reserve(owner(1, 1), session(1), PROCESS_LIMIT / 8)
            }));
        }
        let wins = threads
            .into_iter()
            .filter_map(|t| t.join().unwrap().then_some(()))
            .count();
        assert_eq!(wins, 8);
        let mut l = ledger.lock().unwrap();
        assert_eq!(l.bytes(Key::Process(owner(1, 1))), PROCESS_LIMIT);
        for _ in 0..wins {
            assert!(l.release(owner(1, 1), session(1), PROCESS_LIMIT / 8));
        }
        assert_eq!(l.bytes(Key::Session(session(1).unwrap())), 0);
    }
    #[test]
    fn receipt_rollback_and_last_reference() {
        let who = owner(0x5151, 123);
        let r = reserve(who, session(123), PAGE).unwrap();
        assert_eq!(LEDGER.lock().bytes(Key::Process(who)), PAGE);
        drop(r);
        assert_eq!(LEDGER.lock().bytes(Key::Process(who)), 0);
        let r = Arc::new(reserve(who, session(123), PAGE).unwrap());
        let peer = r.clone();
        drop(r); // owner/old VM gone; peer still retains backing receipt
        assert_eq!(LEDGER.lock().bytes(Key::Process(who)), PAGE);
        let peer2 = peer.clone();
        let t = thread::spawn(move || drop(peer2));
        drop(peer);
        t.join().unwrap();
        assert_eq!(LEDGER.lock().bytes(Key::Process(who)), 0);
    }
}
