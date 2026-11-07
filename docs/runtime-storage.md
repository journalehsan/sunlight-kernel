# Runtime storage registration and privileged mounts

This milestone extends the partitioned-storage implementation and reuses
`BlockDevice`, `PartitionBlockDevice`, the FAT32 driver and authoritative
`KERNEL_VFS`. The original [storage audit](storage-audit-fat32-block-vfs.md)
is unchanged. No commit or push was performed.

## Boot behavior and trust

Ordinary Live ISO boots keep their RAM root and existing boot-volume selection:
legacy superfloppies select `/boot`, explicitly trusted legacy SUNSTATE media
select `/state`, and an unambiguous eligible partition selects `/mnt`. All boot
mounts are protected from runtime unmount, including an automatically mounted
`/mnt`. The legacy share-page snapshot and userspace VFS remain in place.

`SUNLIGHT_TRUST_LEGACY_STATE=1` is still the sole explicit opt-in for legacy
`/state`. Runtime device IDs, partition discovery, FAT labels and StorageAdmin
are **not** state-media trust authorizations. Runtime mounts can only target
`/mnt` or an existing directory below it. The interface cannot mount `/state`,
replace a boot/system mount, or migrate KV data. KV's Live ISO ramfs behavior
and its private broker log remain unchanged.

The explicit `SUNLIGHT_RUNTIME_STORAGE_TEST=1` kernel build suppresses the
boot FAT mount for disposable test media; it does not change the production
default. `SUNLIGHT_STORAGE_TEST=1` retains the older boot-selection fixture.
Neither should be enabled for ordinary images. The runtime probe command is
only compiled with the `sunlight-utils/storage-test` feature.

## Registry and authorization

The initialized first legacy virtio-blk disk is registered even if its table
or filesystem is rejected. Validated discovery registers all partitions,
including non-FAT types. The bounded registry exposes an opaque nonzero ID,
parent disk ID (zero for the disk), absolute starting LBA, capacity in 512-byte
sectors, supported read/write/flush bits, FAT32 eligibility, discovery ordinal
and layout. Read-only virtio devices do not advertise write support; writable
mounts of them are rejected. FAT32 eligibility is a format-probe hint, not a
promise that damaged filesystem metadata will mount.

IDs increase monotonically, never wrap, and never derive from reusable table
slots. Unknown and retired IDs fail lookup. The stale-ID unit test simulates
replacement in a private registry slot; this milestone adds no unregister,
hot removal or transport replacement API. IDs are boot-local, not persistent
disk identity and never proof of provisioning or authenticity.

Every StorageCtl operation requires **both uid 0 and an explicit
`ServiceCapability::StorageAdmin` bit**. The syscall reads these from the
running kernel process, before reading request memory. It accepts no claimed
UID, PID, capability mask or trust assertion. A legacy unrestricted lookup
profile (`None`) is insufficient. Existing service capability profiles apply;
the embedded `storagectl` tool receives the bit only when spawned as root,
using the existing per-control-tool capability mechanism. Its children follow
the existing capability inheritance rules.

Root with StorageAdmin may create/open writable files and sync directories
only on an installed runtime mount, checked under the VFS lock. This scoped
addition does not allow writes to an underlying unmounted `/mnt` directory,
boot/system mounts, `/state` or KV's protected files. Existing descriptor rights
govern subsequent file I/O. Ordinary callers retain the existing write policy.

## Native interface

`StorageCtl = 168`: `rdi` is the operation, `rsi` the index or ID, `rdx` the
output buffer or NUL-terminated target pathname. There is no IPC mount server
or `/dev` filesystem. All lifecycle operations use `KERNEL_VFS` directly.

| Operation | rsi | rdx | Success |
| --- | --- | --- | --- |
| 0: enumerate devices | zero-based index | 64-byte output record | 1 record, 0 end |
| 1: mount FAT32 | device ID | target, fewer than 128 bytes | 0 |
| 2: list mounts | zero-based index | 160-byte output record | 1 record, 0 end |
| 3: unmount | mount ID | unused | 0 |

Device records are eight little-endian u64 fields: ID, parent, starting LBA,
sector count, operations, partition discovery ordinal, layout, sector size.
Operations are read=1, write=2, flush=4, FAT32 eligibility=8. Layout is
unknown/raw=0, superfloppy=1, MBR=2, GPT=3. Ordinals count discovered nonempty
partitions, rather than physical table slots. The physical disk has ordinal 0.

Mount records contain four little-endian u64 fields (mount ID, device ID,
protected flag, path length) followed by 128 zero-filled path bytes. RAM root
has device ID zero. Records are explicitly serialized, so kernel padding is
never copied to callers. Enumeration/listing is a per-call snapshot; a later
mutation can change indexes. Use IDs for subsequent requests.

