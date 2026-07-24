# Hardware Identity and Thermal Telemetry (Phase 1)

Read-only foundation for SMBIOS/DMI product identity and Intel CPU digital
thermal sensor (DTS) telemetry on SunlightOS.

**This phase does not implement fan control, EC writes, MSR writes, custom fan
curves, or automatic live-hardware power clamping.**

## Specification revisions used

| Spec | Revision | Use |
|------|----------|-----|
| DMTF DSP0134 SMBIOS Specification | **3.9.0** (2025-07-07) | Entry points, structure headers, Types 0/1/2/4, string areas |
| UEFI Specification | SMBIOS configuration-table GUIDs (via Limine) | Preferred discovery path on UEFI |
| Intel SDM Volume 3 | Thermal-management chapter (current public SDM) | DTS validity, package thermal, temperature calculation |
| Intel SDM Volume 4 | MSR tables | `IA32_THERM_STATUS` (0x19C), `IA32_TEMPERATURE_TARGET` (0x1A2), `IA32_PACKAGE_THERM_STATUS` (0x1B1) |
| Intel SDM Volume 2A | CPUID | Vendor, family/model decode, leaf 06H thermal features |

GPL-licensed `dmidecode`, Linux `hwmon`/`coretemp`/`thinkpad-acpi` were **not**
copied. They may be used only for behavioral comparison where license policy
permits.

## Boot / SMBIOS discovery trace

| # | Finding | Classification |
|---|---------|----------------|
| 1 | UEFI boot entry uses Limine (firmware-neutral with BIOS) | reusable infrastructure |
| 2 | Limine `SmbiosRequest` provides `entry_32` / `entry_64` pointers | reusable infrastructure |
| 3 | SMBIOS 2.x and 3.x entry points are preserved by Limine when firmware provides them | reusable infrastructure |
| 4 | Kernel maps firmware physical memory via HHDM after validation | reusable infrastructure |
| 5 | Legacy BIOS uses the same Limine path (no userspace physical scan) | already correct |
| 6 | No prior boot-info ABI carried SMBIOS; Limine request is the ABI | missing prerequisite → **fixed** |
| 7 | PMM/VMM HHDM ownership is boot-lifetime | already correct |
| 8 | CPUID wrappers exist (`__cpuid`); family/model decode added | reusable + extended |
| 9 | MSR reads use `x86_64::Msr`; no general userspace MSR API | already correct |
| 10 | Invalid RDMSR has **no** recoverable #GP path (GPF kills process / kernel fault) | safety blocker → **strict allowlist, no speculative probe** |
| 11 | SMP: Limine MP + per-core timer; topology leaf 0xB not fully wired | architecture concern |
| 12 | Per-core MSR read: each CPU samples its own `IA32_THERM_STATUS` on timer | design choice (no remote RDMSR) |
| 13 | deviced inventory is PCI/PS2 oriented | reusable for inventory, not sensors |
| 14 | `thermal-control` capability → `thermald` | already correct |
| 15 | thermald mock/null backends existed | reusable; kernel DTS backend added |
| 16 | thermalctl + Power & Thermal UI existed | extended |
| 17 | powerd thermal constraints with generation | audited; Auto/Custom fixed |
| 18 | Serial/UUID must not appear in public logs/IPC | privacy concern → enforced |

### Discovery algorithm

1. Limine `SmbiosRequest` response → 32-bit and/or 64-bit entry-point addresses.
2. Validate anchors, length, checksum(s), version, table address, length, overflow.
3. Prefer validated SMBIOS **3.x** when both entry points are valid.
4. Map table via HHDM only within `MAX_TABLE_BYTES` (64 KiB).
5. Parse Types 0, 1, 2, 4 with bounded string handling.
6. Store **public** identity in-kernel; privileged serial/UUID never leave the
   privileged store and are not logged.

## Privacy boundary

