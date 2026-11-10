//! Persistent anonymous-munmap ownership. No allocation and no Drop reclamation.
//! The token is only a locator; the table remains authoritative if it is lost.
#[path = "retirement/transaction.rs"]
mod transaction;
#[cfg(not(test))]
pub use transaction::MappingPage;
#[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
pub(crate) use transaction::receipt as mapping_receipt;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backing {
    Frame(u64),
    Swap(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub address: u64,
    pub backing: Backing,
    pub released: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Prepared,
    Withdrawn,
    Pending,
    Synchronized,
    Quarantined,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    FixedDelivery,
    NmiDelivery,
    MissingAck,
}
#[derive(Clone, Copy, Debug)]
pub struct Record<const P: usize> {
    pub root: u64,
    pub vm: u64,
    pub sequence: u64,
    pub targets: u64,
    pub received: u64,
    pub state: State,
    pub failure: Option<Failure>,
    pub failed_cpu: Option<usize>,
    pub entries: [Option<Entry>; P],
    pub transaction: Option<transaction::MappingTransaction<P>>,
}
#[derive(Debug)]
pub struct Token {
    index: usize,
    generation: u64,
}
pub struct Table<const N: usize, const P: usize> {
    next: u64,
    generations: [u64; N],
    records: [Option<Record<P>>; N],
}
impl<const N: usize, const P: usize> Table<N, P> {
    pub const fn new() -> Self {
        Self {
            next: 1,
            generations: [0; N],
            records: [None; N],
        }
    }
    pub fn reserve(&mut self, root: u64, vm: u64) -> Option<Token> {
        if root == 0 || root & 4095 != 0 || vm == 0 {
            return None;
        }
        if self.owns_vm(root, vm) {
            return None;
        }
        let next = self.next.checked_add(1)?;
        let index = self.records.iter().position(Option::is_none)?;
        let generation = self.next;
        self.next = next;
        self.generations[index] = generation;
        self.records[index] = Some(Record {
            root,
            vm,
            sequence: 0,
            targets: 0,
            received: 0,
            state: State::Prepared,
            failure: None,
            failed_cpu: None,
            entries: [None; P],
            transaction: None,
        });
        Some(Token { index, generation })
    }
    #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
    pub fn snapshot(&self, token: &Token) -> Record<P> {
        assert_eq!(self.generations[token.index], token.generation);
        self.records[token.index].expect("lost retirement owner")
    }
    fn get_mut(&mut self, token: &Token) -> Option<&mut Record<P>> {
        if self.generations.get(token.index) != Some(&token.generation) {
            return None;
        }
        self.records[token.index].as_mut()
    }
    /// Establish exact identities before the caller clears any PTE in this chunk.
    pub fn stage(&mut self, token: &Token, entries: &[Entry]) -> bool {
        let Some(r) = self.get_mut(token) else {
            return false;
        };
        if r.state != State::Prepared
            || r.transaction.is_some()
            || entries.len() > P
            || r.entries.iter().any(Option::is_some)
            || entries.iter().any(|e| e.released)
        {
            return false;
        }
        for (slot, entry) in r.entries.iter_mut().zip(entries) {
            *slot = Some(*entry);
        }
        true
    }
    pub fn withdrawn(&mut self, token: &Token) -> bool {
        let Some(r) = self.get_mut(token) else {
            return false;
        };
        if r.state != State::Prepared {
            return false;
        }
        r.state = State::Withdrawn;
        true
    }
    pub fn bind(&mut self, token: &Token, sequence: u64, targets: u64) -> bool {
        let Some(r) = self.get_mut(token) else {
            return false;
        };
        if r.state != State::Withdrawn || sequence == 0 {
            return false;
        }
        r.sequence = sequence;
        r.targets = targets;
        r.state = State::Pending;
        true
    }
    /// Only the synchronous TLB caller supplies observed exact-sequence ACKs.
    pub fn complete(&mut self, token: &Token, sequence: u64, received: u64) -> bool {
        let Some(r) = self.get_mut(token) else {
            return false;
        };
        if r.state != State::Pending || r.sequence != sequence || received != r.targets {
            return false;
        }
        r.received = received;
        r.state = State::Synchronized;
        true
    }
    pub fn quarantine(
        &mut self,
        token: &Token,
        sequence: u64,
        received: u64,
        failure: Failure,
        failed_cpu: Option<usize>,
    ) -> bool {
        let Some(r) = self.get_mut(token) else {
            return false;
        };
        if r.state != State::Pending || sequence != r.sequence || received & !r.targets != 0 {
            return false;
        }
        r.received = received;
        r.failure = Some(failure);
        r.failed_cpu = failed_cpu;
        r.state = State::Quarantined;
        if let Some(transaction) = r.transaction.as_mut() {
            transaction.disposition = transaction::MappingDisposition::Quarantined;
        }
        true
    }
    pub fn released(&mut self, token: &Token, index: usize) -> bool {
        let Some(r) = self.get_mut(token) else {
            return false;
        };
        if r.state != State::Synchronized || r.transaction.is_some() {
            return false;
        }
        let Some(Some(entry)) = r.entries.get_mut(index) else {
            return false;
        };
        if entry.released {
            return false;
        }
        entry.released = true;
        true
    }
    /// Reuse reserved capacity only after backing and attached table retirement.
    pub fn next_chunk(&mut self, token: &Token) -> bool {
        let Some(r) = self.get_mut(token) else {
            return false;
        };
        if r.transaction.is_some() || r.state != State::Synchronized || r.entries.iter().flatten().any(|e| !e.released) {
            return false;
        }
        r.entries = [None; P];
        r.state = State::Prepared;
        r.sequence = 0;
        r.targets = 0;
        r.received = 0;
        true
    }
    pub fn finish(&mut self, token: &Token) -> bool {
        let Some(r) = self.get_mut(token) else {
            return false;
        };
        if r.transaction.is_some() || r.state != State::Prepared || r.entries.iter().any(Option::is_some) {
            return false;
        }
        self.records[token.index] = None;
        true
    }
    pub fn owns_vm(&self, root: u64, vm: u64) -> bool {
        self.records
            .iter()
            .flatten()
            .any(|r| r.root == root && r.vm == vm)
    }
}

// Short metadata critical sections only. Never held across a shootdown or PMM call.
#[cfg(not(test))]
pub(crate) static OWNERS: spin::Mutex<Table<8, 32>> = spin::Mutex::new(Table::new());

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> Entry {
        Entry {
            address: 4096,
            backing: Backing::Frame(8192),
            released: false,
        }
    }
    #[test]
    fn je1_zero_slots_cannot_publish_an_owner() {
        let mut t = Table::<0, 32>::new();
        assert!(t.reserve(4096, 1).is_none());
        assert!(!t.owns_vm(4096, 1));
    }
    #[test]
    fn je4_entire_chunk_is_staged_before_withdrawal() {
        let mut t = Table::<1, 2>::new();
        let a = t.reserve(4096, 1).unwrap();
        let second = Entry { address: 8192, backing: Backing::Swap(17), released: false };
        assert!(!t.stage(&a, &[entry(), second, entry()]));
        assert_eq!(t.records[0].unwrap().state, State::Prepared);
        assert!(t.records[0].unwrap().entries.iter().all(Option::is_none));
        assert!(t.stage(&a, &[entry(), second]));
        assert!(t.withdrawn(&a));
        assert!(t.bind(&a, 91, 0));
        assert!(t.complete(&a, 91, 0));
        assert!(t.released(&a, 0));
        assert!(!t.next_chunk(&a));
        assert!(t.released(&a, 1));
        assert!(!t.released(&a, 1));
        assert!(t.next_chunk(&a));
        assert!(t.finish(&a));
    }
    #[test]
    fn je5_rollback_does_not_cancel_a_reused_slot() {
        let mut t = Table::<1, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        assert!(t.finish(&a));
        let b = t.reserve(8192, 2).unwrap();
        assert!(!t.finish(&a));
        assert!(!t.stage(&a, &[entry()]));
        assert!(t.owns_vm(8192, 2));
        assert!(t.finish(&b));
        assert!(!t.finish(&b));
    }
    #[test]
    fn je6_exhausted_table_retains_quarantined_exit_obligation() {
        let mut t = Table::<1, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        assert!(t.stage(&a, &[entry()]));
        assert!(t.withdrawn(&a));
        assert!(t.bind(&a, 88, 2));
        assert!(t.quarantine(&a, 88, 0, Failure::MissingAck, Some(1)));
        for _ in 0..8 {
            assert!(t.reserve(8192, 2).is_none());
            assert!(!t.finish(&a));
            assert!(!t.released(&a, 0));
            assert!(t.owns_vm(4096, 1));
            assert_eq!(t.records[0].unwrap().entries[0], Some(entry()));
        }
        // The actual process-exit guard must still be validated in a guest.
    }
    #[test]
    fn je3_competing_reservations_have_unique_slots_and_generations() {
        use std::sync::{Arc, Barrier, Mutex};
        let table = Arc::new(Mutex::new(Table::<8, 1>::new()));
        let barrier = Arc::new(Barrier::new(12));
        let threads: std::vec::Vec<_> = (1..=12).map(|id| {
            let table = table.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let token = table.lock().unwrap().reserve(id * 4096, id);
                barrier.wait();
                token
            })
        }).collect();
        let tokens: std::vec::Vec<_> = threads.into_iter().filter_map(|t| t.join().unwrap()).collect();
        assert_eq!(tokens.len(), 8);
        let mut slots = std::collections::BTreeSet::new();
        let mut generations = std::collections::BTreeSet::new();
        let mut t = table.lock().unwrap();
        for token in &tokens {
            assert!(slots.insert(token.index));
            assert!(generations.insert(token.generation));
            assert!(t.finish(token));
            assert!(!t.finish(token));
        }
        assert!(t.records.iter().all(Option::is_none));
    }
    #[test]
    fn exhausted_reservation_preserves_owner() {
        let mut t = Table::<1, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        assert!(t.reserve(8192, 2).is_none());
        assert!(t.owns_vm(4096, 1));
        assert!(t.finish(&a));
    }
    #[test]
    fn failure_retains_exact_identity_without_stack_token() {
        let mut t = Table::<1, 1>::new();
        {
            let a = t.reserve(4096, 1).unwrap();
            assert!(t.stage(&a, &[entry()]));
            assert!(t.withdrawn(&a));
            assert!(t.bind(&a, 42, 6));
            assert!(!t.released(&a, 0));
            assert!(!t.finish(&a));
        }
        assert_eq!(t.records[0].unwrap().entries[0], Some(entry()));
        assert!(t.owns_vm(4096, 1));
        assert!(t.reserve(4096, 1).is_none());
    }
    #[test]
    fn delayed_stale_duplicate_and_exact_once() {
        let mut t = Table::<1, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        assert!(t.stage(&a, &[entry()]));
        assert!(t.withdrawn(&a));
        assert!(t.bind(&a, 42, 6));
        assert!(!t.complete(&a, 41, 6));
        assert!(!t.complete(&a, 42, 2));
        assert!(!t.released(&a, 0));
        assert!(t.complete(&a, 42, 6));
        assert!(!t.complete(&a, 42, 6));
        assert!(!t.next_chunk(&a));
        assert!(t.released(&a, 0));
        assert!(!t.released(&a, 0));
        assert!(t.next_chunk(&a));
        assert!(t.finish(&a));
        let b = t.reserve(4096, 2).unwrap();
        assert!(!t.finish(&a));
        assert!(t.finish(&b));
    }
    #[test]
    fn overlapping_targets_have_distinct_owners() {
        let mut t = Table::<2, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        let b = t.reserve(8192, 2).unwrap();
        for token in [&a, &b] {
            assert!(t.stage(token, &[entry()]));
            assert!(t.withdrawn(token));
        }
        assert!(t.bind(&a, 42, 6));
        assert!(t.bind(&b, 43, 6));
        assert!(!t.complete(&b, 42, 6));
        assert!(t.complete(&a, 42, 6));
        assert!(!t.released(&b, 0));
        assert!(t.owns_vm(8192, 2));
    }
    #[test]
    fn repeated_cleanup_cannot_take_pending_vm() {
        let mut t = Table::<2, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        assert!(t.stage(&a, &[entry()]));
        assert!(t.withdrawn(&a));
        assert!(t.reserve(4096, 1).is_none());
        assert!(!t.finish(&a));
        assert!(t.owns_vm(4096, 1));
        assert!(!t.owns_vm(4096, 2));
    }
    #[test]
    fn local_completion_requires_no_fabricated_remote_ack() {
        let mut t = Table::<1, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        assert!(t.stage(&a, &[entry()]));
        assert!(t.withdrawn(&a));
        assert!(t.bind(&a, 42, 0));
        assert!(!t.complete(&a, 42, 1));
        assert!(t.complete(&a, 42, 0));
        assert!(t.released(&a, 0));
        assert!(t.next_chunk(&a));
        assert!(t.finish(&a));
        std::println!(
            "table_bytes={} record_bytes={}",
            core::mem::size_of::<Table<8, 32>>(),
            core::mem::size_of::<Record<32>>()
        );
    }
    #[test]
    fn concurrent_transfers_survive_thread_return() {
        use std::sync::{Arc, Barrier, Mutex};
        let table = Arc::new(Mutex::new(Table::<2, 1>::new()));
        let barrier = Arc::new(Barrier::new(2));
        let threads: std::vec::Vec<_> = (0..2)
            .map(|id| {
                let table = table.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let a = table
                        .lock()
                        .unwrap()
                        .reserve((id + 1) * 4096, id + 1)
                        .unwrap();
                    {
                        let mut t = table.lock().unwrap();
                        assert!(t.stage(&a, &[entry()]));
                        assert!(t.withdrawn(&a));
                        assert!(t.bind(&a, id + 42, 6));
                    }
                    barrier.wait();
                    if id == 0 {
                        let mut t = table.lock().unwrap();
                        assert!(t.complete(&a, 42, 6));
                        assert!(t.released(&a, 0));
                        assert!(t.next_chunk(&a));
                        assert!(t.finish(&a));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let t = table.lock().unwrap();
        assert!(!t.owns_vm(4096, 1));
        assert!(t.owns_vm(8192, 2));
    }
    #[test]
    fn quarantine_blocks_late_completion_and_repeated_cleanup() {
        let mut t = Table::<1, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        assert!(t.stage(&a, &[entry()]));
        assert!(t.withdrawn(&a));
        assert!(t.bind(&a, 42, 6));
        assert!(!t.quarantine(&a, 41, 0, Failure::FixedDelivery, Some(1)));
        assert!(t.quarantine(&a, 42, 2, Failure::MissingAck, None));
        assert!(!t.complete(&a, 42, 6));
        assert!(!t.released(&a, 0));
        assert!(!t.finish(&a));
        assert!(!t.next_chunk(&a));
        assert!(t.reserve(4096, 1).is_none());
        let r = t.records[0].unwrap();
        assert_eq!(r.received, 2);
        assert_eq!(r.failure, Some(Failure::MissingAck));
        // Delivery errors use the same permanent no-reclamation state.
        let _ = (Failure::FixedDelivery, Failure::NmiDelivery);
    }
    #[test]
    fn capacity_and_swap_identity() {
        let mut t = Table::<1, 1>::new();
        let a = t.reserve(4096, 1).unwrap();
        assert!(!t.stage(&a, &[entry(), entry()]));
        let e = Entry {
            backing: Backing::Swap(7),
            ..entry()
        };
        assert!(t.stage(&a, &[e]));
        assert!(!t.stage(&a, &[entry()]));
        assert_eq!(t.records[0].unwrap().entries[0], Some(e));
    }
}

#[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
pub(crate) fn fault_mode() -> u64 {
    #[cfg(feature = "kfp2a5_transaction_test")]
    if !crate::memory::transaction_gate::fault_armed() {
        return 0;
    }
    match option_env!("KFP2A2_FAULT") {
        Some("1") => 1,
        Some("2") => 2,
        Some("3") => 3,
        Some("4") => 4,
        Some("5") => 5,
        _ => 0,
    }
}

/// Caller is munmap's shootdown while it still owns PMM exclusion. No lock is
/// retained while rendering these bounded, pre-stop receipts.
#[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
pub(crate) fn failure_receipt(token: &Token) {
    let r = OWNERS.lock().snapshot(token);
    crate::serial_println!(
        "[KFP2A2] owner root={} vm={} sequence={} targets={} received={} state={:?} failure={:?} failed_cpu={:?}",
        r.root,
        r.vm,
        r.sequence,
        r.targets,
        r.received,
        r.state, r.failure, r.failed_cpu
    );
    for entry in r.entries.iter().flatten() {
        if let Backing::Frame(frame) = entry.backing {
            let (allocated, owner, releases, reuses) =
                unsafe { super::pmm::retirement_allocation(frame) };
            crate::serial_println!("[KFP2A2] retained va={} frame={} allocated={} pmm_owner={} released={} releases={} reuses={} authoritative=journal",
                entry.address, frame, allocated, owner, entry.released, releases, reuses);
            assert!(allocated && !entry.released && releases == 0 && reuses == 0);
        }
    }
    crate::serial_println!(
        "[KFP2A2] failure-receipt-complete mode={} reclamation_authorized=0 continuation=fail-stop",
        fault_mode()
    );
}