StorageCtl returns standard negative errno values, independent of legacy native
file-syscall error encoding: EPERM=-1, ENOENT=-2, EIO=-5, EFAULT=-14,
EBUSY=-16, EEXIST=-17, ENOTDIR=-20, EINVAL=-22, ENOSPC=-28,
EOPNOTSUPP=-95. A failed FAT probe reports EOPNOTSUPP because the existing
mount parser does not distinguish malformed FAT from probe I/O failure.

Commands from an authenticated root shell:

```text
storagectl devices
storagectl mounts
storagectl mount DEVICE_ID /mnt
storagectl unmount MOUNT_ID
```

An ordinarily auto-mounted boot volume appears in the list as protected and
cannot be detached through this interface. Ambiguous multi-FAT disks remain
unmounted at boot and their individual eligible partition IDs can be selected
explicitly. The default initramfs now contains an empty root-owned `/mnt`.
Other targets must already have been provisioned as empty secure directories.

## Mount lifecycle

Target validation rejects relative, noncanonical, overlong and NUL-containing
paths, system paths, missing/non-directory targets, insecure ownership/modes,
nonempty targets, occupied mountpoints, duplicate devices and overlapping
mount paths. Runtime stacking and mounts over child mounts are excluded.

The same `KERNEL_VFS` mutex used for file access covers validation, FAT probing,
publication, handle accounting, busy checks, sync and detach. Filesystem methods
are synchronous, return no filesystem references, and require exclusive VFS
access; ongoing operations therefore finish before a lifecycle mutation can
proceed. File and directory handles pin their mount. Duplicated descriptors
share a backend handle and the existing fd reference accounting releases it
only on final close, including process cleanup. A CWD is a pathname, not a
retained filesystem object.

Unmount checks protection and live handles, syncs the FAT root/allocation
metadata through the cache and virtio durability barrier, then removes the
mount. Any sync failure leaves the same mount ID, source and complete cache
installed, permitting access and retry. It reports I/O errors separately from
unsupported barriers. It does not roll back writes already accepted by the
device. There is no force-unmount.

Mount IDs and packed handles include nonwrapping mount incarnations, so stale
mount IDs or old handles cannot alias a later filesystem in a reused slot.
The fd table reserves two handle-kind bits; VFS uses 22 local-handle bits and
8 mount-tag bits. A slot retires after 32 mount incarnations rather than
wrapping. With only a root mount this permits 224 runtime mount lifecycles per
VFS instance; protected boot mounts consume slots, reducing that bound. Further
mounts fail with ENOSPC. Existing backend-local handle generation tags remain
in use (16 generation bits); this does not redesign their existing wrap policy.

## Phase 3.75 timeout investigation

The old evidence (`target/partition-final-phase3.75.log` and
`target/wiseowl-phase375-serial.log` as preserved in the earlier task) lacked
the final Wise Owl verdict but showed working trusted `/state`, service
registration, reads/writes and recovery CLI commands. There was no observed
storage panic or reported block error in that timeout.

Before runtime changes, `timeout 300 bash tools/test.sh phase3.75` reproduced
the failure: `target/runtime-before-phase375.log`, `.exit` (1), and
`runtime-before-phase375-serial.log`. The last successful command was another
`wiseowl-indexctl scan`; the injected verdict was never reached.

A temporary detached worktree at unchanged HEAD `232ec79c4` supplied the
pre-partitioned-storage kernel and storage crates. It used the current test
harness (virtio-rng and deterministic disk generation) and existing generated
embedded artifacts, including the locally supplied hello-linux ELF. An initial
300-second **build-only** timeout produced no VM comparison and is not counted:
`runtime-baseline-phase375.exit` is 124 and its empty `.log` belongs to that
attempt. The stale copied serial file from that attempt was deleted.

Then the baseline kernel was built separately with
`SUNLIGHT_INJECT_PHASE=wiseowl3.75 cargo build -p sunlight-kernel --features key_inject`
and packaged as `target/runtime-baseline.iso`; see `runtime-baseline-kernel.log`.
A fresh operator-generated `runtime-baseline-state.img` was booted for exactly
90 seconds with QEMU, two CPUs, 1024 MiB, KVM, legacy virtio-blk,
virtio-rng and the same USB mouse configuration. The result is
`runtime-baseline-phase375.result`: **FAIL: no final marker within 90 seconds**.
Its serial log shows `/state` mounted, Wise Owl services registered, successful
scan/status/search commands, and no final verdict. The comparison worktree was
removed without changing HEAD.

The bounded VM command was equivalent to the following (the reproduction
controller terminated QEMU at 90 seconds and checked the serial marker):

