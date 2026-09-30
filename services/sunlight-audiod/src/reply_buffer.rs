//! One retained buffer for an acknowledged, synchronous request stream.
//!
//! An error is not proof that the peer stopped reading. Retire that buffer
//! instead of overwriting it on the next request. Its owner must release only
//! the local mapping on Drop; a peer's mapping keeps the old backing alive.

use core::cell::Cell;

pub(super) struct ReplyBuffer<B> {
    idle: Cell<Option<B>>,
}

impl<B> ReplyBuffer<B> {
    pub const fn new() -> Self {
        Self {
            idle: Cell::new(None),
        }
    }

    /// `request` may succeed only after the peer acknowledges that it has
    /// finished accessing this buffer. Keep the buffer out of the cache while
    /// the request runs. Cell makes this cache non-Sync, so concurrent callers
    /// cannot mutate the same buffer through a shared reference.
    pub fn request<R, E>(
        &self,
        allocate: impl FnOnce() -> Result<B, E>,
        request: impl FnOnce(&mut B) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut buffer = match self.idle.take() {
            Some(buffer) => buffer,
            None => allocate()?,
        };
        let result = request(&mut buffer)?;
        self.idle.set(Some(buffer));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::ReplyBuffer;
    use core::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct Page {
        bytes: Rc<RefCell<[u8; 4]>>,
        releases: Rc<Cell<usize>>,
    }

    impl Drop for Page {
        fn drop(&mut self) {
            self.releases.set(self.releases.get() + 1);
        }
    }

    fn allocate(allocations: &Cell<usize>, releases: &Rc<Cell<usize>>) -> Result<Page, ()> {
        allocations.set(allocations.get() + 1);
        Ok(Page {
            bytes: Rc::new(RefCell::new([0; 4])),
            releases: Rc::clone(releases),
        })
    }

    #[test]
    fn acknowledged_stream_allocates_once_and_releases_on_drop() {
        let allocations = Cell::new(0);
        let releases = Rc::new(Cell::new(0));
        let cache = ReplyBuffer::new();
        for n in 0..1000 {
            let result = cache.request(
                || allocate(&allocations, &releases),
                |page| {
                    *page.bytes.borrow_mut() = [n as u8; 4];
                    Ok(*page.bytes.borrow())
                },
            );
            assert_eq!(result, Ok([n as u8; 4]));
        }
        assert_eq!(allocations.get(), 1);
        assert_eq!(releases.get(), 0);
        drop(cache);
        assert_eq!(releases.get(), 1);
    }

    #[test]
    fn uncertain_completion_never_overwrites_a_peers_old_buffer() {
        let allocations = Cell::new(0);
        let releases = Rc::new(Cell::new(0));
        let cache = ReplyBuffer::new();
        let mut peer_view = None;
        let failed: Result<(), ()> = cache.request(
            || allocate(&allocations, &releases),
            |page| {
                *page.bytes.borrow_mut() = [7; 4];
                peer_view = Some(Rc::clone(&page.bytes));
                Err(()) // Timeout/cancellation while the peer still has a view.
            },
        );
        assert_eq!(failed, Err(()));
        assert_eq!(releases.get(), 1);
        cache
            .request(
                || allocate(&allocations, &releases),
                |page| {
                    *page.bytes.borrow_mut() = [9; 4];
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(*peer_view.unwrap().borrow(), [7; 4]);
        assert_eq!(allocations.get(), 2);
        drop(cache);
        assert_eq!(releases.get(), 2);
    }

    #[test]
    fn failed_allocation_does_not_submit_and_next_request_can_retry() {
        let allocations = Cell::new(0);
        let releases = Rc::new(Cell::new(0));
        let cache = ReplyBuffer::<Page>::new();
        let failed: Result<(), ()> = cache.request(|| Err(()), |_| panic!("no buffer"));
        assert_eq!(failed, Err(()));
        cache
            .request(|| allocate(&allocations, &releases), |_| Ok(()))
            .unwrap();
        assert_eq!(allocations.get(), 1);
        drop(cache);
        assert_eq!(releases.get(), 1);
    }
}
