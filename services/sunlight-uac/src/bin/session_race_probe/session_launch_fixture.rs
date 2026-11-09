//! Scheduled M5.3R4.1 client; no trusted service roles.
use core::sync::atomic::{AtomicBool, Ordering};
use sunlight_ipc::{CapabilityToken, IpcMsg, SpawnMsg, SpawnRequest};
static THREAD_DONE: AtomicBool = AtomicBool::new(false);
static BLOCK_READY: AtomicBool = AtomicBool::new(false);
static BLOCK_RELEASE: AtomicBool = AtomicBool::new(false);
extern "C" fn blocking_worker(_: *mut u8) -> *mut u8 {
    observe("live-worker");
    BLOCK_READY.store(true, Ordering::Release);
    while !BLOCK_RELEASE.load(Ordering::Acquire) {
        sunlight_ipc::process_yield();
    }
    core::ptr::null_mut()
}
fn wait_quiescent(tid: u32) {
    let start = sunlight_ipc::monotonic_millis();
    while unsafe { sunlight_libc::sys::syscall1(156, tid as u64) } != 1 {
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
            "worker quiescence deadline"
        );
        sunlight_ipc::process_yield();
    }
}
fn start_live_worker() -> sunlight_libc::thread::JoinHandle {
    let worker = unsafe { sunlight_libc::thread::spawn(blocking_worker, core::ptr::null_mut()) }
        .expect("live worker");
    let start = sunlight_ipc::monotonic_millis();
    while !BLOCK_READY.load(Ordering::Acquire) {
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
            "live worker deadline"
        );
        sunlight_ipc::process_yield();
    }
    worker
}

