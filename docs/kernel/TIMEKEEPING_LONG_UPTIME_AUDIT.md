# Long-uptime timekeeping audit

## Result

The first incorrect authoritative layer was the PIT-based TSC calibration in
`kernel/src/arch/x86_64/interrupts.rs::calibrate_tsc_from_pit`.

Channel 0 was programmed in PIT mode 3 (`0x36`) but calibrated as though each
visible counter decrement represented one PIT input clock. In mode 3 the
visible countdown advances in steps of two. The wrap calculation also used a
65,536-count modulus even though channel 0 was reloaded with 11,932. The
calculated TSC frequency therefore did not represent real elapsed time. That
frequency was then used both to calibrate the LAPIC periodic timer and to
produce `now_ns()`, so the old drift warning compared two values derived from
the same bad calibration and could not identify the error.

The permanent fix programs PIT channel 0 in mode 2 (`0x34`), uses the actual
11,932-count reload for wrap arithmetic, and checks the frequency
multiplication. The BSP timekeeper now derives the exported global tick count
from calibrated elapsed nanoseconds instead of counting timer callbacks.
Duplicate callbacks at one timestamp are idempotent, delayed callbacks catch
up, and AP callbacks are rejected at runtime.

The previous date/midnight work was incomplete because it hardened RTC decode
and presentation behavior but did not repair the oscillator calibration and
global clock-ownership layer beneath realtime.

## Authoritative model

The post-fix ownership model is:

```text
PIT channel 0, mode 2, 1,193,182 Hz reference
  -> checked TSC frequency calibration
  -> LAPIC periodic timer calibration
  -> BSP timer callback
  -> elapsed TSC nanoseconds / 10 ms
  -> one global monotonic tick sequence

CMOS RTC, one stable boot read, interpreted as UTC
  -> atomic realtime base: (Unix UTC seconds, monotonic base ticks)
  -> current realtime UTC = base UTC + monotonic tick delta
  -> GetTimeUtc / clock_gettime(CLOCK_REALTIME)
  -> timezone_service applies one local offset
  -> shared GET_LOCAL_TIME wire snapshot, including weekday
  -> Vortex Shell renders the snapshot
```

Natural progression must satisfy:

```text
delta(realtime UTC) == delta(monotonic)
```

Only `set_unix_time` may change the realtime base. It does not modify global
ticks. Timezone changes modify only conversion configuration.

## 1. Hardware RTC acquisition

`kernel/src/arch/x86_64/rtc.rs` reads CMOS registers `0x00`, `0x02`, `0x04`,
`0x07`, `0x08`, `0x09`, status A, and status B. The ACPI FADT century register
is used when present.

- Update-In-Progress is polled with a bounded spin.
- Two complete snapshots are taken outside update windows and must match.
- Status B selects BCD/binary and 12/24-hour decode.
- The 12-hour PM bit handles midnight, noon, and afternoon explicitly.
- Invalid BCD nibbles and invalid field ranges fail closed.
- Without an ACPI century register, years 70-99 map to 1970-1999 and 00-69 map
  to 2000-2069.
- Gregorian leap-year and days-per-month rules are validated before Unix
  conversion.
- CMOS is read only during `rtc::init`; natural runtime progression never
  rereads RTC.
- Kernel policy is UTC CMOS. VMware must expose UTC, including
  `rtc.diffFromUTC = "0"` where required. No VMware-specific compensation is
  applied to physical hardware.

Boot diagnostics expose raw fields, century source, encoding modes, decoded
UTC, Unix epoch, and monotonic baseline.

## 2. Kernel monotonic clock

The physical reference path is PIT -> TSC -> LAPIC.

- PIT input frequency: 1,193,182 Hz.
- PIT reload: 11,932.
- PIT mode after this fix: mode 2 rate generator.
- LAPIC target frequency: 100 Hz.
- Exported resolution: 10 ms.
- Counters and conversions use `u64` or `u128`; no floating point is used.
- Multiplication and wall-time addition are checked; exported tick arithmetic
  saturates rather than wrapping.
- Only CPU 0 is the global timekeeper.
- AP LAPIC interrupts continue local scheduling but cannot advance global time.
- Scheduler tick accounting does not independently advance wall time.
- Timer IPC observes the global sequence; it is not another clock owner.

Before this fix, every BSP callback incremented global time once. After this
fix, a calibrated callback publishes `elapsed_ns / NS_PER_TICK` with monotonic
`fetch-max` semantics. If TSC calibration is unavailable, the bounded fallback
counts BSP interrupts only.

## 3. Kernel realtime clock

Realtime is not a mutable calendar. It is:

```text
realtime_base_unix + (global_ticks - realtime_base_ticks) / TICK_HZ
```

The base pair, validity, and last update owner are protected by a sequence
counter. Readers cannot observe a new Unix base with an old tick base during
an NTP step. Owners currently reported are `boot-rtc` and `ntp-step`.

No timer handler mutates calendar fields. No periodic RTC reconciliation exists.

## 4. IPC, libc, and compatibility APIs

- `GetTimeUtc` returns UTC Unix seconds as `u64`.
- `MonotonicMs` returns monotonic milliseconds.
- `SetTimeUtc` accepts UTC Unix seconds and is restricted to `timed`.
- `clock_gettime(CLOCK_REALTIME)` exposes UTC seconds with current one-second
  realtime resolution.
