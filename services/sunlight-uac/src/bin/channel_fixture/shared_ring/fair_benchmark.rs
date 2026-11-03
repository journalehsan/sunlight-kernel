//! M6.0R bounded request/verify/ack experiment. No production backend changes.
use super::{ring, Native};
use crate::channel_fixture::{self as fixture, doorbell, mapping, Actor};
use sunlight_ipc::{fastlane_authority as fl, measurement_clock as mc, CapabilityToken, IpcMsg};
const DATA: u64 = 0x6a62_0000;
const ACK: u64 = 0x6a63_0000;
const WARM: u64 = 16;
const COUNT: u64 = 128;
const LIMIT: u64 = WARM + COUNT;

pub(crate) fn handles(label: u64) -> bool {
    (DATA..DATA + LIMIT).contains(&label)
}
fn bytes(sequence: u64) -> [u8; 256] {
    core::array::from_fn(|i| ((sequence * 17 + i as u64 * 3) % 251) as u8)
}
fn digest(b: &[u8]) -> u64 {
    b.iter()
        .fold(0u64, |n, v| n.wrapping_mul(257).wrapping_add(*v as u64))
}
fn stamp() -> Option<[u64; 7]> {
    sunlight_ipc::measurement_stamp_raw()
}
fn converted(w: [u64; 7]) -> mc::Stamp {
    mc::Stamp {
        cpu: w[0] as u32,
        epoch: w[1] as u32,
        ns: w[2],
        flags: w[5] as u32,
    }
}
// Raw records are retained even on rejection. No fallback clock or cross-CPU subtraction.
fn observation(
    kind: &str,
    path: u64,
    size: usize,
    seq: u64,
    a: Option<[u64; 7]>,
    b: Option<[u64; 7]>,
) {
    let mut reason = "missing";
    let mut duration = None;
    if let (Some(a), Some(b)) = (a, b) {
        reason = if a[0] != 0 || b[0] != 0 {
            "nonresident"
        } else if a[5] & mc::FLAG_FREQUENCY_STABILITY_CHECKED as u64 == 0
            || b[5] & mc::FLAG_FREQUENCY_STABILITY_CHECKED as u64 == 0
        {
            "unstable"
        } else if a[3] != b[3] || a[4] != b[4] || a[5] != b[5] || a[6] != b[6] {
            "calibration-change"
        } else if a[4] == 0 || a[4] > 20_000 {
            "uncertainty"
        } else {
            match mc::elapsed_ns(&converted(a), &converted(b)) {
                Ok(n) if n > 0 => {
                    duration = Some(n);
                    "accepted"
                }
                Ok(_) => "zero",
                Err(mc::ClockError::StaleEpoch) => "stale-epoch",
                Err(mc::ClockError::Backward) => "backward",
                Err(_) => "invalid",
            }
        };
    }
    let a = a.unwrap_or([0; 7]);
    let b = b.unwrap_or([0; 7]);
    sunlight_ipc::debug_log(&alloc::format!("[FL-M60R-T] k={} p={} z={} s={} pid={} a={} b={} ca={} cb={} ea={} eb={} ppm={} ns={} why={}", kind,path,size,seq,sunlight_libc::getpid(),a[2],b[2],a[0],b[0],a[1],b[1],a[4],duration.unwrap_or(0),reason));
    // Each observation has its full calibration identity, separately kept within DebugLog's 256-byte bound.
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M60R-C] k={} p={} z={} s={} pid={} ha={} hb={} pa={} pb={} fa={} fb={} sa={} sb={}",
        kind,
        path,
        size,
        seq,
        sunlight_libc::getpid(),
        a[3],
        b[3],
        a[4],
        b[4],
        a[5],
        b[5],
        a[6],
        b[6]
    ));
}

