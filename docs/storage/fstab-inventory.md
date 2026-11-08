# Boot fstab and read-only block inventory

This extends the existing mount configuration and storage IPC. It adds no
filesystem, installer, partition editor, formatting, USB protocol changes or
replacement VFS. QEMU results do not constitute physical USB validation.

## Configuration audit and lifecycle

Before this change, `/etc/fstab` was a byte literal in
`sunlight-fs/src/ramfs.rs::INITRAMFS`:

```text
/dev/sda1 /boot bootfs defaults
/dev/ram0 / ramfs defaults
```

`sunlight-fs/src/fstab.rs::parse_fstab` parsed four whitespace-separated fields,
ignoring incomplete lines, extra fields and entries beyond eight. At startup,
`services/vfs_server/src/main.rs::mount_from_fstab` read that embedded file into
its own legacy VFS. It implemented RAM root and the small boot share-page
snapshot only. `/dev/sda1` was a legacy placeholder, not a block-device lookup.
No entry reached StorageCtl or the authoritative `KERNEL_VFS`. Kernel boot
mount selection was separate (see the historical storage audit).

The single configuration now lives at **`sunlight-fs/etc/fstab`**, embedded as
**`/etc/fstab`** by the same INITRAMFS entry. Editing it requires rebuilding the
embedded tools/services, kernel and ISO. It is not a host `/etc/fstab`, a disk
root selector, a writable persistence setting or a hotplug watcher.
The existing parser is shared with the utilities through a Rust source module
reference, avoiding linking the entire VFS/embedded RAMFS into allocation-free
CLIs. There is one parser implementation and one configuration file.

Init synchronously spawns `/bin/storagectl fstab` using the existing native
spawn/credential path before launching its ordinary base services. It waits for
the exit status. Storagectl reads `/etc/fstab` through native KERNEL_VFS file
syscalls, validates the whole file before issuing mounts, enumerates the device
registry over StorageCtl IPC, resolves IDs, and submits each FAT mount using
the existing privileged MOUNT_FAT32 operation. Root **and** an explicit
StorageAdmin grant remain mandatory; the embedded root storagectl receives
that existing per-binary grant even when launched by init's legacy lookup
profile. An unrestricted lookup profile alone never authorizes a mount.

The legacy vfs_server still uses the same parser for its two legacy entries;
FAT disk entries are handled exclusively by storagectl/KERNEL_VFS. Kernel
legacy boot mounting defers a GPT volume explicitly named in a valid fstab to
storagectl. An invalid fstab suppresses ordinary GPT boot automount so rejected
options cannot silently fall back to boot defaults; storagectl reports the
configuration failure. Legacy raw-media boot selection is unchanged.
Default fstab leaves the Live ISO RAM root, ordinary boot selection
and explicit trusted-state selection intact. A different boot mount occupying
`/mnt` can still conflict with a configured mount and is reported as an error.

## Format, identifiers and errors

Exactly four fields, at most eight entries, at most 4096 UTF-8 bytes. Empty
lines, whitespace and `#` comments are accepted. Escaped spaces, six-field
Linux dump/pass values, and mount options outside the list below are rejected.
Errors include the line number and reason; excess entries are errors, not
truncated configuration.

```text
# source                                      target type  options
/dev/sda1                                     /boot  bootfs defaults
/dev/ram0                                     /      ramfs  defaults
PARTUUID=12345678-1234-5678-9abc-123456789abc     /mnt   fat32 rw,nofail
```

The two legacy entries accept exactly those source/target/type/options values.
They describe built-in RAM/snapshot views, do not resolve `/dev` nodes, and do
not require block media to be present. Optional/required rules below apply to
runtime FAT32 entries only; absent legacy boot snapshots retain their existing
non-fatal behavior.
Runtime entries accept **fat32** only, **rw** (or **defaults**, equivalent to
rw), optionally followed/preceded by **nofail**. Duplicate and unknown options,
including `ro`, `auto`, `noauto`, `exec`, ownership and permission options, fail.
The FAT stack does not implement a read-only mount mode.

