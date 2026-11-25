#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv_timeout, ipc_reply, nameserver_endpoint_is_live,
    nameserver_note_diagnostic, CapabilityToken, EndpointId, InitMsg, InitStatus, IpcMsg,
    NameserverDiagnosticEvent, SpawnMsg,
};

/// Base servers that init launches via the kernel spawn capability once it
/// holds the spawn token. These need no privileged memory setup (unlike
/// vfs_server/tty_server, which the kernel spawns directly). sunlightd in turn
/// launches the user-level daemons (timezone_service, niced, gcd).
///
/// Order matters for interactive boot:
/// - `timer_server` first so timed IPC works.
/// - `sunlight-kbd` / `sunlight-mouse` immediately after so the IRQ1 path can
///   come up while login is painted. tty_server is kernel-spawned earlier and
///   blocks on nameserver REGISTER("tty"); without early kbd + draining the
///   nameserver during this spawn list, keyboard looks dead at login.
/// - Remaining daemons (net, power, display, …) follow.
///
/// Spawn returns after the process is created, but REGISTER/LOOKUP traffic from
/// already-running servers (tty, vfs, …) must be drained *between* spawns —
/// init used to spawn the full list before entering the nameserver loop, which
/// delayed `"tty"` registration until after networkd/powerd/etc.
const INIT_SERVICES: [&str; 16] = [
    "/sbin/timer_server",
    "/sbin/sunlight-kbd",
    "/sbin/sunlight-mouse",
    "/sbin/sunlight-usb-mouse",
    "/sbin/sunlight-swapd",
    "/sbin/deviced",
    "/sbin/sunlightd",
    "/sbin/networkd",
    "/sbin/resolved",
    "/sbin/powerd",
    "/sbin/thermald",
    "/sbin/audiod",
    "/sbin/sunlight-display",
    "/sbin/mezzo",
    // PTY broker used by sunlight-libc::openpty() for tty/session spawning.
    "/sbin/pty_server",
    "/sbin/net_server",
];

/// How many nameserver messages to accept after each spawn before continuing.
const DRAIN_MSGS_PER_SPAWN: usize = 8;
/// Per-recv timeout (ms) while draining; keeps boot moving if the queue is empty.
const DRAIN_TIMEOUT_MS: u64 = 2;
/// Extra drain passes after the last boot spawn so late REGISTERs land.
const DRAIN_MSGS_AFTER_BOOT: usize = 16;
const DRAIN_TIMEOUT_AFTER_BOOT_MS: u64 = 5;
const REGISTRY_CAPACITY: usize = 64;

type Registry = [RegistryEntry; REGISTRY_CAPACITY];

/// Spawn a service by absolute path using the kernel spawn capability.
fn spawn_service(spawn_cap: CapabilityToken, path: &str) -> bool {
    let mut msg = IpcMsg::with_label(SpawnMsg::SPAWN);
    // Pack the path into the first 4 words (32 bytes max), little-endian.
    let path_bytes = path.as_bytes();
    let mut i = 0;
    while i < 4 {
        let mut word: u64 = 0;
        let mut j = 0;
        while j < 8 {
            let idx = i * 8 + j;
            if idx < path_bytes.len() {
                word |= (path_bytes[idx] as u64) << (j * 8);
            }
            j += 1;
        }
        msg.words[i] = word;
        i += 1;
    }
    let reply = ipc_call(spawn_cap, msg);
    reply.label == SpawnMsg::REPLY
}

fn handle_nameserver_msg(registry: &mut Registry, msg: IpcMsg) -> IpcMsg {
    match msg.label {
        InitMsg::REGISTER => register_service(
            registry,
            msg.words[0],
            CapabilityToken(msg.words[1]),
            msg.words[2] as u32,
            msg.badge as usize,
        ),
        InitMsg::LOOKUP => lookup_service(registry, msg.words[0]),
        _ => IpcMsg::with_label(InitMsg::DENY),
    }
}

