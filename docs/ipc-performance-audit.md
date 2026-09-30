# Incremental IPC performance audit — 2026-09-30

This change keeps the synchronous, capability-checked IPC ABI. It reduces
unnecessary work rather than introducing a new transport. Findings below come
from the current source; operation counts are not measured latency gains.

## Applied changes

### Collect completed calls before taking unrelated locks

`kernel/src/arch/x86_64/syscall.rs::ipc_call` now checks the calling task's
terminal result while holding only `SCHEDULER`. A successful collection avoids
the capability-broker lock, endpoint-shard lock, endpoint resolution and the
handler's linear caller lookup. Errors such as timeout and peer closure use
the same path.

This preserves the existing ordering: `handle_ipc_call` already consumed a
terminal outcome before checking the submitted capability. New requests still
perform the original capability checks. Reply generation validation and
one-time consumption remain in `take_terminal_result`; no user-provided
identity becomes authoritative.

### Avoid scanning every process after a successful reply

`kernel/src/ipc/mod.rs::finish_call` previously searched every process's deferred
reply list for every completion. On successful delivery, the exclusive server
target has already been consumed by `ipc_reply_target.take()` or by removing
the deferred token. Skip the redundant search for `ReplyDelivered` only.
Timeout, cancellation and peer closure retain the cleanup search.

This shortens work under the global scheduler lock, helping the contention
budget on SMP without changing scheduler ownership, lock ordering or wakeups.
The obsolete no-op fastpath eligibility branch and misleading lock-free/O(1)
comments were also removed/corrected; no speedup is attributed to those edits.

### Reuse the audio client's acknowledged PCM page

`services/sunlight-audiod/src/lib.rs::submit_pcm_chunk` now retains one lazily
allocated 4 KiB page per client. The existing server copies data into its
private PCM queue and releases its mapping before acknowledging the request.
Only a successful, validated reply permits reuse.

The private `reply_buffer.rs` cache takes the page out of circulation while a
request is pending. Every error retires the local mapping; a remaining peer
mapping keeps the old backing alive. This is essential after a timeout: it is
not evidence that the server stopped accessing the payload. A later request
allocates a new page instead of overwriting the timed-out page. Drop releases
the cached mapping. Shorter writes erase the previous payload's unused tail.

The client is now thread-affine (`!Send`, as well as `!Sync`). Native SHM
mapping records belong to the calling task, so an unsafe Send implementation
would be inappropriate even for threads sharing the address space. The media
worker creates and uses its own client.

For N acknowledged submissions during one client lifetime:

| Work | Before | After |
| --- | ---: | ---: |
| Client SHM allocations | N | 1 |
| Client SHM releases | N | 1, at drop |
| Server maps/releases | 2N | 2N |
| Payload copies | Unchanged | Unchanged |

For 1,000 successful submissions this implies 4,000 -> 2,002 SHM syscall
operations including final client cleanup. The host policy test independently
counts 1 allocation and 1 release for 1,000 mock acknowledged requests. It does
not execute native syscalls or measure audio performance.

## Remaining opportunities and boundaries

1. **Measure SMP contention next.** Calls still take the global scheduler lock;
   16 queue shards do not imply independent cross-core call processing. Add
   bounded, per-core lock-wait/hold and round-trip measurements before changing
   scheduler ownership or lock order. Report latency distributions, throughput,
   CPU time and context switches at 1/2/4 CPUs with the same workload.
2. **Persistent server mappings need a protocol.** Removing audiod's remaining
   map/unmap pair requires explicit attach/detach, ownership, bounds and
   timeout/exit handling. A token-to-pointer cache alone can retain dead peers'
   pages indefinitely. Keep the current bounded copy until that contract exists.
3. **Batching and zero-copy are service-specific.** Prefer offset/length
   descriptors in a bounded buffer pool. Define when producers regain write
   ownership; never use a timeout as a completion acknowledgement. Keep kernel
   badges/capability checks as the source of caller identity.
4. **SHM security is not complete.** `sys_shm_map` currently maps RW and resolves
   bearer tokens rather than recipient-specific grants. Read-only mappings,
   checked object lengths, restricted recipients, quotas and revocation
   semantics need their own compatibility/security work before generalized
   zero-copy services.
5. **Notifications are not ready for asynchronous queues.** `ipc_notify_send`
   is a no-op and `ipc_notify_wait` does not implement a matching notification
   protocol. A future doorbell needs a lost-wakeup-safe sleep/wake contract.
6. **Register count validation remains separate work.** The wire carries four
   words and two caps although `IpcMsg` has eight word slots. Decoding clamps
   the counts before the current validation. Rejecting oversize raw counts
   requires checking/migrating existing protocols that rely on truncation.

## Verification

Host tests cover acknowledged reuse/drop, uncertain-completion isolation and
allocation-failure recovery. Native boot self-tests additionally exercise
deferred reply success, cancellation, peer closure, one-time collection and
stale deferred-token rejection. The phase 2.6 gate requires the deferred
lifecycle marker and the existing capability/deadline/arbitration checks.

The existing timer server emits `round-trip test: 1000 calls OK` after 100 timer
ticks; it is **not an actual 1,000-call benchmark**. Do not use this marker as
performance evidence. Likewise, the existing boot terminal-state stress loop
runs under the scheduler lock: booting with multiple CPUs does not turn that
loop into a simultaneous cross-core race test.

Results:

- Host `sunlight-audiod` library: 19 tests passed, including buffer retirement,
  one allocation across 1,000 mock requests, and clearing a shorter payload's
  old tail using the production write routine.
- Host `sunlight-ipc` library: 38 tests passed; `sunlight-media`: 36 passed.
- `cargo check -p sunlight-kernel --bin sunlight-kernel` passed. The subsequent
  phase 2.6 gate built the kernel and userspace binaries with the changes.
- Two-CPU QEMU: `tools/test.sh phase2.6` passed with the added lifecycle markers.
- Four-CPU QEMU: the same ISO produced all 29 non-comment phase 2.6 markers
  (adjusting the expected CPU count), with no `UNEXPECTED` or `PANIC` text.
  The VM was intentionally terminated by a 45-second timeout (exit 124).
- `git diff --check` and rustfmt check of the new buffer module passed.

Local evidence: `target/ipc-improvements-host.log`,
`target/ipc-improvements-audiod-tests.log`,
`target/ipc-improvements-2cpu.gate.log`,
`target/ipc-improvements-2cpu.serial.log`, and
`target/ipc-improvements-4cpu.serial.log`. These generated files are not tracked.

Reproduce the host suites with:

```sh
cargo test -p sunlight-audiod -p sunlight-ipc -p sunlight-media --lib --target x86_64-unknown-linux-gnu
SUNLIGHT_TEST_SERIAL_LOG=target/ipc-improvements-2cpu.serial.log bash tools/test.sh phase2.6
timeout --signal=TERM 45 qemu-system-x86_64 \
  -cdrom target/sunlightos.iso -serial file:target/ipc-improvements-4cpu.serial.log \
  -display none -m 1024M -smp 4 -device virtio-rng-pci,disable-modern=on \
  -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 -no-reboot -no-shutdown
```

These boots used QEMU TCG, not a hardware performance benchmark. The native
audio PCM submission/reuse path was compiled but not exercised in a playback
run; its buffer policy and write routine were host-tested. No claim of
human-audible improvement, latency reduction, or hardware SMP speedup follows
from these tests alone. The kernel's host library is a memory-management
harness and does not run `kernel/src/ipc/mod.rs` tests; IPC state-machine
validation here comes from the native boot checks.
