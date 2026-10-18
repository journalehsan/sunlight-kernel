//! Synchronized C1.2B single-vCPU fairness and creation-order experiment.
//!
//! The workload deliberately uses the existing native ThreadSpawn ABI through
//! `thread.rs`. Four borrower threads execute a deterministic xorshift/rotate
//! integer mix until a monotonic deadline. The coordinator only yields while
//! waiting, so the workers exercise timer preemption and the real guest
//! scheduler rather than a host-side worker pool.

#![no_std]
#![no_main]
#![deny(warnings)]

#[allow(dead_code)]
mod thread;

use core::hint::black_box;
use core::sync::atomic::{AtomicU64, Ordering};

use sunlight_ipc::{debug_log, monotonic_millis, process_yield, ProcessExit};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[SCHED-BENCH] panic");
    ProcessExit::exit(1);
}

const WORKERS: usize = 4;
const DEFAULT_DURATION_MS: u64 = 1_000;
const DEFAULT_SEED: u64 = 1;
const DEFAULT_CPUS: u64 = 1;
const MAX_DURATION_MS: u64 = 10_000;

static READY: AtomicU64 = AtomicU64::new(0);
static START_MS: AtomicU64 = AtomicU64::new(0);
static STOP_MS: AtomicU64 = AtomicU64::new(0);
static SEED: AtomicU64 = AtomicU64::new(DEFAULT_SEED);
static FINISHED: AtomicU64 = AtomicU64::new(0);
static PROGRESS: [AtomicU64; WORKERS] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; WORKERS]
};
static CHECKSUM: [AtomicU64; WORKERS] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; WORKERS]
};

#[derive(Clone, Copy)]
struct Config {
    duration_ms: u64,
    seed: u64,
    cpus: u64,
    run: u64,
    repetition: u64,
    reverse: bool,
}

unsafe extern "C" fn worker(arg: u64) {
    let slot = arg as usize;
    if diagnostic(1, arg) != 0 {
        ProcessExit::exit(1);
    }
    READY_AT[slot].store(monotonic_millis(), Ordering::Relaxed);
    READY.fetch_add(1, Ordering::Release);
    while START_MS.load(Ordering::Acquire) == 0 {
        process_yield();
    }

    let seed = SEED.load(Ordering::Acquire) ^ 0x9E37_79B9_7F4A_7C15;
    let mut state = seed;
    let mut operations = 0u64;
    let start = START_MS.load(Ordering::Acquire);
    // START is the release/acquire gate for the already-published deadline.
    let stop = STOP_MS.load(Ordering::Relaxed);
    ENTERED_AT[slot].store(monotonic_millis(), Ordering::Relaxed);

    while monotonic_millis() < stop {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state = state.wrapping_mul(0x2545_F491_4F6C_DD1D).rotate_left(7);
        state = state.wrapping_add(start ^ operations);
        operations = operations.wrapping_add(1);
        if operations & 0x3ff == 0 {
            PROGRESS[slot].store(operations, Ordering::Release);
        }
    }

    EXITED_AT[slot].store(monotonic_millis(), Ordering::Relaxed);
    if diagnostic(3, arg) != 0 {
        ProcessExit::exit(1);
    }
    black_box(state);
    PROGRESS[slot].store(operations, Ordering::Release);
    CHECKSUM[slot].store(state, Ordering::Release);
    FINISHED.fetch_add(1, Ordering::Release);
}

// Feature-private observer ABI. These calls are outside the arithmetic loop.
fn diagnostic(action: u64, value: u64) -> u64 {
    syscall(0x5342, action, value)
}

fn syscall(number: u64, a: u64, b: u64) -> u64 {
    let result: u64;
    unsafe {
        core::arch::asm!("syscall", inlateout("rax") number => result,
            in("rdi") a, in("rsi") b, lateout("rcx") _, lateout("r11") _,
            options(nostack));
    }
    result
}

static READY_AT: [AtomicU64; WORKERS] = [const { AtomicU64::new(0) }; WORKERS];
static ENTERED_AT: [AtomicU64; WORKERS] = [const { AtomicU64::new(0) }; WORKERS];
static EXITED_AT: [AtomicU64; WORKERS] = [const { AtomicU64::new(0) }; WORKERS];

fn parse_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((byte - b'0') as u64)?;
    }
    Some(value)
}

unsafe fn arg<'a>(argc: u64, argv: *const *const u8, index: usize) -> &'a [u8] {
    if argv.is_null() || index >= argc as usize {
        return &[];
    }
    let ptr = *argv.add(index);
    if ptr.is_null() {
        return &[];
    }
    let mut len = 0usize;
    while len < 64 && *ptr.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}