/// Non-blocking-ish drain: answer up to `budget` pending REGISTER/LOOKUP calls.
///
/// Critical during boot so tty_server can register `"tty"` (and kbd can look it
/// up) while init is still launching the rest of the service tree.
fn drain_nameserver(ep: EndpointId, registry: &mut Registry, budget: usize, timeout_ms: u64) {
    for _ in 0..budget {
        match ipc_recv_timeout(ep, timeout_ms) {
            Some(msg) => {
                let reply = handle_nameserver_msg(registry, msg);
                ipc_reply(reply);
            }
            None => break,
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start(spawn_token: u64) -> ! {
    debug_log("[ init] SunlightOS init process started");
    debug_log("[ init] Waiting for system services to register...");

    let ep = endpoint_create();
    debug_log("[init] Name server: listening");
    if name_to_u64("sunlight-kv") != name_to_u64("sunlight-tls") {
        debug_log("[init] nameserver long-name keys OK");
    } else {
        debug_log("[init] ERROR: nameserver long-name key collision");
    }

    let mut registry = [RegistryEntry::empty(); REGISTRY_CAPACITY];
    run_registry_self_tests();

    // Register the kernel spawn endpoint if token was passed.
    if spawn_token != 0 {
        let name = name_to_u64("spawn");
        let _ = registry_insert_kernel(&mut registry, name, CapabilityToken(spawn_token));
        debug_log("[init] Registered kernel spawn endpoint");

        // Launch base servers, draining nameserver traffic between each spawn
        // so early clients (especially tty_server → "tty") are not stuck until
        // the entire list is created.
        let spawn_cap = CapabilityToken(spawn_token);
        // Spawn inheritance snapshots an already-bound namespace. Starting a
        // child before this handshake would copy sticky service routing without
        // any namespace authority, permanently denying its later app lookups.
        // Keep answering REGISTER/LOOKUP while VFS finishes its boot handshake.
        loop {
            use sunlight_ipc::vfs_rpc::{response::Kind, stat_transport as api};
            drain_nameserver(ep, &mut registry, DRAIN_MSGS_PER_SPAWN, DRAIN_TIMEOUT_MS);
            if matches!(api::root_metadata(), Ok(metadata)
                if metadata.kind == Kind::Directory && metadata.links > 0)
            {
                debug_log("[init] VFS namespace ready before service spawn");
                break;
            }
        }
        // Native spawn stamps storagectl with its existing root-only StorageAdmin grant.
        // Complete the bounded pass before launching ordinary boot services.
        #[cfg(feature = "vfs-l1c-1-test")]
        {
            #[cfg(feature = "vfs-l1c-r3-test")]
            {
                let pid = unsafe { sunlight_libc::sys::syscall2(0xf1c1, 8, 0) };
                assert!(pid != 0 && pid != u64::MAX);
                loop {
                    match unsafe { sunlight_libc::sys::syscall2(0xf1c1, 8, 0) } {
                        0 => sunlight_libc::yield_now(),
                        1 => break,
                        _ => panic!("unexpected R3 checkpoint result"),
                    }
                }
                assert_eq!(sunlight_libc::waitpid(pid), Ok(0));
                assert!(sunlight_libc::waitpid(pid).is_err());
                debug_log("[VFS-R3] init consumed child status=0 once");
                let observer = loop {
                    match unsafe { sunlight_libc::sys::syscall2(0xf1c1, 10, 0) } {
                        0 => sunlight_libc::yield_now(),
                        1 | u64::MAX => panic!("unexpected R3 cleanup result"),
                        observer => break observer,
                    }
                };
                assert_eq!(sunlight_libc::waitpid(observer), Ok(0));
                debug_log("[VFS-R3] init reaped observer status=0");
                #[cfg(feature = "vfs-l1c-adapter-accounting-test")]
                {
                    await_adapter_accounting();
                    debug_log("[VFS-ADAPTER] complete");
                }
                loop {
                    sunlight_libc::yield_now();
                }
            }
            #[cfg(all(feature = "vfs-l1c-r2-test", not(feature = "vfs-l1c-r3-test")))]
            {
                let observer = unsafe { sunlight_libc::sys::syscall2(0xf1c1, 7, 0) };
                assert!(observer != 0 && observer != u64::MAX);
                loop {
                    match unsafe { sunlight_libc::sys::syscall2(0xf1c1, 7, 0) } {
                        0 => sunlight_libc::yield_now(),
                        u64::MAX => break,
                        _ => panic!("unexpected R2 publication result"),
                    };
                }
                assert_eq!(sunlight_libc::waitpid(observer), Ok(0));
                debug_log("[VFS-R2] init observed creation failure and reaped observer status=0");
                #[cfg(feature = "vfs-l1c-adapter-accounting-test")]
                {
                    await_adapter_accounting();
                    debug_log("[VFS-ADAPTER] complete");
                }
                loop {
                    sunlight_libc::yield_now();
                }
            }
            #[cfg(all(feature = "vfs-l1c-r1-test", not(any(feature = "vfs-l1c-r2-test", feature = "vfs-l1c-r3-test"))))]
            {
                let mode = if cfg!(feature = "vfs-l1c-r1b-test") { 6 } else { 5 };
                let pid = unsafe { sunlight_libc::sys::syscall2(0xf1c1, mode, 0) };
                assert!(pid != u64::MAX && sunlight_libc::waitpid(pid) == Ok(0));
                debug_log("[VFS-R1] init reaped observer status=0");
                #[cfg(feature = "vfs-l1c-adapter-accounting-test")]
                {
                    await_adapter_accounting();
                    debug_log("[VFS-ADAPTER] complete");
                }
                loop {
                    sunlight_libc::yield_now();
                }
            }
            #[cfg(feature = "vfs-l1c-file-ipc-test")]
            run_file_ipc_accounting();
            #[cfg(all(feature = "vfs-l1c-parent-lifecycle-test", not(feature = "vfs-l1c-file-ipc-test")))]
            run_parent_lifecycle();
            #[cfg(all(feature = "vfs-l1c-r5-test", not(any(feature = "vfs-l1c-r1-test", feature = "vfs-l1c-parent-lifecycle-test"))))]
            {
                for mode in [0, 4] {
                    let pid = unsafe { sunlight_libc::sys::syscall2(0xf1c1, mode, 0) };
                    assert!(pid != u64::MAX && sunlight_libc::waitpid(pid) == Ok(0));
                    assert!(sunlight_libc::waitpid(pid).is_err());
                    debug_log(if mode == 0 {
                        "[VFS-R5] init consumed published status once"
                    } else {
                        "[VFS-R5] init consumed observer status once"
                    });
                }
                #[cfg(feature = "vfs-l1c-adapter-accounting-test")]
                for mode in [1, 2, 3, 11] {
                    await_adapter_accounting();
                    let observer = unsafe { sunlight_libc::sys::syscall2(0xf1c1, mode, 0) };
                    assert!(observer != 0 && observer != u64::MAX);
                    assert_eq!(sunlight_libc::waitpid(observer), Ok(0));
                    assert!(sunlight_libc::waitpid(observer).is_err());
                    await_adapter_accounting();
                }
                #[cfg(feature = "vfs-l1c-adapter-accounting-test")]
                debug_log("[VFS-ADAPTER] complete");
                for cycle in 0..=1000 {
                    let pid = if cycle == 0 {
                        0
                    } else {
                        let pid = sunlight_libc::spawn(b"/bin/true", &[], None)
                            .expect("ordinary heap-accounting spawn");
                        assert_eq!(sunlight_libc::waitpid(pid), Ok(0));
                        pid
                    };
                    loop {
                        match unsafe { sunlight_libc::sys::syscall2(0xf1c2, cycle, pid) } {
                            0 => sunlight_libc::yield_now(),
                            1 => break,
                            _ => panic!("heap-accounting checkpoint rejected"),
                        };
                    }
                }
                loop {
                    sunlight_libc::yield_now();
                }
            }
            #[cfg(all(feature = "vfs-l1c-r4-test", not(any(feature = "vfs-l1c-r1-test", feature = "vfs-l1c-r5-test"))))]
            for _ in 0..8 {
                let pid = unsafe { sunlight_libc::sys::syscall2(0xf1c1, 4, 0) };
                assert!(pid != u64::MAX && sunlight_libc::waitpid(pid) == Ok(0));
            }
            #[cfg(all(feature = "vfs-l1c-2-test", not(feature = "vfs-l1c-r4-test")))]
            for _ in 0..34 {
                for mode in [1, 2, 0] {
                    let pid = unsafe { sunlight_libc::sys::syscall2(0xf1c1, mode, 0) };
                    assert!(pid != u64::MAX && sunlight_libc::waitpid(pid) == Ok(0));
                }
            }
            #[cfg(all(feature = "vfs-l1c-r4-test", not(any(feature = "vfs-l1c-r1-test", feature = "vfs-l1c-r5-test"))))]
            debug_log("[VFS-R4] init completed cycles=8");
            #[cfg(all(feature = "vfs-l1c-2-test", not(feature = "vfs-l1c-r4-test")))]
            debug_log("[VFS-L1C-2] mixed transactions complete cycles=102");
            #[cfg(not(feature = "vfs-l1c-2-test"))]
            {
            let result = unsafe { sunlight_libc::sys::syscall2(0xf1c1, 0, 0) };
            if result != u64::MAX && sunlight_libc::waitpid(result) == Ok(0) {
                debug_log("[VFS-L1C-1] init observed result");
            } else {
                debug_log("[VFS-L1C-1] init observed failure");
            }
            }
            #[cfg(feature = "vfs-l1c-2-test")]
            debug_log("[VFS-L1C-1] init observed result");
        }
        #[cfg(feature = "zram-lifecycle-test")]
        run_zram_lifecycle();
        match sunlight_libc::spawn(b"/bin/storagectl", &[b"storagectl", b"fstab"], None)
            .and_then(sunlight_libc::waitpid)
        {
            Ok(0) => debug_log("[FSTAB] boot apply complete"),
            _ => {
                debug_log("[FSTAB] REQUIRED FAILURE: boot degraded; inspect storagectl diagnostics")
            }
        }

        for path in INIT_SERVICES.iter() {
            #[cfg(feature = "zram-lifecycle-test")]
            if *path == "/sbin/sunlight-swapd" {
                continue;
            }
            if spawn_service(spawn_cap, path) {
                debug_log("[init] launched base service");
            } else {
                debug_log("[init] FAILED to launch base service");
            }
            #[cfg(feature = "recovery-test")]
            if *path == "/sbin/sunlightd" {
                if !spawn_service(spawn_cap, path) {
                    debug_log("[D36R-TEST] live manager bootstrap competitor rejected PASS");
                } else {
                    debug_log("[D36R-TEST] FAIL duplicate bootstrap authority");
                }
            }
            drain_nameserver(ep, &mut registry, DRAIN_MSGS_PER_SPAWN, DRAIN_TIMEOUT_MS);
        }
        drain_nameserver(
            ep,
            &mut registry,
            DRAIN_MSGS_AFTER_BOOT,
            DRAIN_TIMEOUT_AFTER_BOOT_MS,
        );
        debug_log("[init] base service spawn pass complete");
    }

    // The kernel retains the authenticated bootstrap/grant even if a spawn
    // reply was lost. Finished never permits restart; only its exact reap does.
    let mut monitor = sunlight_ipc::supervision::BootstrapMonitor::default();
    let mut next_probe = 0;
    let mut root_stat_verified = false;
    let mut path_stat_verified = false;
    loop {
        if let Some(msg) = ipc_recv_timeout(ep, 100) {
            ipc_reply(handle_nameserver_msg(&mut registry, msg));
        }
        if !root_stat_verified {
            use sunlight_ipc::vfs_rpc::{response::Kind, stat_transport as api};
            match api::root_metadata() {
                Ok(metadata) if metadata.kind == Kind::Directory && metadata.links > 0 => {
                    root_stat_verified = true;
                    debug_log(
                        "[VFS-RPC] native root stat caller resume/result copy OK transport=copy",
                    );
                }
                Err(api::NOT_READY) => {} // Provisioning can follow this turn.
                _ => {
                    root_stat_verified = true;
                    debug_log("[VFS-RPC] ERROR native root stat failed");
                }
            }
        }
        if root_stat_verified && !path_stat_verified {
            path_stat_verified = true;
            if verify_service_stat_paths() {
                debug_log(
                    "[VFS-RPC] native path stat traversal/provenance/errors OK transport=copy",
                );
                verify_native_stat_path();
                #[cfg(feature = "stat-failure-test")]
                {
                    let cap = registry
                        .iter()
                        .find(|r| r.name == name_to_u64("vfs") && r.is_live())
                        .expect("live VFS probe service")
                        .public_cap;
                    verify_stat_failures(cap);
                }
            } else {
                debug_log("[VFS-RPC] ERROR native path stat failed");
            }
        }
        use sunlight_ipc::supervision::{self as own, BootstrapAction};
        let now = sunlight_ipc::monotonic_millis();
        if now < next_probe || spawn_token == 0 {
            continue;
        }
        next_probe = now.saturating_add(100);
        if let Some(status) = own::query(own::STATUS, 0) {
            let registered = registry
                .iter()
                .any(|r| r.name == name_to_u64("sunlightd") && r.is_live());
            match monitor.step(now, status, registered) {
                BootstrapAction::Launch => {
                    debug_log("[INIT-RECOVERY] exact manager reap; bounded diagnostic restart");
                    if !spawn_service(CapabilityToken(spawn_token), "/sbin/sunlightd") {
                        debug_log("[INIT-RECOVERY] diagnostic spawn failed");
                    }
                }
                BootstrapAction::Exhausted => {
                    debug_log("[INIT-RECOVERY] restart budget exhausted; recovery blocked")
                }
                BootstrapAction::RegistrationTimeout => {
                    debug_log("[INIT-RECOVERY] registration timeout; terminal recovery blocked")
                }
                BootstrapAction::ReapTimeout => {
                    debug_log("[INIT-RECOVERY] manager reap timeout; terminal recovery blocked")
                }
                BootstrapAction::Wait => {}
            }
        }
    }
}

/// Exercise authenticated service-only metadata plus ordinary path semantics.
/// The pinned-directory fixture exists in the service's live namespace.
fn verify_service_stat_paths() -> bool {
    use sunlight_ipc::vfs_rpc::{response::Kind, stat_transport as api};
    let Ok(absolute) = api::stat("/etc/motd") else {
        return false;
    };
    let Ok(relative) = api::stat("etc/./motd") else {
        return false;
    };
    if absolute != relative || absolute.kind != Kind::File || absolute.size == 0 {
        return false;
    }
    // Put the target name at the end of a maximum-size input: truncation would
    // return root metadata and must fail this comparison.
    let mut long = [b'/'; api::MAX_LOOKUP_PATH];
    let suffix = b"etc/motd";
    let start = long.len() - suffix.len();
    long[start..].copy_from_slice(suffix);
    if api::stat(core::str::from_utf8(&long).unwrap()) != Ok(absolute) {
        return false;
    }

    let Ok(fixture) = api::stat("/tmp/vfs-pins") else {
        return false;
    };
    if fixture.kind != Kind::Directory || fixture.uid != 71 || fixture.gid != 72 {
        return false;
    }
    let Ok(root) = api::root_metadata() else {
        return false;
    };
    let Ok(clamped) = api::stat("../../../../") else {
        return false;
    };
    absolute.provenance != fixture.provenance
        && root == clamped
        && api::stat("/etc/.vfs-stat-missing") == Err((-2i64) as u64)
        && api::stat("/etc/motd/..") == Err((-20i64) as u64)
        && api::stat("/etc/motd/") == Err((-20i64) as u64)
}

fn verify_native_stat_path() {
    use sunlight_ipc::vfs_rpc::{response::Kind, stat_transport as api};
    for path in ["/etc/motd", "etc/./motd", "/tmp/vfs-pins", "../../../../"] {
        let expected = api::stat(path).unwrap();
        let actual = sunlight_libc::stat(path.as_bytes()).expect("native StatPath");
        assert_eq!(actual.size, expected.size);
        assert_eq!(actual.uid, expected.uid);
        assert_eq!(actual.gid, expected.gid);
        assert_eq!(actual.mode & 0o7777, expected.mode);
        let kind = if expected.kind == Kind::File {
            0o100000
        } else {
            0o040000
        };
        assert_eq!(actual.mode & 0o170000, kind);
        assert_eq!(actual.file_type, expected.kind as u8);
        assert_eq!(actual.nlinks, expected.links);
    }
    assert!(matches!(
        sunlight_libc::stat(b"/etc/.vfs-stat-missing"),
        Err(sunlight_libc::Errno::NoEntry)
    ));
    assert!(matches!(
        sunlight_libc::stat(b"/etc/motd/.."),
        Err(sunlight_libc::Errno::NotDir)
    ));
    assert!(matches!(
        sunlight_libc::stat(b""),
        Err(sunlight_libc::Errno::Inval)
    ));
    // Copy failure must consume its completion and permit a fresh request.
    assert_eq!(
        unsafe { sunlight_libc::sys::syscall2(61, b"/etc/motd\0".as_ptr() as u64, 0) },
        u64::MAX
    );
    assert!(sunlight_libc::stat(b"/etc/motd").is_ok());
    verify_linux_service_stat();
    debug_log("[VFS-RPC] native StatPath service routing/ABI/errors/copy recovery OK");
}

// Invoke the normalized Linux entry from native init; Linux personalities also
// reach this handler through stat/lstat/newfstatat translation.
fn verify_linux_service_stat() {
    let call = |path: &[u8], fd: i32, flags: u64, output: &mut [u8; 144]| unsafe {
        sunlight_libc::sys::syscall4(
            1026,
            fd as u64,
            path.as_ptr() as u64,
            output.as_mut_ptr() as u64,
            flags,
        )
    };
    let mut absolute = [0; 144];
    let mut relative = [0; 144];
    assert_eq!(call(b"/etc/motd\0", -1, 0, &mut absolute), 0);
    assert_eq!(call(b"etc/./motd\0", -100, 0x100, &mut relative), 0);
    assert_eq!(absolute, relative);
    let metadata = sunlight_ipc::vfs_rpc::stat_transport::stat("/etc/motd").unwrap();
    assert_eq!(&absolute[..8], &metadata.provenance.mount_id.to_le_bytes());
    assert_eq!(
        &absolute[8..16],
        &metadata.provenance.object_id.to_le_bytes()
    );
    assert_eq!(&absolute[48..56], &metadata.size.to_le_bytes());
    assert_eq!(&absolute[28..32], &metadata.uid.to_le_bytes());
    let mut canary = [0xa5; 144];
    for (path, fd, flags, errno) in [
        (b"/etc/.vfs-stat-missing\0".as_slice(), -100, 0, 2),
        (b"/etc/motd/..\0".as_slice(), -100, 0, 20),
        (b"\0".as_slice(), -100, 0, 2),
        (b"etc/motd\0".as_slice(), 5, 0, 95),
        (b"/etc/motd\0".as_slice(), -100, 0x1000, 22),
    ] {
        assert_eq!(call(path, fd, flags, &mut canary), 0u64.wrapping_sub(errno));
        assert_eq!(canary, [0xa5; 144]);
    }
    assert_eq!(
        unsafe {
            sunlight_libc::sys::syscall4(
                1026,
                (-100i64) as u64,
                b"/etc/motd\0".as_ptr() as u64,
                0,
                0,
            )
        },
        (-14i64) as u64
    );
    assert_eq!(call(b"/etc/motd\0", -100, 0, &mut relative), 0);
    debug_log("[VFS-RPC] Linux stat service routing/ABI/errors/copy recovery OK");
}

#[cfg(feature = "stat-failure-test")]
fn verify_stat_failures(cap: CapabilityToken) {
    use sunlight_ipc::vfs_rpc::stat_transport as api;
    let arm = |mode| {
        let reply = ipc_call(cap, IpcMsg::with_label(0x53544641).word(0, mode));
        assert_eq!(reply.words[0], 0);
    };
    let original = api::stat("/etc/motd").expect("original metadata");
    arm(1);
    let start = sunlight_ipc::monotonic_millis();
    assert_eq!(api::stat("/etc/motd"), Err(api::TIMED_OUT));
    let elapsed = sunlight_ipc::monotonic_millis().saturating_sub(start);
    assert!((4900..15000).contains(&elapsed));
    // The service sends the saved late reply while this new ID is waiting.
    assert_eq!(api::stat("/etc/motd"), Ok(original));
    debug_log("[VFS-RPC] native stat timeout/resume/new request OK");
    arm(1);
    assert!(matches!(
        sunlight_libc::stat(b"/etc/motd"),
        Err(sunlight_libc::Errno::Failed)
    ));
    assert!(sunlight_libc::stat(b"/etc/motd").is_ok());
    debug_log("[VFS-RPC] native StatPath timeout/resume/recovery OK");
    arm(1);
    let mut linux_output = [0xa5; 144];
    let linux_stat = |output: &mut [u8; 144]| unsafe {
        sunlight_libc::sys::syscall4(
            1026,
            (-100i64) as u64,
            b"/etc/motd\0".as_ptr() as u64,
            output.as_mut_ptr() as u64,
            0,
        )
    };
    assert_eq!(linux_stat(&mut linux_output), (-110i64) as u64);
    assert_eq!(linux_output, [0xa5; 144]);
    assert_eq!(linux_stat(&mut linux_output), 0);
    debug_log("[VFS-RPC] Linux stat timeout/resume/recovery OK");
    verify_stat_output_vm(original);
    stat_vm_race::verify(original);
    verify_stat_caller_lifecycle(cap, original);
    verify_file_caller(cap);
    arm(2);
    assert_eq!(api::stat("/etc/motd"), Err(api::IO_ERROR));
    // No fallback to the still-present kernel filesystem after epoch loss.
    assert_eq!(api::stat("/etc/motd"), Err(api::NOT_READY));
    assert!(matches!(
        sunlight_libc::stat(b"/etc/motd"),
        Err(sunlight_libc::Errno::Again)
    ));
    linux_output.fill(0xa5);
    assert_eq!(linux_stat(&mut linux_output), (-11i64) as u64);
    assert_eq!(linux_output, [0xa5; 144]);
    debug_log("[VFS-RPC] Linux stat service loss/no kernel fallback OK");
    debug_log("[VFS-RPC] native StatPath service loss/no kernel fallback OK");
    arm(3); // Wait for the service's post-fault log and legacy receive loop.
    debug_log("[VFS-RPC] native stat service loss/resume/no fallback OK");
}

/// Mutate real user mappings after publication and before result collection.
/// The reply may win before mutation; both orderings must validate at copy time.
#[cfg(feature = "stat-failure-test")]
fn verify_stat_output_vm(original: sunlight_ipc::vfs_rpc::response::Metadata) {
    use sunlight_ipc::vfs_rpc::stat_transport as api;
    use sunlight_libc::mman::*;
    const PAGE: usize = 4096;
    const BYTES: usize = 56;
    for remap in [false, true] {
        let base = mmap(
            core::ptr::null_mut(),
            2 * PAGE,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
        .expect("stat output mapping");
        // A cross-page destination detects writes performed before validating
        // the entire output range. Keep the first page as a canary.
        let output = unsafe { base.add(PAGE - BYTES / 2) };
        unsafe {
            core::ptr::write_bytes(output, 0xa5, BYTES);
        }
        assert_eq!(
            unsafe { api::probe_stat_attempt("/etc/motd", output as u64) },
            api::PENDING
        );
        let second = unsafe { base.add(PAGE) };
        munmap(second, PAGE).expect("withdraw output page");
        if remap {
            assert_eq!(
                mmap(
                    second,
                    PAGE,
                    PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
                    -1,
                    0
                )
                .expect("replace output page"),
                second
            );
        }
        let result = loop {
            let result = unsafe { api::probe_stat_attempt("/etc/motd", output as u64) };
            if result != api::PENDING {
                break result;
            }
            sunlight_ipc::process_yield();
        };
        if remap {
            assert_eq!(result, 0);
            // Copy through an owned array rather than creating an unaligned reference.
            let mut bytes = [0; BYTES];
            unsafe {
                core::ptr::copy_nonoverlapping(output, bytes.as_mut_ptr(), BYTES);
            }
            use sunlight_ipc::vfs_rpc::{
                codec::Request,
                response::{Reply, ReplyContract},
                Completion,
            };
            assert_eq!(
                ReplyContract::for_request(Request::Stat { path: "/etc/motd" })
                    .unwrap()
                    .validate(
                        Completion {
                            errno: 0,
                            output_bytes: BYTES as u64
                        },
                        &bytes
                    ),
                Ok(Reply::Metadata(original))
            );
            munmap(second, PAGE).expect("release replacement output");
        } else {
            assert_eq!(result, u64::MAX);
            let prefix = unsafe { core::slice::from_raw_parts(output, BYTES / 2) };
            assert!(prefix.iter().all(|b| *b == 0xa5));
        }
        munmap(base, PAGE).expect("release output canary");
        assert_eq!(api::stat("/etc/motd"), Ok(original));
    }
    debug_log("[VFS-RPC] native stat output unmap/remap atomic copy/recovery OK");
}

#[cfg(feature = "stat-failure-test")]
mod stat_vm_race;

#[derive(Clone, Copy, PartialEq, Eq)]
struct RegistryEntry {
    name: u64,
    public_cap: CapabilityToken,
    endpoint_id: u32,
    owner_pid: usize,
    kernel_entry: bool,
}

impl RegistryEntry {
    const fn empty() -> Self {
        Self {
            name: 0,
            public_cap: CapabilityToken::INVALID,
            endpoint_id: 0,
            owner_pid: 0,
            kernel_entry: false,
        }
    }

    fn is_empty(self) -> bool {
        self.name == 0
    }

    fn is_live(self) -> bool {
        self.kernel_entry || nameserver_endpoint_is_live(self.public_cap, self.endpoint_id)
    }
}

fn registry_insert_kernel(registry: &mut Registry, name: u64, cap: CapabilityToken) -> bool {
    for entry in registry.iter_mut() {
        if entry.is_empty() {
            *entry = RegistryEntry {
                name,
                public_cap: cap,
                endpoint_id: 0,
                owner_pid: 0,
                kernel_entry: true,
            };
            return true;
        }
    }
    false
}

fn register_service(
    registry: &mut Registry,
    name: u64,
    public_cap: CapabilityToken,
    endpoint_id: u32,
    owner_pid: usize,
) -> IpcMsg {
    if name == 0 || public_cap == CapabilityToken::INVALID || owner_pid == 0 {
        return deny(InitStatus::INVALID_REGISTRATION);
    }

    if let Some(index) = registry.iter().position(|entry| entry.name == name) {
        let current = registry[index];
        return register_existing(
            registry,
            index,
            current.is_live(),
            true,
            name,
            public_cap,
            endpoint_id,
            owner_pid,
        );
    }

    register_new(registry, true, name, public_cap, endpoint_id, owner_pid)
}

fn register_new(
    registry: &mut Registry,
    emit_diagnostics: bool,
    name: u64,
    public_cap: CapabilityToken,
    endpoint_id: u32,
    owner_pid: usize,
) -> IpcMsg {
    let Some(slot) = registry.iter_mut().find(|entry| entry.is_empty()) else {
        if emit_diagnostics {
            nameserver_note_diagnostic(NameserverDiagnosticEvent::REGISTRY_FULL);
        }
        return deny(InitStatus::REGISTRY_FULL);
    };
    *slot = RegistryEntry {
        name,
        public_cap,
        endpoint_id,
        owner_pid,
        kernel_entry: false,
    };
    grant()
}

fn register_existing(
    registry: &mut Registry,
    index: usize,
    current_is_live: bool,
    emit_diagnostics: bool,
    name: u64,
    public_cap: CapabilityToken,
    endpoint_id: u32,
    owner_pid: usize,
) -> IpcMsg {
    let current = registry[index];
    if current.kernel_entry {
        if emit_diagnostics {
            nameserver_note_diagnostic(NameserverDiagnosticEvent::LIVE_CONFLICT);
        }
        return deny(InitStatus::LIVE_NAME_CONFLICT);
    }
    if current_is_live {
        if current.owner_pid == owner_pid
            && current.endpoint_id == endpoint_id
            && current.public_cap == public_cap
        {
            return grant();
        }
        if emit_diagnostics {
            nameserver_note_diagnostic(NameserverDiagnosticEvent::LIVE_CONFLICT);
        }
        return deny(InitStatus::LIVE_NAME_CONFLICT);
    }

    if emit_diagnostics {
        nameserver_note_diagnostic(NameserverDiagnosticEvent::STALE_REMOVAL);
    }
    registry[index] = RegistryEntry {
        name,
        public_cap,
        endpoint_id,
        owner_pid,
        kernel_entry: false,
    };
    if emit_diagnostics {
        nameserver_note_diagnostic(NameserverDiagnosticEvent::DEAD_REPLACEMENT);
    }
    grant()
}

fn lookup_existing(
    registry: &mut Registry,
    index: usize,
    entry_is_live: bool,
    emit_diagnostics: bool,
) -> IpcMsg {
    let entry = registry[index];
    if !entry_is_live {
        registry[index] = RegistryEntry::empty();
        if emit_diagnostics {
            nameserver_note_diagnostic(NameserverDiagnosticEvent::STALE_REMOVAL);
            nameserver_note_diagnostic(NameserverDiagnosticEvent::STALE_LOOKUP);
        }
        return deny(InitStatus::NOT_FOUND);
    }
    IpcMsg::with_label(InitMsg::GRANT)
        .word(0, entry.public_cap.0)
        .word(1, InitStatus::OK)
}

fn lookup_service(registry: &mut Registry, name: u64) -> IpcMsg {
    let Some(index) = registry.iter().position(|entry| entry.name == name) else {
        return deny(InitStatus::NOT_FOUND);
    };
    let is_live = registry[index].is_live();
    lookup_existing(registry, index, is_live, true)
}

fn run_registry_self_tests() {
    let name = name_to_u64("registry-self-test");
    let old_cap = CapabilityToken(0x1111);
    let new_cap = CapabilityToken(0x2222);
    let mut registry = [RegistryEntry::empty(); REGISTRY_CAPACITY];
    registry[0] = RegistryEntry {
        name,
        public_cap: old_cap,
        endpoint_id: 10,
        owner_pid: 20,
        kernel_entry: false,
    };

    let idempotent = register_existing(&mut registry, 0, true, false, name, old_cap, 10, 20).label
        == InitMsg::GRANT;
    let conflict = register_existing(&mut registry, 0, true, false, name, new_cap, 11, 21);
    let conflict_preserved = conflict.label == InitMsg::DENY
        && conflict.words[0] == InitStatus::LIVE_NAME_CONFLICT
        && registry[0].public_cap == old_cap;
    let replaced = register_existing(&mut registry, 0, false, false, name, new_cap, 11, 21);
    let dead_replaced = replaced.label == InitMsg::GRANT
        && registry[0].public_cap == new_cap
        && registry[0].endpoint_id == 11
        && registry[0].owner_pid == 21;
    let stale_lookup = lookup_existing(&mut registry, 0, false, false);
    let stale_removed =
        stale_lookup.label == InitMsg::DENY && registry[0] == RegistryEntry::empty();

    let mut full = [RegistryEntry::empty(); REGISTRY_CAPACITY];
    for (index, entry) in full.iter_mut().enumerate() {
        *entry = RegistryEntry {
            name: (index + 1) as u64,
            public_cap: CapabilityToken((index + 1) as u64),
            endpoint_id: (index + 1) as u32,
            owner_pid: index + 1,
            kernel_entry: true,
        };
    }
    let before = full;
    let full_reply = register_new(&mut full, false, 99, CapabilityToken(99), 99, 99);
    let full_preserved = full_reply.label == InitMsg::DENY
        && full_reply.words[0] == InitStatus::REGISTRY_FULL
        && full == before;

    if idempotent && conflict_preserved && dead_replaced && stale_removed && full_preserved {
        debug_log("[init] nameserver registry integrity tests: OK");
    } else {
        debug_log("[init] nameserver registry integrity tests: UNEXPECTED");
    }
}

fn grant() -> IpcMsg {
    IpcMsg::with_label(InitMsg::GRANT).word(0, InitStatus::OK)
}

fn deny(status: u64) -> IpcMsg {
    IpcMsg::with_label(InitMsg::DENY).word(0, status)
}

fn name_to_u64(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 1;
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

/// Fresh process identities exercise the real scheduler exit and exec hooks.
#[cfg(feature = "stat-failure-test")]
fn verify_stat_caller_lifecycle(
    cap: CapabilityToken,
    original: sunlight_ipc::vfs_rpc::response::Metadata,
) {
    use sunlight_ipc::vfs_rpc::stat_transport as api;
    let mut digits = [0u8; 20];
    let mut number = cap.0;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (number % 10) as u8;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    for mode in [
        b"initial-cwd".as_slice(),
        b"narrow-root".as_slice(),
        b"failed-exec".as_slice(),
        b"exit".as_slice(),
        b"exec".as_slice(),
        b"failed-chdir".as_slice(),
        b"private-chdir".as_slice(),
        b"chdir".as_slice(),
        b"shared-chdir".as_slice(),
        b"same-credentials".as_slice(),
        b"setuid".as_slice(),
        b"setgid".as_slice(),
    ] {
        let pid = sunlight_libc::spawn(
            b"/bin/vfs-stat-caller",
            &[b"vfs-stat-caller", mode, &digits[start..]],
            None,
        )
        .expect("native stat caller");
        assert_eq!(sunlight_libc::waitpid(pid), Ok(0));
        if mode == b"exit" {
            // Exit has discarded the original caller. Exec tests rejection
            // inside the live replacement image, before that image exits.
            assert_eq!(
                ipc_call(cap, IpcMsg::with_label(0x53544641).word(0, 6)).words[0],
                0
            );
        }
        assert_eq!(api::stat("/etc/motd"), Ok(original));
    }
    debug_log("[VFS-RPC] native stat caller exit/exec stale reply/fresh recovery OK");
}

/// Dedicated native caller for the copied regular-file channel. The kernel
/// selects this exact fixture for `vfs_file_routed`; the child exercises open,
/// read, write, seek, fstat, dup, close and the exec descriptor transfer.
#[cfg(feature = "stat-failure-test")]
fn verify_file_caller(cap: CapabilityToken) {
    let mut digits = [0u8; 20];
    let mut number = cap.0;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (number % 10) as u8;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    let pid = sunlight_libc::spawn(
        b"/bin/vfs-stat-caller",
        &[b"vfs-stat-caller", b"file", &digits[start..]],
        None,
    )
    .expect("native file caller");
    assert_eq!(sunlight_libc::waitpid(pid), Ok(0));
}

#[cfg(feature = "vfs-l1c-file-ipc-test")]
fn run_file_ipc_accounting() -> ! {
    for cycle in 1..=10 {
        assert_eq!(unsafe { sunlight_libc::sys::syscall2(0xf1c3, cycle, 0) }, 1);
        let pid = sunlight_libc::spawn_with_stdio(
            b"/bin/vfs-lifecycle-parent",
            &[b"vfs-lifecycle-parent", b"file-ipc", b"run"],
            Some(sunlight_libc::Fd(0)),
            Some(sunlight_libc::Fd(1)),
            Some(sunlight_libc::Fd(2)),
        )
        .expect("ordinary file/IPC spawn");
        assert_eq!(sunlight_libc::waitpid(pid), Ok(37));
        assert!(sunlight_libc::waitpid(pid).is_err());
        let start = sunlight_ipc::monotonic_millis();
        loop {
            match unsafe { sunlight_libc::sys::syscall2(0xf1c3, cycle, pid) } {
                1 => break,
                0 => assert!(sunlight_ipc::monotonic_millis() - start < 5000),
                _ => panic!("file/IPC settlement rejected"),
            }
            sunlight_libc::yield_now();
        }
    }
    debug_log("[VFS-FILE-IPC] complete");
    loop {
        sunlight_libc::yield_now();
    }
}

#[cfg(all(feature = "vfs-l1c-parent-lifecycle-test", not(feature = "vfs-l1c-file-ipc-test")))]
fn run_parent_lifecycle() -> ! {
    for mode in [b"12", b"13", b"14", b"15", b"16"] {
        let pid = sunlight_libc::spawn(
            b"/bin/vfs-lifecycle-parent",
            &[b"vfs-lifecycle-parent", b"parent-lifecycle", mode],
            None,
        )
        .expect("spawn disposable lifecycle parent");
        assert_eq!(sunlight_libc::waitpid(pid), Ok(37));
        assert!(sunlight_libc::waitpid(pid).is_err());
        debug_log("[VFS-PARENT] init collected exit 37 exactly once");
        await_adapter_accounting();
    }
    debug_log("[VFS-PARENT] complete");
    loop {
        sunlight_libc::yield_now();
    }
}

#[cfg(feature = "vfs-l1c-adapter-accounting-test")]
fn await_adapter_accounting() {
    loop {
        match unsafe { sunlight_libc::sys::syscall2(0xf1c2, 2, 0) } {
            0 => sunlight_libc::yield_now(),
            1 => return,
            _ => panic!("adapter accounting checkpoint rejected"),
        }
    }
}

#[cfg(feature = "zram-lifecycle-test")]
fn run_zram_lifecycle() {
    let swapd = sunlight_libc::spawn(b"/sbin/sunlight-swapd", &[], None).expect("spawn swapd");
    assert_eq!(sunlight_libc::waitpid(swapd), Ok(0));
    wait_zram_reap(swapd);
    for _ in 0..100 {
        let pid = sunlight_libc::spawn(b"/bin/freezram", &[b"freezram", b"lifecycle-test"], None)
            .expect("spawn freezram");
        assert_eq!(sunlight_libc::waitpid(pid), Ok(37));
        assert!(sunlight_libc::waitpid(pid).is_err());
        wait_zram_reap(pid);
    }
    debug_log("[ZRAM-LIFE] completed cycles=100 status=37 collected_once=1");
}

#[cfg(feature = "zram-lifecycle-test")]
fn wait_zram_reap(pid: u64) {
    loop {
        match unsafe { sunlight_libc::sys::syscall2(0xf1c4, 0, pid) } {
            0 => sunlight_libc::yield_now(),
            1 => return,
            _ => panic!("reap checkpoint rejected"),
        }
    }
}
