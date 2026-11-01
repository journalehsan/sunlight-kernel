use super::*;
use core::{cell::Cell, mem};
use std::{sync::Barrier, thread};

fn pair(pages: &AtomicPages) -> (Producer<View<'_>>, Consumer<View<'_>>) {
    (
        Producer::new(pages.view(Role::Producer), 7).unwrap(),
        Consumer::new(pages.view(Role::Consumer), 7).unwrap(),
    )
}

fn put(pages: &AtomicPages, region: Region, word: usize, value: u64) {
    pages
        .view(if region == Region::Consumer {
            Role::Consumer
        } else {
            Role::Producer
        })
        .store(region, word, value)
        .unwrap();
}

#[test]
fn geometry_and_role_permissions() {
    assert_eq!(core::mem::size_of::<AtomicPage>(), PAGE_BYTES);
    assert_eq!(core::mem::align_of::<AtomicPage>(), PAGE_BYTES);
    assert_eq!(CAPACITY * PAYLOAD_BYTES, PAGE_BYTES);
    let pages = AtomicPages::new();
    for role in [Role::Producer, Role::Consumer] {
        let view = pages.view(role);
        for region in [Region::Producer, Region::Consumer, Region::Payload] {
            for word in 0..WORDS {
                assert_eq!(view.load(region, word), Ok(0));
            }
            let allowed = (role == Role::Consumer) == (region == Region::Consumer);
            assert_eq!(
                view.store(region, WORDS - 1, 9),
                if allowed { Ok(()) } else { Err(Error::Access) }
            );
            if allowed {
                view.store(region, WORDS - 1, 0).unwrap();
            }
            assert_eq!(view.load(region, WORDS), Err(Error::Access));
            assert_eq!(view.load(region, usize::MAX), Err(Error::Access));
            assert_eq!(view.store(region, usize::MAX, 0), Err(Error::Access));
        }
    }
    assert!(matches!(
        Producer::new(pages.view(Role::Producer), 0),
        Err(Error::Configuration)
    ));
}

#[test]
fn all_lengths_and_private_identity() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    for len in 0..=PAYLOAD_BYTES {
        let mut write = p.try_reserve(len).unwrap();
        let id = write.identity();
        assert_eq!(id.slot, len % CAPACITY);
        assert_eq!(id.generation, (len / CAPACITY + 1) as u64);
        write.payload_mut().fill(len as u8);
        write.commit().unwrap();
        let read = c.try_read().unwrap();
        assert_eq!(read.identity(), id);
        assert_eq!(read.payload(), &[len as u8; PAYLOAD_BYTES][..len]);
        read.release().unwrap();
    }
    assert!(matches!(p.try_reserve(usize::MAX), Err(Error::Length)));
    assert!(matches!(c.try_read(), Err(Error::Empty)));
}

#[test]
fn malformed_lengths_offsets_and_overflow_are_terminal() {
    for (offset, len, expected) in [
        (0, 257, Error::Length),
        (0, u64::MAX, Error::Offset),
        (1, 0, Error::Offset),
        (4096, 0, Error::Offset),
        (u64::MAX, 1, Error::Offset),
        (u64::MAX - 3, 8, Error::Offset),
    ] {
        let pages = AtomicPages::new();
        let (mut p, mut c) = pair(&pages);
        p.try_send(b"safe").unwrap();
        put(&pages, Region::Producer, 4, offset);
        put(&pages, Region::Producer, 5, len);
        let mut out = [0xa5; PAYLOAD_BYTES];
        assert_eq!(c.try_recv(&mut out), Err(expected));
        assert_eq!(out, [0xa5; PAYLOAD_BYTES]);
        assert_eq!(pages.view(Role::Producer).load(Region::Consumer, 0), Ok(0));
        // Repairing hostile bytes never reopens a failed endpoint.
        put(&pages, Region::Producer, 4, 0);
        put(&pages, Region::Producer, 5, 4);
        assert!(matches!(c.try_read(), Err(e) if e == expected));
    }
}

