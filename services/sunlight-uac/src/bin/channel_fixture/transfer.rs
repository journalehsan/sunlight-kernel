//! One native message per freshly authenticated channel. All payload parsing and
//! private copying run in the independent consumer VM, using M3.2 unchanged.
use super::*;
use core::sync::atomic::{AtomicBool, Ordering};
static INTERLEAVE: AtomicBool = AtomicBool::new(false);
use sunlight_fastlane_ring::hostile::{self as ring, Memory, Region};
const DONE: u64 = 0xf1c550;
const SNAPSHOT: u64 = 0xf1c551;
const COPYING: u64 = 0xf1c552;
const BYTES: [u8; 256] = [0x11; 256];
fn error(e: ring::Error) -> u64 {
    match e {
        ring::Error::Length => 101,
        ring::Error::Offset => 102,
        ring::Error::Identity => 103,
        ring::Error::Closed => 104,
        ring::Error::Empty => 105,
        _ => 106,
    }
}
struct Native {
    h: fl::Handle,
    parent: CapabilityToken,
}
impl Memory for Native {
    fn load(&self, r: Region, word: usize) -> Result<u64, ring::Error> {
        if word >= ring::WORDS {
            return Err(ring::Error::Access);
        }
        let value = fl::transfer_word(self.h, false, r as u64 * 512 + word as u64, 0)
            .map_err(|_| ring::Error::Closed)?;
        if r == Region::Payload && word == 0 && INTERLEAVE.swap(false, Ordering::Relaxed) {
            // Real producer runs while this consumer's try_read is in progress.
            // It changes metadata too; remaining accesses must use local bounds.
            sunlight_ipc::ipc_call_timeout(
                self.parent,
                IpcMsg::with_label(COPYING).word(0, self.h.generation),
                5000,
            )
            .expect("during-copy peer acknowledgement");
        }
        Ok(value)
    }
    fn store(&self, r: Region, word: usize, value: u64) -> Result<(), ring::Error> {
        if word >= ring::WORDS {
            return Err(ring::Error::Access);
        }
        fl::transfer_word(self.h, true, r as u64 * 512 + word as u64, value)
            .map(|_| ())
            .map_err(|_| ring::Error::Closed)
    }
}
enum Role {
    Producer(ring::Producer<Native>),
    Consumer(ring::Consumer<Native>),
}
pub(super) struct Endpoint {
    role: Option<Role>,
    h: fl::Handle,
}
impl Endpoint {
    pub(super) fn new() -> Self {
        Self {
            role: None,
            h: fl::EMPTY,
        }
    }
    pub(super) fn command(&mut self, op: u64, h: fl::Handle, arg: u64, parent: CapabilityToken) {
        match op {
            60 => {
                assert!(arg <= 1);
                let mut payload_va = 0;
                for region in 0..3 {
                    let (va, len) = fl::call(8, h, arg * 3 + region, fl::NO_SESSION).unwrap();
                    assert_eq!(len, 4096);
                    assert_eq!(va % 4096, 0);
                    if region == 2 {
                        payload_va = va;
                    }
                }
                let memory = Native { h, parent };
                self.role = Some(if arg == 0 {
                    Role::Producer(ring::Producer::new(memory, h.generation).unwrap())
                } else {
                    Role::Consumer(ring::Consumer::new(memory, h.generation).unwrap())
                });
                self.h = h;
                reply(0, payload_va, h.generation);
            }
            61 => {
                assert_eq!(self.h, h);
                let Some(Role::Producer(p)) = &mut self.role else {
                    panic!("producer role")
                };
                let r = p.try_send(&BYTES);
                reply(r.err().map_or(0, error), 0, h.generation);
            }
            62 => {
                assert_eq!(self.h, h);
                let Some(Role::Consumer(c)) = &mut self.role else {
                    panic!("consumer role")
                };
                // Release supervisor so it can drive an independently sleeping
                // consumer or the mutation/retirement boundary during acquisition.
                reply(0, 0, h.generation);
                let code = receive(c, h, parent, arg);
                sunlight_ipc::ipc_call_timeout(
                    parent,
                    IpcMsg::with_label(DONE)
                        .word(0, code)
                        .word(1, h.generation)
                        .word(2, 0)
                        .word(3, 1),
                    5000,
                )
                .expect("native consumer completion");
            }
            63 => {
                // Hostile producer-owned descriptor mutations, using only fixed
                // trusted addresses, never using the malformed values as bounds.
                let (word, value) = match arg {
                    0 => (5, 257),
                    1 => (4, 4096),
                    2 => (3, 2),
                    3 => (1, h.generation + 1),
                    _ => panic!("malformed selector"),
                };
                let (code, value) = result(fl::transfer_word(h, true, word, value));
                reply(code, value, h.generation);
            }
            64 => {
                for word in 0..32 {
                    fl::transfer_word(h, true, 1024 + word, arg * 0x0101010101010101).unwrap();
                }
                reply(0, 0, h.generation);
            }
            66 => {
                let (code, value) = result(fl::transfer_word(h, false, 512, 0));
                reply(code, value, h.generation);
            }
            67 | 68 => {
                let (code, value) = result(fl::transfer_word(h, op == 68, arg, 1));
                reply(code, value, h.generation);
            }
            _ => panic!("transfer operation"),
        }
    }
}
fn receive(
    c: &mut ring::Consumer<Native>,
    h: fl::Handle,
    parent: CapabilityToken,
    mode: u64,
) -> u64 {
    INTERLEAVE.store(mode == 1 || mode == 4, Ordering::Relaxed);
    // Observe BEFORE checking publication. A notification between the empty
    // check and wait registration changes that sequence and cannot be lost.
    // Every wake returns through the publication/metadata checks, including a
    // notification before publication or repeated/spurious notifications.
    for _ in 0..16 {
        let observed = match fl::operate(11, h, 0) {
            Ok(v) => v,
            Err(e) => return e as u64,
        };
        let lease = match c.try_read() {
            Ok(r) => r,
            Err(ring::Error::Empty) => {
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
            Err(e) => return error(e),
        };
        assert_eq!(lease.identity().channel, h.generation);
        assert_eq!(lease.identity().sequence, 0);
        assert_eq!(lease.identity().generation, 1);
        assert_eq!(lease.payload().len(), 256);
        assert_eq!(lease.integrity(), ring::Integrity::PRIVATE_COPY);
        assert!(!lease.integrity().coherent_publication());
        assert!(!lease.integrity().application_authenticated());
        let check = |bytes: &[u8]| {
            for (i, b) in bytes.iter().enumerate() {
                assert_eq!(*b, if mode == 1 && i >= 8 { 0x22 } else { 0x11 });
            }
        };
        check(lease.payload());
        sunlight_ipc::ipc_call_timeout(
            parent,
            IpcMsg::with_label(SNAPSHOT)
                .word(0, h.generation)
                .word(1, 0)
                .word(2, 1)
                .word(3, 256),
            5000,
        )
        .expect("private snapshot acknowledgement");
        check(lease.payload()); // Producer has now changed ALL shared bytes.
        sunlight_ipc::debug_log(&alloc::format!(
            "[FL-M5-COPY] generation={} message=0 slot_generation=1 pid={} bytes=256 stable=1 mixed={} coherent=0 authentic=0",
            h.generation, sunlight_libc::getpid(), u8::from(mode == 1)));
        return lease.release().err().map_or(0, error);
    }
    106
}
fn mark(case: &str, h: fl::Handle, detail: u64) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M5] case={} generation={} message=0 slot_generation=1 detail={} pid={}",
        case,
        h.generation,
        detail,
        sunlight_libc::getpid()
    ));
}
fn setup(p: Actor, c: Actor) -> fl::Handle {
    let h = doorbell::channel(p, c);
    expect(p, 60, h, 0, 0);
    expect(c, 60, h, 1, 0);
    h
}
fn message(ep: EndpointId, a: Actor, label: u64, h: fl::Handle) -> IpcMsg {
    let m = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("independent peer receipt");
    assert_eq!(m.badge, a.pid);
    assert_eq!(m.label, label);
    assert_eq!(m.words[if label == DONE { 1 } else { 0 }], h.generation);
    m
}
fn respond(label: u64) {
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(label)).unwrap();
}
fn done(ep: EndpointId, c: Actor, h: fl::Handle, code: u64) {
    let m = message(ep, c, DONE, h);
    assert_eq!(m.words[0], code);
    assert_eq!(m.words[2..4], [0, 1]);
    respond(DONE);
}
fn snapshot(ep: EndpointId, p: Actor, c: Actor, h: fl::Handle, close: bool) {
    let m = message(ep, c, SNAPSHOT, h);
    assert_eq!(m.words[1..4], [0, 1, 256]);
    // The consumer is holding a private lease. IPC to producer changes backing
    // before acknowledging the consumer; there is no host or kernel data actor.
    expect(p, 64, h, 0x33, 0);
    if close {
        expect(p, 4, h, 0, 0);
    }
    respond(SNAPSHOT);
}
fn clean(p: Actor, h: fl::Handle, checkpoint: u64) {
    expect(p, 4, h, 0, 0);
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 0);
    mapping::census(checkpoint);
    zero();
}
pub fn supervisor() -> ! {
    doorbell::pin(0);
    mark("start", fl::EMPTY, 0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let mut p = doorbell::spawn(ep, public, 0);
    let mut c = doorbell::spawn(ep, public, 1);
    let foreign = doorbell::spawn(ep, public, 1);
    for cycle in 0..8 {
        let h = setup(p, c);
        // No role or pointer is conferred by possession of an exact handle.
        expect(foreign, 67, h, 0, 1);
        expect(foreign, 68, h, 0, 1);
        expect(c, 68, h, 0, 1);
        expect(p, 68, h, 512, 1);
        expect(p, 67, h, 1536, 1);
        expect(
            p,
            67,
            fl::Handle {
                generation: h.generation + 1,
                ..h
            },
            0,
            2,
        );
        expect(c, 62, h, 0, 0);
        assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
        assert_eq!(expect(p, 47, h, 0, 0).words[1], 1 | (1 << 32));
        // Notification before publication is a real spurious data wake.
        expect(p, 40, h, 0, 0);
        assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
        assert_eq!(expect(p, 47, h, 0, 0).words[1], 1 | (1 << 32));
        expect(p, 61, h, 0, 0);
        expect(p, 68, h, 0, 3);
        expect(p, 40, h, 0, 0);
        snapshot(ep, p, c, h, false);
        done(ep, c, h, 0);
        assert_eq!(expect(p, 66, h, 0, 0).words[1], 1);
        expect(p, 61, h, 0, 104); // second publication cannot be admitted
        clean(p, h, 500 + cycle);
        mark("cycle", h, cycle);
    }
    for bad in 0..4 {
        let h = setup(p, c);
        expect(p, 61, h, 0, 0);
        expect(p, 63, h, bad, 0);
        expect(p, 40, h, 0, 0);
        expect(c, 62, h, 0, 0);
        done(
            ep,
            c,
            h,
            match bad {
                0 => 101,
                1 => 102,
                _ => 103,
            },
        );
        clean(p, h, 520 + bad);
        mark("malformed", h, bad);
    }
    let h = setup(p, c);
    expect(p, 61, h, 0, 0);
    expect(p, 40, h, 0, 0);
    expect(c, 62, h, 1, 0);
    message(ep, c, COPYING, h);
    expect(p, 64, h, 0x22, 0);
    expect(p, 63, h, 0, 0);
    expect(p, 63, h, 1, 0);
    respond(COPYING);
    snapshot(ep, p, c, h, false);
    done(ep, c, h, 0);
    clean(p, h, 530);
    mark("mutation-during-copy", h, 0);
    let h = setup(p, c);
    expect(p, 61, h, 0, 0);
    expect(c, 62, h, 0, 0);
    snapshot(ep, p, c, h, true);
    done(ep, c, h, 104);
    clean(p, h, 531);
    mark("close-before-ack", h, 0);
    let h = setup(p, c);
    expect(p, 61, h, 0, 0);
    expect(c, 62, h, 4, 0);
    message(ep, c, COPYING, h);
    expect(p, 4, h, 0, 0);
    respond(COPYING);
    done(ep, c, h, 104);
    clean(p, h, 532);
    mark("revoke-during-copy", h, 0);
    let h = setup(p, c);
    expect(c, 62, h, 0, 0);
    assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
    assert_eq!(expect(p, 47, h, 0, 0).words[1], 1 | (1 << 32));
    expect(p, 44, h, 0, 0);
    done(ep, c, h, 9);
    clean(p, h, 533);
    mark("cancel-wait", h, 0);
    // Real lifecycle exits, with ordinary waitpid acknowledgements.
    for published in 0..2 {
        let h = setup(p, c);
        if published == 1 {
            expect(p, 61, h, 0, 0);
        }
        stop(p);
        expect(c, 62, h, 0, 0);
        done(ep, c, h, 4);
        zero();
        mapping::census(540 + published);
        mark("producer-exit", h, published);
        p = doorbell::spawn(ep, public, 0);
    }
    let h = setup(p, c);
    expect(p, 61, h, 0, 0);
    stop(c);
    expect(p, 40, h, 0, 4);
    clean(p, h, 542);
    mark("consumer-exit-before-copy", h, 0);
    c = doorbell::spawn(ep, public, 1);
    // Retire a transferred channel while the consumer VM is genuinely remote
    // and actively using its original view. Existing M2 sacrificial fault probe.
    let h = doorbell::channel(p, c);
    expect(p, 60, h, 0, 0);
    let va = expect(c, 60, h, 1, 0).words[1];
    expect(p, 61, h, 0, 0);
    expect(c, 62, h, 0, 0);
    snapshot(ep, p, c, h, false);
    done(ep, c, h, 0);
    expect(c, 22, h, va, 0);
    expect(p, 4, h, 0, 0);
    assert_eq!(super::super::reap(c.pid), 139);
    clean(p, h, 543);
    mark("remote-retirement", h, 0);
    stop(p);
    stop(foreign);
    assert!(sunlight_ipc::endpoint_destroy(ep));
    zero();
    mark("complete", fl::EMPTY, 0);
    sunlight_libc::exit(0)
}
