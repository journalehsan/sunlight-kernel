//! Opt-in native client of the real UAC/VFS daemons. No synthetic issuer roles.
#![no_std]
#![no_main]
extern crate alloc;

use sunlight_ipc::{
    ipc_call, nameserver_lookup, vfs_grants as api, CapabilityToken, IpcMsg, VfsMsg,
};
#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();
static mut HEAP: [u8; 256 * 1024] = [0; 256 * 1024];

fn note(phase: &str) {
    use core::fmt::Write;
    let mut text = heapless::String::<160>::new();
    let _ = write!(
        text,
        "[GRANT-C] client pid={} uid={} gid={} phase={} PASS",
        sunlight_libc::getpid(),
        sunlight_libc::getuid(),
        sunlight_libc::getgid(),
        phase
    );
    sunlight_ipc::debug_log(&text);
}
fn endpoint(name: &str) -> CapabilityToken {
    let deadline = sunlight_ipc::monotonic_millis() + 15_000;
    loop {
        if let Some(token) = nameserver_lookup(name) {
            return token;
        }
        assert!(
            sunlight_ipc::monotonic_millis() < deadline,
            "production service unavailable"
        );
        sunlight_ipc::process_yield();
    }
}
fn base(uac: CapabilityToken) -> IpcMsg {
    // Fixture credentials from the repository's development account database.
    // The real password verifier, SHM transport and kernel mint all execute.
    let auth = sunlight_uac::auth::authenticate_password_for_session(b"root", b"root")
        .expect("root fixture authentication");
    assert_eq!(auth.uid, 0);
    let msg = IpcMsg::with_label(api::BASE_CAPS)
        .word(0, 0)
        .with_cap(0, auth.session_grant);
    let reply = ipc_call(uac, msg);
    if reply.label == api::REPLY_OK {
        assert_eq!(reply.cap_count, 2);
        // The password proof has been consumed, never echoed as a VFS token.
        assert!(reply.caps[0] != auth.session_grant, "proof echoed");
        assert!(reply.caps[1] != auth.session_grant, "proof echoed");
        assert_eq!(ipc_call(uac, msg).label, api::REPLY_ERR);
    }
    reply
}
fn authorize(vfs: CapabilityToken, token: CapabilityToken, path: &[u8]) -> bool {
    let mut msg = IpcMsg::with_label(VfsMsg::AUTHORIZE_EXEC).with_cap(0, token);
    assert!(path.len() < 32);
    for (i, byte) in path.iter().enumerate() {
        msg.words[i / 8] |= (*byte as u64) << (8 * (i % 8));
    }
    msg.word_count = (path.len() / 8 + 1) as u32;
    let reply = ipc_call(vfs, msg);
    reply.label == VfsMsg::REPLY && reply.words[0] == 0 && reply.words[1] == 1
}
fn exact(uac: CapabilityToken, vfs: CapabilityToken) {
    let auth = sunlight_uac::auth::authenticate_password_for_session(b"root", b"root")
        .expect("exact-path fixture authentication");
    let mut msg = IpcMsg::with_label(api::DELEGATE)
        .word(0, 0)
        .word(2, 0)
        .with_cap(0, auth.session_grant);
    for (i, byte) in b"/bin/true".iter().enumerate() {
        msg.words[1 + i / 8] |= (*byte as u64) << (8 * (i % 8));
    }
    let reply = ipc_call(uac, msg);
    assert_eq!(reply.label, api::REPLY_OK);
    assert_eq!(reply.cap_count, 1);
    assert!(reply.caps[0] != auth.session_grant, "proof echoed");
    assert_eq!(ipc_call(uac, msg).label, api::REPLY_ERR);
    assert!(!authorize(vfs, reply.caps[0], b"/bin/truex"));
    assert!(authorize(vfs, reply.caps[0], b"/bin/true"));
    assert!(!authorize(vfs, reply.caps[0], b"/bin/true"));
    note("exact-path-consumed");
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        ALLOCATOR
            .lock()
            .init(core::ptr::addr_of_mut!(HEAP).cast::<u8>(), 256 * 1024);
    }
    assert_eq!(sunlight_libc::getuid(), 1000);
    assert_eq!(sunlight_libc::getgid(), 1000);
    match sunlight_libc::getpid() {
        0xF4F0 => race_controller(),
        0xF4F1 | 0xF4F2 => race_recipient(),
        0xF4F3 | 0xF4F4 => consumption_recipient(),
        _ => {}
    }
    let uac = endpoint("uac");
    let vfs = endpoint("vfs");
    // A public endpoint token is not an administrative proof, even though it
    // is nonzero and was obtained legitimately by this process.
    assert_eq!(
        ipc_call(
            uac,
            IpcMsg::with_label(api::BASE_CAPS)
                .word(0, 0)
                .with_cap(0, vfs)
        )
        .label,
        api::REPLY_ERR
    );
    assert!(api::mint(vfs, None).is_none()); // direct untrusted issuer denied
    note("invalid-issuer-and-proof");
    for cycle in 0..4 {
        let reply = base(uac);
        if cycle == 2 {
            // The opt-in kernel fixture cancels this actual UAC request after
            // commit, before its reply. A cancellation is not a grant reply.
            assert_ne!(reply.label, api::REPLY_OK);
            note("delivery-cancelled");
            continue;
        }
        assert_eq!(reply.label, api::REPLY_OK);
        assert!(!authorize(vfs, reply.caps[0], b"/binary/true"));
        assert!(authorize(vfs, reply.caps[0], b"/bin/true"));
        assert!(!authorize(vfs, reply.caps[0], b"/bin/true"));
        assert!(api::cancel(reply.caps[1]));
        assert!(api::cancel(reply.caps[1]));
        assert!(!authorize(vfs, reply.caps[1], b"/usr/bin/true"));
        note("delivered-consumed-cancelled");
    }
    exact(uac, vfs);
    let reply = base(uac);
    assert_eq!(reply.label, api::REPLY_OK);
    note("recipient-exec-prepared");
    // The kernel fixture checks actual successful exec withdrawal and probes
    // the retired token against the new VM identity without printing it.
    sunlight_libc::exec(b"/bin/true", &[b"true"]).expect("recipient exec");
    panic!("exec returned");
}

