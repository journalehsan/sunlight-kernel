//! M6.1A: one mediated accounting qualification, never a throughput benchmark.
use super::ring;
use crate::channel_fixture::{self as fixture, doorbell, mapping, Actor};
use sunlight_ipc::{fastlane_authority as fl, CapabilityToken, IpcMsg};
const DATA: u64 = 0x6a64_0000;
const ACK: u64 = 0x6a65_0000;
const WARM: u64 = 16;
const COUNT: u64 = 128;
const LIMIT: u64 = WARM + COUNT;
pub(crate) fn handles(label: u64) -> bool {
    (DATA..DATA + LIMIT).contains(&label)
}
fn bytes(seq: u64) -> [u8; 32] {
    core::array::from_fn(|i| ((seq * 17 + i as u64 * 3) % 251) as u8)
}
fn digest(b: &[u8]) -> u64 {
    b.iter()
        .fold(0u64, |n, v| n.wrapping_mul(257).wrapping_add(*v as u64))
}
fn map(h: fl::Handle, role: u64) {
    for r in 0..3 {
        assert_eq!(
            fl::call(8, h, role * 3 + r, fl::NO_SESSION).unwrap().1,
            4096
        );
    }
}
fn resident(cpu: u64) {
    let deadline = sunlight_ipc::monotonic_millis().saturating_add(1000);
    let mut last = None;
    for attempts in 0..131_072 {
        last = sunlight_ipc::measurement_stamp_raw();
        if last.is_some_and(|s| s[0] == cpu) {
            if cpu == 1 {
                sunlight_ipc::debug_log(&alloc::format!(
                    "[FL-M61A-RESIDENT] target={} observed={} attempts={}",
                    cpu,
                    last.unwrap()[0],
                    attempts
                ));
            }
            return;
        }
        if sunlight_ipc::monotonic_millis() >= deadline {
            break;
        }
        sunlight_ipc::process_yield();
    }
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M61A-RESIDENT] target={} observed={} exhausted=1",
        cpu,
        last.map(|s| s[0]).unwrap_or(u64::MAX)
    ));
    panic!("bounded measurement residency");
}

