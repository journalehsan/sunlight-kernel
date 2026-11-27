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
    #[cfg(feature = "vfs-d1b-test")]
    match args[1] {
        #[cfg(feature = "vfs-d2a-test")]
        "private-mmap" => run_private_mmap(),
        "descriptor-lifecycle" => run_native_descriptors(args[2] == "exec"),
        "descriptor-exec" => {
            assert_eq!(unsafe { sunlight_libc::sys::syscall2(0xf1c1, 23, 2) }, 1);
            let mut bytes = [0; 10];
            assert_eq!(sunlight_libc::read(sunlight_libc::Fd(8), &mut bytes), Err(sunlight_libc::Errno::BadFd));
            assert_eq!(sunlight_libc::read(sunlight_libc::Fd(9), &mut bytes[..5]), Ok(5));
            assert_eq!(&bytes[..5], b" from");
            assert_eq!(sunlight_libc::read(sunlight_libc::Fd(10), &mut bytes[..5]), Ok(5));
            assert_eq!(&bytes[..5], b" cat\n");
            sunlight_ipc::debug_log("[VFS-D1B] exec inherited shared cursor and closed CLOEXEC");
            sunlight_libc::exit(0);
        }
        _ => {}
    }
    #[cfg(feature = "vfs-l1c-file-ipc-test")]
    if args[1] == "file-ipc" {
        let start = sunlight_ipc::monotonic_millis();
        loop {
            match unsafe { sunlight_libc::sys::syscall2(0xf1c3, 0, 1) } {
                1 => break,
                0 => assert!(sunlight_ipc::monotonic_millis() - start < 5000),
                _ => panic!("file/IPC admission rejected"),
            }
            sunlight_ipc::process_yield();
        }
        let first = sunlight_libc::open(b"/etc/motd").unwrap();
        let second = sunlight_libc::open(b"/tests/cat-hello").unwrap();
        let duplicate = sunlight_libc::dup(second).unwrap();
        let mut bytes = [0; 5];
        assert!(sunlight_libc::read(first, &mut bytes).unwrap() > 0);
        bytes.fill(0);
        assert_eq!(sunlight_libc::read(second, &mut bytes[..2]), Ok(2));
        assert_eq!(sunlight_libc::read(duplicate, &mut bytes[..3]), Ok(3));
        assert_eq!(&bytes, b"llo\0\0");
        for _ in 0..2 {
            let endpoint = sunlight_ipc::endpoint_create();
            assert_ne!(endpoint.0, 0);
            assert!(sunlight_ipc::ipc_try_recv(endpoint).is_none());
        }
        for size in [4096, 8192] {
            let (memory, _) = sunlight_ipc::shm_create(size, 0).unwrap();
            unsafe {
                memory.write_volatile(42);
                memory.add(size - 1).write_volatile(17);
            }
        }
        assert_eq!(unsafe { sunlight_libc::sys::syscall2(0xf1c3, 0, 0) }, 1);
        sunlight_libc::exit(37);
    }
    #[cfg(feature = "vfs-l1c-parent-lifecycle-test")]
    if args[1] == "parent-lifecycle" {
        let mode = args[2].parse::<u64>().unwrap();
        let start = sunlight_ipc::monotonic_millis();
        loop {
            match unsafe { sunlight_libc::sys::syscall2(0xf1c1, mode, 0) } {
                0 => assert!(sunlight_ipc::monotonic_millis() - start < 5000),
                1 if mode == 15 => {
                    assert!(sunlight_libc::exec(b"/etc/motd", &[b"motd"]).is_err());
                    let result = unsafe { sunlight_libc::sys::syscall2(0xf1c1, 17, 0) };
                    assert!((2..u64::MAX).contains(&result));
                    assert_eq!(sunlight_libc::waitpid(result - 2), Ok(0));
                    sunlight_libc::exit(37);
                }
                1 if mode == 16 => {
                    let _ = sunlight_libc::exec(
                        b"/bin/vfs-lifecycle-parent",
                        &[b"vfs-lifecycle-parent", b"exec-replacement", b"16"],
                    );
                    panic!("exec returned to retired image");
                }
                1 => sunlight_libc::exit(37),
                result if mode == 14 && result != u64::MAX => {
                    assert_eq!(sunlight_libc::waitpid(result - 2), Ok(0));
                    sunlight_libc::exit(37);
                }
                result => panic!("parent lifecycle stage returned {}", result),
            }
            sunlight_libc::yield_now();
        }
    }
    #[cfg(feature = "vfs-l1c-parent-lifecycle-test")]
    if args[1] == "exec-replacement" {
        let start = sunlight_ipc::monotonic_millis();
        while unsafe { sunlight_libc::sys::syscall2(0xf1c1, 18, 0) } == 0 {
            assert!(sunlight_ipc::monotonic_millis() - start < 5000);
            sunlight_libc::yield_now();
        }
        sunlight_ipc::debug_log("[VFS-EXEC] replacement image executed");
        sunlight_libc::exit(37);
    }
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