fn race_recipient() -> ! {
    let uac = endpoint("uac");
    // Registration must precede the UAC rendezvous, including on one CPU.
    let control = endpoint("grant-racer");
    let auth = sunlight_uac::auth::authenticate_password_for_session(b"root", b"root")
        .expect("race password verification");
    let request = IpcMsg::with_label(api::BASE_CAPS)
        .word(0, 0)
        .with_cap(0, auth.session_grant)
        .with_cap(1, control);
    let _ = sunlight_ipc::ipc_call_timeout(uac, request, 20_000);
    panic!("race recipient survived forced exit");
}

fn race_controller() -> ! {
    let ep = sunlight_ipc::endpoint_create();
    assert!(sunlight_ipc::nameserver_register("grant-racer", ep));
    let mut seen = [0u8; 2];
    let mut seen_consumers = [false; 2];
    for _ in 0..6 {
        let msg = sunlight_ipc::ipc_recv_timeout(ep, 20_000).expect("bounded race request");
        if msg.label == 0xF11B0F {
            let pid = msg.words[0];
            let index = if pid == 0xF4F3 {
                0
            } else {
                assert_eq!(pid, 0xF4F4);
                1
            };
            assert!(!seen_consumers[index]);
            seen_consumers[index] = true;
            if index == 0 {
                sunlight_libc::kill(pid, 9).expect("recipient retirement before VFS consume");
            } else {
                // The recipient's real IPC deadline cancels its request, then it
                // execs /bin/true. Hold VFS until that image has exited. The kernel
                // independently asserts the actual VM change, not just death.
                let deadline = sunlight_ipc::monotonic_millis() + 10_000;
                while sunlight_ipc::process_is_alive(pid) {
                    assert!(sunlight_ipc::monotonic_millis() < deadline);
                    sunlight_ipc::process_yield();
                }
            }
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xF11B0F)).expect("resume VFS");
            continue;
        }
        assert_eq!(msg.label, 0xF11B0D);
        assert_eq!(msg.cap_count, 0);
        let index = match msg.words[0] {
            0xF4F1 => 0,
            0xF4F2 => 1,
            _ => panic!("unexpected race recipient"),
        };
        assert_eq!(msg.words[1], u64::from(seen[index]) + 1);
        assert_eq!(msg.words[2], index as u64);
        if seen[index] == 0 {
            assert!(sunlight_ipc::process_is_alive(msg.words[0]));
            sunlight_libc::kill(msg.words[0], 9).expect("ordinary recipient SIGKILL");
            assert!(!sunlight_ipc::process_is_alive(msg.words[0]));
        }
        seen[index] += 1;
        if seen[index] == 2 {
            sunlight_ipc::debug_log(if index == 0 {
                "[GRANT-D] case=exit-before-commit checkpoint=uac-resumed result=aborted"
            } else {
                "[GRANT-D] case=exit-after-commit checkpoint=uac-resumed result=committed"
            });
        }
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xF11B0D)).expect("race acknowledgement");
    }
    assert_eq!(seen, [2, 2]);
    assert_eq!(seen_consumers, [true; 2]);
    // A later synchronous UAC request proves both stale reply attempts have
    // passed through the real single-threaded service loop before completion.
    let uac = endpoint("uac");
    let reply = sunlight_ipc::ipc_call_timeout(uac, IpcMsg::with_label(0xF11B0D), 10_000)
        .expect("post-race service progress");
    assert_eq!(reply.label, api::REPLY_ERR);
    let vfs = endpoint("vfs");
    assert!(!authorize(vfs, CapabilityToken::INVALID, b"/bin/true"));
    sunlight_ipc::debug_log(
        "[GRANT-F] consumer-progress retired_exit=1 retired_exec=1 stale_reply=excluded PASS",
    );
    sunlight_ipc::debug_log(
        "[GRANT-D] driver COMPLETE precommit=aborted postcommit=retired stale_reply=excluded",
    );
    // Observe death independently of waitpid's retained-slot exit receipt.
    // The kernel's actual reaper marker validates clean exit and full cleanup.
    let mut previous = 0;
    for _ in 0..8 {
        let pid =
            sunlight_libc::spawn(b"/bin/true", &[b"true"], None).expect("ordinary allocator spawn");
        assert!(pid > previous);
        let deadline = sunlight_ipc::monotonic_millis() + 10_000;
        while sunlight_ipc::process_is_alive(pid) {
            assert!(sunlight_ipc::monotonic_millis() < deadline);
            sunlight_ipc::process_yield();
        }
        previous = pid;
    }
    sunlight_ipc::debug_log("[GRANT-F] pid-contract cycles=8 same_numeric_pid=0 policy=non-reuse scope=ordinary-runtime-spawn PASS");
    sunlight_libc::exit(0)
}

