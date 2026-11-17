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
    if args[1] == "replacement" {
        // Successful exec must not inherit the old image's pilot permission
        // or expose its held completion to the replacement address space.
        // Reject the old reply while this replacement is alive, before exit
        // can provide a second retirement opportunity.
        assert_eq!(control(6), 0);
        assert_eq!(api::stat("/etc/motd"), Err(api::NOT_READY));
        verify_native_denied();
        sunlight_ipc::debug_log(
            "[VFS-RPC] native stat exec replacement denied old continuation OK",
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
        // The existing fail-closed cwd lifecycle still covers this profile.
        sunlight_libc::chdir(b"/").unwrap();
        assert_eq!(api::stat("/etc/motd"), Err(api::NOT_READY));
        verify_native_denied();
        sunlight_ipc::debug_log(
            "[VFS-RPC] initial nonroot cwd native/Linux stat and withdrawal OK",
        );
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
            sunlight_ipc::debug_log(if uid_change {
                "[VFS-RPC] native stat setuid withdraws profile/stale reply OK"
            } else {
                "[VFS-RPC] native stat setgid withdraws profile/no regrant OK"
            });
            sunlight_libc::exit(0);
        }
        "chdir" | "shared-chdir" => {
            if args[1] == "chdir" {
                sunlight_libc::chdir(b"/etc").unwrap();
            } else {
                sibling_chdir(true);
            }
            let mut cwd = [0; sunlight_libc::MAX_PATH];
            let len = sunlight_libc::getcwd(&mut cwd).unwrap();
            assert_eq!(&cwd[..len], b"/etc");
            // Reject while the original caller is alive. Neither a late reply
            // nor a new authenticated service snapshot may restore authority.
            assert_eq!(control(6), 0);
            assert_eq!(control(5), 0);
            assert_eq!(
                unsafe { api::probe_stat_attempt("/etc/motd", output.as_mut_ptr() as u64) },
                api::NOT_READY
            );
            assert!(output.iter().all(|b| *b == 0xa5));
            assert_eq!(api::stat("motd"), Err(api::NOT_READY));
            sunlight_libc::chdir(b"/").unwrap();
            assert_eq!(control(5), 0);
            assert_eq!(api::stat("/etc/motd"), Err(api::NOT_READY));
            verify_native_denied();
            sunlight_ipc::debug_log(if args[1] == "chdir" {
                "[VFS-RPC] native stat chdir withdraws profile/stale reply OK"
            } else {
                "[VFS-RPC] native stat shared sibling cwd withdraws profile OK"
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
