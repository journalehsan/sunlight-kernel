//! Disposable native caller: exercise production exit and exec retirement.
#![no_std]
#![no_main]
use sunlight_ipc::{ipc_call, vfs_rpc::stat_transport as api, CapabilityToken, IpcMsg};

extern "C" fn change_cwd(_: *mut u8) -> *mut u8 {
    sunlight_libc::chdir(b"/etc").expect("sibling chdir");
    core::ptr::null_mut()
}

fn sibling_chdir(shared: bool) {
    let handle = unsafe {
        sunlight_libc::thread::spawn_with_stack(change_cwd, core::ptr::null_mut(), 65536, shared)
    }
    .expect("cwd worker");
    assert_eq!(sunlight_libc::waitpid(u64::from(handle.tid)), Ok(0));
    sunlight_libc::mman::munmap(handle.stack_base, handle.stack_size).unwrap();
    sunlight_libc::mman::munmap(handle.tls_base, handle.tls_size).unwrap();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    struct Log;
    impl core::fmt::Write for Log {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            sunlight_ipc::debug_log(s);
            Ok(())
        }
    }
    let _ = write!(Log, "[VFS-RPC] caller fixture failure: {}", info);
    sunlight_libc::exit(99)
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut args = [""; 3];
    assert_eq!(
        unsafe { sunlight_libc::crt0::collect_utf8_args(argc, argv, &mut args, 32) },
        3
    );
    let cap = CapabilityToken(args[2].parse::<u64>().unwrap());
    let control = |mode| {
        let reply = ipc_call(cap, IpcMsg::with_label(0x53544641).word(0, mode));
        assert_eq!(reply.label, 0x53544641);
        assert_eq!(reply.word_count, 1);
        reply.words[0]
    };
    if args[1] == "file-post-exec" {
        // fd 9 was a non-CLOEXEC service descriptor rekeyed by the kernel
        // TRANSFER publication; fd 8 was close-on-exec and must be gone. Read
        // immediately, before any lease wait, so the ordering is exercised.
        let mut buf = [0u8; 16];
        let read = sunlight_libc::read(sunlight_libc::Fd(9), &mut buf).expect("surviving read");
        assert_eq!(&buf[..read], b" from cat\n");
        assert_eq!(
            sunlight_libc::read(sunlight_libc::Fd(8), &mut buf[..1]),
            Err(sunlight_libc::Errno::BadFd)
        );
        sunlight_ipc::debug_log("[VFS-RPC] native copied file exec transfer/cloexec close OK");
        sunlight_libc::exit(0);
    }
    if args[1] == "denied-child" {
        assert_eq!(control(5), 0);
        assert_eq!(api::stat("/etc/motd"), Err(api::NOT_READY));
        verify_native_denied();
        sunlight_ipc::debug_log(
            "[VFS-RPC] withdrawn parent spawn remains selected without regrant OK",
        );
        sunlight_libc::exit(0);
    }
    if args[1] == "replacement" {
        // Reject the old held completion before provisioning a fresh VM scope.
        assert_eq!(control(6), 0);
    }
    if matches!(args[1], "derived-root" | "exec-root") {
        let start = sunlight_ipc::monotonic_millis();
        loop {
            assert_eq!(control(5), 0);
            match api::stat("/motd") {
                Ok(_) => break,
                Err(api::NOT_READY) => assert!(sunlight_ipc::monotonic_millis() - start < 5000),
                result => panic!("derived root: {:?}", result),
            }
            sunlight_ipc::process_yield();
        }
        assert!(sunlight_libc::stat(b"/motd").is_ok());
        assert!(sunlight_libc::stat(b"/etc/motd").is_err());
        assert_eq!(
            sunlight_libc::stat(b"../../motd").unwrap().size,
            sunlight_libc::stat(b"/motd").unwrap().size
        );
        if args[1] == "derived-root" {
            sunlight_libc::exec(
                b"/bin/vfs-stat-caller",
                &[b"vfs-stat-caller", b"exec-root", args[2].as_bytes()],
            )
            .unwrap();
            panic!("exec returned");
        }
        sunlight_ipc::debug_log(
            "[VFS-RPC] spawn/exec inherit narrowed root and independent leases OK",
        );
        sunlight_libc::exit(0);
    }
    let start = sunlight_ipc::monotonic_millis();
    let original = loop {
        // Drive the service poll while init waits for this child. Namespace
        // selection is staged by its authenticated snapshot on that turn.
        assert_eq!(control(5), 0);
        match api::stat("/etc/motd") {
            Ok(metadata) => break metadata,
            Err(api::NOT_READY) => assert!(sunlight_ipc::monotonic_millis() - start < 5000),
            result => panic!("provisioning: {:?}", result),
        }
        sunlight_ipc::process_yield();
    };
    if args[1] == "file" {
        // Routed read path: open/read/lseek/fstat/dup/close on a real regular
        // file in the service namespace. No kernel VFS fallback is possible.
        let fd = sunlight_libc::open(b"/tests/cat-hello").expect("routed open");
        let mut buf = [0u8; 32];
        let read = sunlight_libc::read(fd, &mut buf).expect("routed read");
        assert_eq!(&buf[..read], b"hello from cat\n");
        assert_eq!(
            sunlight_libc::lseek(fd, 0, sunlight_libc::SEEK_SET).unwrap(),
            0
        );
        let read = sunlight_libc::read(fd, &mut buf[..5]).unwrap();
        assert_eq!(&buf[..read], b"hello");
        let stat = sunlight_libc::fstat(fd).expect("routed fstat");
        assert_eq!(stat.size, 15);
        assert_eq!(stat.file_type, 1);
        // A duplicate shares the service description, including its offset.
        let duplicate = sunlight_libc::dup(fd).expect("routed dup");
        let read = sunlight_libc::read(duplicate, &mut buf[..2]).unwrap();
        assert_eq!(&buf[..read], b" f");
        sunlight_libc::close(fd).expect("close original");
        let read = sunlight_libc::read(duplicate, &mut buf[..3]).unwrap();
        assert_eq!(&buf[..read], b"rom");
        sunlight_libc::close(duplicate).expect("close duplicate");

        // Routed write path on the same regular file, then restore it.
        let write = sunlight_libc::open_with_flags(b"/tests/cat-hello", sunlight_libc::O_WRONLY)
            .expect("routed write open");
        assert_eq!(sunlight_libc::write(write, b"HELLO").unwrap(), 5);
        sunlight_libc::close(write).unwrap();
        let read = sunlight_libc::open(b"/tests/cat-hello").expect("routed reread open");
        let count = sunlight_libc::read(read, &mut buf).unwrap();
        assert_eq!(&buf[..count], b"HELLO from cat\n");
        sunlight_libc::close(read).unwrap();
        let restore = sunlight_libc::open_with_flags(b"/tests/cat-hello", sunlight_libc::O_WRONLY)
            .expect("routed restore open");
        assert_eq!(sunlight_libc::write(restore, b"hello").unwrap(), 5);
        sunlight_libc::close(restore).unwrap();

        // Unsupported routed opens fail closed instead of reaching kernel VFS.
        for (path, flags) in [
            (
                b"/tests/vfs-file-new".as_slice(),
                sunlight_libc::O_WRONLY | sunlight_libc::O_CREAT,
            ),
            (
                b"/tests/cat-hello".as_slice(),
                sunlight_libc::O_WRONLY | sunlight_libc::O_TRUNC,
            ),
            (b"/tests".as_slice(), 0x1_0000u64),
        ] {
            assert_eq!(
                sunlight_libc::open_with_flags(path, flags),
                Err(sunlight_libc::Errno::Failed)
            );
        }
        sunlight_ipc::debug_log(
            "[VFS-RPC] native copied file open/read/write/seek/stat/dup/close OK",
        );

        // Leave one non-CLOEXEC and one close-on-exec descriptor, then re-exec.
        let fd = sunlight_libc::open(b"/tests/cat-hello").unwrap();
        let mut first = [0u8; 5];
        assert_eq!(sunlight_libc::read(fd, &mut first).unwrap(), 5);
        assert_eq!(&first, b"hello");
        assert_eq!(sunlight_libc::dup2(fd, sunlight_libc::Fd(9)).unwrap().0, 9);
        assert_eq!(sunlight_libc::dup2(fd, sunlight_libc::Fd(8)).unwrap().0, 8);
        // fcntl(F_SETFD=2, FD_CLOEXEC=1) on the duplicate.
        assert_eq!(unsafe { sunlight_libc::sys::syscall3(49, 8, 2, 1) }, 0);
        sunlight_libc::close(fd).unwrap();
        sunlight_libc::exec(
            b"/bin/vfs-stat-caller",
            &[b"vfs-stat-caller", b"file-post-exec", args[2].as_bytes()],
        )
        .unwrap();
        loop {
            sunlight_ipc::process_yield();
        }
    }
    if args[1] == "replacement" {
        assert!(sunlight_libc::stat(b"/etc/motd").is_ok());
        sunlight_ipc::debug_log(
            "[VFS-RPC] native stat exec replacement rejects old continuation/fresh profile OK",
        );
        sunlight_libc::exit(0);
    }
    if args[1] == "narrow-root" {
        let narrow = |path: &[u8]| unsafe {
            sunlight_libc::sys::syscall1(
                sunlight_ipc::SunlightSyscall::VfsNarrowRoot as u64,
                path.as_ptr() as u64,
            )
        };
        sunlight_libc::chdir(b"/etc").unwrap();
        assert_ne!(narrow(b"missing-root\0"), 0);
        assert_ne!(narrow(b"motd\0"), 0);
        assert_eq!(api::stat("motd"), Ok(original));
        assert_eq!(narrow(b".\0"), 0);
        let mut cwd = [0; sunlight_libc::MAX_PATH];
        let len = sunlight_libc::getcwd(&mut cwd).unwrap();
        assert_eq!(&cwd[..len], b"/");
        for path in ["motd", "/motd", "../../motd"] {
            assert_eq!(api::stat(path), Ok(original));
            assert_eq!(
                sunlight_libc::stat(path.as_bytes()).unwrap().size,
                original.size
            );
        }
        assert!(api::stat("/etc/motd").is_err());
        assert!(sunlight_libc::stat(b"/etc/motd").is_err());
        let mut linux = [0u8; 144];
        assert_eq!(
            unsafe {
                sunlight_libc::sys::syscall4(
                    1026,
                    (-100i64) as u64,
                    b"/motd\0".as_ptr() as u64,
                    linux.as_mut_ptr() as u64,
                    0,
                )
            },
            0
        );
        sunlight_libc::chdir(b"../../..").unwrap();
        let len = sunlight_libc::getcwd(&mut cwd).unwrap();
        assert_eq!(&cwd[..len], b"/");
        let before = control(8);
        for _ in 0..64 {
            assert_eq!(narrow(b".\0"), 0);
        }
        assert_eq!(control(8), before);
        assert_eq!(api::stat("/motd"), Ok(original));
        sunlight_ipc::debug_log(
            "[VFS-RPC] native root narrowing rollback/confinement/cwd reset/heap cleanup OK",
        );
        let pid = sunlight_libc::spawn(
            b"/bin/vfs-stat-caller",
            &[b"vfs-stat-caller", b"derived-root", args[2].as_bytes()],
            None,
        )
        .unwrap();
        assert_eq!(sunlight_libc::waitpid(pid), Ok(0));
        assert_eq!(api::stat("/motd"), Ok(original));
        sunlight_libc::exit(0);
    }
    if args[1] == "initial-cwd" {
        assert_eq!(api::stat("motd"), Ok(original));
        assert_eq!(api::stat("../etc/motd"), Ok(original));
        let absolute = sunlight_libc::stat(b"/etc/motd").unwrap();
        let relative = sunlight_libc::stat(b"motd").unwrap();
        assert_eq!(absolute.size, relative.size);
        let mut linux_absolute = [0u8; 144];
        let mut linux_relative = [0u8; 144];
        for (path, output) in [
            (b"/etc/motd\0".as_slice(), &mut linux_absolute),
            (b"motd\0".as_slice(), &mut linux_relative),
        ] {
            assert_eq!(
                unsafe {
                    sunlight_libc::sys::syscall4(
                        1026,
                        (-100i64) as u64,
                        path.as_ptr() as u64,
                        output.as_mut_ptr() as u64,
                        0,
                    )
                },
                0
            );
        }
        assert_eq!(linux_absolute, linux_relative);
        sunlight_libc::chdir(b"/").unwrap();
        assert_eq!(api::stat("etc/motd"), Ok(original));
        sunlight_ipc::debug_log(
            "[VFS-RPC] initial nonroot cwd native/Linux stat and live chdir OK",
        );
        sunlight_libc::exit(0);
    }
    if matches!(args[1], "chdir" | "shared-chdir") {
        if args[1] == "chdir" {
            sunlight_libc::chdir(b"/etc").unwrap();
        } else {
            sibling_chdir(true);
        }
        let mut cwd = [0; sunlight_libc::MAX_PATH];
        let len = sunlight_libc::getcwd(&mut cwd).unwrap();
        assert_eq!(&cwd[..len], b"/etc");
        assert_eq!(api::stat("motd"), Ok(original));
        let native = sunlight_libc::stat(b"motd").unwrap();
        assert_eq!(native.size, original.size);
        assert!(sunlight_libc::chdir(b"missing-cwd").is_err());
        assert!(sunlight_libc::chdir(b"motd").is_err());
        assert_eq!(api::stat("motd"), Ok(original));
        sunlight_libc::chdir(b"..").unwrap();
        let len = sunlight_libc::getcwd(&mut cwd).unwrap();
        assert_eq!(&cwd[..len], b"/");
        assert_eq!(api::stat("etc/motd"), Ok(original));
        if args[1] == "chdir" {
            let before = control(8);
            for _ in 0..64 {
                sunlight_libc::chdir(b"/etc").unwrap();
                sunlight_libc::chdir(b"/").unwrap();
            }
            assert_eq!(control(8), before);
            assert_eq!(api::stat("etc/motd"), Ok(original));
            sunlight_ipc::debug_log(
                "[VFS-RPC] native repeated chdir releases service heap/pins OK",
            );
        }
        sunlight_ipc::debug_log(if args[1] == "chdir" {
            "[VFS-RPC] native copied chdir continuation/cwd commit/stat recovery OK"
        } else {
            "[VFS-RPC] native copied shared sibling chdir/cwd commit/stat recovery OK"
        });
        sunlight_libc::exit(0);
    }
    assert_eq!(control(4), 0);
    let mut output = [0xa5; 56];
    assert_eq!(
        unsafe { api::probe_stat_attempt("/etc/motd", output.as_mut_ptr() as u64) },
        api::PENDING
    );
    while control(5) == 0 {
        assert!(sunlight_ipc::monotonic_millis() - start < 5000);
    }
    assert!(output.iter().all(|b| *b == 0xa5));
    match args[1] {
        "exit" => sunlight_libc::exit(0),
        "failed-exec" | "failed-chdir" | "private-chdir" | "same-credentials" => {
            match args[1] {
                "failed-exec" => {
                    assert!(sunlight_libc::exec(b"/missing-vfs-stat-image", &[]).is_err());
                }
                "failed-chdir" => {
                    assert!(sunlight_libc::chdir(b"/missing-vfs-stat-directory").is_err());
                    assert!(sunlight_libc::chdir(b"/etc/motd").is_err());
                }
                "same-credentials" => unsafe {
                    assert_eq!(sunlight_libc::sys::syscall1(37, sunlight_libc::getuid()), 0);
                    assert_eq!(sunlight_libc::sys::syscall1(38, sunlight_libc::getgid()), 0);
                },
                _ => sibling_chdir(false),
            }
            assert_eq!(control(7), 0);
            assert_eq!(
                unsafe { api::probe_stat_attempt("/etc/motd", output.as_mut_ptr() as u64) },
                0
            );
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
                            output_bytes: 56
                        },
                        &output
                    ),
                Ok(Reply::Metadata(original))
            );
            assert_eq!(api::stat("/etc/motd"), Ok(original));
            sunlight_ipc::debug_log(match args[1] {
                "failed-exec" => "[VFS-RPC] native stat failed exec preserves continuation OK",
                "failed-chdir" => "[VFS-RPC] native stat failed chdir preserves profile/result OK",
                "same-credentials" => {
                    "[VFS-RPC] native stat unchanged credentials preserve profile/result OK"
                }
                _ => "[VFS-RPC] native stat private sibling cwd preserves profile/result OK",
            });
            sunlight_libc::exit(0);
        }
        "setuid" | "setgid" => {
            let uid_change = args[1] == "setuid";
            let syscall = if uid_change { 37 } else { 38 };
            assert_eq!(unsafe { sunlight_libc::sys::syscall1(syscall, 1234) }, 0);
            if uid_change {
                assert_eq!(sunlight_libc::getuid(), 1234);
                assert_eq!(unsafe { sunlight_libc::sys::syscall1(37, 0) }, u64::MAX);
                assert_eq!(unsafe { sunlight_libc::sys::syscall1(38, 1234) }, u64::MAX);
            } else {
                assert_eq!(sunlight_libc::getgid(), 1234);
                // Restoring the original credentials must not regrant a profile.
                assert_eq!(unsafe { sunlight_libc::sys::syscall1(38, 0) }, 0);
            }
            assert_eq!(control(6), 0);
            assert_eq!(control(5), 0);
            assert_eq!(
                unsafe { api::probe_stat_attempt("/etc/motd", output.as_mut_ptr() as u64) },
                api::NOT_READY
            );
            assert!(output.iter().all(|b| *b == 0xa5));
            assert_eq!(api::stat("/etc/motd"), Err(api::NOT_READY));
            verify_native_denied();
            let pid = sunlight_libc::spawn(
                b"/bin/vfs-stat-caller",
                &[b"vfs-stat-caller", b"denied-child", args[2].as_bytes()],
                None,
            )
            .unwrap();
            assert_eq!(sunlight_libc::waitpid(pid), Ok(0));
            sunlight_ipc::debug_log(if uid_change {
                "[VFS-RPC] native stat setuid withdraws profile/stale reply OK"
            } else {
                "[VFS-RPC] native stat setgid withdraws profile/no regrant OK"
            });
            sunlight_libc::exit(0);
        }
        "exec" => {
            sunlight_libc::exec(
                b"/bin/vfs-stat-caller",
                &[b"vfs-stat-caller", b"replacement", args[2].as_bytes()],
            )
            .unwrap();
            loop {
                sunlight_ipc::process_yield();
            }
        }
        _ => panic!("unknown fixture"),
    }
}

fn verify_native_denied() {
    let mut output = [0xa5; 24];
    let result = unsafe {
        sunlight_libc::sys::syscall2(
            61,
            b"/etc/motd\0".as_ptr() as u64,
            output.as_mut_ptr() as u64,
        )
    };
    assert_eq!(result, sunlight_libc::sys::EAGAIN_RAW);
    assert_eq!(output, [0xa5; 24]);
    let mut linux_output = [0xa5; 144];
    assert_eq!(
        unsafe {
            sunlight_libc::sys::syscall4(
                1026,
                (-100i64) as u64,
                b"/etc/motd\0".as_ptr() as u64,
                linux_output.as_mut_ptr() as u64,
                0,
            )
        },
        (-11i64) as u64
    );
    assert_eq!(linux_output, [0xa5; 144]);
    sunlight_ipc::debug_log("[VFS-RPC] Linux stat withdrawn profile/no kernel fallback OK");
    sunlight_ipc::debug_log("[VFS-RPC] native StatPath withdrawn profile/no kernel fallback OK");
}