| Field | Public identity | Boot log | Unprivileged default | Crash/telemetry |
|-------|-----------------|----------|----------------------|-----------------|
| Manufacturer / product / version | Yes | Yes | Via gated syscall | No auto-include |
| Board manufacturer/product | Yes | Optional | Via gated syscall | No |
| BIOS vendor/version | Yes | Yes | Via gated syscall | No |
| Serial number | **No** | **No** | **No** | **No** |
| UUID | **No** | **No** | **No** | **No** |

Thermald needs product-family identity, not device serial numbers.

## Sensor ABI

Unit: **signed milli-degrees Celsius** (`i32`).

Statuses: `Valid`, `Unavailable`, `Unsupported`, `Stale`, `Invalid`, `HardwareError`.

Rules:

- Unavailable is **never** converted to 0°C for consumers (`temp_milli_c()` → `None`).
- Sensor IDs are stable for a boot (`class << 16 | index`), not kernel pointers.
- Interface is read-only; no arbitrary MSR numbers exposed to userspace.
- Kernel syscalls `SystemIdentity` (133) and `ThermalSensors` (134) are gated by
  process name (`thermald`, `thermalctl`, `control-panel`, `deviced`, …).

## Thermal state reporting

| State | When reported |
|-------|----------------|
| `Unavailable` | No valid controlling sensor (missing, stale, unsupported, unknown) |
| `Normal` | Valid fresh controlling temp &lt; Warm threshold |
| `Warm` / `Hot` / `Critical` | Valid fresh controlling temp at/above thresholds |

`Normal` is **never** reported without at least one valid, fresh controlling
temperature. Missing sensors use `Unavailable` (not a fabricated Normal).

## Intel DTS backend

### Feature / model gate (before any thermal MSR)

1. Vendor `GenuineIntel`.
2. CPUID max leaf ≥ 6.
3. CPUID.06H:EAX bit 0 (digital temperature sensor).
4. Display family/model from base + extended fields.
5. **Strict allowlist** (Haswell only for this phase):
   - Family 6, models **`0x3C`**, **`0x45`**, **`0x46`** (Haswell).
   - **`0x3E` is Ivy Bridge-E/EN/EP — not allowlisted** (not a T440p CPU;
     not independently justified for this phase).
6. Unknown model → `Unsupported`, **no RDMSR**.
7. AMD → `Unsupported`, **no RDMSR**.

### Allowlisted models and official MSR evidence

| Display model | Microarchitecture | MSRs | Official evidence |
|---------------|-------------------|------|-------------------|
| **0x3C** | Haswell (client) | 0x19C, 0x1A2, 0x1B1* | Intel SDM Vol. 4 **06_3CH** tables list thermal MSRs; Vol. 3 thermal chapter defines DTS readout/valid bit; `IA32_THERM_STATUS` if CPUID.06H:EAX[0]=1; package MSR if CPUID.06H:EAX[6]=1 |
| **0x45** | Haswell ULT | 0x19C, 0x1A2, 0x1B1* | SDM Vol. 4 **06_45H**; same Vol. 3 / CPUID.06H architectural gates |
| **0x46** | Haswell H | 0x19C, 0x1A2, 0x1B1* | SDM Vol. 4 **06_46H**; same Vol. 3 / CPUID.06H architectural gates |
| 0x3E (removed) | Ivy Bridge-E/EN/EP | — | Documented only for naming accuracy; **no RDMSR** |

\* `IA32_PACKAGE_THERM_STATUS` (0x1B1) only when CPUID.06H:EAX[6]=1 at runtime.

### MSRs read (justification)

| MSR | Address | Why |
|-----|---------|-----|
| `IA32_TEMPERATURE_TARGET` | 0x1A2 | TjMax / temperature target (bits 23:16); Vol. 4 model tables for 06_3CH/45H/46H |
| `IA32_THERM_STATUS` | 0x19C | Per-core digital readout (bits 22:16), valid bit 31; if CPUID.06H:EAX[0]=1 |
| `IA32_PACKAGE_THERM_STATUS` | 0x1B1 | Package digital readout when CPUID.06H:EAX[6]=1 and model allowlisted |

**No WRMSR** of these or any EC/fan/power-limit register is implemented.