#[cfg(feature = "vfs-d1b-test")]
fn run_native_descriptors(exec: bool) -> ! {
    use sunlight_libc::{
        close, dup, dup2, lseek, open, open_with_flags, read, write, Errno, Fd, O_CLOEXEC,
        O_RDWR, SEEK_CUR, SEEK_SET,
    };

    const PATH: &[u8] = b"/tests/cat-hello";
    let checkpoint = |mode, arg| unsafe { sunlight_libc::sys::syscall2(0xf1c1, mode, arg) };
    assert_eq!(checkpoint(20, 0), 1);

    let fd = open_with_flags(PATH, O_RDWR).expect("service OPEN and FD installation");
    let mut bytes = [0; 5];
    assert_eq!(read(fd, &mut bytes), Ok(5));
    assert_eq!(&bytes, b"hello");
    let duplicate = dup(fd).unwrap();
    assert_eq!(lseek(duplicate, 0, SEEK_CUR), Ok(5));
    let pread = |fd: Fd, bytes: &mut [u8], offset| unsafe {
        sunlight_libc::sys::syscall4(
            0xf1c5, fd.0 as u64, bytes.as_mut_ptr() as u64, bytes.len() as u64, offset,
        )
    };
    assert_eq!(pread(fd, &mut bytes, 6), 5);
    assert_eq!(&bytes, b"from ");
    assert_eq!(pread(duplicate, &mut bytes, 10), 5);
    assert_eq!(&bytes, b" cat\n");
    assert_eq!(lseek(fd, 0, SEEK_CUR), Ok(5));
    assert_eq!(lseek(fd, 0, SEEK_SET), Ok(0));
    assert_eq!(write(duplicate, b"HELLO"), Ok(5));
    assert_eq!(pread(fd, &mut bytes, 0), 5);
    assert_eq!(&bytes, b"HELLO");
    assert_eq!(lseek(duplicate, 0, SEEK_SET), Ok(0));
    assert_eq!(write(fd, b"hello"), Ok(5));
    close(fd).unwrap();
    assert_eq!(read(duplicate, &mut bytes), Ok(5));
    assert_eq!(&bytes, b" from");
    close(duplicate).unwrap();
    sunlight_ipc::debug_log("[VFS-D1B] native open/read/write/seek/pread/dup/final close");

    let fd = open(PATH).unwrap();
    for slot in 0..256 {
        if slot != fd.0 {
            assert_eq!(dup2(fd, Fd(slot)), Ok(Fd(slot)));
        }
    }
    assert_eq!(checkpoint(23, 256), 1);
    assert_eq!(open(PATH), Err(Errno::Failed));
    assert_eq!(checkpoint(23, 256), 1);
    assert_eq!(read(Fd(255), &mut bytes), Ok(5));
    assert_eq!(&bytes, b"hello");
    sunlight_ipc::debug_log("[VFS-D1B] full FD table preserved");

    if exec {
        for slot in 0..256 {
            if !matches!(slot, 9 | 10) {
                close(Fd(slot)).unwrap();
            }
        }
        let cloexec = open_with_flags(PATH, O_CLOEXEC).unwrap();
        assert_ne!(cloexec, Fd(9));
        assert_ne!(cloexec, Fd(10));
        assert_eq!(dup2(cloexec, Fd(8)), Ok(Fd(8)));
        assert_eq!(unsafe { sunlight_libc::sys::syscall3(49, 8, 2, 1) }, 0);
        close(cloexec).unwrap();
        assert_eq!(checkpoint(23, 3), 1);
        sunlight_libc::exec(
            b"/bin/vfs-d1b-caller",
            &[b"vfs-d1b-caller", b"descriptor-exec", b"0"],
        ).unwrap();
        panic!("exec returned to original image");
    }
    sunlight_libc::exit(0);
}