Runtime sources must use `PARTUUID=` plus a nonzero canonical GPT GUID
(`8-4-4-4-12` hex digits, case-insensitive digits; uppercase literal prefix).
GPT discovery already validates both tables, checksums, unique nonzero GUIDs
and extents; this change retains the validated unique/type GUID metadata.
It does not change discovery acceptance rules. MBR disk signatures, filesystem
UUIDs, labels, `/dev` names and boot-local runtime IDs are not supported fstab
identities. Superfloppies and MBR devices remain available to manual storagectl
and inventory but cannot yet be persistent fstab sources. A GUID is persistent
identification, **not authenticity**. Cloned GUIDs across disks fail as ambiguous.

Targets are canonical absolute paths restricted to `/mnt` or descendants, less
than 128 bytes. Components use ASCII letters, digits, dot, underscore or hyphen;
empty components and `.` / `..` components are rejected. The existing kernel validator additionally requires an empty,
root-owned directory without group/other write permission, no occupied target,
no duplicate source, no stacked mount and available mount capacity. Directories
are not automatically created. Duplicate fstab sources (including differing
GUID case) and targets fail before any mount. Reapplying an already-mounted
identical source and target succeeds; a conflicting source or target fails.
A pass is sequential, not transactional: a later error does not undo earlier
successful entries.

- **Optional (`nofail`)**: a source absent from the registry is logged and
  skipped. The pass succeeds; Live ISO boot and login continue. No retries,
  waiting for future attachment, or automatic hotplug mounting are added.
- **Required (default)**: a missing source fails the pass (storagectl exits 1).
  Init logs `REQUIRED FAILURE: boot degraded` and continues boot so the Live ISO
  remains usable for diagnosis. This is not an emergency-shell or boot-stop
  mechanism. Inspect diagnostics before depending on configured storage.
- Malformed configuration, ambiguous IDs, authorization denial and mount/I/O
  errors fail even with `nofail`. Retired media still in the registry is not
  treated as absent. Other independent entries are attempted after runtime
  failures; a parse error prevents the whole pass.

## State trust

No fstab entry, label (including SUNSTATE), partition UUID, or inventory record
grants `/state` trust. Ordinary mounts cannot target `/state`. The existing sole
explicit mechanism is the operator's `SUNLIGHT_TRUST_LEGACY_STATE=1` kernel build
opt-in for the first legacy virtio SUNSTATE **superfloppy**. The operator's
provisioning choice is its provenance; the label only checks compatibility.
USB and partitioned volumes do not gain this trust. There is no new UUID-based
trust provisioning or authentication mechanism. Keep this build opt-in disabled
for normal Live ISO builds and untrusted/replaceable media.

## Inventory CLI and structured interface

```sh
lsblk
lsblk --json
storagectl devices            # retained privileged legacy listing
storagectl devices --json     # same v1 presentation; GUI should use public IPC
storagectl mounts
storagectl fstab               # explicit reapply/diagnosis
```

`lsblk` is a separate embedded utility with **no StorageAdmin grant**; native
spawn strips any inherited StorageAdmin for this executable. It only issues
INVENTORY and INVENTORY_MOUNTS operations on the existing StorageCtl boundary.
Those operations copy registry/VFS metadata under KERNEL_VFS serialization;
they never probe a disk, write, mount, unmount, format or edit partitions.
The four original StorageCtl operations remain privileged, including their
pointer-validation ordering. Public inventory does not change their checks.
Physical disks, FAT candidates and non-FAT partitions are listed, mounted or
unmounted. Currently the registered transports are legacy virtio and USB MSC.
No private kernel memory or ad hoc device probing is used by the CLI.

The human table shows runtime ID, parent (0 for a physical disk), transport,
capacity bytes, logical sector bytes, partition scheme, known filesystem,
read-only/removable indicators, trusted-state status, persistent ID and mountpoint. `?` means unavailable and
`-` means unmounted. It is for humans. **Future GUI/tooling must consume the
versioned JSON or the structured IPC records, never parse the table.**

The machine-readable contract is also available as
[`storage-inventory-v1.schema.json`](storage-inventory-v1.schema.json).
JSON v1 has one root object: `{"schema_version":1,"devices":[...]}`. All keys
below are always emitted. Numeric fields are unsigned integers (up to u64);
consumers must preserve their precision. `null` explicitly means unavailable
or inapplicable; an empty devices array is valid.

