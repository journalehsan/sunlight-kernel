use alloc::format;
use sunlight_ipc::{current_process_generation, debug_log, getpid, process_is_alive, ProcessExit};
use sunlight_libc::{self as libc, Errno};

use super::{sun_exec, LaunchRequest, LaunchSource};

const CONTROLS: &[&str] = &[
    "static", "pie", "relocation", "real", "graph",
    #[cfg(feature = "dynamic-elf-tls-fixture")]
    "tls",
    #[cfg(feature = "dynamic-elf-tls-fixture")]
    "tls-shared",
    #[cfg(feature = "runtime-mmap")]
    "mmap",
    #[cfg(feature = "runtime-page-allocator")]
    "page-allocator",
    #[cfg(feature = "runtime-malloc")]
    "malloc",
    #[cfg(feature = "runtime-c-startup")]
    "c-startup",
];
const PARENT: &[u8] = b"/bin/helios-orphan-parent";

fn launch(variant: &str, _replacement: bool) -> u64 {
    #[cfg(feature = "runtime-c-startup")]
    if variant == "c-startup" {
        return super::launch_c_startup(if _replacement { b'P' } else { b'L' });
    }
    #[cfg(feature = "runtime-page-allocator")]
    if variant == "page-allocator" {
        return super::launch_page_allocator(b'd');
    }
    #[cfg(feature = "runtime-malloc")]
    if variant == "malloc" {
        return super::launch_malloc(b'd');
    }
    let bundle = format!("/Applications/elf-{variant}.sunapp");
    sun_exec::launch(LaunchRequest::new(
        sun_exec::next_cli_trace(LaunchSource::Boot),
        LaunchSource::Boot,
        bundle.as_bytes(),
    ))
    .expect("lifecycle control spawn")
    .pid
}

pub(super) fn parent() -> ! {
    let pid = getpid();
    let generation = current_process_generation();
    for &variant in CONTROLS {
        let child = launch(variant, false);
        let start = sunlight_ipc::monotonic_millis();
        while process_is_alive(child) {
            assert!(sunlight_ipc::monotonic_millis().wrapping_sub(start) < 30_000);
            let delay = libc::time::Timespec {
                tv_sec: 0,
                tv_nsec: 10_000_000,
            };
            unsafe { libc::time::nanosleep(&delay, core::ptr::null_mut()) };
        }
        debug_log(&format!(
            "[ORPHAN] uncollected variant={variant} parent={pid}:{generation} child={child}\n"
        ));
        libc::write_all(libc::STDOUT, &child.to_le_bytes()).expect("send child PID");
    }
    debug_log(&format!(
        "[ORPHAN] parent-exiting parent={pid}:{generation}\n"
    ));
    ProcessExit::exit(0)
}

pub(super) fn supervise() {
    let pid = getpid();
    let generation = current_process_generation();
    debug_log(&format!("[ORPHAN] begin observer={pid}:{generation}\n"));
    let (reader, writer) = libc::pipe().expect("lifecycle pipe");
    let parent = libc::spawn(PARENT, &[PARENT, b"parent"], Some(writer)).expect("spawn parent");
    libc::close(writer).expect("close supervisor writer");
    let status = libc::waitpid(parent);
    debug_log(&format!(
        "[ORPHAN] parent-wait target={parent} result={status:?}\n"
    ));
    assert_eq!(status, Ok(0));
    let mut children = [0; CONTROLS.len()];
    for child in &mut children {
        let mut value = [0; 8];
        let mut received = 0;
        while received < value.len() {
            let count = libc::read(reader, &mut value[received..]).expect("read child PID");
            assert_ne!(count, 0);
            received += count;
        }
        *child = u64::from_le_bytes(value);
    }
    libc::close(reader).expect("close supervisor reader");
    for child in children {
        check_unrelated("after-parent", child);
    }

    let mut replacements = [0; 3 * CONTROLS.len()];
    for (index, child) in replacements.iter_mut().enumerate() {
        let variant = CONTROLS[index % CONTROLS.len()];
        #[cfg(feature = "runtime-page-allocator")]
        if variant == "page-allocator" {
            *child = super::launch_page_allocator(b'e' + (index / CONTROLS.len()) as u8);
        } else {
            #[cfg(feature = "runtime-malloc")]
            if variant == "malloc" {
                *child = super::launch_malloc(b'e' + (index / CONTROLS.len()) as u8);
            } else {
                *child = launch(variant, true);
            }
            #[cfg(not(feature = "runtime-malloc"))]
            { *child = launch(variant, true); }
        }
        #[cfg(not(feature = "runtime-page-allocator"))]
        {
            *child = launch(variant, true);
        }
        debug_log(&format!(
            "[ORPHAN] replacement variant={variant} child={child}\n"
        ));
    }
    for child in children {
        check_unrelated("after-reuse-batch", child);
    }
    for child in replacements {
        let status = libc::waitpid(child);
        debug_log(&format!(
            "[ORPHAN] current target={child} result={status:?}\n"
        ));
        assert_eq!(status, Ok(0));
    }
    debug_log("[ORPHAN] done\n");
}

fn check_unrelated(phase: &str, child: u64) {
    let status = libc::waitpid(child);
    debug_log(&format!(
        "[ORPHAN] {phase} target={child} result={status:?}\n"
    ));
    assert_eq!(status, Err(Errno::Failed));
}
