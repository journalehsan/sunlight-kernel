# powerd v0

`powerd` is the central userspace power profile policy service for SunlightOS.

It owns the selected power profile, computes an effective profile (especially for `Auto`), and exposes a small set of derived policy knobs for other services (cache, prefetch, scheduler, effects, background work, etc.).

## Design goals (v0)

- Keep high-level power policy out of the kernel.
- Provide a stable model and IPC/CLI surface early.
- Make the data model ready for real battery/ACPI, CPU, scheduler, and device consumers.
- Follow existing SunlightOS service patterns (deviced, networkd, resolved).
- Remain small, reliable, and inspectable.
- No regressions to boot, deviced, networkd, resolved, TLS, fetch, input, or virtio.

## Profiles

| Profile      | Meaning                                      |
|--------------|----------------------------------------------|
| Turbo        | Maximum short-term performance, high power.  |
| Performance  | High performance, less extreme than Turbo.   |
| Balanced     | Default normal operating point.              |
| LowPower     | Reduce power while remaining usable.         |
| Stamina      | Maximum battery saving (low battery / tiny devices). |
| Custom       | User-defined policy (v0: conservative defaults; editing is future). |
| Auto         | Let powerd pick a concrete effective profile based on power context. |

## PowerContext (v0)

```rust
pub struct PowerContext {
    pub on_ac: Option<bool>,
    pub battery_percent: Option<u8>,
    pub battery_present: bool,
    pub load_percent: Option<u8>,
    pub user_active: bool,
}
```

In v0:
- Real ACPI/battery data is usually unavailable.
- Unknown fields are `None` / safe defaults.
- `UpdateContext` accepts best-effort updates (future battery service or driver can push here).
- The service never blocks waiting for power data.

## Auto policy (v0)

```rust
fn choose_auto_profile(ctx: &PowerContext) -> PowerProfile {
    if ctx.on_ac == Some(true) {
        if ctx.load_percent.unwrap_or(0) > 70 {
            PowerProfile::Performance
        } else {
            PowerProfile::Balanced
        }
    } else if let Some(battery) = ctx.battery_percent {
        if battery <= 15 {
            PowerProfile::Stamina
        } else if battery <= 30 {
            PowerProfile::LowPower
        } else {
            PowerProfile::Balanced
        }
    } else {
        PowerProfile::Balanced
    }
}
```

## Derived PowerPolicy

`powerd` exposes not only the profile but also policy hints:

```rust
pub struct PowerPolicy {
    pub selected_profile: PowerProfile,
    pub effective_profile: PowerProfile,
    pub cache_mode: CacheMode,           // Minimal | Normal | Aggressive
    pub prefetch_mode: PrefetchMode,     // Off | Light | Normal | Aggressive
    pub effects_mode: EffectsMode,       // Minimal | Normal | Rich
    pub scheduler_bias: SchedulerBias,   // Battery | Balanced | Interactive | Performance
    pub background_work_allowed: bool,
}
```

Suggested v0 mappings (see source for exact table):

- Turbo → cache=aggressive, prefetch=aggressive, effects=rich, scheduler=performance, bg=allowed
- Performance → aggressive/normal cache, normal prefetch, normal/rich effects, performance scheduler, bg=allowed
- Balanced → normal everything, bg=allowed
- LowPower → normal/minimal cache, light prefetch, minimal effects, balanced scheduler, bg=limited
- Stamina → minimal cache, prefetch=off, minimal effects, battery scheduler, bg=limited/disabled

Consumers (future):
- Scheduler / niced bias
- Cache manager
- Prefetch service
- Display / Vortex effects
- Network power saving
- Disk write coalescing (sm)
- Thermal management

None of these consumers are implemented in v0.

## IPC (registered as "powerd")

Core operations (compact register IPC):

- `GET_STATUS`
- `GET_PROFILE`
- `SET_PROFILE <tag>`
- `SET_AUTO`
- `LIST_PROFILES <index>`
- `GET_POLICY`
- `UPDATE_CONTEXT` (packed best-effort context)
- `SET_CUSTOM_POLICY` / `GET_CUSTOM_POLICY` → ERR_UNSUPPORTED in v0

Replies use `PowerdMsg::REPLY`. Errors use `PowerdMsg::ERROR`.

See `ipc/src/lib.rs`: `PowerdMsg`, `PowerProfile`, `PowerPolicy`, `PowerContext`, `CacheMode` etc.

## CLI: powerctl

```
powerctl status
powerctl profiles
powerctl set turbo
powerctl set performance
powerctl set balanced
powerctl set low-power
powerctl set stamina
powerctl set custom
powerctl set auto     # accepted (prefer 'powerctl auto' for clarity)
powerctl auto
powerctl policy
```

Example output:

```
Power:
  selected:  Balanced
  effective: Balanced
  AC:        unknown
  Battery:   unknown

Policy:
  cache:       Normal
  prefetch:    Normal
  effects:     Normal
  scheduler:   Balanced
  background:  allowed
```

For Auto:

```
Power:
  selected:  Auto
  effective: Balanced
```

## What v0 does NOT do

- No real battery or ACPI power source driver integration yet.
- No cpufreq, P-states, C-states, or thermal control.
- No scheduler bias enforcement.
- No cache/prefetch/display/network consumers wired up.
- No persistence of selected profile (in-memory only).
- Custom profile editing is a stub.

## Future work (hooks left in the design)

- Battery / power supply service pushing `UPDATE_CONTEXT`.
- ACPI FADT / battery deviced driver registration (`DriverKind::Power` already exists).
- Scheduler integration via `niced` or direct bias messages.
- CPU feature service for frequency scaling.
- Cache manager, prefetcher, disk coalescing, display effects, network power hints.
- Thermal governor.
- Optional profile persistence via sunlight-kv / sm.

## Validation

- `cargo check -p sunlight-powerd`
- Boot and run all `powerctl` subcommands.
- Verify no regressions in: devicectl, networkctl, resolvectl, ping, fetch, input devices, and stable boot with missing battery data.
