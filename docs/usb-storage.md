# First USB mass-storage path

This implementation adds actual USB I/O. It does not use virtio-blk as a USB
stand-in. It reuses `BlockDevice`, MBR/GPT discovery, the runtime device
registry, privileged `storagectl`, FAT32 and the authoritative `KERNEL_VFS`.
It does not alter Live ISO root selection, `/state` trust, storage authorization
or KV behavior. USB volumes are registered without implicit mounting. Explicit GPT PARTUUID
fstab entries may now mount through init-launched storagectl; see
[Boot fstab and inventory](fstab-inventory.md).

## Starting capabilities and missing pieces

Before this change, `services/sunlight-usb-mouse/src/usb_mouse.rs` was the only
xHCI driver. It ran in ring 3 through XhciInfo/MapMmio/DmaAlloc, owned the entire
controller, enabled one slot and enumerated the first attached root port. It
implemented EP0 setup/data/status TRBs and a single Normal-TRB interrupt-IN
ring, including endpoint reset/dequeue recovery. It had no bulk-IN/OUT endpoint
contexts, generic bulk transfer submission, BOT wrappers, SCSI, USB block
adapter or USB registry binding. Completion waits matched endpoint only, and
could not safely support concurrent transfer owners.

BOT needed two bulk contexts/rings, DMA buffers with controlled lifetime,
completion matching by slot/endpoint/TRB, actual transfer lengths, deadlines,
clear-halt and class-reset recovery, descriptor validation, CBW/CSW validation,
SCSI initialization/read/write/barrier commands and storage identity routing.

`kernel/src/usb_xhci.rs` adapts the existing xHCI initialization, control and
producer/event-ring logic into a kernel bulk transport. Kernel ownership is
necessary for synchronous `BlockDevice` I/O without a ring-3 IPC/interrupt
proxy. A controller reserved by storage is excluded from the HID driver's
PCI BAR sizing and MMIO grant. Unsuccessful non-MSC probes halt the controller
before releasing it to the existing HID driver. DMA buffers are pinned for the
kernel lifetime even after failed probes/timeouts; a timeout disables further
submission through that transport. A controller that cannot be halted stays
reserved. MMIO has a separate uncached mapping.

## Supported profile and limits

- One boot-discovered USB storage device, on a **dedicated PCI xHCI controller**,
  with exactly one connected root device. A separate xHCI controller can serve
  the existing HID mouse. Shared-controller HID/MSC, hubs, hotplug/re-enumeration,
  EHCI/OHCI/UHCI and additional USB storage devices are not implemented.
- USB2 high speed only, EP0 max packet 64. No USB3 companions, streams, bursts,
  full/low-speed devices, scratchpad-requiring controllers or IOMMU integration.
- First configuration, exactly one interface numbered zero, alternate setting zero,
  class/subclass/protocol `08/06/50`, exactly one 512-byte bulk-IN and one
  512-byte bulk-OUT endpoint. Composite, alternate and other class profiles,
  extra class descriptors and malformed descriptors are rejected.
- LUN 0 only. GET MAX LUN is unnecessary for this restricted scope; other LUNs
  are never addressed. UAS and CBI are unsupported.
- INQUIRY, TEST UNIT READY, READ CAPACITY(10), READ(10), WRITE(10),
  SYNCHRONIZE CACHE(10), fixed-format REQUEST SENSE. Only direct-access devices
  and exactly 512-byte logical sectors are accepted. READ CAPACITY(16) sentinel
  geometry is rejected. One 512-byte sector is transferred per read/write.
- One outstanding TD, synchronous polling, two-second calibrated TSC deadline
  per wait; uncalibrated clocks fail immediately. Separate bulk rings, 16 TRBs
  each, link/cycle wrap, 16-entry event ring. DMA buffers are page-aligned and
  bounded to 512 bytes, so a TRB never crosses a 64 KiB boundary.
