# CPU & Core Detection — SunlightOS SMP Analysis

**Date:** 2026-06-26
**Status:** Uniprocessor only — SMP not yet implemented

---

## Summary

SunlightOS is a strict uniprocessor kernel. Every CPU/SMP-related field is hardcoded to `1` or exists as an unreferenced dead type. No hardware topology detection runs at boot. This document records the current state and a concrete roadmap for adding SMP support.

---

## Part A: Hardware Detection

### A1. ACPI MADT — Struct defined, never parsed

**File:** `kernel/src/arch/x86_64/acpi.rs:101–108`

```rust
#[repr(C, packed)]
pub struct ACPIMADT {
    pub header: ACPITableHeader,
    pub local_apic_addr: u32,
    pub flags: u32,
    // Followed by variable-length APIC structures
}
```

The helper `find_table_by_signature()` exists and could locate MADT with the signature `b"APIC"`, but it is only ever called for `b"FACP"` (FADT — power management). The `ACPIMADT` struct is entirely unreferenced at runtime: no Local APIC entries (type `0x00`), I/O APIC entries (type `0x01`), or interrupt source overrides (type `0x02`) are ever enumerated.

The ACPI `init()` function calls `parse_fadt()` and `parse_dsdt()` exclusively for shutdown/reset purposes.

### A2. CPUID — Only used for RDRAND detection

**File:** `kernel/src/arch/x86_64/syscall.rs:3700`

```rust
let has_rdrand = core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0;
```

This is the only kernel-side CPUID call. The following topology leaves are absent:

| Leaf | Purpose | Status |
|---|---|---|
| `0x1` EDX bit 28 | HTT — multiple logical processors present | Not probed |
| `0x4` subleaves | Cache topology, reveals core count per package | Not probed |
| `0x0B` | x2APIC extended topology (logical/core/package counts) | Not probed |
| `0x1F` | V2 extended topology (successor to `0x0B`) | Not probed |

A userland binary (`cpu-utils/src/cpufeat.rs`) probes feature flags (SSE/AVX/BMI/x86-64 levels) but not topology.

### A3. Local APIC (LAPIC) — Completely absent

The only LAPIC reference in `kernel/src/` is the `local_apic_addr: u32` field inside the never-parsed `ACPIMADT` struct. There is no:

- Read of MSR `0x1B` (`IA32_APIC_BASE`)
- LAPIC MMIO mapping
- Spurious interrupt vector setup
- LAPIC timer, EOI, or ICR (Interrupt Command Register) writes

The kernel runs on the **legacy 8259 PIC + 8254 PIT** exclusively.

---

## Part B: SMP Startup

**There is zero SMP startup code anywhere in the kernel.**

The word `trampoline` appears in `kernel/src/arch/x86_64/syscall.rs:873–964` only in the context of user-space thread stubs — not CPU AP startup. No SIPI sequence, no 16-bit real-mode trampoline page, no `wake_ap()` function exists.

### Bootloader (Limine) — SmpRequest not declared

**File:** `kernel/src/main.rs:31–42`

```rust
static MEMMAP_REQ:     limine::request::MemmapRequest     = ...;
static HHDM_REQ:       limine::request::HhdmRequest       = ...;
static FB_REQ:         limine::request::FramebufferRequest = ...;
static RSDP_REQ:       limine::request::RsdpRequest       = ...;
static STACK_SIZE_REQ: limine::request::StackSizeRequest  = ...;
// ← limine::request::SmpRequest is ABSENT
```

Without `SmpRequest`, Limine parks all Application Processors in an infinite `hlt` loop permanently. The kernel never receives CPU topology from the bootloader.

The crate `limine = "0.6"` (current `Cargo.toml` version) **does** expose `limine::request::SmpRequest` — it is simply not declared.

When `SmpRequest` is present, Limine:
1. Parses the MADT to find all Local APICs
2. Fills a response with `cpu_count` and an array of `LimineSmpInfo` (LAPIC ID, processor ID, `goto_address` function pointer for each AP entry)

---

## Part C: Scheduler CPU Assumptions

**File:** `kernel/src/sched/mod.rs:257–269`

```rust
pub struct Scheduler {
    pub processes:          Vec<Process>,
    pub ready_queue_high:   VecDeque<usize>,
    pub ready_queue_medium: VecDeque<usize>,
    pub ready_queue_low:    VecDeque<usize>,
    pub current: usize,        // single integer — one process "on CPU"
    pub current_ticks: u64,
    pub global_tick:  u64,
    pub idle_context_rsp: u64,
}
```

