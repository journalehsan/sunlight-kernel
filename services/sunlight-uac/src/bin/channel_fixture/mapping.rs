//! No Rust references into peer-writable memory. Access probes are explicit
//! single instructions, isolated from the controller's VM and lifetime.
use super::*;
const ACCESS: u64 = 20;
const FAULT: u64 = 21;
const SPIN: u64 = 22;
const COLLIDE: u64 = 23;
const ESCALATE: u64 = 24;
const WORD_ACCESS: u64 = 25;
const WORD_FIRST: u64 = 0x8877_6655_4433_2211;
const WORD_SECOND: u64 = 0x1020_3040_5060_7080;
fn mark(case: &str, generation: u64, detail: u64) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M2] case={} generation={} detail={} pid={}",
        case,
        generation,
        detail,
        sunlight_libc::getpid()
    ));
}
unsafe fn load(va: u64) -> u8 {
    let value: u8;
    core::arch::asm!("mov {value}, byte ptr [{va}]", value=out(reg_byte) value, va=in(reg) va, options(nostack, readonly));
    value
}
unsafe fn store(va: u64) {
    core::arch::asm!("mov byte ptr [{va}], 0x5a", va=in(reg) va, options(nostack));
}
// SAFETY contract for both helpers: the fixture supplies an aligned word wholly
// inside its own authenticated M2 view; stores target only its role-owned page.
// These are machine accesses, never Rust references into hostile memory. Normal
// M2 retirement can still fault/terminate an actor, as in the existing probes.
unsafe fn load_word(va: u64) -> u64 {
    let value: u64;
    core::arch::asm!("mov {value}, qword ptr [{va}]", value=out(reg) value, va=in(reg) va, options(nostack, readonly));
    value
}
unsafe fn store_word(va: u64, value: u64) {
    core::arch::asm!("mov qword ptr [{va}], {value}", value=in(reg) value, va=in(reg) va, options(nostack));
}
pub(super) fn actor_command(op: u64, h: fl::Handle, arg: u64) {
    match op {
        #[cfg(feature = "fastlane-census-hold-test")]
        26 => {
            let (_, cap) = sunlight_ipc::shm_create(4096, 0).expect("independent census backing");
            mark("foreign-shm-held", 0, cap.0);
            reply(0, cap.0, 0);
        }
        #[cfg(feature = "fastlane-census-hold-test")]
        27 => {
            sunlight_ipc::shm_free(sunlight_ipc::CapabilityToken(arg))
                .expect("release independent backing");
            mark("foreign-shm-released", 0, arg);
            reply(0, 0, 0);
        }
        WORD_ACCESS => {
            let va = arg & !7;
            let value = unsafe {
                if arg & 1 != 0 {
                    store_word(
                        va,
                        if arg & 2 == 0 {
                            WORD_FIRST
                        } else {
                            WORD_SECOND
                        },
                    );
                }
                load_word(va)
            };
            reply(0, value, 0);
        }
        ACCESS => {
            // Low two bits encode read/write; address is page aligned.
            let va = arg & !3;
            if arg & 2 != 0 {
                for offset in 0..4096 {
                    assert_eq!(unsafe { load(va + offset) }, 0);
                }
            }
            let value = unsafe {
                if arg & 1 != 0 {
                    store(va);
                }
                load(va)
            };
            reply(0, value as u64, 0);
        }
        FAULT => {
            let va = arg & !3;
            reply(0, 0, 0);
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-ACCESS] generation={} pid={} va={} write={} expected=fault",
                h.generation,
                sunlight_libc::getpid(),
                va,
                arg & 1
            ));
            unsafe {
                if arg & 1 != 0 {
                    store(va);
                } else {
                    let _ = load(va);
                }
            }
            panic!("forbidden access returned");
        }
        SPIN => {
            // Ordinary private IPC releases the supervisor, then this VM keeps
            // its channel translation active without kernel impersonation.
            let va = arg;
            reply(0, 0, 0);
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-ACCESS] generation={} pid={} va={} write=0 expected=remote-stale",
                h.generation,
                sunlight_libc::getpid(),
                va
            ));
            loop {
                unsafe {
                    let _ = load(va);
                }
                core::hint::spin_loop();
            }
        }
        COLLIDE => {
            let value =
                unsafe { sunlight_libc::sys::syscall6(50, arg, 4096, 3, 0x32, u64::MAX, 0) };
            assert_eq!(value, arg);
            reply(0, value, 0);
        }
        ESCALATE => {
            let protect = unsafe { sunlight_libc::sys::syscall3(52, arg, 4096, 3) };
            let replace =
                unsafe { sunlight_libc::sys::syscall6(50, arg, 4096, 3, 0x32, u64::MAX, 0) };
            let unmap = unsafe { sunlight_libc::sys::syscall2(51, arg, 4096) };
            assert_ne!(protect, 0);
            assert_ne!(replace, arg);
            assert_ne!(unmap, 0);
            reply(0, 0, 0);
        }
        _ => unreachable!(),
    }
}
fn map(h: fl::Handle, role: u64, region: u64) -> u64 {
    fl::operate(8, h, role * 3 + region).expect("attached mapping")
}
fn actor_map(a: Actor, h: fl::Handle, role: u64, region: u64) -> u64 {
    expect(a, 8, h, role * 3 + region, 0).words[1]
}
pub(super) fn census(checkpoint: u64) {
    let start = sunlight_ipc::monotonic_millis();
    loop {
        let snapshot = fl::call(9, fl::EMPTY, checkpoint, fl::NO_SESSION).unwrap();
        assert_eq!(snapshot.0, 0);
        assert_eq!(snapshot.1 & 0xffff_ffff, 0);
        if snapshot.1 == 0 {
            break;
        }
        assert!(
            sunlight_ipc::monotonic_millis() - start < 5000,
            "physical reaping deadline"
        );
        sunlight_ipc::process_yield();
    }
    zero();
}
fn pair_victim(a: Actor, role: u64) -> fl::Handle {
    let h = if role == 1 {
        pair(a)
    } else {
        let m = expect(a, AUTHORIZE, fl::EMPTY, 60000, 0);
        let h = fl::Handle {
            token: m.words[1],
            generation: m.words[2],
        };
        let receipt = authorize(1, h, 0).unwrap();
        expect(a, 2, h, receipt.token, 0);
        h
    };
    assert_eq!(fl::operate(3, h, 1 - role), Ok(0));
    expect(a, 3, h, role, 0);
    h
}
fn fault_reap(a: Actor, h: fl::Handle, kind: &str) {
    assert_eq!(super::super::reap(a.pid), 139);
    mark(kind, h.generation, a.pid);
    assert_eq!(fl::operate(8, h, 0), Err(fl::Error::Terminal));
    census(h.generation);
}
pub fn supervisor() -> ! {
    mark("start", 0, 0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let foreign = spawn(ep, public);
    // Each permission fault gets fresh, independently authorized peers.
    for role in 0..2 {
        let victim = spawn(ep, public);
        let h = pair_victim(victim, role);
        let mut pages = [0; 3];
        for region in 0..3 {
            pages[region as usize] = actor_map(victim, h, role, region);
            expect(victim, ACCESS, h, pages[region as usize] | 2, 0);
            let own = map(h, 1 - role, region);
            let write = if region == 1 { role == 1 } else { role == 0 };
            expect(
                victim,
                ACCESS,
                h,
                pages[region as usize] | u64::from(write),
                0,
            );
            unsafe {
                if !write {
                    store(own);
                }
                let _ = load(own);
            }
            // M3 compatibility probe only: the last aligned u64 on each M2
            // page can be copied privately and can subsequently change. This
            // does not attach a ring or execute the native transfer algorithm.
            let peer_word = pages[region as usize].checked_add(4088).unwrap();
            let own_word = own.checked_add(4088).unwrap();
            let (writer, reader, writer_va, reader_va) = if write {
                expect(victim, WORD_ACCESS, h, peer_word | 1, 0);
                let first = unsafe { load_word(own_word) };
                assert_eq!(first, WORD_FIRST);
                expect(victim, WORD_ACCESS, h, peer_word | 3, 0);
                assert_eq!(unsafe { load_word(own_word) }, WORD_SECOND);
                assert_eq!(first, WORD_FIRST); // private scalar remains stable
                (
                    victim.pid,
                    sunlight_libc::getpid(),
                    pages[region as usize],
                    own,
                )
            } else {
                unsafe { store_word(own_word, WORD_FIRST) };
                let first = expect(victim, WORD_ACCESS, h, peer_word, 0).words[1];
                assert_eq!(first, WORD_FIRST);
                unsafe { store_word(own_word, WORD_SECOND) };
                assert_eq!(
                    expect(victim, WORD_ACCESS, h, peer_word, 0).words[1],
                    WORD_SECOND
                );
                assert_eq!(first, WORD_FIRST);
                (
                    sunlight_libc::getpid(),
                    victim.pid,
                    own,
                    pages[region as usize],
                )
            };
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-M3-WORD] generation={} region={} writer={} reader={} writer_va={} reader_va={} offset=4088 first={} second={} private_copy=stable",
                h.generation, region, writer, reader, writer_va, reader_va, WORD_FIRST, WORD_SECOND
            ));
            expect(foreign, 8, h, role * 3 + region, 1);
            expect(victim, 8, h, role * 3 + region, 3);
        }
        mark("authorized-regions", h.generation, role);
        let forbidden = pages[if role == 0 { 1 } else { 2 }];
        expect(victim, ESCALATE, h, forbidden, 0);
        mark("alias-escalation-denied", h.generation, role);
        expect(victim, FAULT, h, forbidden | 1, 0);
        fault_reap(victim, h, "protected-write-fault");
    }
    // Consumer's second producer-owned read-only page is independently probed.
    {
        let a = spawn(ep, public);
        let h = pair_victim(a, 1);
        let va = actor_map(a, h, 1, 0);
        expect(a, FAULT, h, va | 1, 0);
        fault_reap(a, h, "producer-state-write-fault");
    }
    // Stale read and stale write after successful synchronous revocation.
    for write in 0..2 {
        let a = spawn(ep, public);
        let h = pair_victim(a, 1);
        let va = actor_map(a, h, 1, 1);
        let _ = map(h, 0, 1);
        assert_eq!(fl::operate(4, h, 0), Ok(0));
        expect(a, 8, h, 3, 4);
        assert_eq!(fl::operate(8, h, 0), Err(fl::Error::Terminal));
        expect(a, FAULT, h, va | write, 0);
        fault_reap(a, h, "revoked-access-fault");
    }
    // Real VA collision fails after backing allocation. Preserve the already
    // installed first region; a disposable actor owns the collision page.
    {
        let a = spawn(ep, public);
        let h = pair_victim(a, 1);
        let first = actor_map(a, h, 1, 0);
        expect(a, COLLIDE, h, first + 4096, 0);
        expect(a, 8, h, 4, 5);
        expect(a, ACCESS, h, first, 0);
        mark("mapping-rollback", h.generation, 0);
        assert_eq!(fl::operate(4, h, 0), Ok(0));
        stop(a);
        census(h.generation);
    }
    // No mapping before attachment; failed exec preserves views; successful
    // exec retires both VMs and refuses the replaced image's old role.
    {
        let a = spawn(ep, public);
        let h = pair(a);
        assert_eq!(fl::operate(8, h, 0), Err(fl::Error::Incomplete));
        expect(a, 8, h, 3, 7);
        assert_eq!(fl::operate(3, h, 0), Ok(0));
        expect(a, 3, h, 1, 0);
        let va = actor_map(a, h, 1, 1);
        let _ = map(h, 0, 1);
        expect(a, FAILED_EXEC, h, 0, 0);
        expect(a, ACCESS, h, va | 1, 0);
        assert!(sunlight_ipc::ipc_call_timeout(
            a.send,
            IpcMsg::with_label(CMD)
                .word(0, EXEC)
                .word(1, h.token)
                .word(2, h.generation),
            5000
        )
        .is_err());
        assert_eq!(super::super::reap(a.pid), 0);
        assert_eq!(fl::operate(8, h, 0), Err(fl::Error::Terminal));
        census(h.generation);
        mark("exec-retirement", h.generation, 0);
    }
    // A still-running independent VM makes the shootdown use a real remote
    // target. Its next access must fault after the revoker withdraws the PTE.
    {
        let a = spawn(ep, public);
        let h = pair_victim(a, 1);
        let va = actor_map(a, h, 1, 0);
        let _ = map(h, 0, 0);
        expect(a, SPIN, h, va, 0);
        // Yield permits actual peer execution; the raw instruction witness and
        // shootdown target mask, rather than timing, decide acceptance.
        for _ in 0..16 {
            sunlight_ipc::process_yield();
        }
        assert_eq!(fl::operate(4, h, 0), Ok(0));
        fault_reap(a, h, "remote-revocation-fault");
    }
    // Same-guest repeated lifecycles, alternating peer exit and revoke.
    for cycle in 0..6 {
        let a = spawn(ep, public);
        let h = pair_victim(a, 1);
        for region in 0..3 {
            let _ = map(h, 0, region);
            let _ = actor_map(a, h, 1, region);
        }
        if cycle % 2 == 0 {
            assert_eq!(fl::operate(4, h, 0), Ok(0));
        }
        stop(a);
        assert_eq!(fl::operate(8, h, 0), Err(fl::Error::Terminal));
        // Force real row reuse while the supervisor retains the old handle.
        let b = spawn(ep, public);
        let fresh = pair_victim(b, 1);
        assert_ne!(fresh.generation, h.generation);
        assert_eq!(fl::operate(8, h, 0), Err(fl::Error::Stale));
        assert_eq!(fl::operate(4, fresh, 0), Ok(0));
        stop(b);
        #[cfg(feature = "fastlane-census-hold-test")]
        let held = if cycle == 0 {
            Some(expect(foreign, 26, h, 0, 0).words[1])
        } else {
            None
        };
        census(100 + cycle);
        #[cfg(feature = "fastlane-census-hold-test")]
        if let Some(cap) = held {
            expect(foreign, 27, h, cap, 0);
        }
        mark("lifecycle-cycle", h.generation, cycle);
    }
    #[cfg(feature = "fastlane-census-leak-test")]
    {
        // Safe negative control: keep a normally authorized fixture mapping
        // alive at its required quiescence checkpoint. No retirement code or
        // permission is bypassed. An unrelated ordinary-SHM free offsets its
        // global frame delta but must never discharge this exact obligation.
        let held = expect(foreign, 26, fl::EMPTY, 0, 0).words[1];
        let peer = spawn(ep, public);
        let leaked = pair_victim(peer, 1);
        let _ = map(leaked, 0, 0);
        expect(foreign, 27, fl::EMPTY, held, 0);
        mark("fixture-owned-retained", leaked.generation, peer.pid);
        census(999);
        panic!("fixture-owned leak incorrectly accepted");
    }
    #[cfg(not(feature = "fastlane-census-leak-test"))]
    finish(foreign, ep)
}

#[cfg(not(feature = "fastlane-census-leak-test"))]
fn finish(foreign: Actor, ep: sunlight_ipc::EndpointId) -> ! {
    stop(foreign);
    assert!(sunlight_ipc::endpoint_destroy(ep));
    census(999);
    mark("complete", 0, 0);
    sunlight_libc::exit(0)
}