- `clock_gettime(CLOCK_MONOTONIC)` exposes monotonic time independently.
- Error sentinels and signed conversion limits are validated by the libc proof.
- `TzMsg::GET_LOCAL_TIME` uses a shared encoder/decoder in
  `sunlight-tz/src/wire.rs`; services and clients no longer duplicate packing.
- The reply carries year, month, day, hour, minute, second, signed UTC offset,
  DST state, abbreviation, and ISO weekday.

The audit found no second boot-epoch addition, tick-to-second addition, or
legacy epoch in the active panel path.

## 5. Timezone conversion

UTC remains authoritative in the kernel and `timed`.

`timezone_service` is the only UTC-to-local conversion point. It obtains
`GetTimeUtc`, applies the active offset once using signed integer seconds, and
returns local civil fields. `+03:30`, whole-hour positive offsets, negative
offsets, and negative half-hour offsets are covered.

Changing `/etc/localtime` changes future presentation only. The notification to
`timed` selects an NTP pool region and does not rewrite UTC.

The current source does not apply the timezone twice.

## 6. Civil calendar conversion

`sunlight-tz/src/offset.rs` performs Unix-day decomposition using integer
Gregorian arithmetic. Tests cover:

- July 31 -> August 1
- August 31 -> September 1
- December 31 -> January 1
- February 28 -> March 1 in a non-leap year
- February 28 -> February 29 in a leap year
- February 29 -> March 1
- weekday progression across each boundary

The displayed weekday is computed by `timezone_service` from the same local
date snapshot. It is no longer independently recomputed for the top-panel
clock. Vortex retains separate weekday math only for its Sunday-first calendar
grid, not for advancing or labeling the live clock.

## 7. Vortex Shell

`query_local_full` performs a fresh bounded IPC query to `timezone_service`.
The returned snapshot includes the service-computed weekday. Vortex caches the
latest snapshot for drawing but does not increment day, date, weekday, or time.

Session or panel restart creates another consumer, not another time owner.
Repeated refresh registration cannot change monotonic or realtime progression.
The lock presenter uses the same query and ignores only the weekday field it
does not display.

## 8. Suspend, resume, and adjustment

SunlightOS currently implements ACPI S5 shutdown but has no completed S3
suspend/resume clock-reconciliation path. Therefore no hidden RTC resume owner
was found. A future suspend implementation must rebaseline realtime explicitly
without changing monotonic semantics and must not introduce periodic RTC reads.

NTP steps atomically replace the realtime base and report `ntp-step`. Forward
and backward steps leave uptime unchanged. Timezone changes leave both
monotonic and UTC realtime unchanged.

## Diagnostics

Boot output includes:

- PIT mode, input frequency, reload, start/end counts, elapsed counts, TSC
  delta, and calibrated TSC Hz
- LAPIC frequency and initial count
- raw RTC fields and decode modes
- decoded UTC and Unix epoch

At 1 minute, 1 hour, 6 hours, 24 hours, and 132 hours 17 minutes, the BSP emits
one bounded kernel checkpoint containing CPU, monotonic nanoseconds, global
ticks, realtime UTC, realtime base, and last update owner.

On the first local-time request after each checkpoint,
`timezone_service` emits UTC, monotonic milliseconds, zone, offset, local civil
time, and weekday. There is no per-tick logging.

These two lines identify whether the first divergence is hardware calibration,
kernel realtime, or UTC-to-local conversion.

## Deterministic verification

`tools/test-time-proof.sh` covers:

- PIT mode-2 calibration and reload-aware wrap math
- the demonstrated mode-3 factor-of-two defect
- BSP-only and duplicate-callback-safe global advancement
- 1 second, 1 minute, 24 hours, and 132 hours 17 minutes
- many small steps versus one large step
- realtime/monotonic delta equality
- forward and backward realtime steps
- repeated reads at midnight
- RTC BCD/binary, 12/24-hour, PM, century, leap, and range validation
- timezone offsets and midnight crossing
- calendar and weekday boundaries
- shared timezone IPC wire encoding
- exact Vortex strings for Friday, July 31, 2026 and Saturday, August 1, 2026

The Phase 3.0 QEMU gate additionally requires mode-2 PIT calibration, decoded
RTC UTC, and timezone-service boot diagnostics before the normal VFS milestone.

The before/after QEMU calibration values provide direct evidence of the defect:

- pre-fix mode-3 runs reported about 1.24 GHz TSC and a LAPIC initial count near
  312,000;
- the post-fix mode-2 run reported 2,476,169,360 Hz TSC and LAPIC initial count
  623,919.

Both post-fix values are almost exactly twice the old values, matching the
mode-3 visible decrement-by-two error. The post-fix boot decoded
`2026-07-31T18:02:43Z`; the timezone service returned
`2026-07-31T18:02:44`, Friday, for UTC.

## Hardware interpretation

The mode-3 calibration defect is confirmed in the production hardware path and
is fixed independently of VMware RTC policy. It can make interrupt-count-based
monotonic and realtime advance at the wrong rate while the old self-referential
warning remains quiet.

The reported T440p observations are approximate: “Monday to Friday,
132 hours,” and “about five and a half days” are not mutually exact. Because
uptime and realtime consumed the same global ticks, a precise pre-fix external
timestamp/uptime series would be required to apportion the entire observed
three-day error between rate error and initial RTC basis.

A post-fix T440p boot must still capture the new PIT and RTC diagnostics and
compare them with trusted UTC. This implementation supports the hardware path
and deterministic long-duration proof, but the audit does not claim a
post-fix physical-machine observation that has not yet occurred.
