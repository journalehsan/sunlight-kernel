//! cpufeat — x86-64 microarchitecture level detection for SunlightOS.
//!
//! Reports CPU vendor, brand string, individual feature flags (SSE, AVX, etc.),
//! and whether the CPU is x86-64-v2 or v3 capable according to the psABI spec.
//!
//! Phase 1: visibility only. Does not change kernel CPU setup or process state.

#![no_std]
#![no_main]

use sunlight_libc as libc;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = libc::write(libc::STDOUT, b"cpufeat: panic\n");
    libc::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8) -> ! {
    let code = run();
    libc::exit(code);
}

fn run() -> u64 {
    // CPUID leaf 0: max basic leaf + vendor
    let (max_leaf, vendor) = cpuid_vendor();
    print_str(b"CPU Vendor: ");
    print_str(&vendor);
    print_str(b"\n");

    // Extended brand string (leaves 0x8000_0002..0x8000_0004)
    if cpuid(0x8000_0000, 0).eax >= 0x8000_0004 {
        let brand = cpuid_brand_string();
        if !brand.is_empty() {
            print_str(b"CPU Brand:  ");
            print_str(brand.as_bytes());
            print_str(b"\n");
        }
    }

    print_str(b"\n--- Feature Flags ---\n");

    // Leaf 1: basic features
    let f1 = cpuid(1, 0);
    let sse3 = (f1.ecx & (1 << 0)) != 0;
    let ssse3 = (f1.ecx & (1 << 9)) != 0;
    let sse41 = (f1.ecx & (1 << 19)) != 0;
    let sse42 = (f1.ecx & (1 << 20)) != 0;
    let popcnt = (f1.ecx & (1 << 23)) != 0;
    let aes = (f1.ecx & (1 << 25)) != 0;
    let xsave = (f1.ecx & (1 << 26)) != 0;
    let osxsave = (f1.ecx & (1 << 27)) != 0;
    let avx = (f1.ecx & (1 << 28)) != 0;
    let f16c = (f1.ecx & (1 << 29)) != 0;

    let cmpxchg16b = (f1.ecx & (1 << 13)) != 0;
    let fma = (f1.ecx & (1 << 12)) != 0;

    print_flag(b"SSE3", sse3);
    print_flag(b"SSSE3", ssse3);
    print_flag(b"SSE4.1", sse41);
    print_flag(b"SSE4.2", sse42);
    print_flag(b"POPCNT", popcnt);
    print_flag(b"CMPXCHG16B", cmpxchg16b);
    print_flag(b"AES-NI", aes);
    print_flag(b"XSAVE", xsave);
    print_flag(b"OSXSAVE", osxsave);
    print_flag(b"AVX", avx);
    print_flag(b"F16C", f16c);
    print_flag(b"FMA", fma);

    // Extended leaf 1: LAHF/SAHF, LZCNT
    if max_leaf >= 0x8000_0001 {
        let ef1 = cpuid(0x8000_0001, 0);
        let lahf_sahf = (ef1.ecx & (1 << 0)) != 0;
        let lzcnt = (ef1.ecx & (1 << 5)) != 0;
        print_flag(b"LAHF/SAHF", lahf_sahf);
        print_flag(b"LZCNT", lzcnt);
    }

    // Leaf 7.0: AVX2, BMI1, BMI2, MOVBE
    let f7 = if max_leaf >= 7 {
        cpuid(7, 0)
    } else {
        CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
        }
    };
    let bmi1 = (f7.ebx & (1 << 3)) != 0;
    let avx2 = (f7.ebx & (1 << 5)) != 0;
    let bmi2 = (f7.ebx & (1 << 8)) != 0;
    let movbe = (f7.ecx & (1 << 22)) != 0;

    print_flag(b"BMI1", bmi1);
    print_flag(b"BMI2", bmi2);
    print_flag(b"AVX2", avx2);
    print_flag(b"MOVBE", movbe);

    // XCR0 check for AVX state support
    let xcr0_avx = if osxsave {
        let xcr0 = read_xcr0();
        // Bits 1 (SSE) and 2 (AVX/YMM) must be set
        (xcr0 & 0b110) == 0b110
    } else {
        false
    };
    print_flag(b"XCR0 AVX state", xcr0_avx);

    print_str(b"\n--- Microarchitecture Levels ---\n");

    // x86-64-v2: SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B, LAHF/SAHF
    let ef1 = if max_leaf >= 0x8000_0001 {
        cpuid(0x8000_0001, 0)
    } else {
        CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
        }
    };
    let lahf_sahf = (ef1.ecx & (1 << 0)) != 0;
    let v2_capable = sse3 && ssse3 && sse41 && sse42 && popcnt && cmpxchg16b && lahf_sahf;
    print_capability(b"x86-64-v2", v2_capable);

    // x86-64-v3: v2 + AVX, AVX2, BMI1, BMI2, F16C, FMA, LZCNT, MOVBE + OS AVX support
    let lzcnt = (ef1.ecx & (1 << 5)) != 0;
    let v3_capable =
        v2_capable && avx && avx2 && bmi1 && bmi2 && f16c && fma && lzcnt && movbe && xcr0_avx;
    print_capability(b"x86-64-v3", v3_capable);

    if !xcr0_avx && avx {
        print_str(b"\nNote: CPU has AVX instructions, but OS has not enabled XCR0 AVX state.\n");
        print_str(b"      v3 requires kernel support for XSAVE/YMM context switching.\n");
    }

    0
}

