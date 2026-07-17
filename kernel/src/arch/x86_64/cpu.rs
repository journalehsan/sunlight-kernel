use crate::serial_println;
use core::sync::atomic::{AtomicUsize, Ordering};

const EFER_MSR: u32 = 0xC000_0080;
const EFER_NXE: u64 = 1 << 11;

static NXE_ENABLED_CPUS: AtomicUsize = AtomicUsize::new(0);

fn nx_supported() -> bool {
    let max_extended = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    max_extended >= 0x8000_0001 && (core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 20)) != 0
}

pub fn nxe_enabled_cpu_count() -> usize {
    NXE_ENABLED_CPUS.load(Ordering::Acquire)
}

pub fn nxe_active() -> bool {
    unsafe { x86_64::registers::model_specific::Msr::new(EFER_MSR).read() & EFER_NXE != 0 }
}

/// Enable x87/SSE state so user-space x86_64 binaries can execute normal
/// compiler-emitted XMM instructions.
pub fn init_cpu_features() {
    if !nx_supported() {
        panic!("[CPU] NX unsupported; MM-0 executable-memory hardening requires NX");
    }

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

        let mut efer_msr = x86_64::registers::model_specific::Msr::new(EFER_MSR);
        let efer = efer_msr.read();
        efer_msr.write(efer | EFER_NXE);
    }

    assert!(nxe_active(), "EFER.NXE did not remain set");
    let cpu_count = NXE_ENABLED_CPUS.fetch_add(1, Ordering::AcqRel) + 1;
    serial_println!("[CPU] x87/SSE enabled; NXE active cpu_count={}", cpu_count);
}
