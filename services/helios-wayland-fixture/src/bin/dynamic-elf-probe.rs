#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use sunlight_ipc::{debug_log, launch_trace::LaunchSource, nameserver_lookup, ProcessExit};
use sunlight_libc::sun_exec::{self, LaunchRequest};

#[cfg(feature = "dynamic-elf-orphan-fixture")]
#[path = "dynamic_elf/orphan.rs"]
mod orphan;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[DYNAMIC-ELF] launcher panic\n");
    ProcessExit::exit(1)
}

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8) -> ! {
    #[cfg(feature = "dynamic-elf-orphan-fixture")]
    if _argc == 2 {
        orphan::parent();
    }
    let start = sunlight_ipc::monotonic_millis();
    while nameserver_lookup("vfs").is_none() || nameserver_lookup("display_server").is_none() {
        if sunlight_ipc::monotonic_millis().wrapping_sub(start) >= 30_000 {
            debug_log("[DYNAMIC-ELF] launcher services unavailable\n");
            ProcessExit::exit(1);
        }
        let delay = sunlight_libc::time::Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        unsafe { sunlight_libc::time::nanosleep(&delay, core::ptr::null_mut()) };
    }
    #[cfg(feature = "dynamic-elf-relocation-fixture")]
    let mut completed = [0; 4 + 3 * cfg!(feature = "dynamic-elf-shared-fixture") as usize];
    for (_index, bundle) in [
        "/Applications/elf-pie.sunapp",
        "/Applications/elf-exec.sunapp",
        "/Applications/elf-static.sunapp",
        #[cfg(feature = "dynamic-elf-relocation-fixture")]
        "/Applications/elf-relocation.sunapp",
        #[cfg(feature = "dynamic-elf-shared-fixture")]
        "/Applications/elf-real.sunapp",
        #[cfg(feature = "dynamic-elf-shared-fixture")]
        "/Applications/elf-graph.sunapp",
        #[cfg(feature = "dynamic-elf-shared-fixture")]
        "/Applications/elf-shared.sunapp",
    ].into_iter().enumerate() {
        debug_log(&format!("[DYNAMIC-ELF] launch {bundle}\n"));
        match sun_exec::launch(LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            bundle.as_bytes(),
        )) {
            Ok(child) => {
                let status = sunlight_libc::waitpid(child.pid);
                debug_log(&format!("[DYNAMIC-ELF] pid={} waitpid={status:?}\n", child.pid));
                if status != Ok(0) {
                    ProcessExit::exit(1);
                }
                #[cfg(feature = "dynamic-elf-relocation-fixture")]
                for &old in &completed[.._index] {
                    if old == 0 {
                        continue;
                    }
                    let stale = sunlight_libc::waitpid(old);
                    debug_log(&format!(
                        "[WAIT-REGRESSION] old={old} new={} stale={stale:?}\n",
                        child.pid
                    ));
                    if stale != Err(sunlight_libc::Errno::Failed) {
                        ProcessExit::exit(1);
                    }
                }
                #[cfg(feature = "dynamic-elf-relocation-fixture")]
                {
                    completed[_index] = child.pid;
                }
            }
            Err(error) => debug_log(&format!("[DYNAMIC-ELF] launch failed: {error:?}\n")),
        }
    }
    #[cfg(feature = "dynamic-elf-tls-fixture")]
    {
        let child = sun_exec::launch(LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            b"/Applications/elf-tls-init.sunapp",
        ))
        .expect("initialized TLS control spawn");
        let status = sunlight_libc::waitpid(child.pid);
        debug_log(&format!("[HELIOS-TLS-INIT] control pid={} waitpid={status:?}\n", child.pid));
        assert_eq!(status, Ok(0));
    }
    #[cfg(feature = "dynamic-elf-tls-fixture")]
    for _ in 0..3 {
        let child = sun_exec::launch(LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            b"/Applications/elf-tls.sunapp",
        ))
        .expect("sequential TLS spawn");
        let status = sunlight_libc::waitpid(child.pid);
        debug_log(&format!("[HELIOS-TLS] sequential pid={} waitpid={status:?}\n", child.pid));
        assert_eq!(status, Ok(0));
    }
    #[cfg(feature = "dynamic-elf-orphan-fixture")]
    #[cfg(feature = "dynamic-elf-tls-fixture")]
    for _ in 0..3 {
        let child = sun_exec::launch(LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            b"/Applications/elf-tls-shared.sunapp",
        )).expect("sequential shared TLS spawn");
        let status = sunlight_libc::waitpid(child.pid);
        debug_log(&format!("[HELIOS-TLS-SHARED] sequential pid={} waitpid={status:?}\n", child.pid));
        assert_eq!(status, Ok(0));
    }
    #[cfg(feature = "dynamic-elf-orphan-fixture")]
    orphan::supervise();
    #[cfg(feature = "dynamic-elf-tls-fixture")]
    for case in [b'm', b'o', b'x', b'p'] {
        let request = LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            b"/Applications/elf-tls-shared.sunapp",
        );
        let mut environment = *b"HELIOS_HANDOFF=fixture\0HELIOS_TLS_NEG=?\0";
        let value = environment.len() - 2;
        environment[value] = case;
        let child = sun_exec::launch_with_environment(request, Some(&environment))
            .expect("resolver negative spawn");
        let status = sunlight_libc::waitpid(child.pid);
        let collected = sunlight_libc::waitpid(child.pid);
        debug_log(&format!(
            "[HELIOS-TLS-NEG] case={} pid={} wait={status:?} again={collected:?}\n",
            case as char, child.pid
        ));
        assert_eq!(status, Ok(1));
        assert_eq!(collected, Err(sunlight_libc::Errno::Failed));
    }
    #[cfg(feature = "runtime-mmap")]
    for marker in b'1'..=b'7' {
        let child = sun_exec::launch_with_environment(
            LaunchRequest::new(
                sun_exec::next_cli_trace(LaunchSource::Boot),
                LaunchSource::Boot,
                b"/Applications/elf-mmap.sunapp",
            ),
            Some(&[b'M', b'=', marker, 0]),
        )
        .expect("anonymous mmap probe spawn");
        let status = sunlight_libc::waitpid(child.pid);
        debug_log(&format!(
            "[ANON-MMAP] marker={} pid={} waitpid={status:?}\n",
            marker as char, child.pid
        ));
        assert_eq!(status, Ok(0));
    }
    #[cfg(feature = "runtime-mmap")]
    for (marker, expected) in [(b'P', 0), (b'S', 42), (b'Z', 0)] {
        let child = sun_exec::launch_with_environment(
            LaunchRequest::new(
                sun_exec::next_cli_trace(LaunchSource::Boot),
                LaunchSource::Boot,
                b"/Applications/elf-mmap.sunapp",
            ),
            Some(&[b'M', b'=', marker, 0]),
        )
        .expect("anonymous mmap fault probe spawn");
        let status = sunlight_libc::waitpid(child.pid);
        assert_eq!(status, Ok(expected));
        debug_log(&format!(
            "[ANON-MMAP-FAULT-WAIT] marker={} pid={} waitpid={status:?}\n",
            marker as char, child.pid
        ));
    }
    #[cfg(feature = "runtime-page-allocator")]
    for marker in *b"AFTLQEabc" {
        let child = launch_page_allocator(marker);
        let status = sunlight_libc::waitpid(child);
        assert_eq!(status, Ok(0));
        let collected = sunlight_libc::waitpid(child);
        assert_eq!(collected, Err(sunlight_libc::Errno::Failed));
        debug_log(&format!(
            "[PAGE-ALLOC-WAIT] marker={} pid={child} waitpid={status:?} again={collected:?}\n",
            marker as char
        ));
    }
    #[cfg(feature = "runtime-malloc")]
    for (marker, expected) in b"AFTLQEabcB".iter().copied().map(|marker| (marker, 0))
        .chain([(b'J', 43)])
    {
        let child = launch_malloc(marker);
        let status = sunlight_libc::waitpid(child);
        let collected = sunlight_libc::waitpid(child);
        assert_eq!(status, Ok(expected));
        assert_eq!(collected, Err(sunlight_libc::Errno::Failed));
        debug_log(&format!(
            "[MALL-ALLOC-WAIT] marker={} pid={child} waitpid={status:?} again={collected:?}\n",
            marker as char
        ));
    }
    #[cfg(feature = "runtime-malloc")]
    {
        let child = sun_exec::launch_with_environment(
            LaunchRequest::new(
                sun_exec::next_cli_trace(LaunchSource::Boot),
                LaunchSource::Boot,
                b"/Applications/elf-pie.sunapp",
            ),
            Some(b"HELIOS_HANDOFF=fixture\0HELIOS_MALLOC_SETUP=reject\0"),
        )
        .expect("malloc interpreter fixture spawn")
        .pid;
        let status = sunlight_libc::waitpid(child);
        let collected = sunlight_libc::waitpid(child);
        assert_eq!(status, Ok(44));
        assert_eq!(collected, Err(sunlight_libc::Errno::Failed));
        debug_log(&format!(
            "[MALL-ALLOC-WAIT] marker=I pid={child} waitpid={status:?} again={collected:?}\n"
        ));
    }
    #[cfg(feature = "runtime-malloc")]
    check_foreign_malloc_token();
    #[cfg(feature = "runtime-c-startup")]
    for (mode, expected) in [(b'P', 0), (b'N', 42), (b'A', 43), (b'W', 44), (b'L', 0)] {
        let child = launch_c_startup(mode);
        let status = sunlight_libc::waitpid(child);
        let collected = sunlight_libc::waitpid(child);
        assert_eq!(status, Ok(expected));
        assert_eq!(collected, Err(sunlight_libc::Errno::Failed));
        debug_log(&format!(
            "[C-STARTUP-WAIT] mode={} pid={child} waitpid={status:?} again={collected:?}\n",
            mode as char
        ));
    }
    #[cfg(feature = "runtime-c-dso")]
    for attempt in 1..=2 {
        const PROBE: &[u8] = b"/Applications/elf-c-dso.sunapp/Program/probe";
        let child = sunlight_libc::spawn_with_stdio_env(
            PROBE, &[PROBE, b"P"], None, None, None, b"",
        ).expect("utility DSO fixture spawn");
        let status = sunlight_libc::waitpid(child);
        let collected = sunlight_libc::waitpid(child);
        debug_log(&format!(
            "[C-DSO-WAIT] attempt={attempt} pid={child} waitpid={status:?} again={collected:?}\n"
        ));
        assert_eq!(status, Ok(0));
        assert_eq!(collected, Err(sunlight_libc::Errno::Failed));
    }
    debug_log("[DYNAMIC-ELF] done\n");
    ProcessExit::exit(0)
}

