//! Power management: shutdown / reboot via the kernel's `PowerCtl` syscall.
//!
//! See `docs/ACPI_IMPLEMENTATION.md` for the full ACPI implementation this
//! wraps. The kernel exposes syscall 80 (`PowerCtl`) with `rdi` = 0 for
//! shutdown (S5 soft-off) or 1 for reboot; both are handled entirely in
//! kernel/arch ACPI code and never return on success.
//!
//! `sunshell` has its own inline-asm copy of this call (predates this
//! module); new callers (e.g. the Vortex Shell Start Menu) should use these
//! helpers instead of duplicating the raw syscall.

use crate::sys::syscall1;

/// `PowerCtl` syscall number (kernel/src/arch/x86_64/syscall.rs).
const SYS_POWERCTL: u64 = 80;
const POWERCTL_SHUTDOWN: u64 = 0;
const POWERCTL_REBOOT: u64 = 1;

/// Request an ACPI S5 shutdown. Does not return on success.
///
/// # Safety
/// Issues a raw `syscall` instruction; safe to call from any userland
/// context but, like `ProcessExit::exit`, never returns control to the
/// caller when the kernel accepts the request.
pub fn shutdown() -> ! {
    unsafe {
        syscall1(SYS_POWERCTL, POWERCTL_SHUTDOWN);
    }
    // Only reached if the kernel rejected the request (e.g. no ACPI support).
    loop {
        crate::yield_now();
    }
}

/// Request an ACPI reboot. Does not return on success.
pub fn reboot() -> ! {
    unsafe {
        syscall1(SYS_POWERCTL, POWERCTL_REBOOT);
    }
    loop {
        crate::yield_now();
    }
}