unsafe fn config_from_args(argc: u64, argv: *const *const u8) -> Config {
    let mut config = Config {
        duration_ms: DEFAULT_DURATION_MS,
        seed: DEFAULT_SEED,
        cpus: DEFAULT_CPUS,
        run: 1,
        repetition: 1,
        reverse: false,
    };
    let mut index = 1usize;
    while index + 1 < argc as usize {
        let key = arg(argc, argv, index);
        let value = parse_u64(arg(argc, argv, index + 1));
        match (key, value) {
            (b"--duration-ms", Some(value)) => config.duration_ms = value,
            (b"--seed", Some(value)) => config.seed = value,
            (b"--cpus", Some(value)) => config.cpus = value.max(1),
            _ => {}
        }
        index += 2;
    }
    config.duration_ms = config.duration_ms.clamp(1, MAX_DURATION_MS);
    config
}

struct JsonRecord {
    bytes: [u8; 256],
    len: usize,
}

impl JsonRecord {
    fn new() -> Self {
        Self {
            bytes: [0; 256],
            len: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let remaining = self.bytes.len().saturating_sub(self.len);
        let count = bytes.len().min(remaining);
        self.bytes[self.len..self.len + count].copy_from_slice(&bytes[..count]);
        self.len += count;
    }

    fn number(&mut self, value: u64) {
        let mut digits = [0u8; 20];
        let mut index = digits.len();
        let mut value = value;
        if value == 0 {
            self.push(b"0");
            return;
        }
        while value != 0 {
            index -= 1;
            digits[index] = b'0' + (value % 10) as u8;
            value /= 10;
        }
        self.push(&digits[index..]);
    }

    fn finish(self) {
        if let Ok(text) = core::str::from_utf8(&self.bytes[..self.len]) {
            debug_log(text);
        }
    }
}

fn run_id(record: &mut JsonRecord, config: Config) {
    record.number(config.cpus);
    record.push(b"-");
    record.number(config.run);
    record.push(b"-");
    record.number(config.seed);
}

fn emit_start(config: Config) {
    let mut record = JsonRecord::new();
    record.push(b"[SCHED-BENCH] START {\"run\":\"");
    run_id(&mut record, config);
    record.push(b"\",\"seed\":");
    record.number(config.seed);
    record.push(b",\"cpus\":");
    record.number(config.cpus);
    record.push(b",\"workers\":4,\"duration_ms\":");
    record.number(config.duration_ms);
    record.push(b",\"repetition\":");
    record.number(config.repetition);
    record.push(b",\"order\":\"");
    record.push(if config.reverse {
        b"reverse"
    } else {
        b"forward"
    });
    record.push(b"\",\"samples\":1,\"clock\":\"monotonic_ms\"}");
    record.finish();
}

fn emit_sample(config: Config, elapsed_ms: u64, total: u64) {
    let mut record = JsonRecord::new();
    record.push(b"[SCHED-BENCH] SAMPLE {\"run\":\"");
    run_id(&mut record, config);
    record.push(b"\",\"index\":0,\"elapsed_ms\":");
    record.number(elapsed_ms);
    record.push(b",\"operations\":");
    record.number(total);
    record.push(b",\"ops_per_sec\":");
    let ops_per_sec = total
        .saturating_mul(1_000)
        .checked_div(elapsed_ms.max(1))
        .unwrap_or(0);
    record.number(ops_per_sec);
    record.push(b",\"worker0\":");
    record.number(PROGRESS[0].load(Ordering::Acquire));
    record.push(b",\"worker1\":");
    record.number(PROGRESS[1].load(Ordering::Acquire));
    record.push(b",\"worker2\":");
    record.number(PROGRESS[2].load(Ordering::Acquire));
    record.push(b",\"worker3\":");
    record.number(PROGRESS[3].load(Ordering::Acquire));
    record.push(b",\"checksum\":");
    let mut checksum = 0u64;
    for slot in 0..WORKERS {
        checksum ^= CHECKSUM[slot].load(Ordering::Acquire);
    }
    record.number(checksum);
    record.push(b"}");
    record.finish();
}

fn emit_end(config: Config, total: u64, elapsed_ms: u64, success: bool) {
    let mut record = JsonRecord::new();
    record.push(b"[SCHED-BENCH] END {\"run\":\"");
    run_id(&mut record, config);
    record.push(b"\",\"sample_count\":1,\"status\":\"");
    record.push(if success { b"ok" } else { b"error" });
    record.push(b"\",\"elapsed_ms\":");
    record.number(elapsed_ms);
    record.push(b",\"operations\":");
    record.number(total);
    record.push(b",\"all_workers_progress\":");
    record.push(
        if (0..WORKERS).all(|slot| PROGRESS[slot].load(Ordering::Acquire) > 0) {
            b"true"
        } else {
            b"false"
        },
    );
    record.push(b"}");
    record.finish();
}

fn run(config: Config) -> bool {
    emit_start(config);
    if diagnostic(0, 0) != 0 {
        return false;
    }
    READY.store(0, Ordering::Release);
    START_MS.store(0, Ordering::Release);
    STOP_MS.store(0, Ordering::Release);
    FINISHED.store(0, Ordering::Release);
    SEED.store(config.seed, Ordering::Release);
    for slot in 0..WORKERS {
        PROGRESS[slot].store(0, Ordering::Release);
        CHECKSUM[slot].store(0, Ordering::Release);
    }
    let mut tids = [0u64; WORKERS];
    let mut created_at = [0u64; WORKERS];
    for rank in 0..WORKERS {
        let slot = if config.reverse {
            WORKERS - 1 - rank
        } else {
            rank
        };
        created_at[slot] = monotonic_millis();
        tids[slot] = thread::spawn(worker, slot as u64);
        if tids[slot] == 0 {
            return false;
        }
    }
    // A bounded wait fails the whole process, never releases a partial cohort.
    let ready_limit = monotonic_millis() + 30_000;
    while READY.load(Ordering::Acquire) != WORKERS as u64 {
        if monotonic_millis() > ready_limit {
            return false;
        }
        process_yield();
    }
    if diagnostic(2, config.run) != 0 {
        return false;
    }
    let start = monotonic_millis();
    STOP_MS.store(start.saturating_add(config.duration_ms), Ordering::Relaxed);
    START_MS.store(start, Ordering::Release);

    while FINISHED.load(Ordering::Acquire) != WORKERS as u64 {
        if monotonic_millis() > start + config.duration_ms + 30_000 {
            return false;
        }
        process_yield();
    }
    let elapsed = monotonic_millis().saturating_sub(start);
    // Logical completion is not thread exit. Do not reset shared data or reuse
    // benchmark state until every borrower is definitively Reaped.
    for tid in tids {
        while syscall(156, tid, 0) != 1 {
            if monotonic_millis() > start + config.duration_ms + 30_000 {
                return false;
            }
            process_yield();
        }
    }
    if diagnostic(4, config.run) != 0 {
        return false;
    }
    for (slot, created) in created_at.iter().enumerate() {
        let mut record = JsonRecord::new();
        record.push(b"[SCHED-BENCH] LIFECYCLE {\"run\":\"");
        run_id(&mut record, config);
        record.push(b"\",\"slot\":");
        record.number(slot as u64);
        record.push(b",\"created_ms\":");
        record.number(*created);
        record.push(b",\"ready_ms\":");
        record.number(READY_AT[slot].load(Ordering::Acquire));
        record.push(b",\"start_ms\":");
        record.number(start);
        record.push(b",\"entered_ms\":");
        record.number(ENTERED_AT[slot].load(Ordering::Acquire));
        record.push(b",\"exited_ms\":");
        record.number(EXITED_AT[slot].load(Ordering::Acquire));
        record.push(b"}");
        record.finish();
    }
    let total = (0..WORKERS)
        .map(|slot| PROGRESS[slot].load(Ordering::Acquire))
        .fold(0u64, u64::saturating_add);
    let success = (0..WORKERS).all(|slot| PROGRESS[slot].load(Ordering::Acquire) > 0);
    emit_sample(config, elapsed, total);
    emit_end(config, total, elapsed, success);
    success
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    let mut config = unsafe { config_from_args(argc, argv) };
    if !sunlight_ipc::set_nice(0, 0) {
        ProcessExit::exit(1);
    }
    // Alternate order within each repetition to separate slot identity from
    // creation rank. Seed, instructions and input state are identical.
    let mut ordinal = 0;
    for repetition in 0..=5 {
        let durations: &[u64] = if repetition == 0 {
            &[1_000]
        } else {
            &[5_000, 10_000]
        };
        for &duration in durations {
            for reverse in [false, true] {
                ordinal += 1;
                config.duration_ms = duration;
                config.repetition = repetition.max(1);
                config.reverse = reverse;
                config.run = ordinal;
                if !run(config) {
                    debug_log("[SCHED-BENCH] suite failed");
                    ProcessExit::exit(1);
                }
            }
        }
    }
    debug_log("[SCHED-BENCH] SUITE complete runs=22");
    ProcessExit::exit(0);
}
