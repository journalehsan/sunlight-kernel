use crate::serial_println;

/// Enable x87/SSE state so user-space x86_64 binaries can execute normal
/// compiler-emitted XMM instructions.
pub fn init_cpu_features() {
    unsafe {
        let mut cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        cr0 &= !(1 << 2); // EM=0: do not trap x87/SSE instructions as #UD.
        cr0 &= !(1 << 3); // TS=0: allow FPU/SSE use without #NM.
        cr0 |= 1 << 1; // MP=1: recommended when EM=0.
        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack));

        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        cr4 |= 1 << 9; // OSFXSR: enable FXSAVE/FXRSTOR and SSE instructions.
        cr4 |= 1 << 10; // OSXMMEXCPT: enable unmasked SSE exception delivery.
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));

        core::arch::asm!("fninit", options(nomem, nostack));
    }

    serial_println!("[CPU] x87/SSE enabled");
}