`pick_next()` returns one `Option<usize>` from a flat `Vec<Process>`. There is no CPU affinity, no per-CPU run queue, and no IPI-triggered reschedule for remote cores. A single `global_tick` is driven by the BSP timer ISR. The global scheduler is one spinlock-guarded instance — not SMP-safe.

The scheduler module doc comment (line 79) already anticipates SMP in the capacity formula:

```
//!   capacity_delta = interval_ns * online_cpu_count
```

But the implementation always passes `cpu_count = 1` from the telemetry page.

**Global instance** (`sched/mod.rs:1337`):
```rust
pub static SCHEDULER: spin::Mutex<Scheduler> = spin::Mutex::new(Scheduler::new());
```
A single global lock is a severe contention point on SMP and will need to be split per-core.

---

## Part D: Telemetry Integration Points

### Where `cpu_count` is written

**File:** `kernel/src/telemetry.rs:81` — hardcoded at compile time:
```rust
cpu_count: 1,
```

**File:** `kernel/src/telemetry.rs:127–131` — defensive guard in timer ISR:
```rust
// For the current uniprocessor kernel this is 1. When SMP is added,
// this must reflect the number of online CPUs at the time of sampling.
unsafe {
    if TELEMETRY.cpu_count == 0 {
        TELEMETRY.cpu_count = 1;
    }
}
```

`cpu_count` is **never set from hardware** — no MADT walk, no CPUID probe, and no Limine SMP response feeds into it.

### How sunlight-top uses `cpu_count`

**File:** `sunlight-telemetry/src/lib.rs:246–251`

```rust
let cpu_count = if snap.cpu_count > 0 { snap.cpu_count as u64 } else { 1 };
let capacity_delta_ns = interval_ns.saturating_mul(cpu_count);
```

The capacity formula is **already correct for SMP**. Once `cpu_count` reflects real hardware, displayed CPU% will be machine-normalized across all cores with no further changes to sunlight-top's math.

Currently, no `cpu_count` label is shown on screen in `sunlight-top/src/ui/header.rs`.

### sunlight-tasks

`services/sunlight-tasks/src/main.rs:290–295` renders "CPU X% used Y% idle" using the same basis-point values. No `cpu_count` label is displayed there either.

### Procfs / Sysfs

There is no `/proc/cpuinfo` or `/sys/devices/system/cpu/` equivalent anywhere in the kernel VFS.

---

## Part E: Gap Analysis

| Missing piece | Severity | Notes |
|---|---|---|
| `limine::request::SmpRequest` declaration | **Blocker** | One static decl + AP entry fn; Limine handles MADT + SIPI for you |
| MADT walk / APIC enumeration | **Blocker** | `find_table_by_signature(b"APIC")` + type-0 entry walker |
| LAPIC MMIO initialization | **Blocker** | MSR `0x1B` read, map MMIO, spurious vector, EOI; required before any IPI |
| CPUID leaf `0x0B` topology probe | High | Accurate SMT / core / package counts independent of MADT |
| AP trampoline (real-mode page < 1 MB) | High | Per-AP stack allocation, 16-bit stub → protected/long mode → AP entry |
| Per-CPU state structures | High | `PerCpu<T>` via GS-base; per-CPU GDT, IDT, TSS |
| IPI infrastructure | High | ICR writes for reschedule IPIs between cores |
| Per-CPU scheduler run queues | Medium | Split `SCHEDULER` into `[Mutex<Scheduler>; MAX_CPUS]` |
| `TELEMETRY.cpu_count` update from hardware | Low | One-liner once MADT/Limine count is known |
| `cpu_count` label in sunlight-top header | Low | UI string only |
| `/proc/cpuinfo` VFS node | Low | Convenience; not required for correctness |

---

## Part F: Implementation Roadmap

### Step 1 — Passive CPU count (zero risk, ~5 lines)

Add `SmpRequest` to `main.rs`. Limine fills `response().cpus()` with one entry per logical CPU. APs remain halted.

```rust
// kernel/src/main.rs
static SMP_REQ: limine::request::SmpRequest = limine::request::SmpRequest::new();

// inside kmain(), after ACPI init:
let cpu_count = SMP_REQ.response()
    .map(|r| r.cpus().len())
    .unwrap_or(1);
unsafe { telemetry::TELEMETRY.cpu_count = cpu_count as u8; }
```

