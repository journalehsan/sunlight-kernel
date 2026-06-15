//! Kernel operation shims (gcd side).
//!
//! `set_nice` is REAL — it wraps syscall 83 (SetNice) via `sunlight_ipc::set_nice`
//! (gcd doesn't currently call it, but it's included for symmetry/future use).
//! `send_signal`, `force_terminate`, and `reap` do NOT have corresponding
//! kernel syscalls yet. They are implemented as logging no-ops that return
//! `false`.

pub trait KernelOps {
    /// REAL: wraps syscall 83 (SetNice).
    fn set_nice(&self, pid: usize, nice: i8) -> bool;

    /// SHIM — TODO: kernel syscall — send_signal: deliver a POSIX-style
    /// signal to `pid`. No such syscall exists.
    fn send_signal(&self, pid: usize, sig: u8) -> bool;

    /// SHIM — TODO: kernel syscall — force_terminate: unconditionally tear
    /// down a process's address space and PCB. No such syscall exists.
    fn force_terminate(&self, pid: usize) -> bool;

    /// SHIM — TODO: kernel syscall — reap: release a Finished (zombie)
    /// process's PCB slot back to the scheduler. No such syscall exists.
    fn reap(&self, pid: usize) -> bool;
}

pub struct RealKernelOps;

impl KernelOps for RealKernelOps {
    fn set_nice(&self, pid: usize, nice: i8) -> bool {
        sunlight_ipc::set_nice(pid as u64, nice)
    }

    fn send_signal(&self, pid: usize, sig: u8) -> bool {
        serial_println!(
            "[GCD] TODO: kernel syscall — send_signal: pid={} sig={}",
            pid,
            sig
        );
        false
    }

    fn force_terminate(&self, pid: usize) -> bool {
        serial_println!(
            "[GCD] TODO: kernel syscall — force_terminate: pid={}",
            pid
        );
        false
    }

    fn reap(&self, pid: usize) -> bool {
        serial_println!("[GCD] TODO: kernel syscall — reap: pid={}", pid);
        false
    }
}