#[test]
fn wrong_channel_sequence_and_slot_generation() {
    for word in 1..=3 {
        for value in [0, 2, u64::MAX] {
            let pages = AtomicPages::new();
            let (mut p, mut c) = pair(&pages);
            p.try_send(b"x").unwrap();
            let valid = [7, 0, 1][word - 1];
            if value == valid {
                continue;
            }
            put(&pages, Region::Producer, word, value);
            assert!(matches!(c.try_read(), Err(Error::Identity)));
        }
    }
    // A valid-looking old descriptor in a reused slot is rejected.
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    for _ in 0..CAPACITY {
        p.try_send(b"x").unwrap();
        c.try_read().unwrap().release().unwrap();
    }
    p.try_send(b"new").unwrap();
    put(&pages, Region::Producer, 3, 1);
    assert!(matches!(c.try_read(), Err(Error::Identity)));
}

#[test]
fn corrupted_producer_cursor_never_chooses_an_address() {
    for head in [CAPACITY as u64 + 1, u64::MAX] {
        let pages = AtomicPages::new();
        let (_, mut c) = pair(&pages);
        put(&pages, Region::Producer, 0, head);
        assert!(matches!(c.try_read(), Err(Error::ProducerState)));
    }
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    p.try_send(b"x").unwrap();
    c.try_read().unwrap().release().unwrap();
    put(&pages, Region::Producer, 0, 0);
    assert!(matches!(c.try_read(), Err(Error::ProducerState)));
}

#[test]
fn corrupted_consumer_cursor_is_checked_against_private_publication() {
    for tail in [1, CAPACITY as u64, u64::MAX] {
        let pages = AtomicPages::new();
        let (mut p, _) = pair(&pages);
        put(&pages, Region::Consumer, 0, tail);
        assert!(matches!(p.try_reserve(1), Err(Error::ConsumerState)));
    }
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    p.try_send(b"x").unwrap();
    c.try_read().unwrap().release().unwrap();
    drop(p.try_reserve(1).unwrap()); // remembers acknowledged tail 1
    put(&pages, Region::Consumer, 0, 0);
    assert!(matches!(p.try_reserve(1), Err(Error::ConsumerState)));
}

#[test]
fn own_writable_cursors_are_never_trusted() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    put(&pages, Region::Producer, 0, u64::MAX);
    p.try_send(b"local head").unwrap();
    put(&pages, Region::Consumer, 0, u64::MAX);
    let r = c.try_read().unwrap();
    assert_eq!(r.payload(), b"local head");
    r.release().unwrap();
    p.try_send(b"local tail").unwrap();
}

#[test]
fn sequence_and_local_lease_serial_exhaustion_never_wrap() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    // Isolated host boundary, not a fabricated native counter rollover.
    p.local.next = u64::MAX - 1;
    c.local.next = u64::MAX - 1;
    put(&pages, Region::Consumer, 0, u64::MAX - 1);
    p.try_send(b"last").unwrap();
    assert_eq!(p.local.next, u64::MAX);
    let r = c.try_read().unwrap();
    assert_eq!(r.payload(), b"last");
    r.release().unwrap();
    assert_eq!(c.local.next, u64::MAX);
    assert!(matches!(p.try_reserve(0), Err(Error::Exhausted)));
    assert!(matches!(c.try_read(), Err(Error::Exhausted)));
    assert_eq!(Identity::new(7, u64::MAX), Err(Error::Exhausted));
    assert_eq!(Identity::new(7, u64::MAX - 1).unwrap().slot, CAPACITY - 2);
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    p.local.serial = u64::MAX;
    assert!(matches!(p.try_reserve(0), Err(Error::Exhausted)));
    let mut p = Producer::new(pages.view(Role::Producer), 7).unwrap();
    p.try_send(b"x").unwrap();
    c.local.serial = u64::MAX;
    assert!(matches!(c.try_read(), Err(Error::Exhausted)));
}