fn negative(h: fl::Handle) {
    assert_eq!(fl::operate(29, h, 0), Err(fl::Error::Incomplete));
    assert_eq!(
        fl::operate(
            28,
            fl::Handle {
                generation: h.generation + 1,
                ..h
            },
            1
        ),
        Err(fl::Error::Stale)
    );
    fl::operate(30, h, 0).unwrap();
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M61A-NEG] pid={} missing_start=7 stale_generation=2",
        sunlight_libc::getpid()
    ));
}
fn start(h: fl::Handle, id: u64) {
    resident(0);
    fl::operate(28, h, id).unwrap();
    // These must leave the active observation intact.
    assert_eq!(fl::operate(28, h, id), Err(fl::Error::Duplicate));
    assert_eq!(fl::operate(29, h, id + 10), Err(fl::Error::Stale));
}
fn stop(h: fl::Handle, id: u64) {
    fl::operate(29, h, id).unwrap();
    assert_eq!(fl::operate(29, h, id), Err(fl::Error::Duplicate));
    assert_eq!(fl::operate(28, h, id), Err(fl::Error::Stale));
    sunlight_ipc::debug_log(&alloc::format!("[FL-M61A-MATCH] pid={} w={} duplicate_start=3 wrong_window=2 duplicate_stop=3 stale_window=2",sunlight_libc::getpid(),id));
}
pub(crate) struct Consumer {
    h: fl::Handle,
    peer: u64,
    next: u64,
    reader: Option<ring::Consumer<fixture::transfer::Native>>,
}
impl Consumer {
    pub fn new() -> Self {
        Self {
            h: fl::EMPTY,
            peer: 0,
            next: 0,
            reader: None,
        }
    }
    pub fn command(&mut self, m: IpcMsg, parent: CapabilityToken) {
        let h = fl::Handle {
            token: m.words[1],
            generation: m.words[2],
        };
        if m.words[0] == 124 {
            assert_eq!(fl::operate(28, h, 1), Err(fl::Error::Denied));
            assert_eq!(fl::operate(29, h, 1), Err(fl::Error::Denied));
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-M61A-FOREIGN] pid={} start=1 stop=1",
                sunlight_libc::getpid()
            ));
        } else {
            if m.words[0] != 120 {
                assert_eq!(m.badge, self.peer);
                assert_eq!(h, self.h);
            }
            if m.words[0] == 123 {
                start(h, 2);
                // Release the supervisor, then block on its authenticated endpoint.
                // Wake publication uses the existing affinity-aware enqueue path.
                fixture::reply(0, self.next, h.generation);
                doorbell::pin(1);
                let ack = sunlight_ipc::ipc_call_timeout(
                    parent,
                    IpcMsg::with_label(0x6a66_0001).word(0, h.generation),
                    5000,
                )
                .unwrap();
                assert_eq!(ack.label, 0x6a66_0001);
                resident(1);
                fl::operate(29, h, 2).unwrap();
                let ack = sunlight_ipc::ipc_call_timeout(
                    parent,
                    IpcMsg::with_label(0x6a66_0002).word(0, h.generation),
                    5000,
                )
                .unwrap();
                assert_eq!(ack.label, 0x6a66_0002);
                return;
            }
            match m.words[0] {
                120 => {
                    self.h = h;
                    self.peer = m.badge;
                    map(h, 1);
                    self.reader = Some(
                        ring::Consumer::new(fixture::transfer::Native { h, parent }, h.generation)
                            .unwrap(),
                    );
                    negative(h);
                }
                121 => {
                    assert_eq!(self.next, WARM);
                    start(h, 1);
                }
                122 => {
                    assert_eq!(self.next, LIMIT);
                    stop(h, 1);
                    sunlight_ipc::debug_log(&alloc::format!(
                        "[FL-M61A-R] pid={} peer={} received={} verified={} acks={}",
                        sunlight_libc::getpid(),
                        self.peer,
                        self.next,
                        self.next,
                        self.next
                    ));
                }
                // Deliberately leave a final incomplete window for actor-exit retirement.
                125 => {
                    self.reader = None;
                    fl::operate(28, h, 3).unwrap();
                }
                _ => panic!("accounting command"),
            }
        }
        fixture::reply(0, self.next, h.generation);
    }
    pub fn request(&mut self, m: IpcMsg) {
        assert_eq!(m.badge, self.peer);
        assert_eq!(m.label, DATA + self.next);
        assert_eq!(m.word_count, 0);
        assert!(fl::operate(11, self.h, 0).unwrap() > 0);
        fl::transfer_word(self.h, true, 1538, self.next).unwrap();
        let lease = self.reader.as_mut().unwrap().try_read().unwrap();
        assert_eq!(lease.identity().channel, self.h.generation);
        assert_eq!(lease.identity().sequence, self.next);
        assert_eq!(lease.identity().generation, self.next / 16 + 1);
        assert_eq!(lease.integrity(), ring::Integrity::PRIVATE_COPY);
        let expected = bytes(self.next);
        assert_eq!(lease.payload(), &expected);
        fl::transfer_word(self.h, true, 1539, self.next).unwrap();
        lease.release().unwrap();
        let seq = self.next;
        self.next += 1;
        sunlight_ipc::ipc_reply_result(
            IpcMsg::with_label(ACK + seq)
                .word(0, digest(&expected))
                .word(1, self.next),
        )
        .unwrap();
    }
}
pub fn supervisor() -> ! {
    doorbell::pin(0);
    assert!(sunlight_ipc::measurement_clock_recalibrate().is_some());
    resident(0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let c = doorbell::spawn(ep, public, 0);
    let h = fixture::pair(c);
    fl::operate(3, h, 0).unwrap();
    fixture::expect(c, 3, h, 1, 0);
    map(h, 0);
    fl::transfer_word(h, true, 1536, 0).unwrap();
    let mut writer = ring::Producer::new(
        fixture::transfer::Native { h, parent: public },
        h.generation,
    )
    .unwrap();
    fixture::expect(c, 120, h, 0, 0);
    negative(h);
    let foreign = doorbell::spawn(ep, public, 0);
    fixture::expect(foreign, 124, h, 0, 0);
    fixture::stop(foreign);
    for seq in 0..LIMIT {
        if seq == WARM {
            sunlight_ipc::debug_log(
                "[FL-M61A-WARM] completed=16 measured=128 bytes=32 outstanding=1",
            );
            fixture::expect(c, 121, h, 0, 0);
            start(h, 1);
        }
        let payload = bytes(seq);
        fl::transfer_word(h, true, 1537, seq).unwrap();
        writer.try_send(&payload).unwrap();
        fl::operate(10, h, 0).unwrap();
        let ack =
            sunlight_ipc::ipc_call_timeout(c.send, IpcMsg::with_label(DATA + seq), 5000).unwrap();
        assert_eq!(ack.label, ACK + seq);
        assert_eq!(ack.words[..2], [digest(&payload), seq + 1]);
    }
    stop(h, 1);
    fixture::expect(c, 122, h, 0, 0);
    sunlight_ipc::debug_log(&alloc::format!("[FL-M61A-P] producer={} consumer={} generation={} warm=16 measured=128 sent=144 replies=144 bytes=32 integrity=exact",sunlight_libc::getpid(),c.pid,h.generation));
    fixture::expect(c, 123, h, 0, 0);
    for label in [0x6a66_0001, 0x6a66_0002] {
        let request = sunlight_ipc::ipc_recv_timeout(ep, 5000).unwrap();
        assert_eq!(request.badge, c.pid);
        assert_eq!(request.label, label);
        assert_eq!(request.words[0], h.generation);
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(label)).unwrap();
    }

    fixture::expect(c, 125, h, 0, 0);
    fixture::stop(c);
    fl::operate(4, h, 0).unwrap(); // Idempotent revoke after peer-exit retirement.
    fixture::zero();
    mapping::census(999);
    assert!(sunlight_ipc::endpoint_destroy(ep));
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M61A] case=complete resources=0 production=DISABLED pid={}",
        sunlight_libc::getpid()
    ));
    sunlight_libc::exit(0)
}