| Device key | v1 meaning |
| --- | --- |
| `runtime_id` | Boot-local opaque device ID; never persist as configuration |
| `parent_id` | Physical parent's runtime ID; null for a physical disk |
| `persistent_id` | `PARTUUID=<canonical lowercase GUID>` for validated GPT children; otherwise null |
| `transport` | `virtio` or `usb`, from registry backend metadata |
| `removable` | null: no verified per-device removable property is currently exported; USB does not imply removable |
| `capacity_bytes` | blocks × logical sector size; null on overflow |
| `logical_sector_size` | Bytes; all currently registered media use 512 |
| `partition_scheme` | `superfloppy`, `mbr`, `gpt`, or null for unrecognized media |
| `partition_ordinal` | One-based discovery ordinal for children, null for physical disks; not a GPT/MBR on-disk slot number |
| `start_lba`, `blocks` | Validated device extent and capacity in logical sectors |
| `partition_type` | Canonical GPT type GUID, otherwise null |
| `partition_name` | null: names are not retained/exported |
| `filesystem` | `fat32` when mounted from this block source, otherwise null; unmounted candidates have not been probed/verified |
| `fat32_candidate` | Discovery eligibility bit, not proof of a valid FAT filesystem |
| `mountpoint` | Kernel VFS path or null when unmounted |
| `mount_protected` | Kernel boot mount cannot be detached through runtime unmount; null when unmounted |
| `write_supported` | Registry exposes WRITE; not hardware write-protection discovery |
| `read_only` | true if WRITE unavailable, otherwise null (writability not independently verified) |
| `mount_read_only` | false for current mounted FAT32 rw filesystem mode; null when unmounted. This is not file-write permission: protected boot paths and ordinary caller write policy still apply |
| `trusted_state` | true only for a protected boot `/state` mount; false otherwise; this field describes policy and grants nothing |

The inventory record ABI adds operation 4 (96 bytes): the first 64 bytes match
the legacy device record, followed by the GPT unique GUID at bytes 64..80 and
type GUID at 80..96, in on-disk byte order; all-zero GUIDs mean unavailable.
Operation 5 returns the unchanged 160-byte mount record. Other ABI constants
remain unchanged. Enumeration is bounded at 258 records. Each operation is a
serialized snapshot; a complete multi-call listing is not an atomic snapshot
across concurrent administrative mount changes. Registry records do not prove
current media health or presence: USB retirement/liveness is not yet exported.
Errors produce nonzero exit;
a partial JSON stream on query/output failure must not be accepted.

`df` is not provided: VFS has no accurate used/free-space reporting interface.
Disk capacity is not filesystem free space, so inventory does not guess usage.

## Verification and evidence

Commands/results for this change are recorded below after execution. Evidence,
QEMU argv, JSON snapshots, serial logs, images and isolated fixture ISO for the
new harness remain under `target/fstab-inventory/`. Existing regression scripts
also retain their usual `target/usb-storage/`, `target/runtime-storage/` and
`target/partition-storage/` evidence. The fstab harness restores the original source
file in a `finally` block. Builds must not run concurrently with this temporary
configuration edit. Normal embedded tools/services and ISO must be rebuilt
after fixture/regression runs.

Wise Owl Phase 3.75 remains **unresolved** until its complete final native PASS
marker is observed. The recorded pre-storage baseline timeout and earlier USB
final timeout are failures, not passing results. No physical USB hardware has
been validated by these QEMU fixtures. No commit or push is performed.

Implementation files for this extension (pre-existing USB implementation changes
in the worktree are retained):

- `sunlight-fs/etc/fstab`, `src/fstab.rs`, `src/ramfs.rs`: sole embedded config,
  shared strict parser/resolver, legacy entries and inventory executable entry.
- `services/init/{Cargo.toml,src/main.rs}`, `Cargo.lock`: synchronous existing
  storagectl boot lifecycle through native spawn/wait; libc is an existing
  workspace dependency, with no new third-party dependency.
- `services/vfs_server/src/main.rs`: consume the same fallible parser and size/
  UTF-8 checks for the legacy view.
- `sunlight-block/src/partition.rs`, `src/registry.rs`: retain validated GPT
  unique/type GUID metadata; no partition acceptance or I/O behavior changes.
- `ipc/src/storage.rs`, `kernel/src/arch/x86_64/syscall.rs`: public metadata-only
  operations, unchanged privileged-operation checks, explicit embedded-tool
  grant/drop behavior and record serialization.
