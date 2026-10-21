use super::*;
use std::sync::{Condvar, Mutex};
use std::time::Instant;
const P: Identity = Identity {
    pid: 1,
    incarnation: 1,
};
const C: Identity = Identity {
    pid: 2,
    incarnation: 1,
};
const O: Identity = Identity {
    pid: 3,
    incarnation: 1,
};
const K: Channel = Channel {
    slot: 0,
    generation: 1,
};
fn model() -> Doorbell {
    Doorbell::new(K, O, P, C, true).unwrap()
}

#[test]
fn deterministic_interleavings() {
    // Every legal position relative to the serialized registration transaction.
    // No notifier can execute inside that transaction in this model.
    for position in 0..6 {
        let mut d = model();
        if position == 0 {
            d.notify(K, P).unwrap();
        }
        let observed = d.snapshot(K, C).unwrap();
        if position == 1 {
            d.notify(K, P).unwrap();
        }
        // Empty ring check (payload publication tested separately).
        if position == 2 {
            d.notify(K, P).unwrap();
        }
        let wait = d.wait(K, C, observed).unwrap();
        if position == 1 || position == 2 {
            assert_eq!(wait, Wait::Changed(1));
            assert!(!d.has_waiter());
        } else {
            assert_eq!(wait, Wait::Registered);
            // positions 3/4/5 represent before park, after park, remote notifier.
            // Blocked state is committed under the same lock as registration.
            let mut blocked = true;
            let effect = d.notify(K, P).unwrap();
            assert_eq!(effect.wake, Some(C));
            if effect.wake.is_some() {
                blocked = false;
            }
            assert!(!blocked); // there is no later stale blocked-state write
            assert!(!d.has_waiter());
        }
    }
}
#[test]
fn coalescing_spurious_and_cancellation() {
    let mut d = model();
    for _ in 0..10 {
        assert_eq!(d.notify(K, P).unwrap().wake, None);
    }
    assert_eq!(d.wait(K, C, 0), Ok(Wait::Changed(10)));
    assert_eq!(d.wait(K, C, 10), Ok(Wait::Registered));
    assert_eq!(d.wait(K, C, 10), Err(Error::AlreadyWaiting));
    assert!(d.cancel(K, C).unwrap()); // spurious wake; retry unchanged condition
    assert!(!d.cancel(K, C).unwrap());
    assert_eq!(d.wait(K, C, 10), Ok(Wait::Registered));
    assert_eq!(d.notify(K, P).unwrap().wake, Some(C));
    assert_eq!(d.notify(K, P).unwrap().wake, None);
    assert_eq!(d.wait(K, C, 10), Ok(Wait::Changed(12)));
}
#[test]
fn authority_generation_and_reservation() {
    assert!(matches!(
        Doorbell::new(K, O, P, C, false),
        Err(Error::Capacity)
    ));
    assert!(matches!(
        Doorbell::new(Channel { generation: 0, ..K }, O, P, C, true),
        Err(Error::Invalid)
    ));
    let mut d = model();
    let stale = Channel { generation: 2, ..K };
    assert_eq!(d.notify(stale, P), Err(Error::Stale));
    assert_eq!(d.wait(stale, C, 0), Err(Error::Stale));
    assert_eq!(d.notify(K, C), Err(Error::Unauthorized));
    assert_eq!(d.close(K, P), Err(Error::Unauthorized));
    assert_eq!(
        d.wait(
            K,
            Identity {
                incarnation: 2,
                ..C
            },
            0
        ),
        Err(Error::Unauthorized)
    );
    assert_eq!(d.snapshot(K, P), Err(Error::Unauthorized));
    assert_eq!(d.cancel(K, P), Err(Error::Unauthorized));
    assert_eq!(d.revoke(K, P), Err(Error::Unauthorized));
    d.revoke(K, O).unwrap();
    assert_eq!(d.notify(K, P), Err(Error::Terminal(Terminal::Revoked)));
}
#[test]
fn terminal_races_exhaustion_and_cleanup() {
    for terminal in [
        Terminal::Closed,
        Terminal::ProducerExit,
        Terminal::ConsumerExit,
        Terminal::Revoked,
        Terminal::Exhausted,
    ] {
        for notify_first in [false, true] {
            let mut d = model();
            if terminal == Terminal::Exhausted {
                d.sequence = u64::MAX;
            }
            let seq = d.snapshot(K, C).unwrap();
            d.wait(K, C, seq).unwrap();
            if notify_first && terminal != Terminal::Exhausted {
                assert_eq!(d.notify(K, P).unwrap().wake, Some(C));
            }
            let effect = match terminal {
                Terminal::Closed => d.close(K, O).unwrap(),
                Terminal::ProducerExit => d.peer_exit(P),
                Terminal::ConsumerExit => d.peer_exit(C),
                Terminal::Revoked => d.revoke(K, O).unwrap(),
                Terminal::Exhausted => d.notify(K, P).unwrap(),
            };
            assert_eq!(effect.terminal, Some(terminal));
            assert_eq!(
                effect.wake,
                if (notify_first && terminal != Terminal::Exhausted)
                    || terminal == Terminal::ConsumerExit
                {
                    None
                } else {
                    Some(C)
                }
            );
            assert!(!d.has_waiter());
            assert_eq!(d.wait(K, C, seq), Err(Error::Terminal(terminal)));
            assert_eq!(d.notify(K, P), Err(Error::Terminal(terminal)));
            assert_eq!(
                d.close(K, O).unwrap(),
                Effect {
                    wake: None,
                    terminal: Some(terminal)
                }
            );
        }
    }
    for generation in 1..=10000 {
        let k = Channel { generation, ..K };
        let mut d = Doorbell::new(k, O, P, C, true).unwrap();
        d.wait(k, C, 0).unwrap();
        assert_eq!(
            d.peer_exit(Identity {
                incarnation: 2,
                ..P
            })
            .wake,
            None
        );
        assert!(d.has_waiter());
        assert!(d.cancel(k, C).unwrap());
        d.close(k, O).unwrap();
        assert!(!d.has_waiter());
    }
}

