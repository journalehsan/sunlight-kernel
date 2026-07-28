//! memoryctl — Physical memory accounting diagnostics (Phase 1).
//!
//! Usage:
//!   memoryctl accounting
//!   memoryctl accounting --details
//!   memoryctl accounting --tasks
//!   memoryctl accounting --verify

#![no_std]
#![no_main]

use core::fmt::Write;
use sunlight_telemetry::{MemoryAccountingSnapshot, Telemetry, RAMFS_METADATA_UNAVAILABLE};

struct BufWriter {
    buf: [u8; 512],
    len: usize,
}

impl BufWriter {
    fn new() -> Self {
        Self {
            buf: [0; 512],
            len: 0,
        }
    }
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
    fn clear(&mut self) {
        self.len = 0;
    }
}

impl Write for BufWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let space = self.buf.len().saturating_sub(self.len);
        let n = bytes.len().min(space);
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
        Ok(())
    }
}

fn emit(s: &str) {
    sunlight_ipc::debug_log(s);
    // Also try stdout if tty is available — best-effort via debug_log for serial.
    let _ = s;
}

fn emit_line(w: &mut BufWriter) {
    if w.len < w.buf.len() {
        w.buf[w.len] = b'\n';
        w.len += 1;
    }
    if let Ok(s) = core::str::from_utf8(w.as_bytes()) {
        emit(s);
    }
    w.clear();
}

fn mib(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

fn kib(bytes: u64) -> u64 {
    bytes / 1024
}

fn print_accounting(acct: &MemoryAccountingSnapshot, details: bool) {
    let mut w = BufWriter::new();
    let _ = write!(w, "Physical Memory Accounting");
    emit_line(&mut w);
    let _ = write!(
        w,
        "Installed: {} MiB ({} KiB)",
        mib(acct.installed_bytes),
        kib(acct.installed_bytes)
    );
    emit_line(&mut w);
    let _ = write!(w, "Usable:    {} MiB", mib(acct.usable_bytes));
    emit_line(&mut w);
    let _ = write!(w, "Managed:   {} MiB", mib(acct.managed_bytes));
    emit_line(&mut w);
    let _ = write!(w, "Used:      {} MiB", mib(acct.used_bytes()));
    emit_line(&mut w);
    let _ = write!(w, "Free:      {} MiB", mib(acct.free_bytes));
    emit_line(&mut w);
    let _ = write!(w, "Reserved:  {} MiB", mib(acct.reserved_bytes));
    emit_line(&mut w);
    let _ = write!(w, "");
    emit_line(&mut w);
    let _ = write!(w, "Tasks:                 {}", acct.active_task_count);
    emit_line(&mut w);
    let _ = write!(
        w,
        "Task private unique:   {} MiB",
        mib(acct.task_private_unique_bytes)
    );
    emit_line(&mut w);
    let _ = write!(
        w,
        "Shared memory:         {} MiB",
        mib(acct.shared_memory_unique_bytes)
    );
    emit_line(&mut w);
    let _ = write!(
        w,
        "Kernel:                {} MiB",
        mib(acct.kernel_total_bytes())
    );
    emit_line(&mut w);
    if details {
        let _ = write!(
            w,
            "  core: {}  heap: {}  stack: {} MiB",
            mib(acct.kernel_core_bytes),
            mib(acct.kernel_heap_bytes),
            mib(acct.kernel_stack_bytes)
        );
        emit_line(&mut w);
    }
    let _ = write!(
        w,
        "Page tables:           {} MiB",
        mib(acct.page_table_bytes)
    );
    emit_line(&mut w);
    let _ = write!(
        w,
        "RAMFS data:            {} MiB",
        mib(acct.ramfs_file_data_bytes)
    );
    emit_line(&mut w);
    if acct.ramfs_metadata_bytes == RAMFS_METADATA_UNAVAILABLE {
        let _ = write!(w, "RAMFS metadata:        unavailable (under Kernel Heap)");
    } else {
        let _ = write!(
            w,
            "RAMFS metadata:        {} MiB",
            mib(acct.ramfs_metadata_bytes)
        );
    }
    emit_line(&mut w);
    if details {
        let _ = write!(
            w,
            "Retained boot image:   {} bytes",
            acct.retained_boot_image_bytes
        );
        emit_line(&mut w);
    }
    let _ = write!(
        w,
        "Caches:                {} MiB",
        mib(acct.cache_total_bytes())
    );
    emit_line(&mut w);
    let _ = write!(
        w,
        "Graphics/device:       {} MiB",
        mib(acct.graphics_and_device_bytes())
    );
    emit_line(&mut w);
    let _ = write!(
        w,
        "ZRAM physical:         {} MiB",
        mib(acct.zram_physical_bytes)
    );
    emit_line(&mut w);
    if details {
        let _ = write!(
            w,
            "ZRAM logical:          {} MiB",
            mib(acct.zram_logical_bytes)
        );
        emit_line(&mut w);
    }
    let _ = write!(
        w,
        "Other accounted:       {} MiB",
        mib(acct.other_accounted_bytes)
    );
    emit_line(&mut w);
    let _ = write!(
        w,
        "Unclassified:          {} MiB",
        mib(acct.unclassified_bytes)
    );
    emit_line(&mut w);
    let _ = write!(w, "");
    emit_line(&mut w);
    let _ = write!(
        w,
        "Accounting delta:      {} bytes",
        acct.conservation_delta_bytes
    );
    emit_line(&mut w);
    let _ = write!(w, "Snapshot generation:   {}", acct.sample_generation);
    emit_line(&mut w);
    let _ = write!(
        w,
        "Verification:          {}",
        if acct.conservation_ok() {
            "PASS"
        } else {
            "FAIL"
        }
    );
    emit_line(&mut w);
}

fn print_tasks(telem: &Telemetry) {
    let snap = telem.snapshot();
    let mut w = BufWriter::new();
    let _ = write!(w, "Tasks ({}): pid gen mapped_KiB name", snap.proc_count);
    emit_line(&mut w);
    for i in 0..snap.proc_count {
        let p = &snap.procs[i];
        let _ = write!(
            w,
            "  {:>5} {:>8} {:>10} {}",
            p.pid,
            p.generation,
            p.mem_kb,
            p.name_str()
        );
        emit_line(&mut w);
    }
    let _ = write!(
        w,
        "Note: mapped_KiB is present user pages (shared counted per map)."
    );
    emit_line(&mut w);
    let _ = write!(
        w,
        "Unique task-private physical: {} MiB (see accounting).",
        mib(snap.mem_acct.task_private_unique_bytes)
    );
    emit_line(&mut w);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        sunlight_ipc::process_yield();
    }
}

