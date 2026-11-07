# First partitioned-disk milestone

Baseline: `storage-audit-fat32-block-vfs.md` (left unchanged). This milestone
reuses BlockDevice, Fat32, FatFs and the authoritative KERNEL_VFS. No new VFS,
USB transport, exFAT or installer is included.

## Selection and trust policy

Only the existing first legacy virtio-blk disk is examined, once at boot.
Superfloppy images retain `/boot`. A SUNSTATE superfloppy selects `/state`
only when the operator explicitly builds with `SUNLIGHT_TRUST_LEGACY_STATE=1`.
That opt-in declares the sole first virtio disk to be operator-provisioned
trusted state media; the label is then a compatibility format check, not an
identity or authenticity check. Without opt-in a label alone selects only
`/boot`. Do not enable this option for untrusted or replaceable media. Existing
state-image gates in tools/test.sh explicitly opt in when attaching their
operator-generated target/state-test.img. Other test phases explicitly disable
it. For manual dedicated state-image workflows, use
`SUNLIGHT_TRUST_LEGACY_STATE=1 cargo build -p sunlight-kernel` and attach only the
intended trusted raw FAT32 state image to the first legacy virtio-blk device.

Partitioned disks never select `/state` from their FAT label. Discover all
primary entries, then probe MBR types 0x0b/0x0c or GPT Basic Data/EFI System
partitions with zero attributes. Exactly one successfully mounted FAT32
candidate is required; zero or multiple candidates leave the disk unmounted.
The selected partition appears at `/mnt` in KERNEL_VFS only. No partition data
is exported through the legacy vfs_server share-page snapshot. Other valid
partition types are enumerated but not filesystem candidates.

This boot-only selection assumes an operator-supplied virtio disk. It does not
establish media trust. The existing syscall write policy is unchanged; this
milestone grants no user or service new write authority on `/mnt`. The explicit
`SUNLIGHT_STORAGE_TEST=1` kernel build runs a privileged fixture probe before
userspace, bypassing syscall policy through KERNEL_VFS. It must be booted only
with disposable generated test images. Production boots perform no test writes.
Before partitioned `/state` support, require explicit operator provisioning and
selection by disk identity plus partition GUID (or equivalent trusted config),
authorization for writable state, ambiguous/absent-media handling, and a migration
policy. A label alone is never sufficient for new `/state` selection.

## Supported layouts and bounds

All LBAs are 512-byte sectors. The FAT BPB must also specify 512 bytes; no 4Kn
translation exists. PartitionBlockDevice checks nonempty extent, addition
overflow and disk capacity at construction, then rejects each relative LBA
outside that extent. Flush has no LBA: it forwards the underlying durability
barrier and its failure, potentially flushing other device writes as well.
The cache rejects out-of-range requests before accepting dirty writes.

MBR requires 0x55aa, valid status bytes, nonzero starts/sizes, in-disk extents
and no overlap. Extended/EBR and hybrid protective MBR layouts are rejected.
GPT requires a single correctly sized protective MBR entry; primary and backup
revision 1.0 headers of size 92, reciprocal LBAs, matching nonzero disk GUID,
usable ranges, entry geometry and CRC; both entry arrays must agree and pass
CRC. Metadata must lie outside usable space. Entries need nonzero unique GUIDs,
in-range nonoverlapping extents. Maximum 128 entries of 128 bytes; larger or
other header/entry formats fail closed. No recovery from one valid GPT copy is
attempted. No table repair or discovery writes occur. Unknown partition types
are preserved but not mounted.

A recognizable FAT32 BPB with a jump instruction and 512-byte sectors selects
the historical superfloppy path when its partition-entry area is empty and
no GPT signature occurs at LBA 1 (including images with the BIOS signature
cleared). FAT mount then validates geometry and device capacity. This is format
detection, not cryptographic validation of hostile media. FAT chain/metadata
interop and hostile-filesystem hardening remain separate work.

## Validation

Run `python3 tools/test_partitioned_storage.py` from the repository. It builds
an explicitly instrumented kernel, creates unpartitioned, MBR and redundant GPT
64 MiB FAT32 fixtures without sudo, and boots each twice with legacy virtio-blk.
First boot checks fixture reads, writes PROBE.TXT, syncs file/directory, drops
the mounted cache and remounts in KERNEL_VFS. Second boot checks exact persisted
bytes again. Host mtype independently checks file contents; sectors before and
after each partition must be unchanged. Partition fixtures carry SUNSTATE to
check they still mount at `/mnt`. Evidence is in `target/partition-storage/`.
This is orderly guest sync followed by VM termination/relaunch, not a power-loss
or filesystem crash-recovery guarantee, and not USB validation.

## Remaining blockers

Runtime registration and privileged mount/unmount are now implemented; see
[runtime-storage.md](runtime-storage.md) for the API, lifecycle policy and new
verification evidence. The boot selection and explicit `/state` trust policy
described above are preserved. Hotplug/removal remains outside this milestone.
Only one legacy virtio-blk device is supported. ATA/AHCI/NVMe remain absent.
Genuine USB sticks additionally need a storage device grant, xHCI bulk endpoints,
USB mass-storage BOT/UAS support, SCSI commands/capacity/flush translation,
timeouts/reset/disconnect handling, and real-device validation. The HID mouse
driver supplies none of that storage transport. FAT still lacks timestamps,
FSInfo maintenance, dot entries, dirty-volume handling and a read-only mount
mode. No claim of USB or real hardware validation follows from QEMU virtio.

