# Thermal Service (`sunlight-thermald`)

First safe, usable Thermal & Cooling subsystem for SunlightOS, initially
focused on the verified ThinkPad T440p path — **monitoring-first** until a
kernel-owned fan-control lease exists.

## Architecture

Three layers:

1. **Sensor / cooling backends** (`services/sunlight-thermald/src/backend.rs`)
   - Read temperatures and fan RPM/mode.
   - Request validated fan levels only when a backend lease is available.
   - Always able to restore firmware-auto.

2. **Thermal policy engine** (`services/sunlight-thermald/src/lib.rs`)
   - Pure deterministic state machine (mock clock / sensors / fan / power sink).
   - Hysteresis, dwell, profile bias, hot-state classification.
   - No UI, no EC ports, no floating-point protocol units.

3. **Service / UI / CLI**
   - `thermald` IPC service (`thermal.*` labels as `ThermaldMsg`).
   - `thermalctl` CLI.
   - Control Panel **Power & Thermal** page.
   - Coordination with `powerd` via temporary thermal constraints.

Emergency hardware/firmware/CPU critical protection is **never** replaced by
userspace.

## Safety boundaries

| # | Invariant | Status |
|---|-----------|--------|
| 1 | Firmware/EC/CPU emergency protection remains enabled | **Honored** — userspace never disables it |
| 2 | Userspace is not the final critical-temp protection | **Honored** |
| 3 | Manual fan only on verified hardware | **Honored** — T440p only, and gated |
| 4 | Unknown laptop → firmware-auto | **Honored** |
| 5 | Failure/crash/suspend → firmware-auto | **Policy + backend hooks present**; full proof requires kernel lease |
| 6 | UI/apps never get raw EC access | **Honored** — only thermald may request fan control |
| 7 | Only thermal service holds fan capability | **Design** — no EC capability exists yet |
| 8 | Manual mode requires lease fallback | **Safety blocker** — see below |
| 9 | Never use ThinkPad `disengaged` | **Honored** — enum has no disengaged |
| 10 | `full-speed` only at hot threshold | **Honored** in policy curve |
| 11 | Invalid config → safe defaults | **Honored** |
| 12 | Monotonic time for decisions | **Honored** |

### Safety blocker: kernel fan lease

There is **no** kernel EC driver that owns a watchdog lease restoring
firmware-auto after `thermald` hangs or is killed.

Therefore:

- `HardwareModel::manual_fan_allowed()` returns **false** even for T440p.
- Production backends report **MonitoringOnly / Unavailable** for writes.
- Managed fan control is **not claimed complete**.
- Unit tests exercise the policy machine with a simulated lease.

When a kernel EC lease lands:

1. Backend `acquire_lease` installs a ~10s timeout owned by the driver.
2. `thermald` renews every ~2.5s.
3. On timeout, driver restores firmware-auto without userspace help.
4. Flip `manual_fan_allowed()` for positively identified T440p only.

## Ownership model

| Owner | Responsibility |
|-------|----------------|
| **Firmware / kernel** | Final emergency throttle/shutdown |
| **`powerd`** | User-selected power modes (Turbo…Stamina + Auto/Custom), requested vs effective mode, applying thermal upper bounds |
| **`thermald`** | Sensors, fan policy, cooling preference, lease requests, publishing thermal constraints to powerd |
| **UI/CLI** | Inspect and set user preferences only |

### Requested vs effective power mode

```
Requested: Turbo          (persistent user choice in powerd)
Effective: Balanced       (intersection with thermal max)
Constraint: ThermalHot
Temperature: 86°C
```

`thermald` never overwrites the persistent requested mode.

API (powerd):

- `SET_THERMAL_CONSTRAINT` — severity, maximum_allowed_mode, reason, source, generation
- `CLEAR_THERMAL_CONSTRAINT` — generation-safe clear
- `GET_POWER_POLICY_STATUS` / `GET_STATUS` — requested, effective, constraints

Generation rules prevent a delayed Clear from removing a newer Hot constraint.
If thermald dies while a Hot constraint is active, powerd **retains** the
constraint (does not jump back to Turbo).

No circular IPC: powerd does not call thermald while applying a constraint.

## IPC protocol (`ThermaldMsg`)

Registered name: **`thermald`**.

| Op | Purpose |
|----|---------|
| `GET_STATUS` | Compact thermal + fan + power snapshot |
| `LIST_SENSORS` | Per-sensor readings (index) |
| `LIST_COOLING` | Cooling devices (index) |
| `GET_PROFILE` / `SET_PROFILE` | Cooling preference (not power mode) |
| `GET_POLICY` | Policy summary |
| `RESET_SAFE_DEFAULTS` | Balanced + firmware-auto |
| `FORCE_FIRMWARE_AUTO` | Explicit auto |
| `PREPARE_SUSPEND` / `RESUME` | Lifecycle (when suspend exists) |

Capability: `thermal-control` resolves `thermald` (like `power-control` → `powerd`).

### Units

| Quantity | Unit |
|----------|------|
| Temperature | signed milli-degrees Celsius (`i32`) |
| Fan speed | RPM (`u32`), when available |
| Fan level | `FanLevel` 0–7 + FullSpeed (8) |
| Time | monotonic milliseconds |

Missing sensors are **never** reported as 0°C (`i32::MIN` sentinel in status).

## Initial T440p Balanced curve

