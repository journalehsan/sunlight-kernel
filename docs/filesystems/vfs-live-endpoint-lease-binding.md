# VFS-N1J: Live kernel endpoint and device lease binding

**PARTIAL.** The isolated QEMU path qualifies one real VFS service identity
through registration, repeated lease acquisition and release, and service
exit with an outstanding write. Live successor registration and device-backed
N1D attachment remain open.

The ring-3 VFS image calls the existing nameserver registration path. The
kernel checks the trusted service record and endpoint capability ownership,
then passes the exact active registry result to the existing N1E–N1G authority.
The session binds endpoint, PID, incarnation, address-space generation and
epoch. The coordinator compares the scheduler caller's full image identity
before using that binding. No caller payload assigns a device owner. The
test uses one whole-device lease authority and one queue for the marked
disposable disk. There is no user block RPC or production route.

The single-CPU qualification drives transport work after dropping scheduler,
broker and IPC guards. The lease ticket pins the incumbent lease and media
generation; the coordinator tracks the device, queue incarnation, request
generation, operation and LBA. The exit case retains the exact submitted
request ID alongside its lease ticket. Registration withdrawal synchronously
revokes the matching session, drops the caller credit, and closes the lease
for retirement while keeping the transport slot and DMA user. Exact private
image retirement allows the idle coordinator to continue; it supplies no
DMA completion authority. Only confirmed terminal settlement, drain and a
covering flush release the lease. A timeout or unconfirmed transport outcome
fences the lease. Unexpected qualification invariants stop the guest with
the frames pinned.

| Gate | Result | Evidence |
| --- | --- | --- |
| Live registration and authenticated acquisition | PASS | Actual endpoint 1, PID 3, incarnation 43, VM generation 18, epoch 1 in the recorded run. N1I used two fixture registrations. |
| Read, write, flush and lease cycling | PASS | 64 release/reacquire cycles. Each written sector is read under a later lease generation of the same service. 129 admitted data commands and 129 virtual barriers, including retirement. |
| Withdrawal and exit with accepted I/O | PASS | Withdrawal rejected new admission and left zero caller credits and one retained command. Exact image retirement preceded polling and settlement. The barrier was blocked before terminal settlement. Final request and mapping user counts are zero. |
| Cross-service handoff | NOT_RUN | The existing live registry admits one active service. This run uses one image and one epoch. The N1I fixture handoff does not qualify a second kernel-authenticated owner. |
| Untouched-sector integrity | PASS | The existing harness matched the full 4 MiB backing image. Writes were confined to LBAs 128–191. |
| Fault injection, exec retirement, endpoint-only revocation and physical hardware | NOT_RUN | Only ordinary completion and actual exit were exercised. Virtual barriers do not establish physical durability. |
| Physical DMA reclamation | BLOCKED | No allocator handback was attempted. |
| Device-backed N1D integration | BLOCKED | Requires live successor evidence, fault settlement, and a readiness probe bound to the exact N1D reservation through publication and FD installation. |

The coordinator retains one explicit boot-lifetime allocation record: four
queue pages and one bounce page, 20,480 bytes total. The PMM allocated them
once; neither release, reacquisition nor process exit allocates a new set.
After `VerifiedRetained`, the static record still owns the frame addresses and
the terminal device object. This is bounded for repeated ownership of one
attached queue and can support subsequent isolated testing on a live queue.
It does not qualify repeated detach/reinitialize or general attachment
resource reclamation. A production detach that returns frames needs proof
that the exact transport can no longer access them, including platform DMA
drain/isolation when required, and an explicit allocator handback. Status
zero and zero mapping users do not supply that proof.

All 481 baseline names, 59 N1H names and seven N1D names passed. Production
kernel and release VFS compilation passed. The unchanged guard **FAILS**
with 71 references and the same four unmatched findings (statfs, ProcFS
mountinfo, private mmap and runtime FAT mount). Ordinary cutover was
**NOT_RUN**. No mount was published by the probe, and `/vfs-scratch` was
not activated. L1C-3 remains blocked, VFS-D1 deferred and demand-backed
main stacks opt-in.

Use the existing isolated ISO harness and local Limine. Keep the exit image
out of the normal release artifact directory:

```sh
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone" \
  cargo build --offline -p sunlight-vfs-server --release --features vfs-n1j-exit-test \
  --target-dir target/vfs-n1j/service-target
SUNLIGHT_VFS_N1J_ELF="$PWD/target/vfs-n1j/service-target/x86_64-unknown-none/release/sunlight-vfs-server" \
  CARGO_INCREMENTAL=0 cargo build --offline -p sunlight-kernel --features vfs-n1j-test
python3 tools/test_vfs_n1h.py --n1j
```

See [machine-readable results](vfs-live-endpoint-lease-binding-results.json).