### Temperature formula

When thermal-status valid bit is set and TjMax is sane (60–120°C):

```
absolute_°C = temperature_target − digital_readout
milli_°C    = absolute_°C × 1000
```

Checked arithmetic; reserved bits ignored via field masks; result sanity-checked
to [−40°C, 125°C].

### Per-core sampling

Each logical CPU samples its own `IA32_THERM_STATUS` from the LAPIC timer path
(~1 Hz). Topology leaf 0xB is not yet reliable → sensors labeled **logical CPU**
rather than asserting physical-core identity.

Package sample from BSP when package thermal is supported.

### VMware

May report SMBIOS identity without physical DTS. Sensors stay **Unavailable**,
never fabricated 0°C.

## Thermald integration

- Discovers kernel sensors + public identity.
- Controlling temperature = max valid CPU reading.
- If package sensor missing, label as maximum core temperature (not package).
- Fan: **Firmware Auto**; managed control disabled (no EC lease).
- **Live power constraints from real DTS are disabled** until physical T440p
  validation (`power_constraints_allowed() == false` on kernel backend).
- Mock backend still exercises powerd constraint paths in unit tests.

## Powerd semantic audit

| Rule | Status |
|------|--------|
| Turbo…Stamina ordered concrete modes | OK |
| Auto resolved before thermal ceiling | Fixed / verified |
| Custom not compared by discriminant | Fixed / verified |
| Custom mapped via safe Balanced | OK |
| Generation-safe clear | Already correct |
| RequestedMode persistent | Already correct |
| EffectiveMode reflects constraints | Already correct |

## Supported CPU models (Phase 1)

| Model | DTS | Notes |
|-------|-----|-------|
| Intel family 6, models **0x3C / 0x45 / 0x46** (Haswell) | Yes if CPUID DTS | T440p-class |
| Intel family 6, model **0x3E** (Ivy Bridge-E/EN/EP) | **No** | Named for docs only; removed from allowlist |
| Other Intel | Unsupported | No speculative MSR |
| AMD | Unsupported | |
| T480 (Kaby/Coffee) | Architecture only | Not enabled until observed CPUID + SMBIOS |

## Physical T440p validation (2026-07-24 attempt)

### Host environment (where this validation was attempted)

| Field | Value |
|-------|--------|
| Machine available | **ThinkPad T14 Gen 3** (not T440p) |
| Linux product_name | `21AHS1QM00` |
| Linux product_version | `ThinkPad T14 Gen 3` |
| sys_vendor | `LENOVO` |
| CPU | 12th Gen Intel Core i7-1260P |
| CPUID family/model | family **6**, model **154** (0x9A, Alder Lake) |
| Serial/UUID | **not collected** |

**Conclusion:** First physical T440p validation **could not be completed** on this
host. The workstation is a T14 Gen 3 / Alder Lake system. Model 0x9A is outside
the Haswell allowlist; SunlightOS correctly must report DTS **Unsupported** on
this CPU (no speculative RDMSR).

### Sanitized T440p SMBIOS identity

| Field | Status |
|-------|--------|
| manufacturer | **Pending** — record only on physical T440p |
| product_name | **Pending** — exact string only after observation |
| product_version | **Pending** |
| board manufacturer/product | **Pending** |
| Positive `ThinkPadT440p` ID | **Not claimed** (no exact allowlist match performed) |

Do not guess Lenovo product strings. Exact-match allowlist only after observation.

### Observed CPUID on validation host (not T440p)

```
GenuineIntel  family=6  model=154 (0x9A)  stepping=3
Model name: 12th Gen Intel(R) Core(TM) i7-1260P
```

### Linux temperature snapshot on validation host (T14 Gen 3, idle-ish)

Recorded from `coretemp` / `thinkpad` hwmon for environmental context only —
**not** a T440p baseline:

| Sensor | Approx °C |
|--------|-----------|
| Package id 0 | ~68 |
| Core range | ~58–67 |
| thinkpad CPU | ~59 |