#[cfg(feature = "runtime-page-allocator")]
fn launch_page_allocator(marker: u8) -> u64 {
    sun_exec::launch_with_environment(
        LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            b"/Applications/elf-mmap.sunapp",
        ),
        Some(&[b'M', b'=', marker, 0]),
    )
    .expect("page allocator fixture spawn")
    .pid
}


#[cfg(feature = "runtime-malloc")]
fn launch_malloc(marker: u8) -> u64 {
    sun_exec::launch_with_environment(
        LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            b"/Applications/elf-mmap.sunapp",
        ),
        Some(&[b'M', b'=', marker, 0, b'H', b'=', b'1', 0]),
    )
    .expect("malloc fixture spawn")
    .pid
}


#[cfg(feature = "runtime-malloc")]
fn check_foreign_malloc_token() {
    use core::fmt::Write;
    use sunlight_libc as libc;

    const PROBE: &[u8] = b"/Applications/elf-mmap.sunapp/Program/mmap-probe";
    let (reader, writer) = libc::pipe().expect("malloc token pipe");
    let owner = libc::spawn_with_stdio_env(PROBE, &[PROBE], None, Some(writer), None, b"M=X\0")
        .expect("malloc token producer");
    libc::close(writer).expect("close launcher token writer");
    assert_eq!(libc::waitpid(owner), Ok(0));
    let mut token = [0; 48];
    let mut received = 0;
    while received < token.len() {
        let count = libc::read(reader, &mut token[received..]).expect("read malloc token");
        assert_ne!(count, 0);
        received += count;
    }
    libc::close(reader).expect("close launcher token reader");
    let mut words = token.chunks_exact(8).map(|word| {
        u64::from_le_bytes(word.try_into().expect("eight-byte token word"))
    });
    assert_eq!(words.next(), Some(owner));
    let mut environment = format!("M=Y\0C={owner:016x}");
    for word in words {
        write!(environment, "{word:016x}").expect("encode malloc token");
    }
    environment.push('\0');
    let child = sun_exec::launch_with_environment(
        LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            b"/Applications/elf-mmap.sunapp",
        ),
        Some(environment.as_bytes()),
    )
    .expect("malloc token consumer")
    .pid;
    let status = libc::waitpid(child);
    let collected = libc::waitpid(child);
    assert_eq!(status, Ok(0));
    assert_eq!(collected, Err(libc::Errno::Failed));
    debug_log(&format!(
        "[MALL-ALLOC-WAIT] marker=Y pid={child} waitpid={status:?} again={collected:?} owner={owner}\n"
    ));
}

#[cfg(feature = "runtime-c-startup")]
fn launch_c_startup(mode: u8) -> u64 {
    sunlight_libc::spawn_with_stdio_env(
        b"/bin/helios-c-startup",
        &[b"c-startup", &[mode]],
        None,
        None,
        None,
        b"",
    )
    .expect("C startup fixture spawn")
}