enum Read {
    Idle,
    Inline,
    Mediated(ring::Consumer<fixture::transfer::Native>),
    Shared(ring::Consumer<Native>),
}
enum Write {
    Inline,
    Mediated(ring::Producer<fixture::transfer::Native>),
    Shared(ring::Producer<Native>),
}
fn pages(h: fl::Handle, role: u64) -> [u64; 3] {
    core::array::from_fn(|r| {
        let (va, n) = fl::call(8, h, role * 3 + r as u64, fl::NO_SESSION).unwrap();
        assert_eq!(n, 4096);
        va
    })
}
fn copy<M: ring::Memory>(
    c: &mut ring::Consumer<M>,
    h: fl::Handle,
    seq: u64,
    expected: &[u8],
    mediated: bool,
) {
    if mediated {
        fl::transfer_word(h, true, 1538, seq).unwrap();
    }
    let lease = c.try_read().expect("private copy");
    assert_eq!(lease.identity().channel, h.generation);
    assert_eq!(lease.identity().sequence, seq);
    assert_eq!(lease.identity().generation, seq / 16 + 1);
    assert_eq!(lease.integrity(), ring::Integrity::PRIVATE_COPY);
    assert_eq!(lease.payload(), expected);
    if mediated {
        fl::transfer_word(h, true, 1539, seq).unwrap();
    }
    lease.release().expect("exact acknowledgement");
}
pub(crate) struct Consumer {
    path: u64,
    size: usize,
    next: u64,
    peer: u64,
    h: fl::Handle,
    reader: Read,
    // Complete population of local consumer observations, reported after the measured window.
    cpu_begin: Option<[u64; 7]>,
    samples: [(Option<[u64; 7]>, Option<[u64; 7]>); COUNT as usize],
}
impl Consumer {
    pub fn new() -> Self {
        Self {
            path: 0,
            size: 64,
            next: 0,
            peer: 0,
            h: fl::EMPTY,
            reader: Read::Idle,
            cpu_begin: None,
            samples: [(None, None); COUNT as usize],
        }
    }
    pub fn command(&mut self, m: IpcMsg, parent: CapabilityToken) {
        let h = fl::Handle {
            token: m.words[1],
            generation: m.words[2],
        };
        match m.words[0] {
            110 => {
                self.path = m.words[3] & 0xff;
                self.size = (m.words[3] >> 8) as usize;
                self.peer = m.badge;
                self.next = 0;
                self.h = h;
                assert!(self.path <= 2 && self.size == 32);
                self.reader = match self.path {
                    0 => Read::Inline,
                    1 => {
                        pages(h, 1);
                        Read::Mediated(
                            ring::Consumer::new(
                                fixture::transfer::Native { h, parent },
                                h.generation,
                            )
                            .unwrap(),
                        )
                    }
                    _ => {
                        let pages = pages(h, 1);
                        fl::operate(24, h, 1).unwrap();
                        Read::Shared(
                            ring::Consumer::new(
                                Native {
                                    h,
                                    parent,
                                    pages,
                                    role: 1,
                                },
                                h.generation,
                            )
                            .unwrap(),
                        )
                    }
                };
            }
            111 => {
                assert_eq!(m.badge, self.peer);
                accounting_start(self.path, h);
                self.cpu_begin = stamp();
            }
            112 => {
                assert_eq!(m.badge, self.peer);
                let cpu_end = stamp();
                accounting_stop(self.path, h, self.cpu_begin, cpu_end, "actor-consumer");
                assert_eq!(self.next, LIMIT);
                for (i, (a, b)) in self.samples.iter().enumerate() {
                    observation("consume", self.path, self.size, i as u64, *a, *b);
                }
                sunlight_ipc::debug_log(&alloc::format!(
                    "[FL-M60R-R] p={} z={} received={} verified={} acks={} peer={} pid={}",
                    self.path,
                    self.size,
                    self.next,
                    self.next,
                    self.next,
                    self.peer,
                    sunlight_libc::getpid()
                ));
            }
            113 => {
                assert_eq!(m.badge, self.peer);
                self.reader = Read::Idle;
            }
            _ => panic!("benchmark command"),
        }
        fixture::reply(0, self.next, h.generation);
    }
    pub fn request(&mut self, m: IpcMsg) {
        assert_eq!(m.badge, self.peer);
        assert_eq!(m.label, DATA + self.next);
        let seq = self.next;
        let expected = bytes(seq);
        let expected = &expected[..self.size];
        let a = if seq >= WARM { stamp() } else { None };
        match &mut self.reader {
            Read::Idle => panic!("unconfigured reader"),
            Read::Inline => {
                assert_eq!(m.word_count, 4);
                assert_eq!(m.cap_count, 0);
                let mut private = [0u8; 32];
                for (i, w) in m.words[..4].iter().enumerate() {
                    private[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
                }
                assert_eq!(&private[..], expected);
            }
            Read::Mediated(c) => {
                assert_eq!(m.word_count, 0);
                assert!(fl::operate(11, self.h, 0).unwrap() > 0);
                copy(c, self.h, seq, expected, true);
            }
            Read::Shared(c) => {
                assert_eq!(m.word_count, 0);
                assert!(fl::operate(11, self.h, 0).unwrap() > 0);
                copy(c, self.h, seq, expected, false);
            }
        }
        let b = if seq >= WARM { stamp() } else { None };
        if seq >= WARM {
            self.samples[(seq - WARM) as usize] = (a, b);
        }
        self.next += 1;
        sunlight_ipc::ipc_reply_result(
            IpcMsg::with_label(ACK + seq)
                .word(0, digest(expected))
                .word(1, self.next),
        )
        .unwrap();
    }
}
fn configure(c: Actor, h: fl::Handle, path: u64, size: usize) {
    let m = sunlight_ipc::ipc_call_timeout(
        c.send,
        IpcMsg::with_label(fixture::CMD)
            .word(0, 110)
            .word(1, h.token)
            .word(2, h.generation)
            .word(3, path | ((size as u64) << 8)),
        5000,
    )
    .unwrap();
    assert_eq!(m.label, fixture::CMD);
    assert_eq!(m.words[0], 0);
}
fn pair(c: Actor) -> fl::Handle {
    let h = fixture::pair(c);
    fl::operate(3, h, 0).unwrap();
    fixture::expect(c, 3, h, 1, 0);
    h
}
pub fn supervisor() -> ! {
    doorbell::pin(0);
    assert!(
        sunlight_ipc::measurement_clock_recalibrate().is_some(),
        "qualified clock required"
    );
    resident();
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    for (path, size) in [(0u64, 32usize), (1, 32), (2, 32)] {
        // Affinity alone does not prove residency. Bound setup placement before its first stamp.
        resident();
        let setup_a = stamp();
        let c = doorbell::spawn(ep, public, 0);
        let h = if path == 0 { fl::EMPTY } else { pair(c) };
        let mut writer = match path {
            0 => Write::Inline,
            1 => {
                pages(h, 0);
                fl::transfer_word(h, true, 1536, 0).unwrap();
                Write::Mediated(
                    ring::Producer::new(
                        fixture::transfer::Native { h, parent: public },
                        h.generation,
                    )
                    .unwrap(),
                )
            }
            _ => {
                let pages = pages(h, 0);
                fl::operate(24, h, 0).unwrap();
                Write::Shared(
                    ring::Producer::new(
                        Native {
                            h,
                            parent: public,
                            pages,
                            role: 0,
                        },
                        h.generation,
                    )
                    .unwrap(),
                )
            }
        };
        configure(c, h, path, size);
        let setup_b = stamp();
        let mut rtt = [(None, None); COUNT as usize];
        let mut publish = [(None, None); COUNT as usize];
        let mut begin = None;
        let mut cpu_begin = None;
        for seq in 0..LIMIT {
            if seq == WARM {
                fixture::expect(c, 111, h, 0, 0);
                accounting_start(path, h);
                cpu_begin = stamp();
                begin = stamp();
            }
            let payload = bytes(seq);
            let payload = &payload[..size];
            let a = if seq >= WARM { stamp() } else { None };
            let mut request = IpcMsg::with_label(DATA + seq);
            match &mut writer {
                Write::Inline => {
                    for (i, chunk) in payload.chunks_exact(8).enumerate() {
                        request = request.word(i, u64::from_le_bytes(chunk.try_into().unwrap()));
                    }
                }
                Write::Mediated(p) => {
                    fl::transfer_word(h, true, 1537, seq).unwrap();
                    p.try_send(payload).unwrap();
                    fl::operate(10, h, 0).unwrap();
                }
                Write::Shared(p) => {
                    p.try_send(payload).unwrap();
                    fl::operate(10, h, 0).unwrap();
                }
            }
            let published = if seq >= WARM && path != 0 {
                stamp()
            } else {
                None
            };
            let ack =
                sunlight_ipc::ipc_call_timeout(c.send, request, 5000).expect("bounded request");
            assert_eq!(ack.label, ACK + seq);
            assert_eq!(ack.words[..2], [digest(payload), seq + 1]);
            let b = if seq >= WARM { stamp() } else { None };
            if seq >= WARM {
                rtt[(seq - WARM) as usize] = (a, b);
                publish[(seq - WARM) as usize] = (a, published);
            }
        }
        let end = stamp();
        accounting_stop(path, h, cpu_begin, stamp(), "actor-producer");
        fixture::expect(c, 112, h, path, 0);
        observation("setup", path, size, 0, setup_a, setup_b);
        observation("window", path, size, 0, begin, end);
        for (i, (a, b)) in rtt.iter().enumerate() {
            observation("rtt", path, size, i as u64, *a, *b);
        }
        if path != 0 {
            for (i, (a, b)) in publish.iter().enumerate() {
                observation("publish-notify", path, size, i as u64, *a, *b);
            }
        }
        sunlight_ipc::debug_log(&alloc::format!("[FL-M60R-P] p={} z={} warm={} measured={} sent={} replies={} producer={} consumer={} cpu=0 outstanding=1 retries=0",path,size,WARM,COUNT,LIMIT,LIMIT,sunlight_libc::getpid(),c.pid));
        let cleanup_a = stamp();
        fixture::expect(c, 113, h, 0, 0);
        if path != 0 {
            fl::operate(4, h, 0).unwrap();
        }
        fixture::stop(c);
        fixture::zero();
        mapping::census(700 + path * 100 + size as u64);
        let cleanup_b = stamp();
        observation("teardown", path, size, 0, cleanup_a, cleanup_b);
    }
    assert!(sunlight_ipc::endpoint_destroy(ep));
    fixture::zero();
    mapping::census(999);
    fl::operate(25, fl::EMPTY, 0).unwrap();
    sunlight_ipc::debug_log(&alloc::format!("[FL-M60R] case=complete configurations=3 resources=0 integrity=exact historical=UNRESOLVED production=DISABLED pid={}", sunlight_libc::getpid()));
    sunlight_libc::exit(0)
}

// A refused diagnostic is evidence of selector coverage, never a zero CPU charge.
fn accounting_start(path: u64, h: fl::Handle) {
    if path == 2 {
        fl::operate(26, h, 0).unwrap();
    } else if path == 1 {
        assert_eq!(fl::operate(26, h, 0), Err(fl::Error::Incomplete));
    }
}
fn accounting_stop(path: u64, h: fl::Handle, a: Option<[u64; 7]>, b: Option<[u64; 7]>, kind: &str) {
    observation(kind, path, 32, 0, a, b);
    if path == 2 {
        fl::operate(27, h, 0).unwrap();
    } else {
        if path == 1 {
            assert_eq!(fl::operate(27, h, 0), Err(fl::Error::Incomplete));
        }
        sunlight_ipc::debug_log(&alloc::format!("[FL-M60R-CPU] p={} z=32 generation={} pid={} status=NOT_QUALIFIED reason={} start_code={} stop_code={} charged=UNAVAILABLE switches=UNAVAILABLE reconcile=UNAVAILABLE",path,h.generation,sunlight_libc::getpid(),if path==1 {"native-role-required"} else {"no-channel-selector"},if path==1 {"7"} else {"NA"},if path==1 {"7"} else {"NA"}));
    }
}
fn resident() {
    for _ in 0..4096 {
        if stamp().is_some_and(|s| s[0] == 0) {
            return;
        }
        sunlight_ipc::process_yield();
    }
    panic!("bounded CPU0 residency");
}
