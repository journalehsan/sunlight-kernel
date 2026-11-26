//! FIFO storage with stable chunks and fallible, transactional growth.

use alloc::alloc::{Allocator, Global};
use alloc::boxed::Box;
use core::{mem::MaybeUninit, ptr::NonNull};

pub(super) const CHUNK_SLOTS: usize = 170;

struct Chunk<T> {
    next: Option<NonNull<Self>>,
    entries: [MaybeUninit<T>; CHUNK_SLOTS],
}

#[derive(Clone, Copy)]
struct Cursor<T: Copy> {
    chunk: NonNull<Chunk<T>>,
    index: usize,
}

impl<T: Copy> Cursor<T> {
    unsafe fn advance(&mut self) {
        self.index += 1;
        if self.index == CHUNK_SLOTS {
            self.chunk = (*self.chunk.as_ptr()).next.unwrap();
            self.index = 0;
        }
    }

    unsafe fn entry(self) -> *mut MaybeUninit<T> {
        core::ptr::addr_of_mut!((*self.chunk.as_ptr()).entries)
            .cast::<MaybeUninit<T>>()
            .add(self.index)
    }
}

/// Owns a circular list of chunks. Exactly `len` positions starting at
/// `head` are initialized; `tail` is the cursor immediately after them.
/// Empty slots anywhere in the ring can be consumed without allocation.
pub(super) struct CandidateQueue<T: Copy, A: Allocator = Global> {
    head: Option<Cursor<T>>,
    tail: Option<Cursor<T>>,
    before_head: Option<NonNull<Chunk<T>>>,
    len: usize,
    capacity: usize,
    allocator: A,
}

// SAFETY: all pointers refer to exclusively owned chunks. Shared access can
// only read entries; moving the queue transfers ownership of the allocations.
unsafe impl<T: Copy + Send, A: Allocator + Send> Send for CandidateQueue<T, A> {}

