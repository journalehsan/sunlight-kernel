//! Hypervisor/Platform profile detection via CPUID.
//!
//! Uses CPUID leaf 0x40000000 to read the hypervisor vendor string,
//! and CPUID.1:ECX bit 31 to detect whether a hypervisor is present.

/// Detect hypervisor/profile and return a human-readable label.
pub fn detect_profile() -> &'static str {
    let (_, _, ecx1, _) = cpuid(1);
    if ecx1 & (1 << 31) == 0 {
        return "Bare metal";
    }

    let (_, ebx, ecx, edx) = cpuid(0x4000_0000);

    if ebx == 0x6177_4D56 && ecx == 0x4D56_6572 && edx == 0x6572_6177 {
        return "VMware";
    }
    if ebx == 0x4B4D_564B && ecx == 0x564B_4D56 && edx == 0x0000_004D {
        return "QEMU";
    }
    if ebx == 0x5447_4354 && ecx == 0x5447_4354 && edx == 0x5447_4354 {
        return "QEMU";
    }

    "Unknown"
}

fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    let (a, b, c, d): (u32, u32, u32, u32);
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {tmp:e}, ebx",
            "pop rbx",
            inout("eax") leaf => a,
            tmp = out(reg) b,
            out("ecx") c,
            out("edx") d,
        );
    }
    (a, b, c, d)
}
