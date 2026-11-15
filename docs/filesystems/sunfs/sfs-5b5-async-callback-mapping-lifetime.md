# SFS-5B.5 — True asynchronous device access, callback quiescence and mapping lifetime

**PARTIAL. SFS-5B.5 is not COMPLETE.** Genuine asynchronous device access now
overlaps reset processing and is independently witnessed; the callback and
mapping lifetime protocol is implemented and proven for the instrumented
backend. The prior boot stall is narrowed but unexplained, the fencing decision
is qualified only inside the instrumented scope, and two race cuts remain
NOT_RUN. All five historical DMA pages stay pinned; no new queue epoch is
authorised. Native writable SunFS remains BLOCKED.

The authoritative ledger is
[validation-summary.json](../../audit/sunfs/sfs-5b5/validation-summary.json);
every raw attempt is retained under
[the new evidence namespace](../../audit/sunfs/sfs-5b5/).

## Scope and baseline

Branch `main`, revision `22a5947f5fbd4ffe810aee5efee57b0ec18f8915`. The incoming
worktree already carried the uncommitted SFS-5A..SFS-5B.4 changes, 5,589
pre-existing deletions (5,542 under `.threading-evidence`) and 23 untracked
paths. None of those were touched. The SFS-0..SFS-5B.4 evidence namespaces,
`kernel/src/sfs_reset.rs` and `kernel/src/sfs5b4_test.rs` are byte-identical to
their incoming state.

The native transport is unchanged: legacy PCI VirtIO Block `1af4:1001`, split
queue 0, coherent pinned guest RAM, features `0x240`, no negotiated queue-reset
feature. Only new, exclusively created marked 4 MiB images are attached. Seed
`0x5b5001` selects fixed fixture bytes and named barriers; it is not a random
native race seed.

The emulator is a separate isolated build at `/tmp/sfs5b5-qemu-build`; the
system emulator is unchanged and the SFS-5B.4 binary is preserved by hash.

## QEMU concurrency architecture

Full audit: [qemu-concurrency-audit.md](../../audit/sunfs/sfs-5b5/qemu-concurrency-audit.md).

The decisive facts, read from the pinned 11.1.2 tree:

- `address_space_map()` takes `memory_region_ref(mr)` and
  `address_space_unmap()` takes `memory_region_unref(mr)` (`system/physmem.c`),
  so **a DMA mapping cannot outlive its MemoryRegion**.
- `blk_drain()` waits on `BlockBackend.in_flight` and `bdrv_drained_begin()`
  only (`block/block-backend.c`). It covers the block layer, **not** a
  device-private timer, BH or thread.
- `virtio_blk_reset()` asserts the dataplane stopped, calls `blk_drain()`, then
  detaches and frees queued requests under `rq_lock`. Nothing in
  `virtio_reset()` stops device-private asynchronous work.

That last row is the gap SFS-5B.4 identified. A status-zero acknowledgment says
nothing about a device-private worker, so the device must join it itself.

## Prior boot stall (G2 — PARTIAL)

[prior-stall-investigation.json](../../audit/sunfs/sfs-5b5/prior-stall-investigation.json).

The preserved attempt `../sfs-5b4/raw/native-20261115T085640.621413Z` ends after
`[ACPI] OK` with the trailing bytes `[SMBI` — the guest stopped **inside the
serial write** of the first SMBIOS line, before SMP start and before any SFS
code ran.

Wait-for graph: only the BSP is runnable; it holds the `SERIAL` spinlock with
interrupts disabled inside `_print`, and `uart_16550::SerialPort::send` waits
for the transmit-holding-empty bit with **no bound**
(`kernel/src/arch/x86_64/serial.rs`). The only required event is the host
chardev consuming the next byte. There is no reset owner, no outstanding
request, no device callback and no second runnable entity.

Replay of the same logical scenario (cpu-2, writeback, unmodified system QEMU)
completed 5/5 times in 9.0–9.8 s. **The stall is not reproduced.** A pass does
not establish the historical cause, so the root cause stays UNQUALIFIED. What
*is* established is a negative: the stall is not attributable to the reset
coordinator, the scheduler contention fixture or a device callback, because
none of them had run.

## Asynchronous backend (G3, G4 — PASS)

Design: [qemu-backend-design.md](../../audit/sunfs/sfs-5b5/qemu-backend-design.md).

`qemu-async-worker.patch` adds a device-private joinable `QemuThread` to
`hw/block/virtio-blk.c`, reachable only for the disposable attachment and only
when `SFS5B3_MODE` starts with `async`. The worker reads the real 512-byte
payload through the mapping that `virtqueue_pop()` established, off-BQL, while
the main thread executes `virtio_blk_reset()` under BQL.