#[test]
fn abandoned_reservations_and_private_read_leases() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    let mut w = p.try_reserve(8).unwrap();
    w.payload_mut().fill(0xff);
    drop(w);
    assert!(matches!(c.try_read(), Err(Error::Empty)));
    let w = p.try_reserve(8).unwrap();
    let old_serial = w.serial;
    assert_eq!(w.bytes, [0; PAYLOAD_BYTES]);
    mem::forget(w);
    assert!(matches!(p.try_reserve(0), Err(Error::Busy)));
    p.cancel_pending();
    let w = p.try_reserve(8).unwrap();
    assert!(w.serial > old_serial);
    w.commit().unwrap();
    let r = c.try_read().unwrap();
    assert_eq!(r.payload(), &[0; 8]);
    drop(r);
    let r = c.try_read().unwrap();
    mem::forget(r);
    assert!(matches!(c.try_read(), Err(Error::Busy)));
    c.cancel_pending();
    c.try_read().unwrap().release().unwrap();
    assert!(matches!(c.try_read(), Err(Error::Empty)));
}

#[test]
fn stale_local_leases_cannot_publish_or_reclaim() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    let mut w = p.try_reserve(1).unwrap();
    w.serial += 1;
    assert_eq!(w.commit(), Err(Error::StaleLease));
    p.cancel_pending();
    p.try_send(b"x").unwrap();
    let mut r = c.try_read().unwrap();
    r.id.generation += 1;
    assert_eq!(r.release(), Err(Error::StaleLease));
    c.cancel_pending();
    assert_eq!(pages.view(Role::Producer).load(Region::Consumer, 0), Ok(0));
    c.try_read().unwrap().release().unwrap();
}

#[test]
fn capacity_exhaustion_and_abandoned_reader_apply_bounded_backpressure() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    for _ in 0..CAPACITY {
        p.try_send(b"x").unwrap();
    }
    let r = c.try_read().unwrap();
    for _ in 0..1000 {
        assert_eq!(p.try_send(b"full"), Err(Error::Full));
    }
    drop(r);
    assert_eq!(p.try_send(b"still full"), Err(Error::Full));
    c.try_read().unwrap().release().unwrap();
    p.try_send(b"space").unwrap();
    assert_eq!(p.try_send(b"full"), Err(Error::Full));
}

#[test]
fn partial_initialization_and_whole_slot_reuse_clearing() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    p.try_send(&[0xee; PAYLOAD_BYTES]).unwrap();
    c.try_read().unwrap().release().unwrap();
    for _ in 1..CAPACITY {
        p.try_send(b"").unwrap();
        c.try_read().unwrap().release().unwrap();
    }
    let mut w = p.try_reserve(PAYLOAD_BYTES).unwrap();
    w.payload_mut().fill(0xcc);
    drop(w);
    let mut w = p.try_reserve(3).unwrap();
    w.payload_mut()[0] = 0x42;
    w.commit().unwrap();
    let r = c.try_read().unwrap();
    assert_eq!(r.payload(), &[0x42, 0, 0]);
    assert_eq!(r.bytes[3..], [0; PAYLOAD_BYTES - 3]);
    assert_eq!(
        pages.view(Role::Consumer).load(Region::Payload, 0),
        Ok(0x42)
    );
    for word in 1..PAYLOAD_WORDS {
        assert_eq!(
            pages.view(Role::Consumer).load(Region::Payload, word),
            Ok(0)
        );
    }
    r.release().unwrap();
}

#[test]
fn short_output_never_consumes_or_exposes_suffix() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    p.try_send(b"abcd").unwrap();
    let mut short = [0xaa; 2];
    assert_eq!(c.try_recv(&mut short), Err(Error::OutputTooSmall));
    assert_eq!(short, [0xaa; 2]);
    let mut out = [0xbb; 8];
    assert_eq!(c.try_recv(&mut out), Ok(4));
    assert_eq!(&out, b"abcd\xbb\xbb\xbb\xbb");
}

#[test]
fn producer_mutation_after_commit_is_visible_before_private_copy() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    p.try_send(b"original").unwrap();
    put(&pages, Region::Payload, 0, u64::from_le_bytes(*b"modified"));
    let r = c.try_read().unwrap();
    assert_eq!(r.payload(), b"modified");
    put(&pages, Region::Payload, 0, 0);
    assert_eq!(r.payload(), b"modified");
    r.release().unwrap();
}