fn consumption_recipient() -> ! {
    let uac = endpoint("uac");
    let vfs = endpoint("vfs");
    let control = endpoint("grant-racer");
    let grants = base(uac);
    assert_eq!(grants.label, api::REPLY_OK);
    let mut msg = IpcMsg::with_label(VfsMsg::AUTHORIZE_EXEC)
        .with_cap(0, grants.caps[0])
        .with_cap(1, control);
    for (i, byte) in b"/bin/true".iter().enumerate() {
        msg.words[i / 8] |= (*byte as u64) << (8 * (i % 8));
    }
    msg.word_count = 2;
    if sunlight_libc::getpid() == 0xF4F3 {
        // This case is terminated by the controller, never by an IPC deadline.
        // The collector and controller bound failure if SIGKILL is not reached.
        let _ = sunlight_ipc::ipc_call(vfs, msg);
        panic!("exit recipient resumed before controller retirement");
    }
    let result = sunlight_ipc::ipc_call_timeout(vfs, msg, 1000);
    assert_eq!(sunlight_libc::getpid(), 0xF4F4, "exit recipient resumed");
    assert!(
        matches!(result, Err(sunlight_ipc::IpcCallError::Timeout)),
        "held request must expire"
    );
    sunlight_ipc::debug_log("[GRANT-F] recipient deadline=expired next=exec");
    sunlight_libc::exec(b"/bin/true", &[b"true"]).expect("exec held old request");
    panic!("exec returned");
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sunlight_ipc::debug_log("[GRANT-C] client FAIL");
    sunlight_libc::exit(1)
}