- `kernel/{build.rs,src/main.rs,src/process/spawn.rs}`: embed inventory tool,
  delegate configured GPT boot mounts, and disabled-by-default native test
  injection/alias.
- `sunlight-utils/{Cargo.toml,src/lib.rs,src/bin/storagectl.rs,src/bin/lsblk.rs,
  src/storage_inventory.rs,tests/data/inventory-v1-unavailable.json}`: fstab
  application, read-only CLI, JSON/table rendering, tests and schema fixture.
- `tools/test_fstab_inventory.py`: native stable-ID mounting, listing, absence,
  authorization, exact disk preservation and lossless test-only output capture.
- `tools/test_usb_storage.py`: defer existing HID QMP motion until storage probe
  completion to prevent startup diagnostics interleaving; same exact motion
  assertion and deadline, no USB protocol changes.
- `docs/storage/fstab-inventory.md`, `docs/storage-inventory-v1.schema.json`,
  `docs/storage/runtime-storage.md`, `docs/storage/partitioned-storage.md`, `docs/storage/usb-storage.md`:
  configuration, API, trust, limitations and evidence.

### Executed commands and results (2026-11-07)

Run from the repository root. Host rows use the explicit Linux target because
this workspace defaults to freestanding `x86_64-unknown-none`.
For the regression-script/phase rows, the parent environment was
`SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0`,
with `SUNLIGHT_INJECT_PHASE` unset. Each existing harness supplies its own
explicit disposable test switches (and the trusted state-image opt-in where
needed). No expected markers or deadlines were weakened.

| Exact command | Result / evidence |
| --- | --- |
| `CARGO_TARGET_DIR=target/fstab-host cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-ipc -p sunlight-kv -p sunlight-tls --target x86_64-unknown-linux-gnu` | PASS, exit 0: block 29, FAT 9, VFS 78, IPC 94 + 9 integration, KV 27; TLS has zero host tests. `target/fstab-inventory/host-regression.log`, `.exit` |
| `CARGO_TARGET_DIR=target/fstab-host cargo test -p sunlight-usb-mouse --target x86_64-unknown-linux-gnu --lib` | PASS 7, exit 0. `target/fstab-inventory/hid-host.log`, `.exit` |
| `CARGO_TARGET_DIR=target/fstab-host cargo test -p sunlight-fs -p sunlight-utils --target x86_64-unknown-linux-gnu --lib fstab::tests` | PASS 4 shared parser/resolver tests in each crate, exit 0: field/option errors, size/count limits, duplicates, stable-ID reordering, missing optional/required media, ambiguity, forbidden state targets, invalid-config GPT deferral. `target/fstab-inventory/fstab-final.log`, `.exit` |
| `CARGO_TARGET_DIR=target/fstab-host cargo test -p sunlight-utils --target x86_64-unknown-linux-gnu --lib storage_inventory` | PASS 4, exit 0: public-wrapper mutation rejection, table/JSON, golden unavailable-metadata fixture, GUID byte order, JSON escaping, protected/trusted state, USB, overflow and read-only metadata. `target/fstab-inventory/inventory-final.log`, `.exit` |
| `CARGO_TARGET_DIR=target/fstab-host cargo check-host-libs` | PASS, exit 0, combined host library graph. `target/fstab-inventory/workspace-libs.log`, `.exit` |
| `python3 tools/test_fstab_inventory.py` | PASS final run, exit 0: virtio GPT, actual xHCI/USB GPT, absent optional device. `target/fstab-inventory/native-final.log`, `.exit`; `native/build.log`, `*.serial.log`, `*.command.json`, `*.inventory.json`, `*.table.txt` |
| `python3 tools/test_usb_storage.py` | PASS final run, exit 0: all five boots, exact mouse x=37/y=-11, storage authorization/lifecycle, host file bytes and guards, malformed/4Kn rejection. `target/fstab-inventory/usb-final.log`, `.exit`; `target/usb-storage/*` |
| `python3 tools/test_runtime_storage.py` | PASS, exit 0: all three virtio layouts, credential gate, mount/write/sync/busy/unmount/remount, exact host contents and guards. `target/fstab-inventory/runtime.log`, `.exit`; `target/runtime-storage/*` |
| `python3 tools/test_partitioned_storage.py` | PASS, exit 0: six boots, all three layouts, persistence/restart, exact contents and partition guards. `target/fstab-inventory/partition.log`, `.exit`; `target/partition-storage/*` |
| `bash tools/test.sh phase3.0` | PASS, exit 0: boot/VFS foundation and native RAMFS stat/mkdir. `target/fstab-inventory/phase3.0.log`, `.exit` |
| `bash tools/test.sh phase3.5` | PASS, exit 0: block/FAT/share-page boot. `target/fstab-inventory/phase3.5.log`, `.exit` |
| `bash tools/test.sh phase6.5.3` | PASS, exit 0: `ls /`, `mkdir /tmp/x`, `ls /tmp` all native shell completion markers. `target/fstab-inventory/phase6.5.3.log`, `.exit` |
| `bash tools/test.sh phase5x.4` | PASS, exit 0, **legacy printed-marker gate only; not actual TLS verification**. `target/fstab-inventory/phase5x.4.log`, `.exit` |
| `bash tools/test.sh phase3.75` | **FAIL / unresolved**, exit 1, existing 90-second deadline. Missing `[WISEOWL-3.75] native gate PASS`; explicitly trusted virtio `/state` mounted and services registered. No kernel fault/panic marker was found. Some injected/recovery commands returned nonzero; this is not a complete recovery verification. `target/fstab-inventory/phase3.75.log`, `.exit`, `.serial.log` |
| `python3 tools/check_workspace.py` | PASS initial run, all 95 checks, exit 0. `target/fstab-inventory/workspace.log`, `.exit`; final repeat recorded below |

