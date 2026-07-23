# Wall-clock soak audit

## Classification

The root-cause class is a **confirmed UTC/local-time defect**. A VMware
reproduction on a `+03:30` host showed raw CMOS at host-local `20:58` while
trusted UTC was `17:28`. The kernel interpreted `20:58` as UTC. Configuring
`Asia/Tehran` would therefore add another `+03:30`, yielding `00:28` on the
following date. Setting `rtc.diffFromUTC = "0"` made raw CMOS match trusted UTC
on the next VMware boot.

This is the exact mechanism implied by the T440p screenshots: a local-basis RTC
was treated as UTC, then the configured timezone was applied once to an already
local value. The new T440p boot log is still required to prove that machine's
raw CMOS basis directly. SunlightOS policy is now explicit and enforced by its
VMware launcher: CMOS and kernel wall time are UTC; timezone conversion happens
once in `timezone_service`. Physical machines must likewise keep their RTC in
UTC.

Two code defects were confirmed during the audit:

1. The RTC reader said it used a stable double-read, but it only compared the
   seconds register. It also accepted malformed BCD nibbles that arithmetic
   could turn into plausible fields. The reader now compares two complete
   update-synchronized snapshots, uses the ACPI FADT century register when
   available, validates every decoded field, and fails closed instead of
   fabricating a fallback date.
2. Negative fractional timezone offsets used `hours * 3600 + minutes * 60`.
   Thus `-03:30` became `-02:30`. Minutes now inherit the negative hour sign.
   This defect does not explain a positive `Asia/Tehran` anomaly.

## Source inventory and data flow

Wall time:

```text
CMOS RTC (one stable boot read, UTC policy)
  -> kernel/src/arch/x86_64/rtc.rs (validated boot Unix epoch)
  -> RTC epoch + (global_ticks - boot_ticks) / 100
  -> GetTimeUtc syscall / ipc::get_time_utc()
  -> services/timezone_service (one UTC offset application)
  -> TzMsg::GET_LOCAL_TIME
  -> services/sunlight-vortex-shell::query_local_full
  -> format_center_datetime / panel
```

Uptime:

```text
PIT reference -> calibrated TSC -> calibrated periodic Local APIC timer
  -> BSP timer interrupt only
  -> kernel/src/timekeeping.rs::GLOBAL_TIMEKEEPER_TICKS
  -> telemetry::uptime_secs
  -> sunlight-telemetry snapshot
  -> services/sunlight-tasks::overview_strings
```

The PIT is used during boot calibration and its IRQ is then masked. HPET is
discovered through ACPI but is not used as a clocksource. Raw TSC time is used
internally for calibration and scheduler accounting; exported cross-core
monotonic time uses the BSP-only global tick counter. AP Local APIC timers drive
local scheduling but do not advance global time. `timer_server` receives the
coalesced global sequence for timer IPC and is not the source of either the
panel clock or Tasks uptime.

The running wall clock and uptime therefore share the same post-boot tick
delta, although the wall clock has the independent RTC base epoch. No code
periodically rereads CMOS or overwrites the running wall clock.

## Screenshot interpretation

Both displayed deltas are 6h46m because both values consume the same global
tick advancement. That is evidence against a changing relative rate inside
SunlightOS, but without external timestamps it neither proves nor disproves the
absolute LAPIC tick calibration. It strongly directs the date investigation to
the RTC base value, RTC basis, timezone, or presentation boundary rather than
to a timer subsystem rewrite.

## One-shot diagnostics

The kernel now reports:

- selected global clocksource and tick rate;
- LAPIC countdown frequency and initial count;
- PIT-calibrated TSC frequency used as the LAPIC calibration reference;
- raw RTC fields, status-derived BCD/binary and 12h/24h modes;
- ACPI century-register availability and year policy;
- stable-read result, decoded UTC, UTC basis policy, Unix epoch, and boot
  monotonic timestamp.

At service startup, `timezone_service` reports the configured zone, effective
offset, DST state, sole application point, final local civil time, and weekday.
Nothing is logged on timer ticks.

## Validation

QEMU exposed BCD/24h CMOS with ACPI century register `0x32`; the stable raw RTC,
decoded kernel UTC, and host trusted UTC agreed. The Phase 3.0 boot gate passed.

An uncorrected VMware boot reproduced the basis defect. A second boot with
`rtc.diffFromUTC = "0"` exposed `17:32:55` in raw CMOS while trusted host UTC
was `17:33:08` at log inspection, and SunlightOS reached the timer and timezone
service milestones without regression.

## Physical hardware validation still required

On the T440p, capture the new diagnostics and compare `rtc: raw`,
`rtc: decoded_utc`, `wall: utc`, `timezone:`, and `local:` with firmware RTC
settings and a trusted UTC clock. If raw CMOS equals local civil time rather
than UTC, configure the machine RTC as UTC; do not compensate by applying the
timezone twice in SunlightOS.

For a soak run, record trusted UTC, `wall: utc`, local panel time, and Tasks
uptime at start and after several hours. Cross midnight and verify one date and
weekday transition with continuous uptime. The T440p result cannot be claimed
until its resulting serial log is captured.
