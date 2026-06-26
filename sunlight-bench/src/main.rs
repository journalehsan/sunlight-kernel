//! sunlight-bench — SunlightOS performance benchmarking suite.
//!
//! Benchmarks:
//!   Single-core  Pi (Machin fixed-point), Segmented Sieve, Matrix Multiply
//!   Multi-core   Parallel SHA-256
//!
//! Compiled as a no_std/no_main userspace binary using the standard
//! SunlightOS service ABI (user-space.ld linker script, bump allocator).

#![no_std]
#![no_main]

extern crate alloc;

mod bench;
mod matrix;
mod multi;
mod pi;
mod scoring;
mod sha256;
mod sieve;
mod thread;

use bench::Benchmark;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Heap: 16 MiB bump allocator.
// `static mut` ensures it lands in .bss (NOBITS), not .rodata, keeping the
// ELF binary small.  The allocator only hands out pointers once, so the
// absence of a real free() is intentional for this benchmark binary.
//
// Budget:
//   3 × 1024 × 1024 × 4 B = 12 MiB  (matrix A, B, C as Vec<i32>)
//   1 MiB                             (SHA-256 work block)
//   ~3 MiB                            (thread stacks, Vec metadata, format bufs)
// ---------------------------------------------------------------------------
const HEAP_SIZE: usize = 16 * 1024 * 1024;

// SAFETY: single-threaded initialisation; all concurrent allocations go
// through the atomic HEAP_NEXT compare-exchange.
// The newtype wrapper carries the alignment; `static mut` ensures .bss placement.
#[repr(C, align(16))]
struct AlignedHeap([u8; HEAP_SIZE]);
static mut HEAP_DATA: AlignedHeap = AlignedHeap([0u8; HEAP_SIZE]);
static HEAP_NEXT: AtomicUsize = AtomicUsize::new(0);

struct BumpAlloc;

unsafe impl GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        let mut cur = HEAP_NEXT.load(Ordering::Relaxed);
        loop {
            let aligned = (cur + align - 1) & !(align - 1);
            let next = aligned + size;
            if next > HEAP_SIZE {
                return core::ptr::null_mut();
            }
            match HEAP_NEXT.compare_exchange(cur, next, Ordering::SeqCst, Ordering::Relaxed) {
                Ok(_) => {
                    // SAFETY: CAS guarantees exclusive ownership of [aligned, next).
                    // The raw pointer never aliases another live reference.
                    #[allow(static_mut_refs)]
                    return unsafe { HEAP_DATA.0.as_mut_ptr().add(aligned) };
                }
                Err(actual) => cur = actual,
            }
        }
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOC: BumpAlloc = BumpAlloc;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(loc) = info.location() {
        sunlight_ipc::debug_log(&alloc::format!(
            "[BENCH] PANIC at {}:{}", loc.file(), loc.line()
        ));
    } else {
        sunlight_ipc::debug_log("[BENCH] PANIC (no location)");
    }
    sunlight_ipc::ProcessExit::exit(1);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let my_pid = sunlight_ipc::getpid();

    // Raise scheduler priority to reduce interference during measurements.
    // This is the userspace equivalent of kernel set_state(Isolated) —
    // nice −10 biases the BORE scheduler to give us long, uninterrupted quanta.
    sunlight_ipc::set_nice(my_pid, -10);

    sunlight_ipc::debug_log("[BENCH] ============================================");
    sunlight_ipc::debug_log("[BENCH] SunLight-Bench v1.0  starting");
    sunlight_ipc::debug_log("[BENCH] ============================================");

    let ncores = cpu_count();
    sunlight_ipc::debug_log(&alloc::format!("[BENCH] Online cores: {}", ncores));

    let mut results = scoring::Results::new();

    // ----- Single-Core Suite -----
    sunlight_ipc::debug_log("[BENCH] -- Single-Core Suite --");

    {
        let b = pi::PiBench;
        sunlight_ipc::debug_log(&alloc::format!("[BENCH] Running: {}", b.name()));
        let cycles = b.run();
        results.record(b.name(), cycles);
        sunlight_ipc::process_yield();
    }

    {
        let b = sieve::SieveBench;
        sunlight_ipc::debug_log(&alloc::format!("[BENCH] Running: {}", b.name()));
        let cycles = b.run();
        results.record(b.name(), cycles);
        sunlight_ipc::process_yield();
    }

    {
        sunlight_ipc::debug_log("[BENCH] Allocating 1024² matrices (12 MiB)...");
        let b = matrix::MatrixBench::new();
        sunlight_ipc::debug_log(&alloc::format!("[BENCH] Running: {}", b.name()));
        let cycles = b.run();
        results.record(b.name(), cycles);
        sunlight_ipc::process_yield();
    }

    // ----- Multi-Core Suite -----
    sunlight_ipc::debug_log("[BENCH] -- Multi-Core Suite --");

    {
        let b = multi::ParallelSha256::new(ncores);
        sunlight_ipc::debug_log(&alloc::format!(
            "[BENCH] Running: {} (ncores={})",
            b.name(),
            ncores
        ));
        let cycles = b.run();
        results.record(b.name(), cycles);
    }

    // ----- Results Table -----
    results.print_table();

    // Restore original nice level.
    sunlight_ipc::set_nice(my_pid, 0);
    sunlight_ipc::debug_log("[BENCH] Done.");
    sunlight_ipc::ProcessExit::exit(0);
}

// ---------------------------------------------------------------------------
// CPU count from the kernel telemetry page
// ---------------------------------------------------------------------------

/// Read `cpu_count` from the mapped telemetry page.
/// TelemetryPage (repr C) layout up to cpu_count:
///   0  magic      u64   (8)
///   8  version    u32   (4)
///  12  sequence   u32   (4)
///  16  uptime     u64   (8)
///  24  total_ram  u64   (8)
///  32  used_ram   u64   (8)
///  40  zram_orig  u64   (8)
///  48  zram_comp  u64   (8)
///  56  net_rx     u64   (8)
///  64  net_tx     u64   (8)
///  72  tick_hz    u32   (4)
///  76  cpu_count  u8    ← here
fn cpu_count() -> usize {
    const MAGIC: u64 = 0x5355_4E4C_5449_4D45;
    const CPU_COUNT_OFFSET: usize = 76;

    let ptr = sunlight_ipc::map_telemetry();
    if ptr.is_null() {
        return 1;
    }
    // Validate magic.
    let magic = unsafe { core::ptr::read_volatile(ptr as *const u64) };
    if magic != MAGIC {
        return 1;
    }
    let count = unsafe { core::ptr::read_volatile(ptr.add(CPU_COUNT_OFFSET)) };
    (count as usize).max(1)
}
