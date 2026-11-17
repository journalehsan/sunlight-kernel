//! Race the production page walk/copy against shared-VM mapping replacement.
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use sunlight_ipc::vfs_rpc::{response::Metadata, stat_transport as api};
use sunlight_libc::mman::*;

static RUN: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static QUIET: AtomicBool = AtomicBool::new(false);
static REPLACEMENTS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn replace_page(address: *mut u8) -> *mut u8 {
    while !STOP.load(Ordering::Acquire) {
        if !RUN.load(Ordering::Acquire) {
            QUIET.store(true, Ordering::Release);
            sunlight_ipc::process_yield();
            continue;
        }
        QUIET.store(false, Ordering::Release);
        munmap(address, 4096).expect("race unmap");
        sunlight_ipc::process_yield();
        assert_eq!(
            mmap(
                address,
                4096,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
                -1,
                0
            )
            .expect("race remap"),
            address
        );
        REPLACEMENTS.fetch_add(1, Ordering::Release);
    }
    core::ptr::null_mut()
}

pub(super) fn verify(original: Metadata) {
    let base = mmap(
        core::ptr::null_mut(),
        8192,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )
    .expect("race output");
    let output = unsafe { base.add(4096 - 28) };
    let handle =
        unsafe { sunlight_libc::thread::spawn(replace_page, base.add(4096)) }.expect("race worker");
    let deadline = sunlight_ipc::monotonic_millis() + 15000;
    let wait = |condition: &dyn Fn() -> bool| {
        while !condition() {
            assert!(
                sunlight_ipc::monotonic_millis() < deadline,
                "VM race deadline"
            );
            sunlight_ipc::process_yield();
        }
    };
    wait(&|| QUIET.load(Ordering::Acquire));
    let mut successes = 0;
    let mut faults = 0;
    for _ in 0..32 {
        unsafe {
            core::ptr::write_bytes(output, 0xa5, 28);
        }
        QUIET.store(false, Ordering::Release);
        let before = REPLACEMENTS.load(Ordering::Acquire);
        RUN.store(true, Ordering::Release);
        wait(&|| REPLACEMENTS.load(Ordering::Acquire) > before);
        let result = loop {
            let result = unsafe { api::probe_stat_attempt("/etc/motd", output as u64) };
            if result != api::PENDING {
                break result;
            }
            assert!(sunlight_ipc::monotonic_millis() < deadline);
            sunlight_ipc::process_yield();
        };
        RUN.store(false, Ordering::Release);
        wait(&|| QUIET.load(Ordering::Acquire));
        // The second page may have been replaced after a successful copy.
        // The untouched first mapping must contain either the exact prefix
        // or its entire canary, never a partial write on failed validation.
        let prefix = unsafe { core::slice::from_raw_parts(output, 28) };
        if result == 0 {
            successes += 1;
            let expected = api::stat("/etc/motd").expect("race fresh request");
            assert_eq!(expected, original);
            // Compare against a stable copy through the same metadata ABI.
            let mut stable = [0u8; 56];
            loop {
                let status =
                    unsafe { api::probe_stat_attempt("/etc/motd", stable.as_mut_ptr() as u64) };
                if status == api::PENDING {
                    sunlight_ipc::process_yield();
                    continue;
                }
                assert_eq!(status, 0);
                break;
            }
            assert_eq!(prefix, &stable[..28]);
        } else {
            assert_eq!(result, u64::MAX);
            faults += 1;
            assert!(prefix.iter().all(|byte| *byte == 0xa5));
        }
        assert_eq!(api::stat("/etc/motd"), Ok(original));
    }
    STOP.store(true, Ordering::Release);
    assert_eq!(sunlight_libc::waitpid(u64::from(handle.tid)), Ok(0));
    munmap(handle.stack_base, handle.stack_size).expect("race stack cleanup");
    munmap(handle.tls_base, handle.tls_size).expect("race TLS cleanup");
    munmap(base, 8192).expect("race mapping cleanup");
    assert_eq!(successes + faults, 32);
    assert!(REPLACEMENTS.load(Ordering::Acquire) >= 32);
    sunlight_ipc::debug_log(
        "[VFS-RPC] native stat concurrent VM replacement atomic copy/recovery OK",
    );
}