- CBW tags increase without wrapping. CBW=31 bytes and CSW=13 bytes are mandatory.
  Signatures, tag, actual length, residue and status are checked. Short data is
  never block success. Read data is staged until SCSI success. Data stall clears
  the affected halt before status; status stall gets one clear/retry. Protocol
  failure runs at most one class reset plus both halt clears for future commands.
  A possibly executed write is never replayed. Failed CSW requests sense.
- Initial unit attention/becoming-ready gets at most three TUR attempts before
  registration. Every runtime read/write/flush checks readiness. Any runtime
  command failure retires this media incarnation until reboot, including unit
  attention, not-ready, removal and barrier failures. There is no automatic
  media replacement under an old device ID. Enumeration may retain the old ID
  after retirement, but it cannot successfully perform I/O.
- USB bypasses the virtio write-back block cache to avoid serving cached sectors
  after media changes. FAT still holds normal mounted filesystem metadata.
  USB writes and SCSI sync errors propagate; no durability is fabricated when
  SYNCHRONIZE CACHE is unsupported. No write-protect discovery or read-only FAT
  mount mode is added; write-protected media will fail writes.

The retained virtio cache is boxed inside the kernel disk enum, keeping the
runtime mount syscall within its 32 KiB kernel stack. An initial inline enum
made `storage::mount` reserve 0x8518 bytes before nested calls and corrupted
process cleanup. That failed attempt is preserved in
`target/usb-initial-stack-fault.serial.log` and is not counted as a pass.
After boxing, the same function reserves 0x138 bytes
(`target/usb-mount-stack.txt`); the native gate also
requires the normal shell completion marker and rejects kernel fault markers.
An earlier profile rejection was traced to QEMU auto-inserting a full-speed
hub on the last free port; explicit root `port=1` avoids that unsupported
configuration. The diagnostic attempt is in `target/usb-storage/debug.serial.log`.

Registry IDs remain monotonic and boot-local. USB disk/partition records carry
operation bit 16 as transport metadata (`storagectl devices` prints `usb=1`),
without granting authority. Partition parent IDs route to the correct backing
disk; identical extents on virtio and USB cannot alias. Registry capacity covers
two disks with up to 128 partitions each. The syscall record remains 64 bytes
and sector size is 512 for every registered disk.

USB labels, including SUNSTATE, never authorize `/state`. Runtime mounts still
require uid 0 and StorageAdmin and use `/mnt` (or a validated existing directory
below it). Default USB discovery needs no test switch. The existing explicit
runtime fixture suppresses only virtio boot mounting for disposable tests.

## Reproduction

Host protocol tests use a scripted fake transport only; all native I/O uses
actual QEMU xHCI and `usb-storage`:

```sh
python3 tools/test_usb_storage.py
```

The script builds test-only storagectl and key injection into an isolated ISO,
creates disposable SUNSTATE superfloppy/MBR/GPT images, exercises real native
credentials, device enumeration, partition bounds, protected-root and `/state`
denial, `/mnt` mount/source listing, reads, writes, file/directory sync, busy
and duplicated-descriptor unmount checks, unmount/remount, stale mount rejection
and exact contents. Host mtype independently checks `guest durable contents\n`;
all bytes outside the selected partition must remain unchanged. Malformed media
must remain unmounted and byte-identical, and 4Kn media must be rejected before
block registration. The existing HID driver must initialize on its own
controller and decode QMP-injected relative motion exactly as x=37, y=-11.
Each QEMU argv is saved as `target/usb-storage/*.command.json`.

The positive VM command below includes the QMP socket used for HID replay.
The executed argument vector (with absolute paths) is preserved in
`target/usb-storage/mbr.command.json`; KVM was enabled on this host:

