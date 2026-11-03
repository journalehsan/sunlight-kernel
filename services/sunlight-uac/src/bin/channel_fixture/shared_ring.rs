//! M5.2 direct mapped SPSC. Control syscalls authorize setup and notifications only.
use core::sync::atomic::{compiler_fence, AtomicU64, Ordering};
use ring::{Memory, Region};
static PAUSE: AtomicU64 = AtomicU64::new(0);
static WRITE_PAUSE: AtomicU64 = AtomicU64::new(0);
const COPYING: u64 = 0xf1c572;
const COPIED: u64 = 0xf1c573;
const EMPTY_CHECKED: u64 = 0xf1c574;
// Private to this fixture. Never construct a reference to shared bytes. The
// process has one thread/VM; M2 denies alias replacement while live and uses a
// checked monotone VA bump for later channels. Withdrawal may terminate a raw
// accessor with SIGSEGV, and remote TLB ACKs precede backing reuse. No pointer
// survives exec. No Rust unwinding/destructor or success receipt is required on
// this terminal path. This backend must never be exposed as a general safe API.
struct Native {
    h: fl::Handle,
    parent: CapabilityToken,
    pages: [u64; 3],
    role: u64,
}
impl Native {
    fn address(&self, r: Region, w: usize) -> Result<u64, ring::Error> {
        if w >= ring::WORDS {
            return Err(ring::Error::Access);
        }
        self.pages[r as usize]
            .checked_add((w * 8) as u64)
            .ok_or(ring::Error::Access)
    }
}
impl Memory for Native {
    fn load(&self, r: Region, w: usize) -> Result<u64, ring::Error> {
        let address = self.address(r, w)?;
        let value: u64;
        // SAFETY: fixed initialized M2 page, aligned checked local word. Raw MOV
        // is atomic on x86-64, creates no reference, and may terminally fault
        // after withdrawal. TSO + compiler barrier supply Acquire semantics.
        unsafe {
            core::arch::asm!("mov {value}, qword ptr [{address}]", value=out(reg) value, address=in(reg) address, options(nostack));
        }
        compiler_fence(Ordering::Acquire);
        if r == Region::Payload && PAUSE.swap(0, Ordering::Relaxed) != 0 {
            sunlight_ipc::ipc_call_timeout(
                self.parent,
                IpcMsg::with_label(COPYING).word(0, self.h.generation),
                10000,
            )
            .expect("copy boundary");
        }
        Ok(value)
    }
    fn store(&self, r: Region, w: usize, value: u64) -> Result<(), ring::Error> {
        if self.role != if r == Region::Consumer { 1 } else { 0 } {
            return Err(ring::Error::Access);
        }
        let address = self.address(r, w)?;
        compiler_fence(Ordering::Release);
        // SAFETY: same fixed page/lifetime contract as load, plus role-owned
        // writable page. TSO + compiler barrier supply Release semantics.
        unsafe {
            core::arch::asm!("mov qword ptr [{address}], {value}", address=in(reg) address, value=in(reg) value, options(nostack));
        }
        if r == Region::Payload && WRITE_PAUSE.swap(0, Ordering::Relaxed) != 0 {
            sunlight_ipc::ipc_call_timeout(
                self.parent,
                IpcMsg::with_label(COPYING).word(0, self.h.generation),
                10000,
            )
            .expect("publication boundary");
        }
        Ok(())
    }
}
use super::*;
use sunlight_fastlane_ring::hostile::{self as ring};
#[cfg(feature = "fastlane-measurement-test")]
mod measurement;
#[cfg(feature = "fastlane-benchmark-test")]
pub(crate) mod benchmark;
#[cfg(feature = "fastlane-benchmark-test")]
pub(crate) mod fair_benchmark;
const DONE: u64 = 0xf1c571;
fn bytes(seq: u64) -> [u8; 256] {
    core::array::from_fn(|i| (seq.wrapping_mul(17).wrapping_add(i as u64 * 3) % 251) as u8)
}
fn len(seq: u64) -> usize {
    (seq % 257) as usize
}
enum Role {
    Producer(ring::Producer<Native>),
    Consumer(ring::Consumer<Native>),
}
pub(super) struct Endpoint {
    role: Option<Role>,
    next: u64,
    h: fl::Handle,
    mode: u64,
    pages: [u64; 3],
    #[cfg(feature = "fastlane-measurement-test")]
    measure: measurement::Counters,
}
impl Endpoint {
    pub fn new() -> Self {
        Self {
            role: None,
            next: 0,
            h: fl::EMPTY,
            mode: 0,
            pages: [0; 3],
            #[cfg(feature = "fastlane-measurement-test")]
            measure: measurement::Counters::new(),
        }
    }
    pub fn command(&mut self, op: u64, h: fl::Handle, arg: u64, parent: CapabilityToken) {
        if op == 70 {
            assert!(arg <= 1);
            let mut pages = [0; 3];
            for r in 0..3 {
                let (va, size) = fl::call(8, h, arg * 3 + r, fl::NO_SESSION).unwrap();
                assert_eq!(size, 4096);
                assert_eq!(va % 4096, 0);
                assert!((0x1000..0x0000_0004_0000_0000).contains(&va));
                pages[r as usize] = va;
            }
            fl::operate(24, h, arg).unwrap();
            assert_eq!(fl::operate(24, h, arg), Err(fl::Error::Duplicate));
            assert_eq!(fl::transfer_word(h, false, 0, 0), Err(fl::Error::Denied));
            self.pages = pages;
            let memory = Native {
                h,
                parent,
                pages,
                role: arg,
            };
            self.role = Some(if arg == 0 {
                Role::Producer(ring::Producer::new(memory, h.generation).unwrap())
            } else {
                Role::Consumer(ring::Consumer::new(memory, h.generation).unwrap())
            });
            #[cfg(feature = "fastlane-measurement-test")]
            {
                self.measure = measurement::Counters::new();
            }
            self.next = 0;
            self.mode = 0;
            self.h = h;
            reply(0, 0, h.generation);
            return;
        }
        assert_eq!(self.h, h);
        match op {
            71 => {
                let Some(Role::Producer(p)) = &mut self.role else {
                    panic!("producer")
                };
                assert!((1..=16).contains(&arg));
                if self.mode == 5 {
                    reply(0, self.next, h.generation);
                    WRITE_PAUSE.store(1, Ordering::Relaxed);
                }
                for _ in 0..arg {
                    let b = bytes(self.next);
                    #[cfg(feature = "fastlane-measurement-test")]
                    let timestamp = self
                        .measure
                        .wants_sample()
                        .then(sunlight_ipc::monotonic_millis);
                    if let Err(e) = p.try_send(&b[..len(self.next)]) {
                        #[cfg(feature = "fastlane-measurement-test")]
                        if e == ring::Error::Full {
                            self.measure.full();
                        }
                        reply(
                            if e == ring::Error::Full {
                                5
                            } else if e == ring::Error::Exhausted {
                                6
                            } else {
                                106
                            },
                            self.next,
                            h.generation,
                        );
                        return;
                    }
                    #[cfg(feature = "fastlane-measurement-test")]
                    self.measure
                        .success(self.next, len(self.next) as u64, timestamp);
                    #[cfg(feature = "fastlane-measurement-test")]
                    if !self.measure.compact {
                        sunlight_ipc::debug_log(&alloc::format!("[FL-M52-MSG] event=producer generation={} message={} slot_generation={} bytes={} pid={}",h.generation,self.next,self.next/16+1,len(self.next),sunlight_libc::getpid()));
                    }
                    #[cfg(not(feature = "fastlane-measurement-test"))]
                    sunlight_ipc::debug_log(&alloc::format!("[FL-M52-MSG] event=producer generation={} message={} slot_generation={} bytes={} pid={}",h.generation,self.next,self.next/16+1,len(self.next),sunlight_libc::getpid()));
                    self.next += 1;
                }
                reply(0, self.next, h.generation);
            }
            72 => {
                let Some(Role::Consumer(c)) = &mut self.role else {
                    panic!("consumer")
                };
                assert!((1..=16).contains(&arg));
                reply(0, 0, h.generation);
                let mut outcome = 0;
                if (1..=3).contains(&self.mode) {
                    PAUSE.store(1, Ordering::Relaxed);
                }
                for _ in 0..arg {
                    outcome = receive_one(
                        c,
                        h,
                        parent,
                        self.next,
                        self.mode,
                        #[cfg(feature = "fastlane-measurement-test")]
                        &mut self.measure,
                    );
                    if outcome != 0 {
                        break;
                    }
                    self.next += 1;
                }
                sunlight_ipc::ipc_call_timeout(
                    parent,
                    IpcMsg::with_label(DONE)
                        .word(0, h.generation)
                        .word(1, self.next)
                        .word(2, outcome),
                    10000,
                )
                .expect("consumer count receipt");
            }
            73 => {
                let Some(Role::Producer(p)) = &mut self.role else {
                    panic!("producer");
                };
                // A dropped reservation cancels without publishing. The same
                // sequence can be reserved again, with a fresh private serial.
                let lease = p.try_reserve(arg as usize).unwrap();
                assert_eq!(lease.identity().sequence, self.next);
                drop(lease);
                reply(0, self.next, h.generation);
            }
            74 | 75 | 76 => {
                let memory = Native {
                    h,
                    parent,
                    pages: self.pages,
                    role: if op == 75 { 1 } else { 0 },
                };
                let (region, word) = if op == 75 {
                    (Region::Consumer, 0)
                } else {
                    (Region::Producer, 0)
                };
                let code = memory.store(region, word, arg).err().map_or(0, |_| 104);
                reply(code, 0, h.generation);
            }
            77 => {
                // Structurally malformed descriptor at the current canonical slot.
                let slot = self.next.checked_sub(1).unwrap() % 16;
                let base = 1 + slot * 5;
                let (field, value) = match arg {
                    0 => (4, 257),
                    1 => (3, 4096),
                    2 => (2, 0),
                    _ => (0, h.generation + 1),
                };
                let memory = Native {
                    h,
                    parent,
                    pages: self.pages,
                    role: 0,
                };
                let code = memory
                    .store(Region::Producer, (base + field) as usize, value)
                    .err()
                    .map_or(0, |_| 104);
                let v = 0;
                reply(code, v, h.generation);
            }
            78 => {
                self.mode = arg;
                reply(0, 0, h.generation);
            }
            79 => {
                assert!(arg < 16);
                for w in 0..32 {
                    Native {
                        h,
                        parent,
                        pages: self.pages,
                        role: 0,
                    }
                    .store(Region::Payload, (arg * 32 + w) as usize, 0x2222222222222222)
                    .unwrap();
                }
                reply(0, 0, h.generation);
            }
            80 => {
                // Isolated seeded arithmetic fixture on a fresh attached channel.
                assert_eq!(self.next, 0);
                let producer = matches!(self.role, Some(Role::Producer(_)));
                let memory = Native {
                    h,
                    parent,
                    pages: self.pages,
                    role: u64::from(!producer),
                };
                memory
                    .store(
                        if producer {
                            Region::Producer
                        } else {
                            Region::Consumer
                        },
                        0,
                        arg,
                    )
                    .unwrap();
                self.role = Some(if producer {
                    Role::Producer(ring::Producer::fixture_at(memory, h.generation, arg).unwrap())
                } else {
                    Role::Consumer(ring::Consumer::fixture_at(memory, h.generation, arg).unwrap())
                });
                self.next = arg;
                reply(0, arg, h.generation);
            }
            82 => {
                let Some(Role::Consumer(c)) = &mut self.role else {
                    panic!("consumer");
                };
                assert!(matches!(c.try_read(), Err(ring::Error::Exhausted)));
                assert!(matches!(c.try_read(), Err(ring::Error::Exhausted)));
                reply(6, self.next, h.generation);
            }
            #[cfg(feature = "fastlane-measurement-test")]
            83 => {
                // arg=0: warm-up; arg=1: independently reset the measured window.
                assert!(arg <= 1);
                if arg == 1 {
                    fl::operate(26, h, 0).unwrap();
                }
                self.measure
                    .begin(self.next, arg == 1, sunlight_ipc::monotonic_millis());
                reply(0, self.next, h.generation);
            }
            #[cfg(feature = "fastlane-measurement-test")]
            84 => {
                self.measure.finish(sunlight_ipc::monotonic_millis());
                fl::operate(27, h, 0).unwrap();
                self.measure.report(h.generation, self.next);
                reply(0, self.next, h.generation);
            }
            #[cfg(feature = "fastlane-measurement-test")]
            85 => {
                measurement::clock_probe(arg);
                reply(0, self.next, h.generation);
            }
            #[cfg(feature = "fastlane-measurement-test")]
            86 => {
                // arg is the CPU placement to restore after the probe.
                assert!(arg <= 1);
                measurement::qualified_clock_probe(arg);
                reply(0, self.next, h.generation);
            }
            _ => panic!("shared ring command"),
        }
    }
}
// Observe the Doorbell before calling the hostile adapter. The adapter must
// decide Empty versus malformed/rolled-back/exhausted; a raw head precheck must
// never bypass its private observation history or checked terminal state.
fn receive_one(
    c: &mut ring::Consumer<Native>,
    h: fl::Handle,
    parent: CapabilityToken,
    next: u64,
    mode: u64,
    #[cfg(feature = "fastlane-measurement-test")] measure: &mut measurement::Counters,
) -> u64 {
    let mut pause_empty = mode == 6;
    for _ in 0..16 {
        let observed = match fl::operate(11, h, 0) {
            Ok(v) => v,
            Err(e) => return e as u64,
        };
        let lease = match c.try_read() {
            Ok(lease) => lease,
            Err(ring::Error::Empty) => {
                if pause_empty {
                    pause_empty = false;
                    sunlight_ipc::ipc_call_timeout(
                        parent,
                        IpcMsg::with_label(EMPTY_CHECKED).word(0, h.generation),
                        10000,
                    )
                    .expect("empty check boundary");
                }
                match fl::doorbell_wait(h, observed) {
                    Ok(()) => {
                        if let Err(e) = fl::operate(13, h, 0) {
                            return e as u64;
                        }
                    }
                    Err(fl::Error::Changed) => (),
                    Err(e) => return e as u64,
                }
                continue;
            }
            Err(e) => {
                return match e {
                    ring::Error::Length => 101,
                    ring::Error::Offset => 102,
                    ring::Error::Identity => 103,
                    ring::Error::ProducerState => 107,
                    ring::Error::Exhausted => 6,
                    _ => 104,
                }
            }
        };
        assert_eq!(lease.identity().channel, h.generation);
        assert_eq!(lease.identity().sequence, next);
        assert_eq!(lease.identity().generation, next / 16 + 1);
        assert_eq!(lease.integrity(), ring::Integrity::PRIVATE_COPY);
        let mut expected = bytes(next);
        if mode == 1 {
            expected[8..len(next)].fill(0x22);
        }
        assert_eq!(lease.payload(), &expected[..len(next)]);
        if mode == 1 || mode == 4 {
            sunlight_ipc::ipc_call_timeout(
                parent,
                IpcMsg::with_label(COPIED).word(0, h.generation),
                10000,
            )
            .expect("private copy boundary");
            assert_eq!(lease.payload(), &expected[..len(next)]);
        }
        if lease.release().is_err() {
            return 104;
        }
        #[cfg(feature = "fastlane-measurement-test")]
        {
            let timestamp = measure.wants_sample().then(sunlight_ipc::monotonic_millis);
            measure.success(next, len(next) as u64, timestamp);
        }
        #[cfg(feature = "fastlane-measurement-test")]
        if !measure.compact {
            sunlight_ipc::debug_log(&alloc::format!("[FL-M52-MSG] event=consumer generation={} message={} slot_generation={} bytes={} pid={} coherent=0 authentic=0",h.generation,next,next/16+1,len(next),sunlight_libc::getpid()));
        }
        #[cfg(not(feature = "fastlane-measurement-test"))]
        sunlight_ipc::debug_log(&alloc::format!("[FL-M52-MSG] event=consumer generation={} message={} slot_generation={} bytes={} pid={} coherent=0 authentic=0",h.generation,next,next/16+1,len(next),sunlight_libc::getpid()));
        return 0;
    }
    106
}
fn done(ep: EndpointId, c: Actor, h: fl::Handle, count: u64, code: u64) {
    let m = sunlight_ipc::ipc_recv_timeout(ep, 10000).expect("consumer receipt");
    assert_eq!(m.badge, c.pid);
    assert_eq!(m.label, DONE);
    assert_eq!(m.words[..3], [h.generation, count, code]);
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(DONE)).unwrap();
}
fn setup(p: Actor, c: Actor) -> fl::Handle {
    let h = doorbell::channel(p, c);
    expect(p, 80, h, 0, 0);
    expect(c, 80, h, 1, 0);
    h
}
fn clean(p: Actor, h: fl::Handle, checkpoint: u64) {
    let started = sunlight_ipc::monotonic_millis();
    expect(p, 4, h, 0, 0);
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 0);
    mapping::census(checkpoint);
    zero();
    fl::operate(25, fl::EMPTY, 0).unwrap();
    sunlight_ipc::debug_log(&alloc::format!("[FL-M52-CLEAN] generation={} checkpoint={} duration_ms={} backing_allocations=3 data_plane_allocations=0", h.generation,checkpoint,sunlight_ipc::monotonic_millis() - started));
}
fn mark(case: &str, h: fl::Handle, count: u64) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M52] case={} generation={} count={} pid={}",
        case,
        h.generation,
        count,
        sunlight_libc::getpid()
    ));
}
pub fn supervisor() -> ! {
    doorbell::pin(0);
    mark("start", fl::EMPTY, 0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let mut p = doorbell::spawn(ep, public, 0);
    let mut c = doorbell::spawn(ep, public, 1);
    for (cycle, count) in [1u64, 16, 1000, 10000].into_iter().enumerate() {
        let foreign = doorbell::spawn(ep, public, 1);
        #[cfg(feature = "fastlane-measurement-test")]
        let setup_start = sunlight_ipc::monotonic_millis();
        let h = setup(p, c);
        #[cfg(feature = "fastlane-measurement-test")]
        sunlight_ipc::debug_log(&alloc::format!(
            "[FL-M53-SETUP] generation={} start_ms={} end_ms={}",
            h.generation,
            setup_start,
            sunlight_ipc::monotonic_millis()
        ));
        expect(foreign, 91, h, 0, 1);
        expect(foreign, 91, h, 1, 1);
        expect(c, 91, h, 0, 1);
        expect(p, 91, h, 1, 1);
        expect(
            p,
            91,
            fl::Handle {
                generation: h.generation + 1,
                ..h
            },
            0,
            2,
        );
        expect(p, 3, h, 0, 3);
        expect(c, 3, h, 1, 3);
        stop(foreign);
        // Cancel a reservation without publication, then reserve the same sequence.
        expect(p, 83, h, 0, 0);
        expect(p, 83, h, 0, 0);
        #[cfg(feature = "fastlane-measurement-test")]
        {
            // Probe the same public clock before/after affinity migration. Restore
            // the fixture's original producer CPU 0 / consumer CPU 1 placement.
            expect(p, 95, h, 1, 0);
            expect(p, 95, h, 0, 0);
            expect(c, 95, h, 0, 0);
            expect(c, 95, h, 1, 0);
            expect(p, 93, h, 0, 0);
            expect(c, 93, h, 0, 0);
            // M5.4 qualified clock: local probes on CPU 0/1, a rejected
            // cross-CPU pair, and the deterministic rejection self-check. The
            // probe restores the fixture's original producer CPU 0 / consumer
            // CPU 1 placement.
            expect(p, 96, h, 0, 0);
            expect(c, 96, h, 1, 0);
        }
        #[cfg(feature = "fastlane-measurement-test")]
        let warmup = if count > 16 { 16 } else { 0 };
        let mut sent = 0;
        while sent < count {
            #[cfg(feature = "fastlane-measurement-test")]
            if sent == warmup {
                expect(p, 93, h, 1, 0);
                expect(c, 93, h, 1, 0);
            }
            let batch = (count - sent).min(16);
            assert_eq!(expect(p, 81, h, batch, 0).words[1], sent + batch);
            if batch == 16 {
                expect(p, 81, h, 1, 5);
                mark("full", h, sent);
            }
            // Multiple notifications before wait are hints, never message counts.
            expect(p, 40, h, 0, 0);
            expect(p, 40, h, 0, 0);
            expect(c, 82, h, batch, 0);
            sent += batch;
            done(ep, c, h, sent, 0);
        }
        #[cfg(feature = "fastlane-measurement-test")]
        {
            expect(p, 94, h, 0, 0);
            expect(c, 94, h, 0, 0);
        }
        clean(p, h, 600 + cycle as u64);
        expect(p, 40, h, 0, 4);
        expect(p, 91, h, 0, 4);
        mark("cycle", h, count);
    }
    // Slow producer: actual consumer is parked before publication.
    let h = setup(p, c);
    expect(c, 82, h, 1, 0);
    assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 1 | (1 << 32));
    expect(p, 40, h, 0, 0);
    assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 1 | (1 << 32));
    expect(p, 81, h, 1, 0);
    expect(p, 40, h, 0, 0);
    done(ep, c, h, 1, 0);
    clean(p, h, 610);
    mark("slow-producer", h, 1);
    let h = setup(p, c);
    expect(c, 82, h, 1, 0);
    assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 1 | (1 << 32));
    expect(p, 44, h, 0, 0);
    done(ep, c, h, 0, 9);
    clean(p, h, 611);
    mark("cancel", h, 0);
    // All adversarial channels first transfer and acknowledge a full batch.
    // The next publication reuses slot zero with a fresh slot generation.
    for bad in 0..4 {
        let h = setup(p, c);
        prime(ep, p, c, h);
        expect(p, 81, h, 1, 0);
        expect(p, 87, h, bad, 0);
        expect(c, 82, h, 1, 0);
        done(
            ep,
            c,
            h,
            16,
            if bad == 0 {
                101
            } else if bad == 1 {
                102
            } else {
                103
            },
        );
        clean(p, h, 620 + bad);
        mark("malformed", h, bad);
    }
    for mode in 1..=4 {
        let h = setup(p, c);
        prime(ep, p, c, h);
        expect(p, 81, h, 1, 0);
        expect(c, 88, h, mode, 0);
        expect(c, 82, h, 1, 0);
        if mode <= 3 {
            boundary(ep, c, h, COPYING);
            if mode == 1 {
                expect(p, 89, h, 0, 0);
            } // actual producer mutation during copy
            if mode == 2 {
                expect(p, 4, h, 0, 0);
            } // revoke with acquired data
            if mode == 3 {
                sunlight_libc::kill(c.pid, 9).unwrap();
                assert_eq!(super::super::reap(c.pid), 137);
                expect(p, 40, h, 0, 4);
                clean(p, h, 630 + mode);
                mark("consumer-exit-copy", h, 16);
                // Ordinary IPC requires resolving the retained server reply
                // identity even after its caller exits. This bounded late reply
                // is discarded by the existing exact-generation IPC path.
                sunlight_ipc::ipc_reply_result(IpcMsg::with_label(COPYING)).unwrap();
                sunlight_ipc::debug_log(&alloc::format!("[FL-M52-TERMINAL] generation={} operation=copy-caller exit=137 notify=4 late_reply=0",h.generation));
                c = doorbell::spawn(ep, public, 1);
                continue;
            }
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(COPYING)).unwrap();
            if mode == 2 {
                assert_eq!(super::super::reap(c.pid), 139);
                clean(p, h, 630 + mode);
                mark("revoke-copy-fault", h, 16);
                c = doorbell::spawn(ep, public, 1);
                continue;
            }
        }
        if mode == 1 || mode == 4 {
            boundary(ep, c, h, COPIED);
            if mode == 1 {
                expect(p, 89, h, 0, 0);
            } else {
                expect(p, 4, h, 0, 0);
            }
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(COPIED)).unwrap();
            if mode == 4 {
                assert_eq!(super::super::reap(c.pid), 139);
                clean(p, h, 630 + mode);
                mark("close-ack-fault", h, 16);
                c = doorbell::spawn(ep, public, 1);
                continue;
            }
        }
        done(
            ep,
            c,
            h,
            if mode == 1 { 17 } else { 16 },
            if mode == 1 { 0 } else { 104 },
        );
        clean(p, h, 630 + mode);
        mark(
            if mode == 1 {
                "mutation"
            } else if mode == 2 {
                "revoke-copy"
            } else {
                "close-ack"
            },
            h,
            16,
        );
    }
    // Replay the same published/acknowledged cursors after a full reuse cycle.
    // Neither creates another message or another capacity credit.
    let h = setup(p, c);
    prime(ep, p, c, h);
    expect(p, 84, h, 16, 0);
    expect(c, 85, h, 16, 0);
    expect(p, 40, h, 0, 0);
    expect(c, 82, h, 1, 0);
    assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
    expect(p, 81, h, 1, 0);
    expect(p, 40, h, 0, 0);
    done(ep, c, h, 17, 0);
    clean(p, h, 650);
    mark("duplicate-cursors", h, 17);
    // A rollback or an invented acknowledgement credit is terminal, bounded.
    for bad in 0..2 {
        let h = setup(p, c);
        prime(ep, p, c, h);
        if bad == 0 {
            expect(p, 84, h, 15, 0);
            expect(c, 82, h, 1, 0);
            done(ep, c, h, 16, 107);
        } else {
            expect(c, 85, h, 17, 0);
            expect(p, 81, h, 1, 106);
        }
        clean(p, h, 651 + bad);
        mark("invalid-cursor", h, bad);
    }
    // Seed only fresh native endpoints to exercise arithmetic boundaries without
    // claiming to have naturally transferred 2^64 messages or allowing wrap.
    for (index, seed) in [(0, (1u64 << 32) - 1), (1, u64::MAX - 2)] {
        let h = setup(p, c);
        expect(p, 90, h, seed, 0);
        expect(c, 90, h, seed, 0);
        expect(p, 81, h, 2, 0);
        expect(p, 40, h, 0, 0);
        expect(c, 82, h, 2, 0);
        done(ep, c, h, seed + 2, 0);
        if index == 1 {
            expect(p, 81, h, 1, 6);
            expect(c, 82, h, 1, 0);
            done(ep, c, h, u64::MAX, 6);
            expect(c, 92, h, 0, 6);
        }
        clean(p, h, 660 + index);
        mark(
            if index == 0 {
                "seeded-rollover"
            } else {
                "seeded-exhaustion"
            },
            h,
            seed,
        );
    }
    for mode in 1..=2 {
        let h = setup(p, c);
        expect(p, 45, h, mode, 0);
        expect(c, 88, h, 6, 0);
        expect(c, 82, h, 1, 0);
        boundary(ep, c, h, EMPTY_CHECKED);
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(EMPTY_CHECKED)).unwrap();
        expect(p, 81, h, 1, 0);
        expect(p, 40, h, 0, 0);
        done(ep, c, h, 1, 0);
        clean(p, h, 665 + mode);
        mark("registration-race", h, mode);
    }
    for mode in 3..=4 {
        let h = setup(p, c);
        expect(p, 81, h, 1, 0);
        expect(p, 45, h, mode, 0);
        expect(c, RACE_EXIT, h, 0, 0);
        expect(p, 40, h, 0, if mode == 3 { 4 } else { 0 });
        assert_eq!(super::super::reap(c.pid), 0);
        clean(p, h, 675 + mode);
        mark("close-notify-race", h, mode);
        c = doorbell::spawn(ep, public, 1);
    }
    // Close an actual registered ring reader, then reject its stale notification.
    let h = setup(p, c);
    expect(c, 82, h, 1, 0);
    assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 1 | (1 << 32));
    expect(p, 4, h, 0, 0);
    expect(p, 40, h, 0, 4);
    done(ep, c, h, 0, 4);
    clean(p, h, 670);
    mark("close-wait", h, 0);
    // Cancellation during publication: withdraw mappings while the producer has
    // written one bounded word, before descriptor/cursor publication.
    let h = setup(p, c);
    expect(p, 88, h, 5, 0);
    expect(p, 81, h, 1, 0);
    boundary(ep, p, h, COPYING);
    expect(c, 4, h, 0, 0);
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(COPYING)).unwrap();
    assert_eq!(super::super::reap(p.pid), 139);
    // Consumer can revoke, but M4's diagnostic inventory is creator-only.
    mapping::census(671);
    zero();
    mark("cancel-publication-fault", h, 0);
    p = doorbell::spawn(ep, public, 0);
    // Observe sixteen queued messages, acknowledge just one, then hide the
    // outstanding messages by rolling head back to the current consumer cursor.
    // This must reject rather than register an apparently empty ring wait.
    let h = setup(p, c);
    expect(p, 81, h, 16, 0);
    expect(c, 82, h, 1, 0);
    done(ep, c, h, 1, 0);
    expect(p, 84, h, 1, 0);
    expect(c, 82, h, 1, 0);
    done(ep, c, h, 1, 107);
    // Restoring valid shared state cannot resurrect a poisoned local endpoint.
    expect(p, 84, h, 16, 0);
    expect(c, 82, h, 1, 0);
    done(ep, c, h, 1, 107);
    clean(p, h, 680);
    mark("rollback-empty", h, 1);
    let h = setup(p, c);
    prime(ep, p, c, h);
    expect(p, 81, h, 1, 0);
    stop(p);
    expect(c, 82, h, 1, 0);
    done(ep, c, h, 16, 4);
    mapping::census(640);
    zero();
    mark("producer-exit", h, 16);
    p = doorbell::spawn(ep, public, 0);
    stop(p);
    stop(c);
    assert!(sunlight_ipc::endpoint_destroy(ep));
    zero();
    mark("complete", fl::EMPTY, 0);
    sunlight_libc::exit(0)
}

fn prime(ep: EndpointId, p: Actor, c: Actor, h: fl::Handle) {
    expect(p, 81, h, 16, 0);
    expect(c, 82, h, 16, 0);
    done(ep, c, h, 16, 0);
}
fn boundary(ep: EndpointId, c: Actor, h: fl::Handle, label: u64) {
    let m = sunlight_ipc::ipc_recv_timeout(ep, 10000).expect("native boundary");
    assert_eq!(m.badge, c.pid);
    assert_eq!(m.label, label);
    assert_eq!(m.words[0], h.generation);
}