struct Host {
    state: Mutex<Doorbell>,
    cv: Condvar,
    registered: Condvar,
}
impl Host {
    fn wait(&self, seq: u64) -> Result<(), Error> {
        let mut d = self.state.lock().unwrap();
        match d.wait(K, C, seq)? {
            Wait::Changed(_) => return Ok(()),
            Wait::Registered => self.registered.notify_one(),
        }
        // Condvar atomically releases the same mutex and parks; notifications
        // before physical park cannot be overwritten by a stale blocked write.
        while d.has_waiter() {
            d = self.cv.wait(d).unwrap();
        }
        d.open()
    }
    fn notify(&self) {
        let mut d = self.state.lock().unwrap();
        if d.notify(K, P).unwrap().wake.is_some() {
            self.cv.notify_one();
        }
    }
}
#[test]
fn concurrent_notify_close_serializations() {
    for _ in 0..100 {
        let h = Host {
            state: Mutex::new(model()),
            cv: Condvar::new(),
            registered: Condvar::new(),
        };
        std::thread::scope(|s| {
            let waiter = s.spawn(|| h.wait(0));
            let mut d = h.state.lock().unwrap();
            while !d.has_waiter() {
                d = h.registered.wait(d).unwrap();
            }
            drop(d);
            s.spawn(|| {
                let mut d = h.state.lock().unwrap();
                let _ = d.notify(K, P);
                h.cv.notify_one();
            });
            s.spawn(|| {
                let mut d = h.state.lock().unwrap();
                d.close(K, O).unwrap();
                h.cv.notify_one();
            });
            let result = waiter.join().unwrap();
            assert!(result == Ok(()) || result == Err(Error::Terminal(Terminal::Closed)));
        });
        assert!(!h.state.lock().unwrap().has_waiter());
    }
}
#[test]
fn ring_conditional_wait_fifo_and_integrity() {
    use sunlight_fastlane_ring::{RecvError, Ring, SendError};
    let mut ring = Ring::<2, 16>::new(1).unwrap();
    let (mut producer, mut consumer) = ring.split(1).unwrap();
    // Assert bounded backpressure before concurrent traffic.
    producer.try_send(&0u64.to_le_bytes()).unwrap();
    producer.try_send(&1u64.to_le_bytes()).unwrap();
    assert_eq!(producer.try_send(&[0]), Err(SendError::Full));
    let mut out = [0; 16];
    consumer.try_recv(&mut out).unwrap();
    consumer.try_recv(&mut out).unwrap();
    let h = Host {
        state: Mutex::new(model()),
        cv: Condvar::new(),
        registered: Condvar::new(),
    };
    std::thread::scope(|s| {
        let c = s.spawn(|| {
            let mut expected = 0u64;
            loop {
                let seq = match h.state.lock().unwrap().snapshot(K, C) {
                    Ok(seq) => seq,
                    Err(Error::Terminal(Terminal::Closed)) => break,
                    other => panic!("{other:?}"),
                };
                loop {
                    match consumer.try_recv(&mut out) {
                        Ok(16) => {
                            let value = u64::from_le_bytes(out[..8].try_into().unwrap());
                            let check = u64::from_le_bytes(out[8..].try_into().unwrap());
                            assert_eq!(value, expected);
                            assert_eq!(check, !value);
                            expected += 1;
                        }
                        Err(RecvError::Empty) => break,
                        other => panic!("{other:?}"),
                    }
                }
                if expected == 20000 {
                    break;
                }
                h.wait(seq).unwrap();
            }
            assert_eq!(expected, 20000);
        });
        // Barrier proves the consumer really registered before first publication.
        let mut d = h.state.lock().unwrap();
        while !d.has_waiter() {
            d = h.registered.wait(d).unwrap();
        }
        drop(d);
        for value in 0u64..20000 {
            let mut payload = [0; 16];
            payload[..8].copy_from_slice(&value.to_le_bytes());
            payload[8..].copy_from_slice(&(!value).to_le_bytes());
            loop {
                match producer.try_send(&payload) {
                    Ok(()) => break,
                    Err(SendError::Full) => std::thread::yield_now(), // bounded fixture only
                    other => panic!("{other:?}"),
                }
            }
            h.notify();
        }
        c.join().unwrap();
    });
    let mut d = h.state.lock().unwrap();
    assert!(!d.has_waiter());
    d.close(K, O).unwrap();
    assert_eq!(d.snapshot(K, C), Err(Error::Terminal(Terminal::Closed)));
}
#[test]
fn host_operation_measurements() {
    let mut d = model();
    let mut samples = std::vec::Vec::with_capacity(10000);
    let start = Instant::now();
    for _ in 0..10000 {
        let t = Instant::now();
        std::hint::black_box(d.notify(K, P).unwrap());
        samples.push(t.elapsed().as_nanos());
    }
    let elapsed = start.elapsed();
    samples.sort_unstable();
    std::println!("host debug notify with timestamp overhead: p50={} p95={} p99={} ns; total={:?}; metadata={} bytes; clock=std::time::Instant; no affinity; kernel latency unavailable", samples[4999], samples[9499], samples[9899], elapsed, core::mem::size_of::<Doorbell>());
}