Fan behavior (observation only): firmware/ACPI controlled under Linux; no
manual intervention performed.

### SunlightOS on T440p

| Check | Result |
|-------|--------|
| `thermalctl identity` | **Not run on T440p** (hardware absent) |
| `thermalctl sensors` | **Not run on T440p** |
| `thermalctl status` | **Not run on T440p** |
| `powerctl status` | **Not run on T440p** |
| Exact SMBIOS strings | **Pending** |
| Positive T440p ID | **Pending** |
| TjMax / DTS temps | **Pending** |
| No SMT false physical cores | Design: label logical CPU until topology reliable |
| No 0°C transitions | Unit-tested; hardware pending |
| Fan Firmware Auto | Policy enforced in code |
| No live DTS power constraint | `power_constraints_allowed() == false` |
| 15–30 min soak | **Not run** (no T440p) |

### Linux vs SunlightOS temperature comparison (T440p)

**Not available** — requires same T440p at idle and controlled load under both OSes.

### Idle resource usage / poll measurement

| Metric | Measurement / change |
|--------|----------------------|
| Kernel DTS sample | ~1 Hz per logical CPU (timer / 100) |
| Prior thermald IPC wait | 200 ms → ~5 wakes/s with 1 sample/s (**5:1** idle wakes) |
| After measurement | `ipc_recv_timeout(ep, SAMPLE_INTERVAL_MS)` = **1000 ms** |
| Rationale | Same-generation polling: no new sensor data between 1 Hz samples; 5 Hz wake was pure overhead |
| Boot log | `[THERMALD] idle-cost 10s: wakes=… samples=… idle_timeouts=… wait_ms=1000` |

### Supported sensor set (software)

| Sensor | When present |
|--------|----------------|
| CPU package | Allowlisted Haswell + CPUID PTM + valid package MSR |
| Logical CPU N | Allowlisted Haswell + valid per-core THERM_STATUS on that CPU |
| Fan RPM | **Not implemented** |
| EC temps | **Not implemented** |

### Remaining blockers (read-only fan RPM + EC lease)

1. Physical T440p machine for SMBIOS string capture and DTS comparison.
2. Exact product allowlist entry after observation (no substring matching).
3. Kernel EC lease/watchdog outside thermald (managed fan still blocked).
4. Fan RPM read path (read-only first).
5. Explicit review after T440p temp validation before live power constraints.
6. Optional: recoverable #GP for RDMSR (allowlist covers safety today).
7. Topology leaf 0xB for physical-core vs SMT labeling.

## EC / fan control remains disabled

Reasons:

1. No kernel-owned EC lease/watchdog restoring firmware-auto.
2. No verified EC register map for SunlightOS.
3. Manual fan must not activate without positive product allowlist **and** lease.
4. Physical T440p identity not yet recorded on this validation pass.

## Next phase prerequisites (RPM + managed fan)

1. Boot SunlightOS **on a physical T440p**.
2. Record sanitized SMBIOS + CPUID (no serial/UUID).
3. Compare Linux vs SunlightOS idle and controlled-load temperatures.
4. Kernel EC backend with lease timeout outside thermald process.
5. Explicit review to enable live power constraints (still no fan writes until lease).
6. Fan RPM read path (still read-only initially).

## Files (primary)

| Path | Role |
|------|------|
| `sunlight-smbios/` | Bounded SMBIOS parser + public identity |
| `sunlight-sensors/` | Sensor model + Intel DTS pure helpers |
| `kernel/src/smbios.rs` | Limine discovery, HHDM map, identity store |
| `kernel/src/thermal_hw.rs` | Allowlisted DTS sampling |
| `ipc/src/lib.rs` | Syscalls 133/134, records, ThermaldMsg::GET_IDENTITY |
| `services/sunlight-thermald/` | Backend, service, thermalctl |
| `services/sunlight-powerd/` | Auto/Custom ceiling audit |
| `services/sunlight-control-panel/src/power_thermal.rs` | UI |
| `docs/THERMAL_SERVICE.md` | Service doc (updated) |
