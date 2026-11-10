//! Both resource-owner cleanup orders around real SessionD retirement.
//! This module is reachable only from the explicit quota-lifecycle-test payload.
use super::*;

pub(super) struct Retained {
    before: (CapabilityToken, *mut u8),
    after: (CapabilityToken, *mut u8),
    after_pid: u64,
}

pub(super) fn owner(mode: &str, control: *mut u8) -> ! {
    let (slot, bytes) = match mode {
        "r53-cleanup-first" => (8, 8192),
        "r53-retirement-first" => (11, 12288),
        _ => panic!("unknown retirement owner mode"),
    };
    let (data, token) = sunlight_ipc::shm_create(4096, 0).expect("R53 owner backing");
    unsafe {
        write(data, 0, MAGIC);
        write(control, slot + 1, token.0);
    }
    snapshot(mode, bytes);
    sunlight_ipc::debug_log(&alloc::format!(
        "[R53] event=owner-ready mode={} pid={}",
        mode,
        sunlight_libc::getpid()
    ));
    unsafe { write(control, slot, 1) };
    wait(control, slot + 2, 1);
    if mode == "r53-retirement-first" {
        assert_eq!(query("[R5] session-live"), 0);
        assert!(sunlight_ipc::shm_create(4096, 0).is_err());
        assert!(sunlight_libc::spawn(
            b"/bin/session-launch-probe",
            &[b"session-launch-probe", b"child"],
            None,
        )
        .is_err());
        snapshot("r53-owner-retired-denied", 12288);
        sunlight_ipc::debug_log("[R53] event=owner-retired-denied allocation=denied launch=denied");
    }
    // Ordinary exit: safe process cleanup retires this owner's mappings.
    sunlight_libc::exit(0);
}

pub(super) fn prepare(control: *mut u8, token: CapabilityToken) -> Retained {
    sunlight_ipc::debug_log("[R53] event=begin");
    for slot in 8..14 {
        unsafe { write(control, slot, 0) };
    }
    let before_pid = spawn("r53-cleanup-first", token, CapabilityToken(0));
    wait(control, 8, 1);
    let before_token = CapabilityToken(unsafe { read(control, 9) });
    let before_view = sunlight_ipc::shm_map(before_token).expect("R53 first peer attach");
    let after_pid = spawn("r53-retirement-first", token, CapabilityToken(0));
    wait(control, 11, 1);
    let after_token = CapabilityToken(unsafe { read(control, 12) });
    let after_view = sunlight_ipc::shm_map(after_token).expect("R53 second peer attach");
    snapshot("r53-both-ready", 12288);
    sunlight_ipc::debug_log(&alloc::format!(
        "[R53] event=peer-ready before_owner={} after_owner={} peer={}",
        before_pid,
        after_pid,
        sunlight_libc::getpid()
    ));
    assert_eq!(query("[R5] session-live"), 1);
    unsafe { write(control, 10, 1) };
    reaped(before_pid, 0);
    assert_eq!(query("[R5] session-live"), 1);
    snapshot("r53-cleanup-before-retirement", 12288);
    assert_eq!(unsafe { read(before_view, 0) }, MAGIC);
    assert_eq!(unsafe { read(after_view, 0) }, MAGIC);
    Retained {
        before: (before_token, before_view),
        after: (after_token, after_view),
        after_pid,
    }
}

pub(super) fn finish(control: *mut u8, retained: Retained) {
    // Caller has observed registry retirement and actual new backing/launch
    // denial. The second owner cannot exit until this release word is set.
    assert_eq!(query("[R5] session-live"), 0);
    snapshot("r53-retired-admission-denied", 12288);
    unsafe { write(control, 13, 1) };
    reaped(retained.after_pid, 0);
    snapshot("r53-retirement-before-cleanup", 12288);
    assert_eq!(unsafe { read(retained.before.1, 0) }, MAGIC);
    assert_eq!(unsafe { read(retained.after.1, 0) }, MAGIC);
    sunlight_ipc::shm_free(retained.before.0).expect("R53 first final peer release");
    snapshot("r53-first-reclaimed", 8192);
    assert!(sunlight_ipc::shm_free(retained.before.0).is_err());
    assert_eq!(unsafe { read(retained.after.1, 0) }, MAGIC);
    sunlight_ipc::shm_free(retained.after.0).expect("R53 second final peer release");
    snapshot("r53-second-reclaimed", 4096);
    assert!(sunlight_ipc::shm_free(retained.after.0).is_err());
    sunlight_ipc::debug_log("[R53] event=orders-complete");
}