| CPU temperature | Requested fan state |
|-----------------|---------------------|
| below 45°C | level 0 (min/off) |
| 45–49°C | level 1 |
| 50–54°C | level 2 |
| 55–59°C | level 3 |
| 60–64°C | level 4 |
| 65–69°C | level 5 |
| 70–74°C | level 6 |
| 75–79°C | level 7 |
| ≥ 80°C | full-speed |

- Downward hysteresis: **3°C**
- Minimum dwell before decreasing: **10 s**
- Upward transitions: immediate; may skip levels
- Full-speed at ≥80°C is never delayed by dwell
- ~3100 RPM at sustained ~60°C is a **T440p validation reference**, not a
  hard-coded target

## Cooling preferences (not power modes)

| Preference | Intent |
|------------|--------|
| **Balanced** | Verified default curve (Recommended) |
| **Quiet** | +5°C threshold bias (warmer normal), still full-speed at 80°C |
| **Cool** | −5°C bias (earlier cooling) |
| **Performance** | Same fan curve as Balanced; power still constrained when hot |

Hot protection and lease fallback cannot be disabled by any preference.

The five **power** modes remain exclusively in `powerd` / `powerctl`:
Turbo, Performance, Balanced, LowPower, Stamina (+ Auto/Custom).

## Lease / watchdog

| Parameter | Value |
|-----------|-------|
| Lease timeout | 10 s |
| Renew interval | 2.5 s |
| Sample interval | 1 s |

The backend/driver must own the timeout. Emulating it only inside the same
userspace process is **not** accepted for production managed mode.

## Configuration / persistence

Only validated user preferences are intended to persist (cooling profile).
v0 keeps config in memory with safe defaults on boot; invalid blobs are
rejected (`PersistedConfig::validate`).

Boot sequence:

1. Firmware-auto
2. Hardware discovery
3. Load/validate config
4. Read sensors
5. Start lease (when available)
6. Enter managed only if all of the above succeed

## Supported hardware

| Model | Monitoring | Manual fan |
|-------|------------|------------|
| ThinkPad T440p | Prepared (needs sensors/DMI) | **Disabled** until kernel lease + verification |
| ThinkPad T480 | Future / monitoring only | **Disabled** |
| Generic / VMware | Unavailable | **Disabled** |

## CLI

```
thermalctl status
thermalctl sensors
thermalctl fans
thermalctl profile
thermalctl profile set balanced|quiet|cool|performance
thermalctl auto
thermalctl reset-defaults
```

`powerctl` remains the owner of power-mode selection.

## Control Panel

**Power & Thermal** page:

- Five power modes (powerd) with requested vs effective
- Active thermal constraint reason
- CPU temperature, fan mode/level/RPM
- Cooling preference buttons (Balanced recommended)
- Restore Safe Defaults
- Monitoring-only / firmware-auto warnings

## Traced infrastructure (pre-implementation)

| Item | Classification |
|------|----------------|
| ACPI thermal zones | **Missing prerequisite** |
| Temperature sensor APIs | **Missing prerequisite** |
| CPU package/core temps | **Not available** |
| EC / ThinkPad fan control | **Missing prerequisite** / **safety blocker** |
| Fan RPM/level APIs | **Not available** |
| DMI/SMBIOS identity | **Missing prerequisite** |
| powerd five modes + IPC | **Reusable** (extended) |
| Battery/AC real data | **Partial** (context model only) |
| sunlightd supervision | **Reusable** (thermald via init today) |
| Capabilities / IPC auth | **Reusable** (+ ThermalControl) |
| Control Panel pages | **Reusable** |
| CLI patterns (powerctl) | **Reusable** |
| sunlight-kv persistence | **Reusable** (deferred wiring) |
| Suspend/resume service hooks | **Not available** (IPC ops ready) |
| Kernel fan lease | **Safety blocker** |
| Fan mode mutation paths | **None** (already safe) |
| T440p safe control today | **Hardware-specific concern** — not yet |
| VMware testing | Service/UI/CLI/policy only |

## Runtime validation

### Unit tests

```
cargo test -p sunlight-thermald --lib
cargo test -p sunlight-ipc --lib service_capability
```

Policy tests cover upward/downward curve, hysteresis, dwell, sensor failure,
lease expiry, suspend/resume, invalid config, profile hot protection, and
power intersection.

### VMware (expected)

- `thermald` starts, registers, stays in monitoring/unavailable
- `thermalctl status` reports unsupported/missing sensors clearly
- Control Panel Power & Thermal does not crash
- No busy-loop (1 Hz sample + 200 ms IPC timeout)
- Manual fan **not** claimed

### Physical T440p

**Not performed in this change.** Do not claim sustained ~60°C / ~3100 RPM
success until the hardware checklist in the implementation brief is run with a
kernel lease backend.

## Idle resource targets

| Metric | Target |
|--------|--------|
| Sample rate | ~1 Hz |
| IPC | On demand + short timeout poll |
| Histories | None unbounded |
| UI while open | ~1 Hz refresh |
| UI after close | Snapshots dropped |

## Remaining limitations

1. No real temperature sensors or EC fan control.
2. No DMI product identification.
3. No kernel-owned fan lease → managed mode disabled.
4. T480 manual control not verified.
5. Profile persistence not yet wired to sunlight-kv.
6. Suspend framework not present; PREPARE_SUSPEND/RESUME are ready.
7. powerd still does not drive CPU frequency hardware (constraint is policy-level).
8. Unauthorized in-process IPC mutation checks rely on nameserver capability
   mediation (same model as powerd/networkd).
