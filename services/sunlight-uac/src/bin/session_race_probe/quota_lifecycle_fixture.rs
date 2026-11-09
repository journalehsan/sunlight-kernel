//! Authenticated native processes, ordinary SHM and real exit/kill/fault/exec.
//! Handshakes establish syscall-boundary orders; no sleep establishes a race.
use sunlight_ipc::CapabilityToken;
const MAGIC: u64 = 0x5235_4c49_5645;

fn query(command: &str) -> u64 {
    unsafe {
        sunlight_libc::sys::syscall2(
            sunlight_libc::sys::SYS_DEBUG_LOG,
            command.as_ptr() as u64,
            command.len() as u64,
        )
    }
}
fn snapshot(label: &str, expected: u64) {
    assert_eq!(
        query(&alloc::format!("[R5] observe {}", label)),
        expected,
        "Account charge at checkpoint"
    );
}
fn marker(case: &str, event: &str) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[R5-USER] case={} event={} pid={}",
        case,
        event,
        sunlight_libc::getpid()
    ));
}
// Aligned machine-word handshakes avoid references into shared memory. x86 TSO
// orders these ordinary loads/stores; asm also supplies the compiler barrier.
unsafe fn read(p: *mut u8, index: usize) -> u64 {
    let value: u64;
    core::arch::asm!("mov {value}, qword ptr [{addr}]", value=out(reg) value, addr=in(reg) p.add(index*8), options(nostack, readonly));
    value
}
unsafe fn write(p: *mut u8, index: usize, value: u64) {
    core::arch::asm!("mov qword ptr [{addr}], {value}", addr=in(reg) p.add(index*8), value=in(reg) value, options(nostack));
}
fn wait(p: *mut u8, index: usize, value: u64) {
    let start = sunlight_ipc::monotonic_millis();
    for _ in 0..1_000_000 {
        if unsafe { read(p, index) } == value {
            return;
        }
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
            "R5 handshake deadline"
        );
        sunlight_ipc::process_yield();
    }
    panic!("R5 handshake exhausted");
}
fn reaped(pid: u64, expected: u64) {
    assert_eq!(super::reap(pid), expected);
    let start = sunlight_ipc::monotonic_millis();
    while query(&alloc::format!("[R5] reaped {}", pid)) != 1 {
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
            "R5 reaper deadline"
        );
        sunlight_ipc::process_yield();
    }
}
fn spawn(mode: &str, control: CapabilityToken, data: CapabilityToken) -> u64 {
    let c = alloc::format!("{}", control.0);
    let d = alloc::format!("{}", data.0);
    sunlight_libc::spawn(
        b"/bin/session-launch-probe",
        &[
            b"session-launch-probe",
            mode.as_bytes(),
            c.as_bytes(),
            d.as_bytes(),
        ],
        None,
    )
    .expect("R5 authenticated child")
}
pub(super) fn actor(args: &[&str]) -> ! {
    let mode = args[1];
    let control = CapabilityToken(args[2].parse().unwrap());
    let p = sunlight_ipc::shm_map(control).expect("R5 control mapping");
    if mode == "r5-session-retained" {
        let owner: u64 = args[3].parse().expect("retiring owner PID");
        assert_eq!(unsafe { read(p, 3) }, MAGIC);
        unsafe {
            write(p, 0, 1);
        }
        let start = sunlight_ipc::monotonic_millis();
        while query("[R5] session-live") != 0 {
            assert!(
                sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
                "session retirement deadline"
            );
            sunlight_ipc::process_yield();
        }
        assert!(
            sunlight_ipc::shm_create(4096, 0).is_err(),
            "retired allocation admitted"
        );
        assert!(
            sunlight_libc::spawn(
                b"/bin/session-launch-probe",
                &[b"session-launch-probe", b"child"],
                None
            )
            .is_err(),
            "retired launch admitted"
        );
        snapshot("retired-surviving-peer", 4096);
        while query(&alloc::format!("[R5] session-owner-reaped {}", owner)) != 1 {
            assert!(
                sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
                "retiring owner reap deadline"
            );
            sunlight_ipc::process_yield();
        }
        snapshot("retired-owner-reaped", 4096);
        assert_eq!(
            unsafe { read(p, 3) },
            MAGIC,
            "retired peer backing inaccessible"
        );
        sunlight_ipc::shm_free(control).expect("retired session final mapping release");
        snapshot("retired-final-reclaimed", 0);
        assert!(
            sunlight_ipc::shm_free(control).is_err(),
            "retired duplicate release accepted"
        );
        assert!(
            sunlight_ipc::shm_create(4096, 0).is_err(),
            "retired session reused after refund"
        );
        sunlight_ipc::debug_log("[R51-SESSION-USER] event=complete owner_exit=before_peer retired_admission=denied final_charge=0");
        sunlight_libc::exit(0);
    }
    if mode == "r5-replaced" {
        snapshot("exec-replaced", 8192);
        unsafe {
            write(p, 0, 3);
        }
        sunlight_libc::exit(0);
    }
    snapshot(
        "actor-entry",
        if mode.starts_with("r5-peer") {
            8192
        } else {
            4096
        },
    );
    let peer = mode.starts_with("r5-peer");
    let (data, token) = if peer {
        let token = CapabilityToken(args[3].parse().unwrap());
        (
            sunlight_ipc::shm_map(token).expect("R5 peer mapping"),
            token,
        )
    } else {
        let result = sunlight_ipc::shm_create(4096, 0).expect("R5 owned backing");
        unsafe {
            write(result.0, 0, MAGIC);
        }
        result
    };
    assert_eq!(unsafe { read(data, 0) }, MAGIC);
    snapshot("actor-published", 8192);
    unsafe {
        write(p, 1, token.0);
        write(p, 0, 1);
    }
    wait(p, 2, 2);
    if mode == "r5-owner-exec" {
        snapshot("exec-old", 8192);
        let c = alloc::format!("{}", control.0);
        sunlight_libc::exec(
            b"/bin/session-launch-probe",
            &[b"session-launch-probe", b"r5-replaced", c.as_bytes()],
        )
        .expect("R5 exec");
        panic!("R5 exec returned");
    }
    if mode.ends_with("crash") {
        marker(mode, "before-ud2");
        unsafe {
            core::arch::asm!("ud2", options(noreturn));
        }
    }
    if mode.ends_with("kill") {
        // The controller sends the real SIGKILL only after publication receipt.
        loop {
            core::hint::spin_loop();
        }
    }
    sunlight_libc::exit(0);
}