```sh
qemu-system-x86_64 -cdrom target/usb-storage/usb.iso -boot d \
  -serial file:target/usb-storage/mbr.serial.log -display none -m 1024M -smp 2 \
  -device virtio-rng-pci,disable-modern=on \
  -device qemu-xhci,id=hid -device usb-mouse,bus=hid.0 \
  -device qemu-xhci,id=storage,p2=1,p3=0 \
  -drive id=usb0,file=target/usb-storage/mbr.img,if=none,format=raw,cache=writeback \
  -device usb-storage,bus=storage.0,port=1,drive=usb0,removable=on,logical_block_size=512,physical_block_size=512 \
  -qmp unix:target/usb-storage/mbr.qmp,server=on,wait=off \
  -no-reboot -no-shutdown -enable-kvm
```

There is no virtio-blk device in that command. The emulated CD drive only loads
the kernel/embedded RAM root. The writable fixture disk travels through xHCI,
BOT/SCSI, the block interface and existing storage stack.

## Evidence and qualification

Executed results and evidence are recorded below. Generated binaries, disks, pcaps
and logs stay in `target/`. No commit or push is performed.

QEMU results do **not** establish physical-controller or flash-drive
compatibility. No physical controller, drive, hub, firmware ownership handoff,
DMA coherency/IOMMU behavior, surprise removal during DMA, electrical fault,
write-protection, power failure or real device recovery has been validated.
The existing FAT interoperability limitations from the storage audit remain.
Wise Owl Phase 3.75 remains **unresolved** unless its complete final verdict is
observed; a baseline or final timeout is not a pass.

