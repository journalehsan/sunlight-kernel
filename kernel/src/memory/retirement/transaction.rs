extern crate alloc;

use super::{Backing, Entry, State, Table, Token};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingDisposition {
    Preparing,
    Installing,
    Committed,
    Cancelling,
    RolledBack,
    Quarantined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingPage {
    pub address: u64,
    pub original: Option<Backing>,
    pub original_flags: u64,
    pub replacement: u64,
}

#[derive(Clone, Debug)]
pub struct MappingTransaction {
    pub start: u64,
    pub end: u64,
    pub disposition: MappingDisposition,
    pub pages: Vec<MappingPage>,
    entries: Vec<(Entry, bool)>,
    pub count: usize,
    pub prepared: usize,
    pub installed: usize,
    pub capacity: usize,
    pub flags: u64,
}

impl<const N: usize, const P: usize> Table<N, P> {
    pub fn reserve_mapping(
        &mut self,
        root: u64,
        vm: u64,
        pages: &[MappingPage],
        flags: u64,
    ) -> Option<Token> {
        self.reserve_mapping_with_capacity(root, vm, pages, flags, P)
    }

    pub fn reserve_mapping_with_capacity(
        &mut self,
        root: u64,
        vm: u64,
        pages: &[MappingPage],
        flags: u64,
        max_entries: usize,
    ) -> Option<Token> {
        if pages.is_empty() || pages.len() > max_entries {
            return None;
        }
        let start = pages[0].address;
        let end = start.checked_add((pages.len() as u64).checked_mul(4096)?)?;
        let capacity = pages
            .len()
            .checked_add(pages.iter().filter(|page| page.original.is_some()).count())?;
        if start & 4095 != 0
            || capacity > max_entries
            || pages.iter().enumerate().any(|(index, page)| {
                page.address != start + index as u64 * 4096 || page.replacement != 0
            })
        {
            return None;
        }
        let mut stored = Vec::new();
        stored.try_reserve_exact(pages.len()).ok()?;
        stored.extend_from_slice(pages);
        let mut entries = Vec::new();
        entries.try_reserve_exact(capacity).ok()?;
        let token = self.reserve(root, vm)?;
        self.get_mut(&token)?.transaction = Some(MappingTransaction {
            start,
            end,
            disposition: MappingDisposition::Preparing,
            pages: stored,
            entries,
            count: pages.len(),
            prepared: 0,
            installed: 0,
            capacity,
            flags,
        });
        Some(token)
    }

    pub fn prepare_mapping_frame(&mut self, token: &Token, index: usize, frame: u64) -> bool {
        let Some(record) = self.get_mut(token) else {
            return false;
        };
        let Some(transaction) = record.transaction.as_mut() else {
            return false;
        };
        if record.state != State::Prepared
            || transaction.disposition != MappingDisposition::Preparing
            || index != transaction.prepared
            || index >= transaction.count
            || frame == 0
            || frame & 4095 != 0
            || transaction.pages.iter().any(|page| {
                page.replacement == frame || page.original == Some(Backing::Frame(frame))
            })
        {
            return false;
        }
        let page = &mut transaction.pages[index];
        page.replacement = frame;
        if let Some(backing) = page.original {
            transaction.entries.push((
                Entry {
                    address: page.address,
                    backing,
                    released: false,
                },
                false,
            ));
        }
        transaction.entries.push((
            Entry {
                address: page.address,
                backing: Backing::Frame(frame),
                released: false,
            },
            false,
        ));
        transaction.prepared += 1;
        true
    }

    pub fn mapping_page(&mut self, token: &Token, index: usize) -> Option<MappingPage> {
        self.get_mut(token)?
            .transaction
            .as_ref()?
            .pages
            .get(index)
            .copied()
    }

    pub fn mapping_target(
        &mut self,
        token: &Token,
        root: u64,
        vm: u64,
        index: usize,
        restore: bool,
    ) -> Option<(MappingPage, u64)> {
        let record = self.get_mut(token)?;
        let transaction = record.transaction.as_ref()?;
        if record.root != root || record.vm != vm || record.state != State::Withdrawn {
            return None;
        }
        if restore {
            if transaction.disposition != MappingDisposition::Cancelling
                || transaction.installed.checked_sub(1) != Some(index)
            {
                return None;
            }
        } else if transaction.disposition != MappingDisposition::Installing
            || index != transaction.installed
        {
            return None;
        }
        let page = transaction.pages.get(index).copied()?;
        if page.address < transaction.start
            || page.address >= transaction.end
            || page.replacement == 0
        {
            return None;
        }
        Some((page, transaction.flags))
    }

    pub fn begin_mapping(&mut self, token: &Token) -> bool {
        let Some(record) = self.get_mut(token) else {
            return false;
        };
        let Some(transaction) = record.transaction.as_mut() else {
            return false;
        };
        if record.state != State::Prepared
            || transaction.disposition != MappingDisposition::Preparing
            || transaction.prepared != transaction.count
            || transaction.entries.len() != transaction.capacity
        {
            return false;
        }
        transaction.disposition = MappingDisposition::Installing;
        record.state = State::Withdrawn;
        true
    }

    pub fn mapping_installed(&mut self, token: &Token, index: usize) -> bool {
        let Some(record) = self.get_mut(token) else {
            return false;
        };
        let Some(transaction) = record.transaction.as_mut() else {
            return false;
        };
        if record.state != State::Withdrawn
            || transaction.disposition != MappingDisposition::Installing
            || index != transaction.installed
            || index >= transaction.count
        {
            return false;
        }
        transaction.installed += 1;
        true
    }

    pub fn cancel_mapping(&mut self, token: &Token) -> Option<usize> {
        let record = self.get_mut(token)?;
        let transaction = record.transaction.as_mut()?;
        if !matches!(record.state, State::Prepared | State::Withdrawn)
            || !matches!(
                transaction.disposition,
                MappingDisposition::Preparing | MappingDisposition::Installing
            )
        {
            return None;
        }
        transaction.disposition = MappingDisposition::Cancelling;
        record.state = State::Withdrawn;
        Some(transaction.installed)
    }

    pub fn mapping_restored(&mut self, token: &Token, index: usize) -> bool {
        let Some(record) = self.get_mut(token) else {
            return false;
        };
        let Some(transaction) = record.transaction.as_mut() else {
            return false;
        };
        if record.state != State::Withdrawn
            || transaction.disposition != MappingDisposition::Cancelling
            || transaction.installed == 0
            || index != transaction.installed - 1
        {
            return false;
        }
        transaction.installed -= 1;
        true
    }

    pub fn mapping_decide(&mut self, token: &Token, commit: bool) -> bool {
        let Some(record) = self.get_mut(token) else {
            return false;
        };
        let Some(transaction) = record.transaction.as_mut() else {
            return false;
        };
        if record.state != State::Synchronized {
            return false;
        }
        if commit {
            if transaction.disposition != MappingDisposition::Installing
                || transaction.installed != transaction.count
            {
                return false;
            }
            transaction.disposition = MappingDisposition::Committed;
        } else {
            if transaction.disposition != MappingDisposition::Cancelling
                || transaction.installed != 0
            {
                return false;
            }
            transaction.disposition = MappingDisposition::RolledBack;
        }
        true
    }

    #[cfg(not(test))]
    pub fn mapping_entry_count(&mut self, token: &Token) -> Option<usize> {
        Some(self.get_mut(token)?.transaction.as_ref()?.entries.len())
    }

    pub fn mapping_release(&mut self, token: &Token, index: usize) -> Option<(Entry, bool)> {
        let record = self.get_mut(token)?;
        let transaction = record.transaction.as_ref()?;
        if record.state != State::Synchronized
            || !matches!(
                transaction.disposition,
                MappingDisposition::Committed | MappingDisposition::RolledBack
            )
        {
            return None;
        }
        let (entry, transferred) = *transaction.entries.get(index)?;
        if entry.released || transferred {
            return None;
        }
        let page_index = usize::try_from((entry.address - transaction.start) / 4096).ok()?;
        let page = transaction.pages.get(page_index)?;
        let replacement = entry.backing == Backing::Frame(page.replacement);
        let release = replacement == (transaction.disposition == MappingDisposition::RolledBack);
        Some((entry, release))
    }

    pub fn mapping_resolved(&mut self, token: &Token, index: usize, released: bool) -> bool {
        if self
            .mapping_release(token, index)
            .is_none_or(|(_, release)| release != released)
        {
            return false;
        }
        let record = self.get_mut(token).unwrap();
        let (entry, transferred) = &mut record.transaction.as_mut().unwrap().entries[index];
        entry.released = released;
        *transferred = !released;
        true
    }

    pub fn finish_mapping(&mut self, token: &Token) -> bool {
        let Some(record) = self.get_mut(token) else {
            return false;
        };
        let Some(transaction) = record.transaction.as_ref() else {
            return false;
        };
        if record.state != State::Synchronized
            || !matches!(
                transaction.disposition,
                MappingDisposition::Committed | MappingDisposition::RolledBack
            )
            || transaction
                .entries
                .iter()
                .any(|(entry, transferred)| !entry.released && !transferred)
        {
            return false;
        }
        self.records[token.index] = None;
        true
    }

    pub fn quarantine_mapping(&mut self, token: &Token) {
        let record = self.get_mut(token).expect("mapping owner disappeared");
        record
            .transaction
            .as_mut()
            .expect("mapping transaction disappeared")
            .disposition = MappingDisposition::Quarantined;
        record.state = State::Quarantined;
    }
}

#[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
pub(crate) fn receipt(token: &Token, phase: &str, track: bool) {
    let record = super::OWNERS.lock().snapshot(token);
    let transaction = record.transaction.expect("mapping receipt owner");
    if track {
        let mut entries = [Entry {
            address: 0,
            backing: Backing::Frame(0),
            released: false,
        }; 32];
        let count = transaction.entries.len().min(entries.len());
        for (slot, (entry, _)) in entries.iter_mut().zip(&transaction.entries) {
            *slot = *entry;
        }
        unsafe { crate::memory::pmm::retirement_track(&entries[..count]) };
    }
    crate::serial_println!("[MT-OWNER] phase={} slot={} generation={} root={} vm={} start={} end={} disposition={:?} installed={} prepared={} capacity={} sequence={} targets={} received={} state={:?}",
        phase, token.index, token.generation, record.root, record.vm, transaction.start, transaction.end,
        transaction.disposition, transaction.installed, transaction.prepared, transaction.capacity,
        record.sequence, record.targets, record.received, record.state);
    for (index, (entry, transferred)) in transaction.entries.iter().enumerate() {
        let observation = match entry.backing {
            Backing::Frame(frame) => unsafe { crate::memory::pmm::retirement_allocation(frame) },
            Backing::Swap(block) => (crate::memory::zram::block_exists(block), 0, 0, 0),
        };
        crate::serial_println!("[MT-RESOURCE] phase={} slot={} generation={} root={} vm={} index={} va={} backing={:?} released={} transferred={} allocated={} pmm_owner={} releases={} reuses={}",
            phase, token.index, token.generation, record.root, record.vm, index, entry.address, entry.backing,
            entry.released, u8::from(*transferred),
            observation.0, observation.1, observation.2, observation.3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(address: u64, old: Option<Backing>) -> MappingPage {
        MappingPage {
            address,
            original: old,
            original_flags: 3,
            replacement: 0,
        }
    }
    fn prepared() -> (Table<1, 4>, Token) {
        let mut table = Table::new();
        let token = table
            .reserve_mapping(
                4096,
                1,
                &[
                    page(4096, Some(Backing::Frame(8192))),
                    page(8192, Some(Backing::Swap(71))),
                ],
                3,
            )
            .unwrap();
        assert!(table.prepare_mapping_frame(&token, 0, 16384));
        assert!(table.prepare_mapping_frame(&token, 1, 20480));
        (table, token)
    }
    #[test]
    fn whole_request_capacity_and_unused_cancellation() {
        let mut table = Table::<8, 2>::new();
        assert!(table
            .reserve_mapping(
                4096,
                1,
                &[page(4096, Some(Backing::Frame(8192))), page(8192, None)],
                3
            )
            .is_none());
        assert!(!table.owns_vm(4096, 1));
        let token = table
            .reserve_mapping(4096, 1, &[page(4096, None)], 3)
            .unwrap();
        assert!(!table.begin_mapping(&token));
        assert_eq!(table.cancel_mapping(&token), Some(0));
        assert!(table.bind(&token, 1, 0));
        assert!(table.complete(&token, 1, 0));
        assert!(table.mapping_decide(&token, false));
        assert!(table.finish_mapping(&token));
        assert!(!table.finish_mapping(&token));
    }
    #[test]
    fn large_mapping_resolves_entries_past_the_inline_retirement_limit() {
        let mut table = Table::<1, 32>::new();
        let pages: Vec<_> = (1..=40)
            .map(|index| page(index * 4096, Some(Backing::Frame(index * 4096))))
            .collect();
        let token = table
            .reserve_mapping_with_capacity(4096, 1, &pages, 3, 80)
            .unwrap();
        for index in 0..pages.len() {
            assert!(table.prepare_mapping_frame(&token, index, (64 + index as u64) * 4096));
        }
        assert!(table.begin_mapping(&token));
        for index in 0..pages.len() {
            assert!(table.mapping_installed(&token, index));
        }
        assert!(table.bind(&token, 1, 0));
        assert!(table.complete(&token, 1, 0));
        assert!(table.mapping_decide(&token, true));
        for index in 0..80 {
            assert!(table.mapping_resolved(&token, index, index % 2 == 0));
        }
        assert!(table.finish_mapping(&token));
    }

    #[test]
    fn commit_separates_release_from_transfer() {
        let (mut table, token) = prepared();
        assert!(table.begin_mapping(&token));
        assert!(table.mapping_target(&token, 4096, 2, 0, false).is_none());
        assert!(table.mapping_installed(&token, 0));
        assert!(table.mapping_installed(&token, 1));
        assert!(table.bind(&token, 9, 2));
        assert!(!table.complete(&token, 8, 2));
        assert!(!table.complete(&token, 9, 0));
        assert!(table.mapping_release(&token, 0).is_none());
        assert!(table.complete(&token, 9, 2));
        assert!(table.mapping_decide(&token, true));
        for index in 0..4 {
            let (_, release) = table.mapping_release(&token, index).unwrap();
            assert_eq!(release, index % 2 == 0);
            assert!(!table.mapping_resolved(&token, index, !release));
            assert!(table.mapping_resolved(&token, index, release));
            assert!(!table.mapping_resolved(&token, index, release));
        }
        assert!(table.finish_mapping(&token));
    }
    #[test]
    fn rollback_survives_locator_loss_and_blocks_teardown() {
        let (mut table, token) = prepared();
        assert!(table.begin_mapping(&token));
        assert!(table.mapping_installed(&token, 0));
        assert_eq!(table.cancel_mapping(&token), Some(1));
        assert!(!table.mapping_decide(&token, false));
        assert!(!table.mapping_restored(&token, 1));
        assert!(table.mapping_restored(&token, 0));
        assert!(table.bind(&token, 9, 2));
        assert!(table.complete(&token, 9, 2));
        assert!(table.mapping_decide(&token, false));
        for index in 0..4 {
            let (_, release) = table.mapping_release(&token, index).unwrap();
            assert_eq!(release, index % 2 == 1);
        }
        assert!(!table.finish_mapping(&token));
        drop(token);
        assert!(table.owns_vm(4096, 1));
        assert!(table.reserve(4096, 1).is_none());
    }
    #[test]
    fn restoration_failure_retains_both_backings() {
        let (mut table, token) = prepared();
        assert!(table.begin_mapping(&token));
        assert!(table.mapping_installed(&token, 0));
        assert_eq!(table.cancel_mapping(&token), Some(1));
        table.quarantine_mapping(&token);
        assert!(!table.finish_mapping(&token));
        assert!(!table.complete(&token, 9, 0));
        assert!(table.mapping_release(&token, 0).is_none());
        assert_eq!(
            table.mapping_page(&token, 0).unwrap().original,
            Some(Backing::Frame(8192))
        );
        assert_eq!(table.mapping_page(&token, 0).unwrap().replacement, 16384);
        assert!(table.owns_vm(4096, 1));
    }

    #[test]
    fn admission_rejects_same_vm_and_preserves_unprepared_graph() {
        let (mut table, token) = prepared();
        assert!(table
            .reserve_mapping(4096, 1, &[page(4096, None)], 3)
            .is_none());
        assert!(table.mapping_release(&token, 0).is_none());
        assert!(!table.finish(&token));
        assert!(!table.next_chunk(&token));
        assert_eq!(
            table.mapping_page(&token, 1).unwrap().original,
            Some(Backing::Swap(71))
        );
        drop(token);
        assert!(table.owns_vm(4096, 1));
    }

    #[test]
    fn failed_ack_cannot_complete_or_refund_after_exit_guard() {
        let (mut table, token) = prepared();
        assert!(table.begin_mapping(&token));
        assert!(table.mapping_installed(&token, 0));
        assert!(table.mapping_installed(&token, 1));
        assert!(table.bind(&token, 19, 6));
        assert!(table.quarantine(&token, 19, 2, super::super::Failure::MissingAck, Some(2)));
        assert!(!table.complete(&token, 19, 6));
        assert!(!table.mapping_decide(&token, true));
        assert!(!table.mapping_resolved(&token, 0, true));
        assert_eq!(table.cancel_mapping(&token), None);
        assert!(table.owns_vm(4096, 1));
    }
}
