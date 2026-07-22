# `sunlight-libc` Time, Clock, and Sleep Audit

**Audit date:** July 22, 2026  
**Scope:** existing SunlightOS time interfaces only.  No new POSIX time API,
timezone database work, RTC redesign, NTP synchronization, or stdio surface.

## Current exported libc surface

`sunlight-libc` exports only `clock_gettime`.  It has no `time`,
`gettimeofday`, `clock_getres`, `nanosleep`, `clock_nanosleep`, `sleep`, or
`usleep` symbol, declaration, header, feature, or current libc caller.  This
audit deliberately adds none of them.

The production C ABI remains:

```c
int clock_gettime(int clockid, struct timespec *tp);
```

`Timespec` is two signed 64-bit fields (`tv_sec`, `tv_nsec`).  A null output
pointer fails locally with `EFAULT`; unsupported IDs, an out-of-range seconds
value, and unnormalised nanoseconds fail with `EINVAL`.  The output is written
only after a complete timestamp has been validated, and success does not clear
`errno`.  A kernel-source failure (including impossible realtime arithmetic)
uses the existing generic syscall mapping, `EIO`.

## Supported domains and path

| Clock ID | Semantics | Source and precision |
| --- | --- | --- |
| `CLOCK_REALTIME` (0) | UTC Unix calendar timestamp; not for deadlines | CMOS RTC sampled once during boot, then boot epoch plus BSP ticks; one-second precision (`tv_nsec = 0`) |
| `CLOCK_MONOTONIC` (1) | Non-decreasing time from the boot reference point; not a calendar timestamp | BSP-only global timekeeper tick, 100 Hz / 10 ms resolution |

The complete paths are:

```text
clock_gettime -> libc private temporary -> syscall 88 ->
  realtime: RTC boot epoch + global tick advancement -> validated timespec
  monotonic: BSP global tick counter -> nanoseconds -> validated timespec
```

The RTC stores UTC, not local time.  `timed` relays that UTC timestamp;
`timezone_service` owns the UTC-to-local offset and calendar conversion; locale
and weekday labels are presentation-layer work.

The kernel still uses a calibrated TSC internally for scheduler accounting.
It does not establish cross-core TSC synchronization, so public monotonic
clock reads now use the canonical BSP timekeeper instead.  The tick counter
saturates rather than wrapping.  Relative IPC deadlines use the same monotonic
clock and reject overflow rather than silently becoming a far-future deadline.

Realtime presently has no administrative adjustment or NTP update interface.
It is initialized from the RTC (validated only for 2024--2040, otherwise a
documented 2026-07-08 UTC fallback) and advances monotonically during one boot.
Consequently callers must still treat it as wall time, not as a timeout source.

## Consumers and boundary findings

- Silicon Echoes, shell/UI events, telemetry, save/service timeouts, and IPC
  deadline paths use `monotonic_millis`; typewriter pacing uses bounded elapsed
  deadlines rather than one timer per character.
- TLS consumes `get_time_utc` as a Unix UTC epoch, never monotonic time.  It
  rejects the native error sentinel and an uninitialized zero source rather
  than treating either as January 1, 1970; the present ABI still cannot
  distinguish an RTC fallback from a trusted synchronized clock.
- DNS upstream timeout code used realtime and wrapping addition.  It now uses
  a checked monotonic millisecond deadline.
- The reported fixed weekday was not in libc, RTC, timezone conversion, or
  locale mapping.  Vortex Shell passed `weekday_iso = 0` to the locale helper
  for both its center clock and long-date tooltip; that helper intentionally
  maps zero to Sunday.  Shell now derives ISO weekday from the displayed local
  date.  Sunday-first calendar-grid indexing remains separate from ISO weekday
  names.

## Host test isolation and verification

`tools/test-time-proof.sh` includes only `sys`, `errno`, and `time` directly
in a host test binary.  `clock_gettime` is intentionally not exported under
`cfg(test)`, so the host Rust runtime retains its own clock symbol.  The proof
uses injected raw results to cover supported and invalid IDs, out-of-range
seconds, invalid nanoseconds, and syscall error propagation without real
delays.  The same script runs the isolated Vortex Shell weekday proof against
known Thursday, Wednesday, and Sunday dates.

Target/QEMU verification remains the existing boot gate.  It demonstrates RTC
initialization and 100 Hz timer operation, but does not yet provide a dedicated
clock syscall or sleep test binary.  No libc sleep interface exists to test.

## Explicitly deferred

- Full timezone/locale or Persian calendar presentation work.
- RTC driver redesign, century-register support, NTP synchronization, and
  time-validity/certification policy.
- New POSIX time and sleep functions, including a precise interruption errno
  ABI.
- Dedicated bare-metal clock/sleep tolerance tests.