**Result:** Accurate CPU count in telemetry + sunlight-top capacity math, no SMP risk.

### Step 2 — MADT enumeration (for APIC IDs)

Extend `kernel/src/arch/x86_64/acpi.rs`:

```rust
if let Some(madt_ptr) = find_table_by_signature(b"APIC") {
    walk_madt_entries(madt_ptr);
    // collect: Vec<(apic_id: u8, processor_id: u8, flags: u32)>
}
```

Entry type `0x00` = Local APIC (one per logical CPU). Entry type `0x09` = x2APIC (for systems with > 255 cores).

### Step 3 — LAPIC initialization on BSP

```rust
// Read LAPIC base from MSR
let lapic_base_phys = rdmsr(0x1B) & 0xFFFF_F000;
// Map into HHDM
let lapic_mmio = hhdm_offset + lapic_base_phys;
// Enable LAPIC: set bit 8 in Spurious Interrupt Vector Register (offset 0xF0)
write_lapic(lapic_mmio, 0xF0, 0x1FF);
```

Required before sending any IPI.

### Step 4 — AP trampoline + INIT-SIPI-SIPI

With `SmpRequest` (from Step 1), Limine can do this automatically — just write a function pointer to each AP's `goto_address` field:

```rust
for cpu in smp_response.cpus() {
    if cpu.lapic_id != bsp_lapic_id {
        cpu.goto_address.write(ap_entry as u64);
    }
}

extern "C" fn ap_entry(info: &LimineSmpInfo) -> ! {
    // load per-AP GDT/IDT/TSS
    // init per-AP LAPIC
    // signal ready via AtomicBool
    // enter per-AP scheduler loop
    loop { x86_64::instructions::hlt(); }
}
```

Manual approach (without Limine delegation): allocate a 4 KB page in the first 1 MB, write a 16-bit real-mode stub that sets CR0/CR3/CR4, switches to long mode, then jumps to `ap_entry`. Send INIT IPI → 10 ms delay → SIPI → 200 µs delay → SIPI again.

### Step 5 — Per-CPU scheduler

```rust
// Replace single global scheduler with per-CPU array
pub static CPU_SCHEDULERS: [Mutex<Scheduler>; MAX_CPUS] = ...;

// Each CPU reads its own scheduler index from GS-base
fn current_scheduler() -> &'static Mutex<Scheduler> {
    let cpu_id = read_gs_base_cpu_id();
    &CPU_SCHEDULERS[cpu_id]
}
```

`pick_next()` stays unchanged per-core. Work-stealing between cores can be added in a follow-up.

### Step 6 — Telemetry labels

Once Step 1 is done, `cpu_count` is accurate with no further work. Add a display label in `sunlight-top/src/ui/header.rs`:

```
CPUs: 4   CPU 34% used  66% idle
```

Optionally expose `/proc/cpuinfo` through the VFS for compatibility with Linux tools compiled for Helios.

---

## Key File Reference

| File | Relevance |
|---|---|
| `kernel/src/main.rs:31–42` | Limine request declarations — add `SmpRequest` here |
| `kernel/src/arch/x86_64/acpi.rs:101–108` | Dead `ACPIMADT` struct — MADT walker goes here |
| `kernel/src/arch/x86_64/acpi.rs:464` | `find_table_by_signature()` — call with `b"APIC"` for MADT |
| `kernel/src/arch/x86_64/syscall.rs:3700` | Only existing CPUID call — add topology leaves nearby |
| `kernel/src/sched/mod.rs:257–269` | `Scheduler` struct — needs per-CPU split for SMP |
| `kernel/src/sched/mod.rs:1337` | Global `SCHEDULER` spinlock — replace with per-CPU array |
| `kernel/src/telemetry.rs:81` | `cpu_count: 1` hardcode — update from SmpRequest |
| `kernel/src/telemetry.rs:127–131` | Defensive `cpu_count` guard in timer ISR |
| `sunlight-telemetry/src/lib.rs:246–251` | Capacity formula — already SMP-correct |
| `sunlight-top/src/ui/header.rs` | CPU header display — add `CPUs: N` label |
| `services/sunlight-tasks/src/main.rs:290–295` | CPU string rendering |