fn observe(stage: &str) {
    let query = "[SL41] observe";
    let result = unsafe {
        sunlight_libc::sys::syscall2(
            sunlight_libc::sys::SYS_DEBUG_LOG,
            query.as_ptr() as u64,
            query.len() as u64,
        )
    };
    assert_eq!(result, 1, "missing live Account membership");
    sunlight_ipc::debug_log(&alloc::format!(
        "[SL41-USER] stage={} pid={}",
        stage,
        sunlight_libc::getpid()
    ));
}
extern "C" fn worker(_: *mut u8) -> *mut u8 {
    observe("thread");
    let (_, token) = sunlight_ipc::shm_create(4096, 0).expect("thread SHM");
    sunlight_ipc::shm_free(token).expect("thread SHM release");
    THREAD_DONE.store(true, Ordering::Release);
    core::ptr::null_mut()
}
pub fn run(args: &[&str]) -> ! {
    #[cfg(feature = "quota-lifecycle-test")]
    if args.get(1).is_some_and(|s| s.starts_with("r5-")) {
        super::quota_lifecycle_fixture::actor(args);
    }
    observe("entry");
    if sunlight_libc::getuid() == 1000 {
        observe("dropped-credentials-child");
        sunlight_libc::exit(0);
    }
    match args.get(1).copied() {
        Some("exec") => {
            observe("exec");
            sunlight_libc::exit(0);
        }
        Some("child") => {
            observe("child");
            let worker = start_live_worker();
            assert!(
                sunlight_libc::exec(
                    b"/bin/session-launch-probe",
                    &[b"session-launch-probe", b"exec"]
                )
                .is_err(),
                "exec admitted live borrower"
            );
            observe("exec-live-thread-denied");
            BLOCK_RELEASE.store(true, Ordering::Release);
            assert_eq!(super::reap(worker.tid as u64), 0);
            wait_quiescent(worker.tid);
            sunlight_libc::exec(
                b"/bin/session-launch-probe",
                &[b"session-launch-probe", b"exec"],
            )
            .expect("exec child");
            panic!("exec returned");
        }
        Some("drop-credentials") => {
            assert_eq!(unsafe { sunlight_libc::sys::syscall1(37, 1000) }, 0);
            let mut request = IpcMsg::empty();
            SpawnRequest::new("/bin/session-launch-probe", "dropped-child")
                .with_identity(0, 0)
                .pack_into(&mut request);
            let reply =
                sunlight_ipc::ipc_call_timeout(CapabilityToken(0xCAFEBABE_DEADBEEF), request, 5000)
                    .expect("inherited IPC spawn");
            assert_eq!(reply.label, SpawnMsg::REPLY);
            assert_eq!(super::reap(reply.words[0]), 0);
            observe("dropped-credentials-parent");
            sunlight_libc::exit(0);
        }
        Some("exit-live-thread") => {
            let _worker = start_live_worker();
            observe("process-exit-live-thread");
            sunlight_libc::exit(0);
        }
        _ => {}
    }
    let sessiond = super::endpoint(sunlight_ipc::SESSION_ENDPOINT);
    let generation = sunlight_ipc::current_process_generation();
    let hello = sunlight_ipc::ipc_call_timeout(
        sessiond,
        IpcMsg::with_label(sunlight_ipc::SessionMsg::SESSION_COMPONENT_HELLO)
            .word(0, sunlight_ipc::SESSION_PROTOCOL_VERSION as u64)
            .word(1, generation),
        5000,
    )
    .expect("component hello");
    assert_eq!(hello.label, sunlight_ipc::SessionMsg::REPLY);
    let ready = sunlight_ipc::ipc_call_timeout(
        sessiond,
        IpcMsg::with_label(sunlight_ipc::SessionMsg::SESSION_COMPONENT_READY)
            .word(0, hello.words[0])
            .word(1, hello.words[1])
            .word(2, hello.words[2])
            .word(3, generation),
        5000,
    )
    .expect("component ready");
    assert_eq!(ready.label, sunlight_ipc::SessionMsg::REPLY);
    #[cfg(feature = "quota-lifecycle-test")]
    super::quota_lifecycle_fixture::controller();
    #[cfg(feature = "quota-remote-test")]
    {
        let child = sunlight_libc::spawn(
            b"/tmp/session-race-probe",
            &[b"session-race-probe", b"FL-M2"],
            None,
        ).expect("authenticated remote mapping child");
        assert_eq!(super::reap(child), 0);
        sunlight_ipc::debug_log("[R5-REMOTE] event=controller-complete");
    }
    // Session IDs alone must not authorize an ordinary user process.
    let spawn = CapabilityToken(0xCAFEBABE_DEADBEEF);
    let req = SpawnRequest::new("/bin/session-launch-probe", "forged");
    let mut msg = IpcMsg::empty();
    req.pack_into(&mut msg);
    msg.label = SpawnMsg::SPAWN_SESSION_INITIAL;
    msg.caps[0] = CapabilityToken(1);
    msg.caps[1] = CapabilityToken(1);
    let reply = sunlight_ipc::ipc_call_timeout(spawn, msg, 5000).expect("forged request transport");
    assert_eq!(reply.label, SpawnMsg::ERROR);
    sunlight_ipc::debug_log("[SL41-USER] stage=foreign-authority-denied");
    let (_, token) = sunlight_ipc::shm_create(4096, 0).expect("initial SHM");
    let thread = unsafe { sunlight_libc::thread::spawn(worker, core::ptr::null_mut()) }
        .expect("thread launch");
    let start = sunlight_ipc::monotonic_millis();
    while !THREAD_DONE.load(Ordering::Acquire) {
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
            "thread deadline"
        );
        sunlight_ipc::process_yield();
    }
    assert_eq!(super::reap(thread.tid as u64), 0);
    assert!(
        sunlight_libc::spawn(b"/test/launch-bad-elf", &[], None).is_err(),
        "invalid ELF child admitted"
    );
    sunlight_ipc::debug_log("[SL41-USER] stage=child-image-rollback");
    let child = sunlight_libc::spawn(
        b"/bin/session-launch-probe",
        &[b"session-launch-probe", b"child"],
        None,
    )
    .expect("child launch");
    assert_eq!(super::reap(child), 0);
    let linux = sunlight_libc::spawn(b"/bin/linux-thread-probe", &[b"linux-thread-probe"], None)
        .expect("Linux clone probe");
    assert_eq!(super::reap(linux), 0);
    sunlight_ipc::debug_log("[SL41-USER] stage=linux-clone-complete");
    let dropped = sunlight_libc::spawn(
        b"/bin/session-launch-probe",
        &[b"session-launch-probe", b"drop-credentials"],
        None,
    )
    .expect("credential-drop child");
    assert_eq!(super::reap(dropped), 0);
    let exit_child = sunlight_libc::spawn(
        b"/bin/session-launch-probe",
        &[b"session-launch-probe", b"exit-live-thread"],
        None,
    )
    .expect("live-exit child");
    assert_eq!(super::reap(exit_child), 0);
    sunlight_ipc::debug_log("[SL41-USER] stage=process-exit-live-thread-reaped");
    let action = |operation: sunlight_ipc::SessionAction| {
        let reply = sunlight_ipc::ipc_call_timeout(
            sessiond,
            IpcMsg::with_label(sunlight_ipc::SessionMsg::SESSION_ACTION)
                .word(0, hello.words[0])
                .word(1, hello.words[1])
                .word(2, operation as u64),
            5000,
        )
        .expect("session action");
        assert_eq!(reply.label, sunlight_ipc::SessionMsg::REPLY);
    };
    action(sunlight_ipc::SessionAction::Lock);
    assert!(
        sunlight_ipc::shm_create(4096, 0).is_err(),
        "suspended SHM admitted"
    );
    assert!(
        sunlight_libc::spawn(
            b"/bin/session-launch-probe",
            &[b"session-launch-probe", b"exec"],
            None
        )
        .is_err(),
        "suspended child admitted"
    );
    assert!(
        unsafe { sunlight_libc::thread::spawn(worker, core::ptr::null_mut()) }.is_err(),
        "suspended thread admitted"
    );
    sunlight_ipc::debug_log("[SL41-USER] stage=suspended-creation-denied");
    action(sunlight_ipc::SessionAction::UnlockCompleted);
    observe("resumed");
    sunlight_ipc::shm_free(token).expect("retained SHM release after suspension");
    #[cfg(feature = "quota-lifecycle-test")]
    super::quota_lifecycle_fixture::prepare_retirement();
    observe("complete");
    sunlight_ipc::debug_log("[SL41-USER] stage=complete");
    action(sunlight_ipc::SessionAction::Logout);
    loop {
        sunlight_ipc::process_yield();
    }
}