struct Mutating<'a> {
    view: View<'a>,
    reads: Cell<usize>,
}
impl Memory for Mutating<'_> {
    fn load(&self, region: Region, word: usize) -> Result<u64, Error> {
        self.reads.set(self.reads.get() + 1);
        let v = self.view.load(region, word)?;
        if region == Region::Payload {
            // Deterministically interleave hostile writes DURING the copy,
            // after metadata was validated, with no Rust data race.
            put(self.view.pages, Region::Payload, word + 1, u64::MAX);
            put(self.view.pages, Region::Producer, 4, u64::MAX);
            put(self.view.pages, Region::Producer, 5, u64::MAX);
        }
        Ok(v)
    }
    fn store(&self, region: Region, word: usize, value: u64) -> Result<(), Error> {
        self.view.store(region, word, value)
    }
}

#[test]
fn mutation_during_copy_can_mix_bytes_but_cannot_change_bounds() {
    let pages = AtomicPages::new();
    let (mut p, _) = pair(&pages);
    p.try_send(&[0x11; 16]).unwrap();
    let memory = Mutating {
        view: pages.view(Role::Consumer),
        reads: Cell::new(0),
    };
    let mut c = Consumer::new(memory, 7).unwrap();
    let r = c.try_read().unwrap();
    assert_eq!(&r.payload()[..8], &[0x11; 8]);
    assert_eq!(&r.payload()[8..], &[0xff; 8]);
    r.release().unwrap();
    assert_eq!(c.memory.reads.get(), 8); // cursor + five fields + two payload words
}

#[test]
fn false_valid_ack_can_trigger_reuse_but_private_read_is_stable() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    for _ in 0..CAPACITY {
        p.try_send(b"original").unwrap();
    }
    let r = c.try_read().unwrap();
    put(&pages, Region::Consumer, 0, CAPACITY as u64);
    p.try_send(b"overwrite").unwrap();
    assert_eq!(r.payload(), b"original");
    r.release().unwrap(); // next producer call detects the tail's regression
    assert!(matches!(p.try_reserve(0), Err(Error::ConsumerState)));
}

struct Failing<'a> {
    view: View<'a>,
    remaining: Cell<usize>,
}
impl Failing<'_> {
    fn step(&self) -> Result<(), Error> {
        if self.remaining.get() == 0 {
            return Err(Error::Closed);
        }
        self.remaining.set(self.remaining.get() - 1);
        Ok(())
    }
}
impl Memory for Failing<'_> {
    fn load(&self, region: Region, word: usize) -> Result<u64, Error> {
        self.step()?;
        self.view.load(region, word)
    }
    fn store(&self, region: Region, word: usize, value: u64) -> Result<(), Error> {
        self.step()?;
        self.view.store(region, word, value)
    }
}

#[test]
fn retirement_at_each_access_fails_closed_without_publication_or_output() {
    for remaining in 0..=38 {
        let pages = AtomicPages::new();
        let mut p = Producer::new(
            Failing {
                view: pages.view(Role::Producer),
                remaining: Cell::new(remaining),
            },
            7,
        )
        .unwrap();
        assert_eq!(p.try_send(&[0x55; PAYLOAD_BYTES]), Err(Error::Closed));
        assert_eq!(pages.view(Role::Consumer).load(Region::Producer, 0), Ok(0));
        assert!(matches!(p.try_reserve(0), Err(Error::Closed)));
    }
    for remaining in 0..=38 {
        let pages = AtomicPages::new();
        let (mut p, _) = pair(&pages);
        p.try_send(&[0x55; PAYLOAD_BYTES]).unwrap();
        let mut c = Consumer::new(
            Failing {
                view: pages.view(Role::Consumer),
                remaining: Cell::new(remaining),
            },
            7,
        )
        .unwrap();
        let mut out = [0xaa; PAYLOAD_BYTES];
        if remaining < 38 {
            assert_eq!(c.try_recv(&mut out), Err(Error::Closed));
            assert_eq!(out, [0xaa; PAYLOAD_BYTES]);
        } else {
            // Copy completed, but reclamation failed. Output contains a safe
            // private copy; the API still reports retirement, never success.
            assert_eq!(c.try_recv(&mut out), Err(Error::Closed));
            assert_eq!(out, [0x55; PAYLOAD_BYTES]);
        }
        assert_eq!(pages.view(Role::Producer).load(Region::Consumer, 0), Ok(0));
    }
}