Ordering, all from real events:

```
MAPPING_ACQUIRED → REQUEST_FETCHED → ACCESS_ENTERED → ACCESS_ACTIVE
  → [guest DEVICE_ACCESS_HANDSHAKE, TIMEOUT_QUARANTINE, RESET_REQUESTED]
  → RESET_ENTERED → ACCESS_OBSERVED_RESET_ACTIVE → ACCESS_ACTIVE_DURING_RESET
  → ACCESS_EXITED → CALLBACK_QUIESCED → WORKER_JOINED
  → MAPPING_RELEASED → OLD_EPOCH_QUIESCED → RESET_ACKNOWLEDGED
```

The overlap is not inferred from timestamps. The reset thread publishes
`reset_entered` under the worker mutex and waits for the worker to observe it;
the worker publishes `ACCESS_OBSERVED_RESET_ACTIVE`; the reset thread then reads
the iteration counter again. In all 18 native runs the counter advanced during
the reset critical interval — between 42 and 3,804 real payload reads — while
the worker was never cancelled until after the observation.

This is `TRUE_OVERLAP`, not `SERIALIZED_BY_DESIGN`.

## Independent witnesses

Guest side (serial): request/queue identity, task incarnation, CPU, submission,
timeout, quarantine, reset ownership, reset initiation, acknowledgment and
retirement outcome.

Device side (QEMU stderr): descriptor fetch, mapping acquisition, access start,
access in progress, access completion, callback ownership, reset entry,
cancellation/drain, mapping release and any post-acknowledgment access.

The [event graph](../../audit/sunfs/sfs-5b5/independent-event-graph.json) uses
only real edges: per-thread program order, the mutex/condvar handshake, the
device→guest status-byte handshake, the guest→device status-zero write, the
device→guest status-zero readback, and the join. All 18 positive graphs are
acyclic.

## Callback quiescence (G5 — PASS)

`sunlight-block/src/quiesce.rs` (new) gives every asynchronous operation a
stable identity, an explicit owner, an active/inactive state, cancellation
authority and exactly one terminal outcome. Cancellation acceptance is a
distinct state from cancellation completion: `cancel()` returns `Accepted` and
leaves the slot `Cancelling`; only `finish()` commits a terminal, and a slot
that was already `Cancelling` commits `Cancelled` even if a late completion
arrives.

The protocol is wired into the real staged driver path in
`sunlight-virtio/src/blk.rs`: a staged write now owns an `OpSlot` and a
`MappingRecord`; the completion path commits one terminal and retires the
mapping once; the timeout and cancel paths request cancellation only and
therefore retain both, which is exactly why `drain()` returns `Bounded` for a
quarantined request.

Tests C1–C12 plus four negative controls and two exactly-once properties pass
(25 tests). The device side adds the native counterpart: `CALLBACK_QUIESCED`
and `WORKER_JOINED` precede `MAPPING_RELEASED` and the acknowledgment in every
positive run.

## Mapping lifetime (G6 — PASS)

`MappingRecord` tracks user authority independently of request completion:
acquisition precedes use, authority spans the whole access loop, all users
release before retirement, retirement happens exactly once and only at zero
users, a retired mapping can never be reacquired by any epoch, and a new epoch
cannot alias an unretired old mapping. `FenceGate` separates admission closure
from mapping revocation, so reset does not implicitly revoke a mapping.

Negative controls NC1–NC4 attempt use-after-release, callback-after-teardown,
stale-queue access and a delayed write after claimed quiescence; each is
refused and detected.

The distinction the milestone must keep explicit: CPU page protection, emulator
address-space validation, DMA mapping revocation, device reset semantics and
hardware IOMMU fencing are five different mechanisms. Only the second and third
are exercised here.

## Reset and fencing decision (G7 — PARTIAL)

[fencing-decision.json](../../audit/sunfs/sfs-5b5/fencing-decision.json) records
ten properties as PASS inside the instrumented scope: no new I/O admitted, old
requests inventoried, quarantine established, unique reset ownership, verified
acknowledgment, callbacks quiesced, mapping users quiesced, no authorised
old-epoch access, documented platform assumptions, and retirement limited to
the exact qualified resources.

It is **PARTIAL**, not PASS, because four properties are genuinely unproven:
general block-backend callback/drain quiescence (the request is never
submitted, so `blk_drain()` is not exercised), platform mapping/allocator
reuse, hardware DMA fencing/IOMMU, and persistence. A completed reset is still
not proof that every host callback and mapping has been retired, and a
quiescent emulator callback is still not proof of hardware fencing.

## DMA retirement and quarantine (G8 — PASS within scope)

[dma-retirement.json](../../audit/sunfs/sfs-5b5/dma-retirement.json),
[quarantine-accounting.json](../../audit/sunfs/sfs-5b5/quarantine-accounting.json).