impl<T: Copy, A: Allocator> CandidateQueue<T, A> {
    pub(super) const fn new_in(allocator: A) -> Self {
        Self {
            head: None,
            tail: None,
            before_head: None,
            len: 0,
            capacity: 0,
            allocator,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn capacity(&self) -> usize {
        self.capacity
    }

    pub(super) fn try_reserve(&mut self, additional: usize) -> Result<(), ()> {
        let required = self.len.checked_add(additional).ok_or(())?;
        if required <= self.capacity {
            return Ok(());
        }
        let chunks = (required - self.capacity).div_ceil(CHUNK_SLOTS);
        let capacity = chunks
            .checked_mul(CHUNK_SLOTS)
            .and_then(|extra| self.capacity.checked_add(extra))
            .ok_or(())?;

        let mut first = None;
        let mut last: Option<NonNull<Chunk<T>>> = None;
        for _ in 0..chunks {
            let Ok(chunk) = Box::try_new_in(
                Chunk {
                    next: None,
                    entries: [MaybeUninit::uninit(); CHUNK_SLOTS],
                },
                &self.allocator,
            ) else {
                unsafe { self.free_chunks(first) };
                return Err(());
            };
            let chunk = Some(NonNull::from(Box::leak(chunk)));
            if let Some(last) = last {
                unsafe { (*last.as_ptr()).next = chunk };
            } else {
                first = chunk;
            }
            last = chunk;
        }

        // SAFETY: the detached list is nonempty. The queue is exclusively
        // borrowed and its cursors and links point to owned, live chunks.
        unsafe {
            let new_start = Cursor {
                chunk: first.unwrap(),
                index: 0,
            };
            if let (Some(head), Some(tail)) = (self.head, self.tail) {
                // If entries wrap into the prefix of the head chunk, carry
                // that prefix into the first inserted chunk. Growth copies
                // at most one chunk regardless of queue length.
                if tail.chunk == head.chunk && self.len != 0 && tail.index <= head.index {
                    core::ptr::copy_nonoverlapping(
                        Cursor {
                            chunk: head.chunk,
                            index: 0,
                        }
                        .entry(),
                        new_start.entry(),
                        tail.index,
                    );
                    self.tail = Some(Cursor {
                        chunk: new_start.chunk,
                        index: tail.index,
                    });
                }
                (*self.before_head.unwrap().as_ptr()).next = first;
                (*last.unwrap().as_ptr()).next = Some(head.chunk);
            } else {
                self.head = Some(new_start);
                self.tail = Some(new_start);
                (*last.unwrap().as_ptr()).next = first;
            }
        }
        self.before_head = last;
        self.capacity = capacity;
        Ok(())
    }

    pub(super) fn push_back(&mut self, value: T) {
        assert!(self.len < self.capacity);
        let tail = self.tail.as_mut().unwrap();
        unsafe {
            (*tail.entry()).write(value);
            tail.advance();
        }
        self.len += 1;
    }

    pub(super) fn pop_front(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let head = self.head.as_mut().unwrap();
        let value = unsafe { (*head.entry()).assume_init_read() };
        if head.index == CHUNK_SLOTS - 1 {
            self.before_head = Some(head.chunk);
        }
        unsafe { head.advance() };
        self.len -= 1;
        Some(value)
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(T) -> bool) {
        if self.len == 0 {
            return;
        }
        let mut read = self.head.unwrap();
        let mut write = read;
        let mut retained = 0;
        // SAFETY: the write cursor never overtakes unread entries. T is Copy
        // so positions abandoned beyond the new tail need no destruction.
        unsafe {
            for _ in 0..self.len {
                let value = (*read.entry()).assume_init_read();
                if keep(value) {
                    if write.chunk != read.chunk || write.index != read.index {
                        (*write.entry()).write(value);
                    }
                    write.advance();
                    retained += 1;
                }
                read.advance();
            }
        }
        self.tail = Some(write);
        self.len = retained;
    }

    #[cfg(any(test, feature = "zram_lifecycle_test"))]
    pub(super) fn iter(&self) -> impl Iterator<Item = T> + '_ {
        let mut cursor = self.head;
        (0..self.len).map(move |_| {
            let cursor = cursor.as_mut().unwrap();
            let value = unsafe { (*cursor.entry()).assume_init_read() };
            unsafe { cursor.advance() };
            value
        })
    }

    /// Free a detached, null-terminated chain using its original allocator.
    unsafe fn free_chunks(&self, mut chunk: Option<NonNull<Chunk<T>>>) {
        while let Some(current) = chunk {
            let owned = Box::from_raw_in(current.as_ptr(), &self.allocator);
            chunk = owned.next;
        }
    }
}

impl<T: Copy, A: Allocator> Drop for CandidateQueue<T, A> {
    fn drop(&mut self) {
        if let Some(last) = self.before_head {
            unsafe {
                (*last.as_ptr()).next = None;
                self.free_chunks(self.head.map(|head| head.chunk));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{alloc::AllocError, collections::VecDeque, vec, vec::Vec};
    use core::{alloc::Layout, cell::RefCell, mem::size_of};
    use linked_list_allocator::Heap;

    type Entry = [u64; 3];
    const CHUNK_BYTES: usize = size_of::<Chunk<Entry>>();
    const HEAP_BYTES: usize = 16 * 1024 * 1024;

    struct Arena {
        heap: RefCell<Heap>,
        peak: core::cell::Cell<usize>,
        _storage: Vec<MaybeUninit<u64>>,
    }

    impl Arena {
        fn new(bytes: usize) -> Self {
            let mut storage = vec![MaybeUninit::<u64>::uninit(); bytes / 8];
            let mut heap = Heap::empty();
            unsafe { heap.init(storage.as_mut_ptr().cast(), bytes) };
            Self {
                heap: RefCell::new(heap),
                peak: core::cell::Cell::new(0),
                _storage: storage,
            }
        }

        fn used(&self) -> usize {
            self.heap.borrow().used()
        }
    }

    unsafe impl Allocator for Arena {
        fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
            let mut heap = self.heap.borrow_mut();
            let ptr = heap.allocate_first_fit(layout).map_err(|_| AllocError)?;
            self.peak.set(self.peak.get().max(heap.used()));
            Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
        }

        unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
            self.heap.borrow_mut().deallocate(ptr, layout);
        }
    }

    #[test]
    fn fifo_matches_vecdeque_through_wrap_growth_and_stable_removal() {
        for head_offset in [0, 1, CHUNK_SLOTS - 1, CHUNK_SLOTS, CHUNK_SLOTS + 1] {
            for len in [
                0,
                1,
                CHUNK_SLOTS - 1,
                CHUNK_SLOTS,
                2 * CHUNK_SLOTS - 1,
                2 * CHUNK_SLOTS,
            ] {
                let mut queue = CandidateQueue::new_in(Global);
                let mut model = VecDeque::new();
                queue.try_reserve(2 * CHUNK_SLOTS).unwrap();
                for value in 0..head_offset {
                    queue.push_back(value);
                    assert_eq!(queue.pop_front(), Some(value));
                }
                for value in 0..len {
                    queue.push_back(value);
                    model.push_back(value);
                }
                queue.try_reserve(3 * CHUNK_SLOTS).unwrap();
                assert!(queue.iter().eq(model.iter().copied()));
                for value in len..queue.capacity() {
                    queue.push_back(value);
                    model.push_back(value);
                }
                queue.retain(|value| value % 3 != 1);
                model.retain(|value| value % 3 != 1);
                for _ in 0..queue.capacity() {
                    let value = model.pop_front().unwrap();
                    assert_eq!(queue.pop_front(), Some(value));
                    queue.push_back(value);
                    model.push_back(value);
                }
                assert!(queue.iter().eq(model));
            }
        }
    }

    #[test]
    fn fragmented_heap_and_failed_reserve_preserve_live_slot_claims() {
        let arena = Arena::new(4 * (CHUNK_BYTES + 16));
        let mut queue = CandidateQueue::new_in(&arena);
        let mut holes = Vec::new();
        let mut guards = Vec::new();
        for _ in 0..4 {
            holes.push(Box::new_in([0u64; CHUNK_BYTES / 8], &arena));
            guards.push(Box::new_in([0u64; 2], &arena));
        }
        drop(holes);
        assert!(Box::try_new_in([0u64; 2 * CHUNK_BYTES / 8], &arena).is_err());
        queue.try_reserve(2 * CHUNK_SLOTS).unwrap();
        for value in 0..queue.capacity() {
            queue.push_back([value as u64; 3]);
        }
        let swapped = queue.pop_front().unwrap();
        let before = arena.used();
        assert_eq!(queue.try_reserve(1 + 2 * CHUNK_SLOTS + 1), Err(()));
        assert_eq!(arena.used(), before);
        assert_eq!(queue.capacity(), 2 * CHUNK_SLOTS);
        assert!(queue
            .iter()
            .eq((1..2 * CHUNK_SLOTS).map(|value| [value as u64; 3])));
        queue.push_back(swapped);
        assert_eq!(arena.used(), before);
        assert_eq!(queue.iter().last(), Some(swapped));
        assert_eq!(queue.try_reserve(usize::MAX), Err(()));
        drop(queue);
        drop(guards);
        assert_eq!(arena.used(), 0);
    }

    #[test]
    fn matched_heap_peak_for_eager_batch_growth() {
        let old = Arena::new(HEAP_BYTES);
        let new = Arena::new(HEAP_BYTES);
        let baseline = Layout::from_size_align(2_620_536, 8).unwrap();
        let _baseline_old = old.allocate(baseline).unwrap();
        let _baseline_new = new.allocate(baseline).unwrap();
        let journal = Layout::from_size_align(8_192 * 40, 8).unwrap();
        let mut contiguous: VecDeque<Entry, _> = VecDeque::new_in(&old);
        let mut segmented = CandidateQueue::new_in(&new);
        assert_eq!(size_of::<Entry>(), 24);
        assert_eq!(CHUNK_BYTES, 4088);
        std::println!(
            "queue_descriptor_bytes={}",
            size_of::<CandidateQueue<Entry>>()
        );
        for count in [4098].into_iter().chain(core::iter::repeat_n(8192, 21)) {
            let old_journal = old.allocate(journal).unwrap();
            let new_journal = new.allocate(journal).unwrap();
            contiguous.try_reserve_exact(count).unwrap();
            segmented.try_reserve(count).unwrap();
            for value in 0..count {
                let entry = [value as u64; 3];
                contiguous.push_back(entry);
                segmented.push_back(entry);
            }
            unsafe {
                old.deallocate(old_journal.cast(), journal);
                new.deallocate(new_journal.cast(), journal);
            }
        }
        assert!(segmented.iter().eq(contiguous.iter().copied()));
        assert!(new.peak.get() < old.peak.get());
        std::println!(
            "entries={} contiguous_capacity={} segmented_capacity={} old_used={} new_used={} old_peak={} new_peak={}",
            contiguous.len(), contiguous.capacity(), segmented.capacity(), old.used(), new.used(),
            old.peak.get(), new.peak.get()
        );
    }
}
