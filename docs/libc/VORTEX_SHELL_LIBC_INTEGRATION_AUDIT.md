# Vortex Shell / `sunlight-libc` integration audit

**Scope:** synchronize the native Vortex Shell with the hardened libc without
changing its visual design or application-state architecture.

## Resolved native dependency

`sunlight-vortex-shell` resolves one path package:

```text
sunlight-libc v0.2.1965 (/…/sunlight-libc)
features: global-alloc, dynamic-heap, dynamic-heap-8m
```

The shell manifest owns `global-alloc`; its default `dynamic-heap` feature
forwards only `dynamic-heap-8m`.  Thus the normal native build keeps the
existing 8 MiB mmap-backed heap while
`--no-default-features --features stress` checks the same binary against
libc's static heap.  Host tests must continue to use the isolated proof
strategy because linking the complete libc into a host runtime interposes its
native clock ABI.

Run the repeatable proof with:

```sh
./tools/test-vortex-shell-libc-integration.sh
```

It asserts the direct path dependency and feature forwarding, verifies there
is exactly one `sunlight-libc` lockfile package, checks both heap selections,
and runs the isolated allocator and time/weekday proofs.  It deliberately
does not run `cargo test -p sunlight-libc` as one host-linked binary.

## Integration findings

- The shell has no allocator shim, bump allocator, local arena, or direct
  syscall layer.  Its `Vec`, `String`, icon state, notification rows,
  calendar data, window snapshots, and Start Menu all use libc's global
  allocator.  The static wallpaper byte array is deliberately process-lifetime
  storage, not heap compatibility state.
- The `stress` feature now warms its bounded MRU list before sampling allocator
  state, then churns temporary window, calendar, notification, timer, and
  text collections.  It reports a recovery mismatch if requested bytes, live
  allocations, or allocated backing do not return to the pre-churn snapshot.
  This exercises reuse/coalescing without adding permanent telemetry buffers.
- File loaders use raw descriptors only.  Short reads are consumed in loops,
  EOF terminates normally, every post-open branch closes once, and the former
  wallpaper-loader `EAGAIN` retry loop now fails the bounded load instead of
  spinning the event thread.  The dynamically sized file loader reserves with
  `try_reserve_exact`, preserving the prior valid UI/fallback on allocation
  failure.  No stdio layer was added.
- The shell has no randomness consumer.  The Start Menu's `Random Apps` row
  is a fixed, stable list; it does not need a random-service request or a
  fallback seed.
- Wall-clock presentation continues through the timezone service (realtime)
  for the center clock, tooltip, and calendar.  UI deadlines, elapsed hover
  time, app polling, status polling, diagnostics, and launch tracing use
  `monotonic_millis`.  The independent weekday proof covers Thursday,
  Wednesday, Sunday, and the ISO/Sunday-first distinction.
- `RunningAppRegistry` remains the owner of `AppId → AppInstanceId →
  ProcessKey(pid, generation) → WindowKey(id, generation)`.  No libc change
  maps this identity to a raw PID or descriptor.

## Long-run observability and limits

Every 30 seconds the shell writes one bounded `[VORTEX][diag]` line containing
main-loop progress, input/tick/other/drop counts, timed-out shell IPC calls,
the display event-route counters (polls, available/dequeued events, local
ticks, interleaved polls), allocator statistics, windows, running apps, and
known shell IPC/SHM state.  Stress logs include a recovery-failure count.

The display protocol currently exposes event-route counters but not a remote
queue depth, timer-owner registry, lock-hold duration, or descriptor-table
enumeration.  This audit does not fabricate those measurements or broaden the
display/VFS protocol.  `EVENT_POLL` remains an intentional unbounded receive:
a short client timeout can late-drop a key after the display server dequeues
it.  The route counters make that stalled-reply case visible; fixing it needs
a protocol-level non-consuming poll/cancel contract, not an arbitrary timeout.

## Bounded QEMU soak profile

Build the normal dynamic heap shell plus the stress feature, rebuild the
kernel/ISO so the embedded shell ELF is refreshed, and boot a 512 MiB VM:

```sh
SERVICE_RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build -p sunlight-vortex-shell --release --features stress
touch kernel/src/main.rs && cargo build -p sunlight-kernel
./tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightos.iso target/limine .
./tools/runs.sh --memory 512 --cpus 4 --no-disk
```

Open/close the Start Menu and calendar repeatedly; launch, minimize, restore,
exit, and crash-test multiple applications; then leave the desktop idle.  Save
the periodic diagnostics before and after the interaction phase.  A passing
bounded soak has increasing stress-cycle/allocation/free counters, zero stress
recovery failures, stable live/requested allocator values after transient work,
and continuing input/event-route counters after idle.  Several-hour and
bare-metal runs remain manual validation, not CI work.

## Deferred work

This patch does not add stdio or sleep APIs, an allocation-failure injector
for every UI constructor, a display queue-depth ABI, timer/handler ownership
introspection, descriptor enumeration, automatic shell restart, or a broad
compositor redesign.  Those are needed for a complete root-cause proof if the
historic multi-hour freeze reproduces.