The fstab native fixture disables `SUNLIGHT_RUNTIME_STORAGE_TEST` (0), so the
normal kernel handoff of an explicitly configured GPT source is exercised.
The existing test-only loader alias is available only with key injection and
the explicit fstab phase. The embedded fstab names a GPT PARTUUID, never a
runtime ID. A mounted FAT32 partition and an unmounted Linux-type partition
both appear in the complete captured table and schema-key-checked JSON; the
whole disk image must remain byte-identical after mounting/listing/reading.
The same root caller without StorageAdmin successfully reads public inventory
and receives EPERM for every original privileged operation, even with invalid
request pointers. Merely listing cannot create mounts or write media.
An explicit second `storagectl fstab` pass verifies idempotence.
The absent-device boot reaches login and completes both inventory commands,
producing an empty JSON devices array and an optional-missing diagnostic.

The test-only `lsblk` feature emits lossless hex chunks solely for evidence:
the existing TTY serial command log truncates long stdout. The first attempt
failed JSON capture for that reason and is **not a pass**
(`target/fstab-inventory/native-initial-truncated.log`). The final fixture
reconstructs complete output, parses JSON, checks all v1 schema keys, and
requires normal foreground command exits. Normal binaries contain no capture
hook.

The first USB regression attempt timed out in MBR despite storage/native shell
success: the early HID diagnostic was interleaved with display numeric output,
so its exact-motion parser could not recognize it. This failure is preserved
in `target/fstab-inventory/usb-initial/{gate.log,mbr.serial.log,mbr.command.json}`
and `target/fstab-inventory/usb.log` / `.exit` (1). The harness now injects QMP
motion only after foreground storage completion; the exact assertion and
120-second deadline are unchanged. The final five-boot run above passes.

Additional host attempts that are **not passes**:

- `CARGO_TARGET_DIR=target/fstab-host cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-ipc -p sunlight-utils --target x86_64-unknown-linux-gnu --lib`:
  broad utility tests failed in untouched commands (including wc/uniq), while
  comm/regex tests ran beyond 60 seconds. The hung test process was terminated
  with SIGTERM; Cargo exited 101. `target/fstab-inventory/host.log`.
- `CARGO_TARGET_DIR=target/fstab-host timeout 30 cargo test -p sunlight-utils --target x86_64-unknown-linux-gnu --lib wc::tests::empty_file`:
  independently reproduces an untouched output-spacing assertion failure,
  exit 101 (not a timeout). `target/fstab-inventory/utility-unrelated-failure.log`.
  No claim is made that the entire utility host suite passes, or that a separate
  unchanged-HEAD comparison of this suite was performed.
- An initial host invocation including all `sunlight-usb-mouse` targets failed
  with duplicate panic handlers when linking its freestanding binary into std's
  test harness, exit 101 (`target/fstab-inventory/host-final.log`, `.exit`).
  Its supported `--lib` host invocation and actual native HID/QMP checks pass
  as listed above.

