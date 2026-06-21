# Phase 1: x86-64-v2/v3 Feature Reporting

Date: 2026-06-21

## Scope

Phase 1 adds visibility only. It does not change the kernel CPU setup, process
context switching, linker flags, or global Rust target features.

The new `cpufeat` applet is part of the existing `sunlight-utils` multi-call
binary and is exposed through:

- `/bin/cpufeat`
- `/sunlight-utils/cpufeat`
- `sunlight-utils cpufeat`

## Reported Data

`cpufeat` reads CPUID from userspace and prints:

- CPU vendor
- CPU brand string when extended brand leaves are available
- SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B
- AVX, AVX2, BMI1, BMI2, FMA, F16C, LZCNT, AES
- XSAVE and OSXSAVE
- x86-64-v2 capable: yes/no
- x86-64-v3 capable: yes/no

The applet also reports ABI extras used by the class checks:

- LAHF/SAHF for x86-64-v2
- MOVBE for x86-64-v3
- AVX OS state readiness from XCR0

## Capability Rules

x86-64-v2 is reported as capable when the CPU exposes the v2 instruction set
requirements checked by the applet: SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT,
CMPXCHG16B, and LAHF/SAHF.

x86-64-v3 is reported as capable only when v2 is satisfied and the CPU exposes
AVX, AVX2, BMI1, BMI2, FMA, F16C, LZCNT, and MOVBE, with OS AVX state support.

AVX and AVX2 require more than CPUID instruction bits. The OS must enable
XSAVE/OSXSAVE and preserve the x87, SSE, and YMM state in XCR0 during context
switching. Without that, AVX instructions can fault or corrupt vector state
between processes.

## Current SunlightOS Status

`kernel/src/arch/x86_64/cpu.rs` currently enables x87/SSE state with CR0 and
CR4.OSFXSR/OSXMMEXCPT. It does not enable CR4.OSXSAVE, initialize XCR0, or
switch XSAVE/YMM state for processes.

That means a CPU can show AVX/AVX2 instruction support while SunlightOS still
reports `x86-64-v3 capable: no`, because the operating system side of AVX is not
ready yet. This is intentional for Phase 1.

## Next Work

- Add kernel-side CPUID capture for boot diagnostics.
- Decide whether SunlightOS should require x86-64-v2 as a baseline.
- Add an XSAVE-area layout and per-thread save/restore path before enabling
  CR4.OSXSAVE and XCR0 YMM state.
- Only after the kernel can preserve AVX state, consider enabling v3-targeted
  code paths or compiler target features.