Fresh fixture: the request record is freed exactly once after join and detach;
the payload mapping is unmapped exactly once after join; the driver-side
`OpSlot`/`MappingRecord` retire exactly once. PMM free counts reconcile
exactly (`free-before == free-after`) in all 18 native runs.

Historical quarantine: the five SFS-5B.4 attachment pages (20,480 bytes) remain
pinned, `released=0`. Their original ownership cannot be reconstructed, so they
stay pinned. Proving retirement on a fresh fixture does **not** qualify them,
and no new queue epoch was manufactured.

## Race cuts and SMP (G9 — PARTIAL)

[race-cut-matrix.json](../../audit/sunfs/sfs-5b5/race-cut-matrix.json),
[smp-progress.json](../../audit/sunfs/sfs-5b5/smp-progress.json).

PASS: R1, R6, R7, R8, R9, R10, R14. PARTIAL: R2, R3, R11, R12, R13, R15.
NOT_RUN: R4 and R5 — the driver polls the used ring and has no block interrupt
handler, so an IRQ-versus-reset cut cannot be forced without a driver redesign
that is out of scope.

R10 is real SMP contention: at 2 and 4 CPUs the reset owner and the losing
contender run on distinct CPUs, all online CPUs advance their timers
independently, and an independent runnable worker makes progress on every
online CPU. The 1-CPU profile is the serial reference.

## Regressions and source integrity (G10 — PASS)

[regression-results.json](../../audit/sunfs/sfs-5b5/regression-results.json).

Freshly executed: 54 SunFS tests in debug and release, 208 storage tests
(183 inherited + 25 new), 212 kernel host tests, `no_std` check, `rustfmt`
check, scoped Clippy, whitespace check, default and diagnostic kernel builds.
The 13,142 crash-schedule count is **reused**, not re-measured.

Five build attempts failed and are preserved: two ENOSPC failures moving a
rustc dep-graph and three `rust-lld` SIGBUS failures. Diagnosis: the
`target/x86_64-unknown-none/debug/incremental` cache had grown to 5.1 GiB and
was corrupt, while the filesystem still reported 45 GiB free and a 200 MiB
probe write succeeded. Removing only that build-cache directory fixed both
builds. No source, evidence or unrelated build output was deleted.

## Verdicts

| Gate | Status | Note |
|---|---|---|
| G1 | PASS | baseline, source integrity, concurrency audit |
| G2 | PARTIAL | stall narrowed to pre-fixture early boot; not reproduced; cause UNQUALIFIED |
| G3 | PASS | genuine asynchronous backend, no invalid cross-thread access |
| G4 | PASS | real overlap plus independent witnesses, all graphs acyclic |
| G5 | PASS | callback lifetime, exactly-once terminal, bounded drain |
| G6 | PASS | mapping authority, exactly-once retirement, post-fence refusal |
| G7 | PARTIAL | qualified inside the instrumented scope only |
| G8 | PASS | fail-stop ownership plus fresh-fixture retirement; historical pages retained |
| G9 | PARTIAL | R4/R5 NOT_RUN; several native cuts host-model only |
| G10 | PASS | evidence completeness and source integrity |

Overall **PARTIAL**. The declared instrumented virtual-device scope is
qualified for asynchronous access versus reset; the general virtio-blk path,
hardware and persistence are not.

## Reproduce

```
CARGO_BUILD_JOBS=2 python3 tools/sunfs5b5_validate.py native --cpus 1,2,4 \
    --modes async,async-negative --caches writeback,none --repeats 2
CARGO_BUILD_JOBS=2 python3 tools/sunfs5b5_validate.py stall --stall-repeats 5
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=1 python3 tools/sunfs5b5_validate.py regression
python3 docs/audit/sunfs/sfs-5b5/derive.py <raw-native-dir> --out docs/audit/sunfs/sfs-5b5
python3 docs/audit/sunfs/sfs-5b5/build-artifacts.py
```

The isolated emulator is rebuilt with
`cd /tmp/sfs5b5-qemu-build && ninja -j2 qemu-system-x86_64` after applying
`qemu-async-worker.patch` to a copy of the pinned 11.1.2 tree.

## Remaining work

Recommended **SFS-5B.6**: submit the held request to the real block backend and
then reset, so `blk_drain()` quiescence and bounded drain are qualified; add an
interrupt-driven completion path to force R4/R5; and prove allocator reuse on a
fresh attachment before any historical-page reclamation. Until then retain
terminal quarantine and closed admission.

No native SunFS mount, no physical formatting, no boot/root migration, no
CPython or SunApp work, no unrelated kernel redesign, no commit and no push.