## Changed files

- `sunlight-block/src/partition.rs`: bounded adapter, table discovery and MemDisk tests.
- `sunlight-block/src/lib.rs`: exports, cache bounds and checked MemDisk conversion.
- `sunlight-fat/src/fat.rs`: reject unsupported/out-of-device/overflowing BPB geometry.
- `sunlight-fat/src/lib.rs`: malformed BPB regression test.
- `kernel/src/main.rs`: partition selection, bounded cached disk, KERNEL_VFS mount,
  legacy state opt-in, explicit fixture gate hook, and current login-presenter
  sequence for the legacy VFS shell gate.
- `kernel/src/storage_partition_test.rs`: privileged test fixture operations and remount.
- `tools/test_partitioned_storage.py`: no-root image generation and two-boot integration.
- `tools/test.sh`: explicitly opt in to trust only dedicated state-image gates;
  wait for the final VFS shell command with current TTY exit markers.
- `tools/tests/phase6_5_3.expected`: assert current TTY success markers for all
  three VFS shell commands.
- `docs/partitioned-storage.md`: selection/trust policy, scope, evidence and blockers.

The pre-existing untracked audit baseline was read and left untouched.

## Test commands and results (2026-11-07)

| Command | Result | Evidence under target/ |
| --- | --- | --- |
| `cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-kv -p sunlight-tls --target x86_64-unknown-linux-gnu` | PASS: block 13, FAT 9, VFS 74, KV 27; TLS host target has zero tests | `partition-regression.log` |
| `python3 tools/test_partitioned_storage.py` | PASS: six fresh QEMU boots, all three layouts, exact persisted bytes and unchanged partition guards | `partition-qemu-final.log`, `partition-storage/*-boot-*.log`, `partition-storage/build.log` |
| `bash tools/test.sh phase3.0` | PASS: ordinary boot, native VFS stat/mkdir | `partition-final-phase3.0.log` |
| `bash tools/test.sh phase3.5` | PASS: existing virtio/FAT/share-page VFS gate | `partition-existing-fat.log` |
| `bash tools/test.sh phase6.5.3` | PASS after refreshing the obsolete login injection and expected TTY markers | `partition-vfs-final.log` |
| `bash tools/test.sh phase5x.4` | PASS: legacy injected markers only; runtime source review found this does not prove a TLS handshake | `partition-existing-tls.log` |
| `cargo build -p sunlight-kernel` then `bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightos.iso` | PASS: normal kernel and ISO, fixture probe disabled | `partition-build-final.log` |
| `bash -n tools/test.sh`, `python3 -m py_compile tools/test_partitioned_storage.py`, `git diff --check` | PASS | command output |

All final storage fixtures carry SUNSTATE. Without explicit trust the
superfloppy mounts `/boot`; partitioned fixtures mount `/mnt`, never `/state`.
Each first boot reads HELLO.TXT, writes and syncs PROBE.TXT, then replaces the
mounted cache and checks contents after remount. Each second independent guest
boot verifies the exact same persisted bytes. Host mtype checks the contents
independently. These are virtio-blk tests, not USB tests.

The optional extra `bash tools/test.sh phase3.75` service regression timed out
without its broader Wise Owl native-gate final marker; it is **not claimed
passing** (`partition-final-phase3.75.log`). It did verify that explicitly
trusted SUNSTATE media mounted at `/state`. This broader service-gate failure
remains unresolved; it was not reproduced against baseline.

The runtime milestone subsequently reproduced the timeout against unchanged
HEAD as well as the pre-runtime storage tree. The broader gate remains failed,
not passed; see the bounded comparison evidence in `runtime-storage.md`.

The original Phase 6.5.3 shell gate failed at `Login locked`. The same failure
was reproduced at unchanged HEAD `232ec79c4` in a temporary detached worktree
with the existing generated hello-linux ELF supplied
(`partition-baseline-vfs.log`). Its old injection typed the password before
selecting the user, and its expected `[EXEC]` markers no longer exist in the
current TTY. This patch adopts the login-selection/delay sequence already used
by other gates and checks all three current TTY exit markers, waiting for the
last command. No authentication policy was changed. The temporary worktree was
removed and the normal kernel/ISO rebuilt from the final sources.

Harness corrections during development: add virtio-rng so the secure-entropy
boot gate can proceed, and unlink serial logs before launch so an earlier PASS
cannot satisfy a later boot before QEMU asynchronously opens the log. An initial
VFS invocation overlapped editing its script and was restarted. These invalid
attempts are not counted as passing tests. Baseline artifact reuse also required
rebuilding the modified storage crates before the final run.

Final storage serial evidence (GPT, second boot):

```text
[PART] Gpt start=2048 blocks=131072
[VFS] FAT volume mounted at /mnt
[STORAGE] fixture read OK
[STORAGE] restart contents OK
[STORAGE] remount contents OK
[STORAGE] PASS
```

No commit or push was performed.

## Explicit runtime configuration

[Boot fstab and inventory](fstab-inventory.md) now retains already-validated GPT
unique/type GUID metadata for stable PARTUUID resolution. Explicitly configured
GPT volumes defer ordinary kernel boot mounting to init-launched storagectl.
Unconfigured boot selection and the explicit legacy-state trust opt-in above
remain intact. MBR and superfloppy persistent fstab sources are not supported.
