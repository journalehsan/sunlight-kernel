# VFS-N1I: Isolated VirtIO lease integration

**PARTIAL.** The existing N1E–N1G authority now gates ordinary I/O and
ownership transfer on the N1H disposable VirtIO device. The probe uses two
test owners issued by the existing VFS registration model. It runs before
normal disk admission and before the ring-3 service can register. This
qualifies registration-bound leases against a live transport; it does not
qualify kernel authentication of a live service endpoint.

The feature-gated kernel probe links the same lease implementation used by
the host fixture. It owns one canonical VirtIO handle, registers its
verified geometry once, and keeps the same media and queue incarnation
across lease transfers. The host image is compiled out of the probe. The
authority reserves one retained data slot, matching the device's single
active request. Only the serialized coordinator can submit to that handle.

The coordinator dispatches the command stored under the admitted ticket,
which binds device scope, media and lease generations. A staged write
checks the exact submitted request ID. All receipts are checked for device
identity, queue incarnation and consecutive generation, operation, LBA,
and successful terminal status; data receipts must account for 512 bytes.
Only then does the coordinator settle the original ticket. This is
serialized correlation; the legacy device does not echo the software
generation. A probe assertion stops qualification on unexpected settlement.

| Gate | Result | Evidence |
| --- | --- | --- |
| Acquisition and authorized I/O | PASS | 64 acquire–operate–release lifecycles. A second owner cannot acquire while occupied, use the incumbent's lease, or operate outside the granted extent or rights. |
| Ownership transfer and data integrity | PASS | 64 transfers from a whole-device writer to a one-sector read-only successor. The old lease becomes stale. Each successor reads the written data. The harness compares all 4 MiB, including untouched sectors. |
| Drain, flush, accounting | PASS | 128 admitted data commands and 192 virtual barriers, plus one pre-admission marker read. Quiescing closes admission; an outstanding ticket blocks the barrier. The write holds one logical buffer and one DMA mapping user in flight. Settled tickets cannot be consumed twice. Both data counts and the barrier slot are empty on settlement. Each barrier covers the preceding queue generation. |
| Pinned buffer reuse | PASS | Completed commands retire their mapping users; the same pinned queue and bounce allocation serves subsequent requests. |
| Physical DMA page reclamation | BLOCKED | Terminal reset acknowledged as `VerifiedRetained` with no outstanding request. The current driver returns a diagnostic observation, provides no release authority, and leaves the queue and bounce frames pinned. No allocator return was attempted. |
| Live kernel endpoint authentication | BLOCKED | The early probe uses fixture registrations, with no scheduler-backed service process, capability ownership check or kernel registration withdrawal event. |
| Fault settlement and physical hardware | NOT_RUN | No guest timeout, late completion, removal or lost caller tests. QEMU barriers establish no power-loss or physical-controller guarantee. |
| N1D device attachment | BLOCKED | No backend readiness probe, mount reservation or publication. |

The next step is to consume the kernel's authenticated registration and exact
withdrawal events at the admission boundary and resolve aliases to one
device/medium authority. Qualify lost callers, ambiguous effects and late
settlement without releasing ownership on timeout. Before an attachment
handoff, complete predecessor cache writeback, close admission, drain and
confirm its barrier, then verify the successor's geometry, media and root
through a fresh read-only backend. Bind readiness to the exact N1D reservation
through topology and FD installation checks. Preserve rollback fencing and
retained-reference withdrawal. A separate DMA fence and allocator handback
are required before claiming physical reclamation.

Focused regressions passed: 33 VirtIO, 26 lease, and seven N1D attachment
tests. All 59 N1H names matched. The full N1G baseline passed separately:
373 filesystem/IPC/service and 108 registration tests, with all 481 names
matched. The production kernel and release service compile. The unchanged
ownership guard **FAILS** with 71 references and the same four unmatched
findings (statfs, ProcFS mountinfo, private mmap, runtime FAT mount).
Ordinary native cutover was **NOT_RUN**. Production routing, boot disk and
FAT ownership, mount publication and `/vfs-scratch` remain unchanged.
L1C-3 stays blocked, VFS-D1 deferred and demand-backed main stacks opt-in.

Reproduce using the existing N1H boot ISO and local Limine:

```sh
CARGO_INCREMENTAL=0 cargo build -p sunlight-kernel --features vfs-n1i-test --offline
python3 tools/test_vfs_n1h.py --n1i
```

See [machine-readable results](vfs-live-virtio-lease-integration-results.json).