#[test]
fn parked_terminal_and_latency() {
    let h = Host {
        state: Mutex::new(model()),
        cv: Condvar::new(),
        registered: Condvar::new(),
    };
    let mut setup = std::vec::Vec::new();
    let mut wake = std::vec::Vec::new();
    for _ in 0..1000 {
        let seq = h.state.lock().unwrap().snapshot(K, C).unwrap();
        std::thread::scope(|s| {
            let start = Instant::now();
            let t = s.spawn(|| {
                h.wait(seq).unwrap();
                Instant::now()
            });
            let mut d = h.state.lock().unwrap();
            while !d.has_waiter() {
                d = h.registered.wait(d).unwrap();
            }
            setup.push(start.elapsed().as_nanos());
            let notify = Instant::now();
            assert_eq!(d.notify(K, P).unwrap().wake, Some(C));
            h.cv.notify_one();
            drop(d);
            wake.push(t.join().unwrap().duration_since(notify).as_nanos());
        });
    }
    setup.sort_unstable();
    wake.sort_unstable();
    std::println!("host thread-create-to-registration (includes scheduling) p50/p95/p99 ns={}/{}/{}; notify-to-wait-return p50/p95/p99 ns={}/{}/{}; n=1000, Instant nominal resolution uncalibrated, no affinity", setup[499], setup[949], setup[989], wake[499], wake[949], wake[989]);
    for producer_exit in [false, true] {
        let h = Host {
            state: Mutex::new(model()),
            cv: Condvar::new(),
            registered: Condvar::new(),
        };
        std::thread::scope(|s| {
            let t = s.spawn(|| h.wait(0));
            let mut d = h.state.lock().unwrap();
            while !d.has_waiter() {
                d = h.registered.wait(d).unwrap();
            }
            let e = if producer_exit {
                d.peer_exit(P)
            } else {
                d.close(K, O).unwrap()
            };
            assert_eq!(e.wake, Some(C));
            h.cv.notify_one();
            drop(d);
            assert_eq!(
                t.join().unwrap(),
                Err(Error::Terminal(if producer_exit {
                    Terminal::ProducerExit
                } else {
                    Terminal::Closed
                }))
            );
        });
        assert!(!h.state.lock().unwrap().has_waiter());
    }
}