pub(super) fn controller() {
    snapshot("baseline", 0);
    let (control, control_token) = sunlight_ipc::shm_create(4096, 0).expect("R5 control backing");
    snapshot("control-live", 4096);
    for mode in [
        "r5-owner-exit",
        "r5-owner-kill",
        "r5-owner-crash",
        "r5-owner-exec",
        "r5-peer-exit",
        "r5-peer-crash",
    ] {
        marker(mode, "begin");
        for i in 0..3 {
            unsafe {
                write(control, i, 0);
            }
        }
        let peer = mode.starts_with("r5-peer");
        let mut view = core::ptr::null_mut();
        let mut token = CapabilityToken(0);
        if peer {
            (view, token) = sunlight_ipc::shm_create(4096, 0).expect("R5 surviving owner backing");
            unsafe {
                write(view, 0, MAGIC);
            }
        }
        let pid = spawn(mode, control_token, token);
        wait(control, 0, 1);
        if !peer {
            token = CapabilityToken(unsafe { read(control, 1) });
            assert!(
                sunlight_ipc::shm_free(token).is_err(),
                "unmapped foreign release admitted"
            );
            snapshot("foreign-release-denied", 8192);
            view = sunlight_ipc::shm_map(token).expect("R5 surviving peer view");
        }
        snapshot("two-views", 8192);
        marker(mode, "peer-attached");
        unsafe {
            write(control, 2, 2);
        }
        if mode.ends_with("kill") {
            sunlight_libc::kill(pid, 9).expect("R5 real SIGKILL");
        }
        if mode.ends_with("exec") {
            wait(control, 0, 3);
        }
        reaped(
            pid,
            if mode.ends_with("kill") {
                137
            } else if mode.ends_with("crash") {
                132
            } else {
                0
            },
        );
        snapshot("survivor-after-reap", 8192);
        assert_eq!(
            unsafe { read(view, 0) },
            MAGIC,
            "surviving authorized backing"
        );
        sunlight_ipc::shm_free(token).expect("R5 final view retirement");
        snapshot("final-view-reclaimed", 4096);
        assert!(
            sunlight_ipc::shm_free(token).is_err(),
            "R5 duplicate release admitted"
        );
        snapshot("duplicate-release-denied", 4096);
        let (_, fresh) = sunlight_ipc::shm_create(4096, 0).expect("R5 reallocation");
        assert!(
            sunlight_ipc::shm_map(token).is_err(),
            "R5 stale handle reused"
        );
        snapshot("stale-handle-denied", 8192);
        sunlight_ipc::shm_free(fresh).expect("R5 fresh retirement");
        snapshot("iteration-closed", 4096);
        marker(mode, "complete");
    }
    sunlight_ipc::shm_free(control_token).expect("R5 control retirement");
    snapshot("quiescent", 0);
    marker("lifecycle-subset", "complete");
}

/// The real SessionD Logout path revokes authority and sends SIGTERM to this
/// owner. Its independently launched peer retains one ordinary backing page.
pub(super) fn prepare_retirement() {
    let (page, token) = sunlight_ipc::shm_create(4096, 0).expect("retirement backing");
    unsafe {
        write(page, 0, 0);
        write(page, 3, MAGIC);
    }
    let peer = spawn(
        "r5-session-retained",
        token,
        CapabilityToken(sunlight_libc::getpid()),
    );
    wait(page, 0, 1);
    snapshot("retirement-prepared", 4096);
    sunlight_ipc::debug_log(&alloc::format!(
        "[R51-SESSION-USER] event=prepared owner={} peer={}",
        sunlight_libc::getpid(),
        peer
    ));
}
