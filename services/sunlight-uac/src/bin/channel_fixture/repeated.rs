//! M5.1 opt-in native peers. One attachment and three mappings per role/channel.
use super::transfer::Native as Backend;
use core::sync::atomic::{AtomicU64, Ordering};
use ring::{Memory, Region};
static PAUSE: AtomicU64 = AtomicU64::new(0);
const COPYING: u64 = 0xf1c562;
const COPIED: u64 = 0xf1c563;
struct Native {
    h: fl::Handle,
    parent: CapabilityToken,
}
impl Memory for Native {
    fn load(&self, r: Region, w: usize) -> Result<u64, ring::Error> {
        let value = Backend {
            h: self.h,
            parent: self.parent,
        }
        .load(r, w)?;
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
    fn store(&self, r: Region, w: usize, v: u64) -> Result<(), ring::Error> {
        Backend {
            h: self.h,
            parent: self.parent,
        }
        .store(r, w, v)
    }
}
use super::*;
use sunlight_fastlane_ring::hostile::{self as ring};
const DONE: u64 = 0xf1c561;
fn control(h: fl::Handle, op: u64, seq: u64) -> Result<u64, fl::Error> {
    fl::transfer_word(h, true, op, seq)
}
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
}
impl Endpoint {
    pub fn new() -> Self {
        Self {
            role: None,
            next: 0,
            h: fl::EMPTY,
            mode: 0,
        }
    }
    pub fn command(&mut self, op: u64, h: fl::Handle, arg: u64, parent: CapabilityToken) {
        if op == 70 {
            assert!(arg <= 1);
            for r in 0..3 {
                assert_eq!(fl::call(8, h, arg * 3 + r, fl::NO_SESSION).unwrap().1, 4096);
            }
            if arg == 0 {
                control(h, 1536, 0).unwrap();
            }
            self.role = Some(if arg == 0 {
                Role::Producer(ring::Producer::new(Native { h, parent }, h.generation).unwrap())
            } else {
                Role::Consumer(ring::Consumer::new(Native { h, parent }, h.generation).unwrap())
            });
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
                for _ in 0..arg {
                    if let Err(e) = control(h, 1537, self.next) {
                        reply(e as u64, self.next, h.generation);
                        return;
                    }
                    let b = bytes(self.next);
                    p.try_send(&b[..len(self.next)]).unwrap();
                    sunlight_ipc::debug_log(&alloc::format!("[FL-M51-MSG] event=producer generation={} message={} slot_generation={} bytes={} pid={}",h.generation,self.next,self.next/16+1,len(self.next),sunlight_libc::getpid()));
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
                    let mut available = false;
                    for _ in 0..16 {
                        let observed = match fl::operate(11, h, 0) {
                            Ok(v) => v,
                            Err(e) => {
                                outcome = e as u64;
                                break;
                            }
                        };
                        let head = match fl::transfer_word(h, false, 0, 0) {
                            Ok(v) => v,
                            Err(e) => {
                                outcome = e as u64;
                                break;
                            }
                        };
                        if head > self.next {
                            available = true;
                            break;
                        }
                        match fl::doorbell_wait(h, observed) {
                            Ok(()) => {
                                if let Err(e) = fl::operate(13, h, 0) {
                                    outcome = e as u64;
                                    break;
                                }
                            }
                            Err(fl::Error::Changed) => (),
                            Err(e) => {
                                outcome = e as u64;
                                break;
                            }
                        }
                    }
                    if !available {
                        if outcome == 0 {
                            outcome = 106;
                        }
                        break;
                    }
                    control(h, 1538, self.next).unwrap();
                    let lease = match c.try_read() {
                        Ok(v) => v,
                        Err(e) => {
                            outcome = match e {
                                ring::Error::Length => 101,
                                ring::Error::Offset => 102,
                                ring::Error::Identity => 103,
                                _ => 104,
                            };
                            break;
                        }
                    };
                    assert_eq!(lease.identity().sequence, self.next);
                    assert_eq!(lease.identity().generation, self.next / 16 + 1);
                    assert_eq!(lease.integrity(), ring::Integrity::PRIVATE_COPY);
                    let mut expected = bytes(self.next);
                    if self.mode == 1 {
                        expected[8..len(self.next)].fill(0x22);
                    }
                    assert_eq!(lease.payload(), &expected[..len(self.next)]);
                    if self.mode == 1 || self.mode == 4 {
                        sunlight_ipc::ipc_call_timeout(
                            parent,
                            IpcMsg::with_label(COPIED).word(0, h.generation),
                            10000,
                        )
                        .expect("private copy boundary");
                        assert_eq!(lease.payload(), &expected[..len(self.next)]);
                    }
                    if control(h, 1539, self.next).is_err() {
                        outcome = 104;
                        break;
                    }
                    if lease.release().is_err() {
                        outcome = 104;
                        break;
                    }
                    sunlight_ipc::debug_log(&alloc::format!("[FL-M51-MSG] event=consumer generation={} message={} slot_generation={} bytes={} pid={} coherent=0 authentic=0",h.generation,self.next,self.next/16+1,len(self.next),sunlight_libc::getpid()));
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
                let (code, v) = result(control(h, 1537, arg));
                reply(code, v, h.generation);
            }
            74 => {
                let (code, v) = result(fl::transfer_word(h, true, 0, arg));
                reply(code, v, h.generation);
            }
            75 => {
                let (code, v) = result(fl::transfer_word(h, true, 512, arg));
                reply(code, v, h.generation);
            }
            76 => {
                let (code, v) = result(control(h, 1540, arg));
                reply(code, v, h.generation);
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
                let (code, v) = result(fl::transfer_word(h, true, base + field, value));
                reply(code, v, h.generation);
            }
            78 => {
                self.mode = arg;
                reply(0, 0, h.generation);
            }
            79 => {
                assert!(arg < 16);
                for w in 0..32 {
                    fl::transfer_word(h, true, 1024 + arg * 32 + w, 0x2222222222222222).unwrap();
                }
                reply(0, 0, h.generation);
            }
            _ => panic!("repeated command"),
        }
    }
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
    expect(p, 70, h, 0, 0);
    expect(c, 70, h, 1, 0);
    h
}
fn clean(p: Actor, h: fl::Handle, checkpoint: u64) {
    expect(p, 4, h, 0, 0);
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 0);
    mapping::census(checkpoint);
    zero();
}
fn mark(case: &str, h: fl::Handle, count: u64) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M51] case={} generation={} count={} pid={}",
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
        let h = setup(p, c);
        // Cancel a reservation without publication, then reserve the same sequence.
        expect(p, 73, h, 0, 0);
        expect(p, 73, h, 0, 3);
        expect(p, 76, h, 0, 0);
        let mut sent = 0;
        while sent < count {
            let batch = (count - sent).min(16);
            assert_eq!(expect(p, 71, h, batch, 0).words[1], sent + batch);
            expect(p, 74, h, sent + batch, 2); // duplicate publication, including after wrap
            expect(c, 75, h, sent + 1, 3); // no acquisition/copy completion yet
            if batch == 16 {
                expect(p, 71, h, 1, 5);
                mark("full", h, sent);
            }
            // Multiple notifications before wait are hints, never message counts.
            expect(p, 40, h, 0, 0);
            expect(p, 40, h, 0, 0);
            expect(c, 72, h, batch, 0);
            sent += batch;
            done(ep, c, h, sent, 0);
            expect(c, 75, h, sent, 2); // duplicate acknowledgement
            if sent > 16 {
                expect(c, 75, h, sent - 16, 2);
            }
            assert_eq!(expect(p, 66, h, 0, 0).words[1], sent);
        }
        clean(p, h, 600 + cycle as u64);
        mark("cycle", h, count);
    }
    // Slow producer: actual consumer is parked before publication.
    let h = setup(p, c);
    expect(c, 72, h, 1, 0);
    assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 1 | (1 << 32));
    expect(p, 71, h, 1, 0);
    expect(p, 40, h, 0, 0);
    done(ep, c, h, 1, 0);
    clean(p, h, 610);
    mark("slow-producer", h, 1);
    let h = setup(p, c);
    expect(c, 72, h, 1, 0);
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
        expect(p, 71, h, 1, 0);
        expect(p, 77, h, bad, 0);
        expect(c, 72, h, 1, 0);
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
        expect(p, 71, h, 1, 0);
        expect(c, 78, h, mode, 0);
        expect(c, 72, h, 1, 0);
        if mode <= 3 {
            boundary(ep, c, h, COPYING);
            if mode == 1 {
                expect(p, 79, h, 0, 0);
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
                sunlight_ipc::debug_log(&alloc::format!("[FL-M51-TERMINAL] generation={} operation=copy-caller exit=137 notify=4 late_reply=0",h.generation));
                c = doorbell::spawn(ep, public, 1);
                continue;
            }
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(COPYING)).unwrap();
        }
        if mode == 1 || mode == 4 {
            boundary(ep, c, h, COPIED);
            if mode == 1 {
                expect(p, 79, h, 0, 0);
            } else {
                expect(p, 4, h, 0, 0);
            }
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(COPIED)).unwrap();
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
    let h = setup(p, c);
    prime(ep, p, c, h);
    expect(p, 71, h, 1, 0);
    stop(p);
    expect(c, 72, h, 1, 0);
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
    expect(p, 71, h, 16, 0);
    expect(c, 72, h, 16, 0);
    done(ep, c, h, 16, 0);
}
fn boundary(ep: EndpointId, c: Actor, h: fl::Handle, label: u64) {
    let m = sunlight_ipc::ipc_recv_timeout(ep, 10000).expect("native boundary");
    assert_eq!(m.badge, c.pid);
    assert_eq!(m.label, label);
    assert_eq!(m.words[0], h.generation);
}