// No heap needed.
struct NoAlloc;
unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}
#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

fn arg_has(argc: usize, argv: *const *const u8, needle: &str) -> bool {
    if argv.is_null() {
        return false;
    }
    for i in 0..argc {
        let p = unsafe { *argv.add(i) };
        if p.is_null() {
            continue;
        }
        let mut len = 0usize;
        while unsafe { *p.add(len) } != 0 && len < 64 {
            len += 1;
        }
        let s = unsafe { core::slice::from_raw_parts(p, len) };
        if s == needle.as_bytes() {
            return true;
        }
    }
    false
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    let argc = argc as usize;
    let mut telem = match Telemetry::init() {
        Ok(t) => t,
        Err(_) => {
            emit("memoryctl: telemetry unavailable\n");
            sunlight_ipc::ProcessExit::exit(1);
        }
    };

    // Wait for a fresh sample.
    for _ in 0..50 {
        if telem.poll() {
            break;
        }
        sunlight_ipc::process_yield();
    }
    let _ = telem.poll();

    let details = arg_has(argc, argv, "--details");
    let tasks = arg_has(argc, argv, "--tasks");
    let verify = arg_has(argc, argv, "--verify");
    let accounting = arg_has(argc, argv, "accounting") || argc <= 1 || verify || details || tasks;

    if !accounting {
        emit("Usage: memoryctl accounting [--details] [--tasks] [--verify]\n");
        sunlight_ipc::ProcessExit::exit(2);
    }

    let acct = telem.snapshot().mem_acct;
    print_accounting(&acct, details);
    if tasks {
        print_tasks(&telem);
    }

    if verify {
        let ok = acct.conservation_ok()
            && acct.managed_bytes > 0
            && acct.sample_generation > 0
            && acct.used_bytes() <= acct.managed_bytes;
        if !ok {
            emit("memoryctl: verification FAILED\n");
            sunlight_ipc::ProcessExit::exit(1);
        }
        emit("memoryctl: verification PASS\n");
    }

    sunlight_ipc::ProcessExit::exit(0);
}