```sh
bash tools/state-disk.sh target/runtime-baseline-state.img
timeout 90 qemu-system-x86_64 -cdrom target/runtime-baseline.iso \
  -serial file:target/runtime-baseline-phase375-serial.log -display none \
  -m 1024M -smp 2 -enable-kvm \
  -device virtio-rng-pci,disable-modern=on \
  -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
  -drive id=hd0,file=target/runtime-baseline-state.img,if=none,format=raw,cache=writeback \
  -device virtio-blk-pci,disable-modern=on,drive=hd0 -no-reboot -no-shutdown
```

The sequence contains 8,574 injector entries, confirmed by the baseline's
`[KBD] Key injection enabled ... 8574 scancodes` marker. At the configured
100 Hz it alone needs at least 85.74 seconds after injection starts. Startup,
serial work and timer delays leave insufficient guaranteed margin in the
90-second wall-clock harness. The larger rebuilt embedded image also affects
boot time; generated artifacts are not claimed byte-identical between runs.

Thus the timeout is reproduced **before storage changes**, supporting an
independent timing/gate limitation rather than evidence of a storage regression.
This is not a complete Wise Owl service/recovery verification: the sequence was
not observed through its verdict, and nonzero injected/recovery command results
were seen. Neither bounded run is counted as passing. Increasing a deadline
alone would not establish that all recovery assertions are correct.

## Verification and evidence

Run commands from the repository. Generated logs, images and binaries stay in
`target/`; the test scripts use mtools/dosfstools and require no sudo.

| Command | Result / evidence |
| --- | --- |
| `CARGO_TARGET_DIR=target/runtime-host cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-ipc -p sunlight-kv -p sunlight-tls --target x86_64-unknown-linux-gnu` | PASS: block 15, FAT 9, VFS 77, IPC 93 + 9 integration, KV 27; TLS host has zero tests. `target/runtime-regressions-final.log` |
| `python3 tools/test_runtime_storage.py` | PASS all three layouts; `runtime-qemu.log`, `runtime-storage/build.log`, `runtime-storage/*.serial.log`, `*.qemu.log` |
| `SUNLIGHT_RUNTIME_STORAGE_TEST=0 python3 tools/test_partitioned_storage.py` | PASS six boots; exact persisted contents and guards. `target/runtime-partition-gate.log`, `.exit` (0), `partition-storage/*` |
| `SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 bash tools/test.sh phase3.0` | PASS; `target/runtime-phase3.0.log`, `.exit` (0) |
| `... bash tools/test.sh phase3.5` | PASS; `target/runtime-phase3.5.log`, `.exit` (0) |
| `... bash tools/test.sh phase6.5.3` | PASS all three shell commands; `target/runtime-phase6.5.3.log`, `.exit` (0) |
| `... bash tools/test.sh phase5x.4` | PASS legacy marker gate only; `target/runtime-phase5x.4.log`, `.exit`; not real TLS evidence |
| `... bash tools/test.sh phase3.75` | FAIL again after 90 seconds; `target/runtime-phase3.75.log`, `.exit` (1), `wiseowl-phase375-serial.log`; never counted as passing |
| Normal CLI/kernel/ISO rebuild below | PASS; `target/runtime-normal-build.log`, `.exit` (0) |
| `python3 tools/network_tls_gate.py --cpus 2 --timeout 180 --account-broker-storage --evidence target/runtime-tls-kv` | PASS real native TLS, KV restarts and private-store denial; `target/runtime-tls-kv-gate.log`, `.exit` (0), `runtime-tls-kv/serial.log`, `network.pcap` |

The gate rows abbreviated with `...` use the same two environment assignments:

```sh
for phase in phase3.0 phase3.5 phase6.5.3 phase5x.4 phase3.75; do
  SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 bash tools/test.sh "$phase"
done
```

After the fixtures/gates, the normal tool, kernel and ISO were restored:

```sh
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-utils --bin storagectl --release
SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 \
  cargo build -p sunlight-kernel
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightos.iso
```

`bash -n tools/test.sh`, Python bytecode checks for both storage fixture scripts,
targeted rustfmt checks for the new modules and VFS, and `git diff --check`
also pass. The original storage audit's SHA-256 is
`93876b6611abdf77d2f30b5485fc6227addfebf200e88d73686adb853bd9d86b`.

Focused host tests cover root-without-capability, capability-without-root,
unrelated capabilities, unknown/stale/replacement/exhausted device IDs,
partition relationships/operations, invalid/system/occupied/duplicate targets,
file and directory handle busy rejection, sync I/O failure preserving mount and
cache, retry, exact binary contents after unmount/remount, stale handles/mount
IDs, and mount-tag retirement within the kernel fd bit budget.