#[cfg(feature = "vfs-d2a-test")]
fn run_private_mmap() -> ! {
    use sunlight_libc::{close, lseek, open, open_with_flags, read, write, O_RDWR, O_WRONLY, SEEK_CUR, SEEK_SET};
    use sunlight_libc::mman::munmap;
    use sunlight_libc::sys::{syscall2, syscall6};

    const PATH: &[u8] = b"/tests/cat-hello";
    const PAGE: usize = 4096;
    const NX: u64 = 1 << 63;
    const PRESENT: u64 = 1;
    const WRITE: u64 = 2;
    const USER: u64 = 4;
    const PRIVATE: u64 = 2;
    const FIXED: u64 = 0x10;
    const NOREPLACE: u64 = 0x100000;
    const TARGET: u64 = 0x2080_0000_0000;
    let checkpoint = |mode, arg| unsafe { syscall2(0xf1c1, mode, arg) };
    let map = |addr, len, prot, flags, fd: sunlight_libc::Fd, offset| unsafe {
        syscall6(50, addr, len as u64, prot, flags, fd.0 as u64, offset as u64)
    };
    assert_eq!(checkpoint(20, 0), 1);
    let fd = open_with_flags(PATH, O_RDWR).unwrap();
    assert_eq!(lseek(fd, PAGE as i64, SEEK_SET), Ok(PAGE as u64));
    assert_eq!(write(fd, b"tail-data"), Ok(9));
    assert_eq!(lseek(fd, 5, SEEK_SET), Ok(5));
    let baseline = checkpoint(26, 0);
    assert_eq!(map(0, PAGE, 7, PRIVATE, fd, 0), u64::MAX);
    assert_eq!(map(0, PAGE, 1, PRIVATE, fd, 1), u64::MAX);
    let write_only = open_with_flags(PATH, O_WRONLY).unwrap();
    assert_eq!(map(0, PAGE, 1, PRIVATE, write_only, 0), u64::MAX);
    close(write_only).unwrap();

    for fault in 1..=6 {
        assert_eq!(checkpoint(24, fault), 1);
        let placement = if fault == 5 { FIXED } else { NOREPLACE };
        assert_eq!(map(TARGET, PAGE + 9, 3, PRIVATE | placement, fd, 0), u64::MAX);
        assert_eq!(checkpoint(25, TARGET), 0);
        assert_eq!(checkpoint(25, TARGET + PAGE as u64), 0);
        assert_eq!(checkpoint(26, fault), baseline);
    }
    for cycle in 0..4 {
        let address = map(0, PAGE + 9, 3, PRIVATE, fd, 0);
        assert_ne!(address, u64::MAX);
        assert_eq!(checkpoint(25, address), PRESENT | USER | WRITE | NX);
        assert_eq!(checkpoint(25, address + PAGE as u64), PRESENT | USER | WRITE | NX);
        let bytes = unsafe { core::slice::from_raw_parts_mut(address as *mut u8, 2 * PAGE) };
        assert_eq!(&bytes[..15], b"hello from cat\n");
        assert!(bytes[15..PAGE].iter().all(|byte| *byte == 0));
        assert_eq!(&bytes[PAGE..PAGE + 9], b"tail-data");
        assert!(bytes[PAGE + 9..].iter().all(|byte| *byte == 0));
        bytes[0] = b'X';
        assert_eq!(bytes[0], b'X');
        munmap(address as *mut u8, PAGE + 9).unwrap();
        assert_eq!(checkpoint(25, address), 0);
        assert_eq!(checkpoint(26, 10 + cycle), baseline);
    }
    assert_eq!(lseek(fd, 0, SEEK_CUR), Ok(5));
    let mut bytes = [0; 5];
    assert_eq!(read(fd, &mut bytes), Ok(5));
    assert_eq!(&bytes, b" from");
    assert_eq!(lseek(fd, 0, SEEK_SET), Ok(0));
    assert_eq!(read(fd, &mut bytes), Ok(5));
    assert_eq!(&bytes, b"hello");

    let address = map(TARGET, PAGE, 1, PRIVATE | NOREPLACE, fd, PAGE);
    assert_eq!(address, TARGET);
    assert_eq!(checkpoint(25, address), PRESENT | USER | NX);
    assert_eq!(map(TARGET, PAGE, 1, PRIVATE | NOREPLACE, fd, 0), u64::MAX);
    assert_eq!(map(TARGET, PAGE, 5, PRIVATE | FIXED, fd, PAGE), TARGET);
    assert_eq!(checkpoint(25, address), PRESENT | USER);
    let guard = map(0, PAGE, 0, PRIVATE, fd, 2 * PAGE);
    assert_ne!(guard, u64::MAX);
    assert_eq!(checkpoint(25, guard), PRESENT | NX);
    munmap(guard as *mut u8, PAGE).unwrap();
    close(fd).unwrap();
    assert_eq!(unsafe { core::slice::from_raw_parts(address as *const u8, 9) }, b"tail-data");
    let check = open(PATH).unwrap();
    assert_eq!(read(check, &mut bytes), Ok(5));
    assert_eq!(&bytes, b"hello");
    close(check).unwrap();
    assert_eq!(checkpoint(26, 20), baseline + 1);
    sunlight_ipc::debug_log("[VFS-D2A] Ring-3 private mmap checks passed; exiting with a live page");
    sunlight_libc::exit(0);
}