#[test]
fn explicit_retirement_fences_operations_and_preserves_first_error() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    mem::forget(p.try_reserve(1).unwrap());
    p.retire();
    p.cancel_pending();
    c.retire();
    c.cancel_pending();
    assert!(matches!(p.try_reserve(1), Err(Error::Closed)));
    assert!(matches!(c.try_read(), Err(Error::Closed)));
    let (mut p, _) = pair(&pages);
    put(&pages, Region::Consumer, 0, u64::MAX);
    assert!(matches!(p.try_reserve(1), Err(Error::ConsumerState)));
    p.retire();
    assert!(matches!(p.try_reserve(1), Err(Error::ConsumerState)));
}

#[test]
fn concurrent_cancellation_and_transfer_use_no_shared_references() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    let barrier = Barrier::new(2);
    const COUNT: u64 = 20_000;
    thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            for seq in 0..COUNT {
                let mut tries = 0;
                loop {
                    tries += 1;
                    assert!(tries < 10_000_000);
                    match p.try_reserve(8) {
                        Ok(mut w) => {
                            w.payload_mut().fill(0xcc);
                            drop(w); // unpublished concurrent cancellation
                            break;
                        }
                        Err(Error::Full) => thread::yield_now(),
                        Err(e) => panic!("reserve: {e:?}"),
                    }
                }
                p.try_send(&seq.to_le_bytes()).unwrap();
            }
        });
        barrier.wait();
        for seq in 0..COUNT {
            let mut tries = 0;
            loop {
                tries += 1;
                assert!(tries < 10_000_000);
                match c.try_read() {
                    Ok(r) => {
                        assert_eq!(r.payload(), &seq.to_le_bytes());
                        drop(r); // abandon private read, no reclamation
                        break;
                    }
                    Err(Error::Empty) => thread::yield_now(),
                    Err(e) => panic!("read: {e:?}"),
                }
            }
            let r = c.try_read().unwrap();
            assert_eq!(r.payload(), &seq.to_le_bytes());
            r.release().unwrap();
        }
    });
}

#[test]
fn concurrent_hostile_payload_writes_never_mutate_returned_copy() {
    let pages = AtomicPages::new();
    let (mut p, mut c) = pair(&pages);
    p.try_send(&[0; PAYLOAD_BYTES]).unwrap();
    let barrier = Barrier::new(2);
    thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            for value in 0..20_000 {
                put(&pages, Region::Payload, 0, value);
            }
        });
        barrier.wait();
        for _ in 0..1000 {
            let r = c.try_read().unwrap();
            let saved = r.bytes;
            thread::yield_now();
            assert_eq!(r.bytes, saved);
            drop(r);
        }
    });
    c.try_read().unwrap().release().unwrap();
}

struct Counted<'a> {
    view: View<'a>,
    loads: Cell<usize>,
}
impl Memory for Counted<'_> {
    fn load(&self, region: Region, word: usize) -> Result<u64, Error> {
        assert!(word < WORDS, "derived address escaped a page");
        self.loads.set(self.loads.get() + 1);
        self.view.load(region, word)
    }
    fn store(&self, region: Region, word: usize, value: u64) -> Result<(), Error> {
        self.view.store(region, word, value)
    }
}

#[test]
fn adversarial_descriptor_corpus_has_bounded_work_and_access() {
    // Deterministic hostile values, including partially valid descriptor tuples.
    // This corpus is supplementary to the explicit arithmetic boundary cases.
    let mut seed = 0x8877_6655_4433_2211u64;
    for attempt in 0..5000 {
        let pages = AtomicPages::new();
        let (mut p, _) = pair(&pages);
        p.try_send(&[0x5a; PAYLOAD_BYTES]).unwrap();
        for field in 0..DESCRIPTOR_WORDS {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            if field >= attempt % DESCRIPTOR_WORDS {
                put(&pages, Region::Producer, 1 + field, seed);
            }
        }
        let memory = Counted {
            view: pages.view(Role::Consumer),
            loads: Cell::new(0),
        };
        let mut c = Consumer::new(memory, 7).unwrap();
        if let Ok(r) = c.try_read() {
            assert!(r.payload().len() <= PAYLOAD_BYTES);
        }
        assert!(c.memory.loads.get() <= 6 + PAYLOAD_WORDS);
    }
}