The earlier pre-storage Phase 3.75 baseline timeout remains recorded in
`target/runtime-baseline-phase375.result`. Neither that baseline, the prior USB
final timeout, nor this final Phase 3.75 timeout is counted as passing. Wise Owl
and KV implementation/recovery logic were not modified to change this outcome.

Final-source and normal-image checks:

| Exact command/check | Result / evidence |
| --- | --- |
| `python3 tools/check_workspace.py` (same normal parent environment above) | PASS final repeat, all 95 platform-aware checks, exit 0. `target/fstab-inventory/workspace-final.log`, `.exit` |
| Normal tool/service/kernel/ISO commands below | PASS, exit 0. `target/fstab-inventory/normal-build.log`, `.exit`, `normal.sha256` |
| Final default feature / binary hook checks | PASS, kernel features exactly `["default"]`; kernel injection/runtime probe/fstab injector, storagectl probe, lsblk capture and HID debug markers absent from actual normal ELFs. Normal fstab source restored. `target/fstab-inventory/normal-hooks.log`, `.exit` |
| `python3 tools/network_tls_gate.py --cpus 2 --timeout 180 --account-broker-storage --evidence target/fstab-inventory/tls-kv` | PASS actual native TLS/KV, exit 0. `target/fstab-inventory/tls-kv.log`, `.exit`, `tls-kv/serial.log`, `network.pcap`, `qemu.log` |
| `git diff --check` | PASS; `target/fstab-inventory/source-checks.log`, `.exit` |
| `python3 -m py_compile tools/test_fstab_inventory.py tools/test_usb_storage.py tools/test_runtime_storage.py tools/test_partitioned_storage.py` | PASS; same source-check evidence |
| `bash -n tools/test.sh` | PASS; same source-check evidence |
| `rustfmt --check --edition 2021 --config skip_children=true sunlight-fs/src/fstab.rs sunlight-block/src/partition.rs sunlight-block/src/registry.rs ipc/src/storage.rs sunlight-utils/src/storage_inventory.rs sunlight-utils/src/bin/lsblk.rs sunlight-utils/src/bin/storagectl.rs services/init/src/main.rs services/vfs_server/src/main.rs` | PASS; same source-check evidence |
| JSON schema syntax / complete native v1 required keys / restored fstab checks | PASS with Python standard-library JSON parsing and assertions; same source-check evidence. This is not a claim that a third-party JSON Schema validator was installed/run |

The actual native TLS/KV run observed UnknownIssuer rejection, CA installation,
eight verified large transfers, three verified slow requests, SOCKS and direct
requests, hostname mismatch rejection, two KV restarts, broker seed/check/delete/
absent checks (exit 0) and private-store direct-read denial (expected exit 1).
These are Live ISO session/restart checks, not KV migration, reboot durability,
filesystem crash recovery or physical USB validation. No kernel fault/panic
marker appears in that serial log.

Normal artifacts were restored with these exact commands. The parent environment
had the three storage/trust variables set to 0 as above and no injection phase;
no test features were passed:

```sh
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-usb-mouse --release
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-utils --bin storagectl --bin lsblk --release
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-init -p sunlight-vfs-server --release
env -u SUNLIGHT_INJECT_PHASE SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 \
  cargo build -p sunlight-kernel
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightos.iso
```

Normal ISO SHA-256:
`5c87010f03835a6bed978979b0fa9f6a81a6d0ae5e103f542a6d3ff04cc68463`.
The original audit SHA-256 remains
`93876b6611abdf77d2f30b5485fc6227addfebf200e88d73686adb853bd9d86b`.
Bootloader configs, toolchain and linker configuration are unchanged. Generated
artifacts stay in `target/`. No commit or push was performed.

Remaining limits: GPT-only persistent fstab sources; compile-time embedded
configuration; pre-existing secure empty targets; sequential/nontransactional
application; required failures mark degraded boot rather than halt it; no
hotplug retry, filesystem read-only mount mode, verified removable/liveness
property, filesystem usage (`df`), partition edits, formatting or installer.
The existing USB profile and FAT interoperability limitations remain. All USB
results here are QEMU results. **Wise Owl Phase 3.75 remains unresolved.**
