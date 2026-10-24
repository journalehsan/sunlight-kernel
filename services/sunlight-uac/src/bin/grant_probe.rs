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
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sunlight_ipc::debug_log("[GRANT-C] client FAIL");
    sunlight_libc::exit(1)
}