fn print_flag(name: &[u8], present: bool) {
    print_str(name);
    print_str(b": ");
    print_str(if present { b"yes\n" } else { b"no\n" });
}

fn print_capability(level: &[u8], capable: bool) {
    print_str(level);
    print_str(b" capable: ");
    print_str(if capable { b"yes\n" } else { b"no\n" });
}

fn print_str(s: &[u8]) {
    let _ = libc::write(libc::STDOUT, s);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CpuidResult {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "mov {tmp:r}, rbx",
            "cpuid",
            "xchg {tmp:r}, rbx",
            inout("eax") leaf => eax,
            tmp = out(reg) ebx,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
            options(nostack)
        );
    }
    CpuidResult { eax, ebx, ecx, edx }
}

fn cpuid_vendor() -> (u32, [u8; 13]) {
    let r = cpuid(0, 0);
    let mut vendor = [0u8; 13];
    vendor[0..4].copy_from_slice(&r.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&r.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&r.ecx.to_le_bytes());
    vendor[12] = 0;
    (r.eax, vendor)
}

fn cpuid_brand_string() -> &'static str {
    static mut BRAND: [u8; 49] = [0; 49];
    unsafe {
        let r2 = cpuid(0x8000_0002, 0);
        let r3 = cpuid(0x8000_0003, 0);
        let r4 = cpuid(0x8000_0004, 0);
        BRAND[0..4].copy_from_slice(&r2.eax.to_le_bytes());
        BRAND[4..8].copy_from_slice(&r2.ebx.to_le_bytes());
        BRAND[8..12].copy_from_slice(&r2.ecx.to_le_bytes());
        BRAND[12..16].copy_from_slice(&r2.edx.to_le_bytes());
        BRAND[16..20].copy_from_slice(&r3.eax.to_le_bytes());
        BRAND[20..24].copy_from_slice(&r3.ebx.to_le_bytes());
        BRAND[24..28].copy_from_slice(&r3.ecx.to_le_bytes());
        BRAND[28..32].copy_from_slice(&r3.edx.to_le_bytes());
        BRAND[32..36].copy_from_slice(&r4.eax.to_le_bytes());
        BRAND[36..40].copy_from_slice(&r4.ebx.to_le_bytes());
        BRAND[40..44].copy_from_slice(&r4.ecx.to_le_bytes());
        BRAND[44..48].copy_from_slice(&r4.edx.to_le_bytes());
        BRAND[48] = 0;

        // Trim trailing spaces/nulls
        let mut end = 48;
        while end > 0 && (BRAND[end - 1] == 0 || BRAND[end - 1] == b' ') {
            end -= 1;
        }
        core::str::from_utf8_unchecked(&BRAND[..end])
    }
}

fn read_xcr0() -> u64 {
    let eax: u32;
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "xor ecx, ecx",
            "xgetbv",
            out("eax") eax,
            out("edx") edx,
            out("ecx") _,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((edx as u64) << 32) | (eax as u64)
}
