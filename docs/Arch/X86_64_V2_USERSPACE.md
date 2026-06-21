# x86-64-v2 Userspace Build Mode

Date: 2026-06-21

## Overview

SunlightOS now builds userspace binaries with `-C target-cpu=x86-64-v2` while
keeping the kernel at the conservative x86-64 baseline. This enables better
code generation for userspace services without requiring kernel SIMD support.

## Build Configuration

### Kernel Flags (Conservative)

The kernel remains at x86-64 baseline with soft-float:
- Target: `x86_64-unknown-none` (soft-float, no SSE)
- Flags: `-C link-arg=-Tkernel/src/arch/x86_64/linker.ld -C relocation-model=static`
- No `target-cpu` specified = x86-64 baseline only

### Userspace Flags (x86-64-v2)

All userspace crates build with x86-64-v2 baseline:
- Flags: `-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2`
- Enables: SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B, LAHF/SAHF

### Build Scripts

`tools/build.sh` and `tools/test.sh` define separate flag sets:
- `KERNEL_RUSTFLAGS`: Conservative kernel-only flags
- `SERVICE_RUSTFLAGS`: x86-64-v2 userspace flags
- `TLS_RUSTFLAGS`: Userspace v2 + forced software crypto backends

The workspace `.cargo/config.toml` sets the default target to `x86_64-unknown-none`
with kernel linker flags, but these are overridden by `RUSTFLAGS` env var when
building userspace crates.

## Microarchitecture Levels

### x86-64 (Baseline)
- Required: x87, SSE, SSE2, CMOV, CMPXCHG8B
- Used by: kernel

### x86-64-v2
- Required: v1 + SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B, LAHF/SAHF
- Used by: all userspace binaries

### x86-64-v3
- Required: v2 + AVX, AVX2, BMI1, BMI2, F16C, FMA, LZCNT, MOVBE
- Used by: none (requires kernel XSAVE/YMM context switching)
- Detection: `cpufeat` binary reports capability

## CPU Feature Detection

The `cpufeat` binary (`cpu-utils` crate) provides runtime detection of CPU
capabilities:
- Reads CPUID from userspace
- Reports individual feature flags (SSE3, AVX, AVX2, etc.)
- Reports x86-64-v2 and x86-64-v3 capability
- Checks OS AVX state support (XCR0)

Usage:
```bash
cpufeat
```

Output shows:
- CPU vendor and brand string
- Individual feature flags
- x86-64-v2 capable: yes/no
- x86-64-v3 capable: yes/no (requires OS XSAVE support)

## AVX/v3 Requirements

x86-64-v3 requires more than CPU support. The kernel must:
1. Enable CR4.OSXSAVE
2. Initialize XCR0 to enable x87, SSE, and AVX state bits
3. Allocate XSAVE area per thread (size from CPUID leaf 0xD)
4. Save/restore YMM state on context switch

Current status: SunlightOS kernel enables x87/SSE state (CR0, CR4.OSFXSR) but
does not enable XSAVE or preserve AVX state. `cpufeat` will report "x86-64-v3
capable: no" even on capable CPUs until kernel support is added.

## Verified Userspace Crates

The following crates build and run with x86-64-v2:
- `cpu-utils` (cpufeat)
- `sunshell`
- `sunlight-tty`
- `sunlight-top`
- `sunlight-tui`
- `sunlight-kvctl`
- `sunlight-fetch`
- `helios-note` (Linux-compat, musl target)
- `std-proof` crates (sunlight-sunsay, sunlight-zoxide, sunlight-dict)
- All service daemons (init, vfs, tty, net, timer, sunlightd, etc.)

## Build Verification

1. Kernel check (should have no SSE/AVX instructions):
   ```bash
   objdump -d target/x86_64-unknown-none/debug/sunlight-kernel | grep -E "movdqa|addsubpd"
   # Should return empty
   ```

2. Userspace check (may use v2 instructions):
   ```bash
   file target/x86_64-unknown-none/release/cpufeat
   # Should show: ELF 64-bit LSB executable, x86-64
   ```

3. Full build test:
   ```bash
   ./tools/build.sh
   ./tools/test.sh
   # Should pass Phase 3.0 gate
   ```

## Future Work

- Add kernel XSAVE/YMM context switching for x86-64-v3 support
- Consider making x86-64-v2 a hard requirement (drop v1 support)
- Add runtime feature detection for crypto backends (AES-NI, AVX2)
- Profile v2 vs v1 performance improvements in key userspace services

## References

- psABI x86-64 supplement: https://gitlab.com/x86-psABIs/x86-64-ABI
- Intel SDM Volume 1: Chapter 13 (XSAVE feature set)
- `docs/Arch/PHASE1_X86_64_V2_V3_SUPPORT.md` (detection only)
- `docs/ADDING_A_BINARY.md` (cpufeat wiring guide)
