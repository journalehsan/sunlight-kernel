//! Launch latency tracing for app spawn debugging.
//! Minimal overhead: unique atomic IDs, TSC timestamps.
//! Disable at compile time by setting cfg flag `launch_trace_off`; see
//! `next_launch_id` / `LaunchTrace::new` for usage.
//!
//! Observed trace points (nanoseconds since boot, from `interrupts::now_ns()`):
//!   1. launch_request_received   – syscall entry, path known
//!   2. app_resolution_started    – before embedded/VFS lookup
//!   3. app_resolution_finished   – binary bytes resolved
//!   4. spawn_started             – about to exec_into_process + page-table build
//!   5. spawn_returned            – exec_into_process returned Ok
//!   6. child_process_created     – pid assigned, before enqueue
//!   7. enqueue_finished          – child is in the run queue (observable by scheduler)
//!
//! Not yet observable from the kernel side (always reported as "unknown"):
//!   8. display_connection_started
//!   9. window_registration_started
//!  10. first_window_or_first_paint
//!
//! Failures set `result=failed:<stage>` in the trace line.

use core::sync::atomic::{AtomicU64, Ordering};

/// Monotonically increasing per-launch identifier.
static LAUNCH_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique, non-zero launch ID.
#[cfg(feature = "launch_trace")]
pub fn next_launch_id() -> u64 {
    LAUNCH_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// No-op stub when tracing disabled.
#[cfg(not(feature = "launch_trace"))]
#[inline(always)]
pub fn next_launch_id() -> u64 {
    0
}

/// All timestamp fields are nanoseconds since boot (from `now_ns()`).
/// A value of 0 means "not yet reached" or "not observable at this layer".
#[derive(Debug, Clone, Copy, Default)]
pub struct LaunchTrace {
    /// Unique ID for this launch; assigned at syscall entry.
    pub launch_id: u64,
    /// [1] Syscall entry — path has been read from user space.
    pub request_received_ns: u64,
    /// [2] Just before the embedded-image / VFS lookup.
    pub resolve_started_ns: u64,
    /// [3] Binary bytes are ready.
    pub resolve_finished_ns: u64,
    /// [4] exec_into_process is about to be called.
    pub spawn_started_ns: u64,
    /// [5] exec_into_process returned successfully.
    pub spawn_returned_ns: u64,
    /// [6] PID assigned, child struct fully prepared.
    pub child_created_ns: u64,
    /// [7] sched.enqueue_process() returned; child is runnable.
    pub enqueue_finished_ns: u64,
}

impl LaunchTrace {
    /// Create a new trace with `launch_id` set and point [1] stamped.
    #[inline]
    #[cfg(feature = "launch_trace")]
    pub fn new(launch_id: u64, now_ns: u64) -> Self {
        Self {
            launch_id,
            request_received_ns: now_ns,
            ..Default::default()
        }
    }

    /// No-op stub: returns a zeroed trace when tracing is disabled.
    #[cfg(not(feature = "launch_trace"))]
    #[inline(always)]
    pub fn new(_launch_id: u64, _now_ns: u64) -> Self {
        Self::default()
    }

    /// Emit a compact serial trace line.
    ///
    /// ```text
    /// [LAUNCH-TRACE] app=calculator launch_id=42 path=sun-exec \
    ///   resolve_ms=1 spawn_ms=7 queue_or_wait_ms=unknown \
    ///   display_ms=unknown total_ms=8 result=ok pid=17
    /// ```
    #[cfg(feature = "launch_trace")]
    pub fn emit(&self, app_name: &str, path: &str, pid: Option<usize>, result: &str) {
        // Compute delta in ms; 0 means not yet reached or unknown.
        let resolve_ms = ns_to_ms(self.resolve_finished_ns.saturating_sub(self.resolve_started_ns));
        let spawn_ms   = ns_to_ms(self.enqueue_finished_ns.saturating_sub(self.spawn_started_ns));
        let total_ms   = ns_to_ms(self.enqueue_finished_ns.saturating_sub(self.request_received_ns));

        // Format pid as decimal string or "none"
        let mut pid_buf = [0u8; 20];
        let pid_str = match pid {
            Some(p) => fmt_u64(&mut pid_buf, p as u64),
            None => "none",
        };

        crate::serial_println!(
            "[LAUNCH-TRACE] app={} launch_id={} path={} resolve_ms={} spawn_ms={} \
             queue_or_wait_ms=unknown display_ms=unknown total_ms={} result={} pid={}",
            app_name, self.launch_id, path,
            resolve_ms, spawn_ms, total_ms, result, pid_str
        );
    }

    /// No-op stub: does not emit logs when tracing disabled.
    #[cfg(not(feature = "launch_trace"))]
    #[inline(always)]
    pub fn emit(&self, _app_name: &str, _path: &str, _pid: Option<usize>, _result: &str) {}
}

/// Convert nanoseconds to milliseconds.
#[inline]
fn ns_to_ms(ns: u64) -> u64 {
    ns / 1_000_000
}

/// Format a u64 as decimal ASCII into `buf`, returning the slice.
fn fmt_u64(buf: &mut [u8; 20], mut n: u64) -> &str {
    if n == 0 {
        buf[19] = b'0';
        return core::str::from_utf8(&buf[19..]).unwrap_or("0");
    }
    let mut pos = 20usize;
    while n > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    core::str::from_utf8(&buf[pos..]).unwrap_or("?")
}