Protocol references: [USB-IF BOT 1.0](https://www.usb.org/sites/default/files/usbmassbulk_10.pdf)
and [Intel xHCI specification](https://www.intel.com/content/dam/www/public/us/en/documents/technical-specifications/extensible-host-controler-interface-usb-xhci.pdf).

### Executed checks

Commands below were run from the repository. Phase commands use
`SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0`
in their parent environment; the existing phase harness supplies its own explicit
trusted-state fixture when required. No deadlines or expected markers were
weakened.

| Command | Result and evidence |
| --- | --- |
| `CARGO_TARGET_DIR=target/usb-host cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-ipc -p sunlight-kv -p sunlight-tls -p sunlight-usb-mouse --target x86_64-unknown-linux-gnu --lib` | PASS: block 29 (including 13 BOT/SCSI tests and the two-disk registry test), FAT 9, VFS 77, IPC 93, KV 27, HID 7. TLS has no host test harness. `target/usb-host-gates.log` |
| `CARGO_TARGET_DIR=target/usb-host cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-ipc -p sunlight-kv -p sunlight-tls --target x86_64-unknown-linux-gnu` | PASS, additionally IPC integration 9 and documentation tests; `target/usb-host-integration.log`, `.exit` (0) |
| `CARGO_TARGET_DIR=target/usb-host cargo check-host-libs` | PASS combined host library graph; `target/usb-workspace-libs.log` |
| `python3 tools/check_workspace.py` | PASS all 95 platform-aware workspace checks; `target/usb-workspace-check.log`, `.exit` (0) |
| `python3 tools/test_usb_storage.py` | PASS all five real xHCI/usb-storage boots: superfloppy, MBR, GPT, malformed and 4Kn. Exact guest/host contents, partition guards, native authorization, lifecycle and HID QMP motion checks; `target/usb-native-gate.log`, `.exit` (0), `target/usb-storage/build.log`, `*.serial.log`, `*.qemu.log`, `*.command.json`, `usb.iso.sha256` |
| `python3 tools/test_runtime_storage.py` | PASS all three virtio layouts, exact contents and guards; `target/usb-regression-runtime.log`, `.exit` (0), `target/runtime-storage/*` |

Example final GPT serial evidence:

```text
[BLK]  No virtio-blk found — /boot will be unavailable
[USB-MSC] reset port=1 speed=3
[USB-MSC] xHCI port=1 slot=1 class=08 subclass=06 protocol=50 interface=0 bulk-in=0x81 bulk-out=0x2
[USB-MSC] Gpt parent=1 start=2048 blocks=131072
[USB-MSC] registered USB-backed disk id=1 lun=0 blocks=135168 sector=512
[STORAGE-RUNTIME] unauthorized PASS
[STORAGE-RUNTIME] USB-backed registry device observed
[STORAGE-RUNTIME] mount write read sync busy close unmount remount exact PASS
[TTY]  exit: storagectl probe -> 0
```

The shell also captures registry records with `device=2 parent=1 start=2048
blocks=131072 usb=1`. `USB-MOUSE-DIAG` reports contain exact QMP motion x=37,
y=-11; the existing diagnostic emits fields through separate DebugLog calls,
so the harness parses whitespace and interleaved service-log lines.
Negative evidence:

```text
[USB-MSC] media discovery rejected: Malformed
[STORAGE-RUNTIME] USB malformed media rejected PASS
[USB-MSC] SCSI/sector profile rejected: SectorSize(4096)
```

The final line is from a separate 4Kn boot, which never registers a block device.
Both negative fixtures remain unchanged on the host.

Changed files:

- `kernel/src/usb_xhci.rs`, `kernel/src/usb_storage.rs`: bulk transport, dedicated
  controller ownership, SCSI startup, USB registry publication.
- `sunlight-block/src/usb_msc.rs`, `src/lib.rs`, `src/registry.rs`: portable
  BOT/SCSI block adapter, protocol tests, USB metadata, two-disk registry bounds
  and disk-scoped extent lookup.
- `kernel/src/main.rs`, `src/storage.rs`, `src/storage_partition_test.rs`,
  `src/arch/x86_64/syscall.rs`: backend routing, bounded mount stack usage,
  preserve virtio boot selection and exclude kernel-owned BARs from HID grants.
- `sunlight-utils/src/bin/storagectl.rs`: display USB metadata, enumerate the
  expanded registry and extend only the explicit native probe feature.
- `tools/test_usb_storage.py`: actual xHCI fixture, QMP HID replay, negative
  media cases and exact host content/guard checks.
- `docs/runtime-storage.md`, `docs/usb-storage.md`: profile, evidence and limits.

Additional regression results:

| Command | Result and evidence |
| --- | --- |
| `SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 python3 tools/test_partitioned_storage.py` | PASS six virtio boots: superfloppy/MBR/GPT restart, sync, remount, exact host bytes and guards. `target/usb-regression-partition.log`, `.exit` (0), `target/partition-storage/*` |
| `SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 bash tools/test.sh phase3.0` | PASS boot/VFS foundation; `target/usb-regression-phase3.0.log`, `.exit` (0) |
| `SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 bash tools/test.sh phase3.5` | PASS block/FAT boot gate; `target/usb-regression-phase3.5.log`, `.exit` (0) |
| `SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 bash tools/test.sh phase6.5.3` | PASS all three VFS shell commands; `target/usb-regression-phase6.5.3.log`, `.exit` (0) |
| `SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 bash tools/test.sh phase5x.4` | PASS legacy printed-marker gate only; **not actual TLS validation**. `target/usb-regression-phase5x.4.log`, `.exit` (0) |
| `SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 bash tools/test.sh phase3.75` | **FAIL / unresolved**, existing 90-second deadline; registered Wise Owl services and mounted explicit trusted virtio `/state`, but missing `[WISEOWL-3.75] native gate PASS`. `target/usb-regression-phase3.75.log`, `.exit` (1), `.serial.log` |

The preserved pre-storage baseline, `target/runtime-baseline-phase375.result`,
also says `FAIL: no final marker within 90 seconds`; see the timing investigation
in [runtime storage](runtime-storage.md#phase-375-timeout-investigation). The
current run again reports 8,574 injected scancodes and ends after successful
`wiseowl-indexctl memorydb` output, before the final verdict. No kernel fault or
panic appears in its serial log. Some injected recovery operations returned
nonzero, so this is not a complete recovery verification. Neither timeout is
counted as a passing gate, and unrelated Wise Owl/KV logic was not changed.

Final-source verification repeated after tightening reserved configuration bits,
interface numbering and SCSI peripheral qualifiers:

| Command | Result and evidence |
| --- | --- |
| `CARGO_TARGET_DIR=target/usb-host cargo test -p sunlight-block --target x86_64-unknown-linux-gnu` | PASS all 29; `target/usb-final-protocol.log` |
| `python3 tools/test_usb_storage.py` | PASS all five boots again, no virtio-blk and no kernel fault markers in any final serial log. `target/usb-native-final.log`, `.exit` (0); final serial/argv/ISO evidence in `target/usb-storage/` |
| `python3 tools/check_workspace.py` | PASS all 95 checks again; `target/usb-workspace-final.log`, `.exit` (0) |
| `CARGO_TARGET_DIR=target/usb-host cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-ipc -p sunlight-kv -p sunlight-tls --target x86_64-unknown-linux-gnu` | PASS final host unit/integration/doc tests; `target/usb-host-final.log`, `.exit` (0) |
| Normal image restoration below | PASS; `target/usb-normal-build.log`, `.exit` (0), `target/usb-normal.iso.sha256` |

Normal release tools, the default-feature kernel and normal ISO were restored
with these exact commands (no test features):

```sh
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-usb-mouse --release
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-utils --bin storagectl --release
env -u SUNLIGHT_INJECT_PHASE SUNLIGHT_RUNTIME_STORAGE_TEST=0 SUNLIGHT_STORAGE_TEST=0 SUNLIGHT_TRUST_LEGACY_STATE=0 \
  cargo build -p sunlight-kernel
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightos.iso
```

`target/usb-normal-hooks-check.log`, `.exit` (0), confirms the final kernel's
features are exactly `["default"]`; key-injection and runtime-storage probe
markers are absent from the actual ELF, runtime-probe markers are absent from
the release storagectl, and debug HID diagnostics are absent from the release
mouse driver. `limine.conf`, `limine.cfg`, `.cargo/config.toml` and
`rust-toolchain.toml` have no changes. The original storage audit SHA-256 remains
`93876b6611abdf77d2f30b5485fc6227addfebf200e88d73686adb853bd9d86b`.

`python3 -m py_compile tools/test_usb_storage.py`, `bash -n tools/test.sh`,
focused `rustfmt --check` on the new/edited modules and `git diff --check` pass;
see `target/usb-source-checks.log`, `.exit` (0).

Actual native TLS/KV verification on that restored normal ISO:

```sh
python3 tools/network_tls_gate.py --cpus 2 --timeout 180 --account-broker-storage --evidence target/usb-tls-kv
```

**PASS**, exit 0: `target/usb-tls-kv-gate.log`, `.exit`,
`target/usb-tls-kv/serial.log`, `network.pcap`, `qemu.log`. This observed native
UnknownIssuer rejection, CA installation through KV, eight verified large
transfers, three verified slow requests, hostname-mismatch rejection, verified
SOCKS/direct requests, two KV restarts, broker seed/check/delete/absent checks
(all exit 0) and direct broker-store denial (expected exit 1). These are Live
ISO session/restart checks; they are not KV migration or reboot durability.
No kernel fault appears in the native TLS/KV serial log.

All requested verification completed; Phase 3.75 remains the sole unresolved
(non-passing) gate. Passing QEMU USB tests does not establish physical USB
controller/flash-drive compatibility; the hardware limitations above remain.
No commit or push was performed.

The [fstab/inventory extension](fstab-inventory.md) uses these existing USB
registry records without changing BOT/SCSI, transport discovery or state trust.
It adds explicit configured GPT mounting and public read-only inventory.