The legacy Phase 5.x.4 expected messages are printed directly by the
`SUNLIGHT_INJECT_PHASE` block in `kernel/src/main.rs`; its passing result is
only a marker regression. It cannot substantiate the earlier milestone's
claimed example.com handshake. Actual native TLS/KV verification uses
`python3 tools/network_tls_gate.py --cpus 2 --timeout 180 --account-broker-storage --evidence target/runtime-tls-kv`
on the restored normal ISO. This starts a real local TLS server and checks
UnknownIssuer rejection, CA installation through KV, verified requests,
hostname mismatch rejection, SOCKS requests, KV daemon restart, 1K/4K broker
records, replayed deletion and denial of direct broker-store reads. See
`target/runtime-tls-kv-gate.log`, `.exit`, and `runtime-tls-kv/serial.log` /
`network.pcap` for the result.

This actual native gate passed on the normal image. Serial evidence includes
`client config ready (3 roots)`, `hs_OK host=10.0.2.2`, verified long-hostname
requests and `InvalidCertificate` failures for the negative cases. The broker's
storage seed/check/delete/absent probes all exited 0 across two KV restarts;
the direct `cat /var/lib/sunlight-kv/broker.store` attempt exited 1. These are
in-session KV replay checks on the Live ISO, not migration or reboot durability.
Phase 3.75 is the only unresolved gate from this verification set.

The native fixture begins with only protected RAM root in the mount table.
The same embedded ELF runs first as a real root caller without StorageAdmin
via a test-only loader alias; all four operations must return EPERM, even with
invalid request pointers. A separately spawned root `storagectl` receives the
real capability and checks enumeration, protected-root rejection, unknown IDs,
`/state` denial, mount/source listing, duplicate rejection, fixture reads,
exact write/read, file and directory sync, busy unmount, duplicate-fd pinning,
close/unmount, stale mount ID rejection, remount, exact contents and final
unmount. Host mtype verifies the same bytes; partition guard sectors must stay
unchanged. All generated volumes carry SUNSTATE, so label-based trust escalation
would fail the native assertions.

Native serial evidence includes:

```text
[STORAGE-RUNTIME] unauthorized PASS
[STORAGE-RUNTIME] mount write read sync busy close unmount remount exact PASS
[SCHED] process_mark_finished ... name='storagectl' reason=exit code=0
```

The first runtime harness attempt completed the privileged probe with exit 0
but lacked a namespace entry for its unauthorized alias and hit a shell
foreground-completion timeout. It is not counted. The corrected fixture adds
the alias entry and emits its assertions through DebugLog as well as stdout,
requiring both the explicit success marker and a normal process exit. Final
logs also contain the ordinary TTY command/exit markers. This avoids depending
solely on unrelated shell completion IPC for the storage result.

## Changed files in this milestone

- `sunlight-block/src/registry.rs`, `src/lib.rs`: bounded runtime registry,
  monotonic identities, partition metadata and host tests.
- `ipc/src/storage.rs`, `src/lib.rs`: syscall ABI and shared authorization test.
- `kernel/src/storage.rs`: bounded registry-backed FAT mounting and errors.
- `kernel/src/main.rs`: register and retain the virtio disk and all discovered
  partitions; connect boot sources; explicit initially-unmounted fixture.
- `kernel/src/arch/x86_64/syscall.rs`: real-credential gate, KERNEL_VFS lifecycle,
  record serialization and scoped runtime file-write authority.
- `kernel/src/process/spawn.rs`, `kernel/build.rs`: embedded control tool and
  test-only loader alias; preserve the existing per-binary capability model.
- `sunlight-fs/src/vfs.rs`, `src/error.rs`, `src/ramfs.rs`,
  `src/vfs/storage_tests.rs`: lifecycle, owned target paths, handle/mount tags,
  secure mount directory, control-tool entries and focused tests.
- `sunlight-fat/src/fat.rs`: preserve underlying sync errors.
- `sunlight-virtio/src/blk.rs`: report and enforce read-only media support.
- `sunlight-utils/Cargo.toml`, `src/bin/storagectl.rs`: control CLI and explicit
  native test feature.
- `tools/test_runtime_storage.py`: no-root native interface/fixture gate.
- `docs/partitioned-storage.md`, `docs/runtime-storage.md`: policy and evidence.

The pre-existing uncommitted partitioned-storage changes and original audit
were retained. This milestone does not add USB storage, additional transports,
hot removal, an installer, automatic KV migration, a replacement userspace VFS,
a new filesystem, a read-only FAT mount mode, power-loss recovery guarantees
or real-hardware validation.
