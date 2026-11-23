# BTop-07: Guest-backed storage telemetry

This continues [BTop-06](btop-06-atomic-masked-pselect.md) and the storage
gate in [BTop-02](btop-02-procfs.md). The binary at `target/btop-01/btop`
is unchanged (version `1.4.5+a1099d0`): SHA-256
`bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702`.
The retained upstream source is commit
`a1099d075fa914452366b28c67ff171e21a1f02e` and the linked musl is from
Alpine `musl 1.2.5-r24` (see `target/btop-01/HANDOFF.md`). The baseline
kernel source commit was `ab9ccb14d7094482742e7178f926e1102f76ba1b`.

## Caller contract

The 4 GiB, four-vCPU baseline boot with the disposable VirtIO disk logged
`[PART] Mbr start=2048 blocks=131072` and
`[VFS] FAT volume mounted at /mnt`. The unchanged btop repeatedly called
x86-64 Linux `open` (2) on `/proc/self/mountinfo`; Helios returned
`-ENOENT` (serial in `target/btop-07/baseline/`, e.g. trace lines
1579, 1626 and 1715). The collector never admitted a mount, so the trace
cannot prove downstream syscalls issued by btop itself.

The retained `src/linux/btop_collect.cpp` (around lines 2010–2305)
separates four concerns:

- `show_disks` enables collection. `only_physical` consults
  `/proc/filesystems` only if `use_fstab` is off. `use_fstab` opts into
  `/etc/fstab`. This replay enables storage and I/O display, disables both
  admission modes and selects `/mnt` with `disks_filter`. Those optional
  interfaces are outside this gate.
- The collector reads `/proc/self/mountinfo`, takes the mountpoint
  (decodes octal escapes), filesystem type, source, and third field
  `major:minor`. It canonicalizes the source on a best-effort basis.
  The pinned extraction uses a fixed number of tokens before the
  filesystem type; optional mountinfo fields can shift that parse.
  Do not invent a propagation tag to satisfy it.
- A worker calls `statvfs(mountpoint)`. Total is
  `f_blocks * f_frsize`. Free is `f_bavail * f_frsize` with
  `disk_free_priv=False` and `f_bfree * f_frsize` with it enabled. A
  failed query places the mount on the collector's ignore list.
  The linked disassembly in `target/btop-07/musl-linked.dis` and
  musl v1.2.5 `src/stat/statvfs.c` show the wrapper around x86-64 syscall
  137 (`statfs(path, buf)`, 120-byte kernel record). It copies those
  counts and falls back to `f_bsize` only for a zero fragment size.
- Device identity joins through `/sys/dev/block/MAJOR:MINOR/stat`:
  the file must exist and pass `access(R_OK)`. For this non-ZFS path
  the collector samples columns 3 and 7 (cumulative read and write
  sectors, 512 bytes per sector) and 10 (busy milliseconds). It
  differences successive samples. This collector has no
  `/proc/diskstats` fallback. At `ENOENT` on mountinfo there is no
  `major:minor` with which to attempt the join. This is a discovery
  failure, not an observed mismatch or an idle device.

The independent static musl probe on Linux confirms the normal join on
`/boot/efi`: mountinfo `259:1`, FAT `frsize=4096 blocks=523262
bfree=523089 bavail=523089`; `/sys/dev/block/259:1/stat` read sectors
25312, write sectors 2, busy ms 22 in that sample. The same read-only
probe on Linux `/` found Btrfs `0:30`, with no block stat for that
filesystem ID, and observed distinct `bfree=31972655` and
`bavail=31095807`. These are host observations, not guest references.

## Backing and limits

The new Linux path query reuses authorized pathname resolution and
selects capacity by the resolved VFS mount incarnation. A concurrent
replacement cannot substitute the new mount's capacity for the old one.
Only FAT mounts with a live allocation count return capacity. Absent
paths retain normal Linux lookup errors; unsupported mounts return
`EOPNOTSUPP`. No descriptor variant is registered. Linux statfs fields
not collected by btop (file counts, fsid, flags) are not a general
filesystem information API in this tier.

The FAT allocator uses its first FAT for allocation. Capacity counts
actual data clusters from the BPB and free entries from that FAT,
skipping reserved entries and masking reserved high bits. The count does
not use optional FSInfo hints or image size. It is initialized before
mount publication, bounded to 1024 FAT sectors on VirtIO and unreported
on USB, oversized maps or sampling failure. The existing FAT writer
updates the count under VFS serialization when a cluster transitions
between free and allocated. It withdraws the count before changing the
mirrored entries and publishes it after all copies have accepted the
write; a write error leaves it unavailable. The count represents the
live filesystem, which can include dirty blocks in the writeback cache.
Poisoned mounts do not publish it. FAT has no privileged block reserve,
so available and free are equal. RAM filesystems are unsupported.

The device registry identifies the partition by its parent and exact
extent. Helios has no published Linux device-number mapping for that
identity. The remaining discovery work is to use an authorized VFS
mount snapshot and define a stable Linux device number tied to a
registered device incarnation. No mountinfo rows or fabricated device
nodes are added here.

The audited block path is `PartitionBlockDevice` (relative-to-absolute
LBA translation), the kernel disk's writeback cache, `KernelBlkDev`,
and VirtIO typed completions under the device lock. The cache can serve
reads without touching VirtIO and can defer writes until eviction or
flush. Typed completion reports an outcome but maintains no cumulative
read/write/time counters. A future accounting implementation needs a
single completion boundary for all relevant callers, LBA-to-extent
attribution there, successful completion and error rules, a consistent
concurrent snapshot, and elapsed busy-time accounting. Flush does not
have an LBA, and unrelated parent-device traffic must stay out of the
partition totals. No Linux block-stat file is published, including for
idle devices. QEMU per-device statistics are an independent check on
transport activity, not guest per-partition counters.

## Reproduction

These commands run from the repository root. `target/btop-07` retains
outputs; each replay requires a fresh evidence directory. The ISO is the
boot source, and the only VirtIO disk is the regular-file fixture with
QEMU `snapshot=on`. No host raw block device is opened. The generator
uses exclusive creation and formats a new 64 MiB file, then embeds it as
one MBR partition at sector 2048. The independent reader verifies the
disk signature, extent, BPB label `BTOP07`, serial `b7070001` and both
FAT copies. The replay checks the observed boot partition geometry before
any optional ordinary file writes under `/mnt`.

```sh
out=target/btop-07
mkdir -p "$out/final"
printf '%s  %s\n' bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 target/btop-01/btop | sha256sum -c
python3 tools/btop-07/make-fixture.py "$out/fixture"
python3 tools/btop-07/fat-reference.py "$out/fixture/disk.img"
musl-gcc -Wall -Wextra -Werror -O2 -static tools/btop-07/probe.c -o "$out/probe.elf"
"$out/probe.elf" /boot/efi
"$out/probe.elf" /
objdump -d --start-address=0x60ea26 --stop-address=0x60eb46 target/btop-01/btop > "$out/musl-linked.dis"
cp "$out/probe.elf" "$out/final/probe.elf"
bash tools/stamp_helios_elf.sh "$out/final/probe.elf"
SUNLIGHT_BTOP06_WAIT_TRACE=1 SUNLIGHT_HELIOS_SIG01_TRACE=1 bash tools/btop-07/image.sh trace
iso="$out/final/guest-ramfs-trace-config.iso"
cargo test --target x86_64-unknown-linux-gnu -p sunlight-fat -p sunlight-fs -p sunlight-block -p sunlight-virtio -p sunlight-compat-linux
cargo test --manifest-path sunlight-tty/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check --package sunlight-kernel --features native_document_viewer
bash tools/runtime-parity/run.sh "$out/parity"
git diff --check
```

Production changes: `compat-linux/src/{abi,lib}.rs`,
`kernel/src/arch/x86_64/syscall.rs`, `kernel/src/{main,storage}.rs`,
`sunlight-fat/src/{fat,lib}.rs`, `sunlight-fat/src/fat/write.rs` and
`sunlight-fs/src/vfs.rs`. Reproduction files are under
`tools/btop-07/`. The test ISO recipe reverses its temporary RAMFS
packing and syscall traces.

Final replay commands (use the `iso` variable above):

```sh
python3 tools/btop-07/replay.py "$iso" "$out/final/disk" --disk "$out/fixture/disk.img" --probe --allocate
python3 tools/btop-07/replay.py "$iso" "$out/final/disk-replay" --disk "$out/fixture/disk.img" --probe
python3 tools/btop-07/replay.py "$iso" "$out/final/none" --probe
BTOP_TCG=1 python3 tools/helios-sig01/guest.py "$iso" "$out/final/sig01"
BTOP_TCG=1 python3 tools/btop-05/guest.py "$iso" "$out/final/unmasked" --unmasked
python3 tools/btop-06/guest.py "$iso" "$out/final/masked"
sha256sum -c "$out/final/artifacts.sha256"
```

Each replay has `serial.log`; btop runs additionally have
`network-command.json`, dashboard and Help PNG/OCR, and `network.pcap`.
The weaker early attempts are retained separately: the first probe image
reused a root service that did not contain the packaged probe, the
first SIG-01 run lacked its trace flag, and one no-disk shell startup
missed its endpoint. These attempts are not the final controls.

| Artifact | SHA-256 |
| --- | --- |
| BTop-06 baseline ISO | `2e1fe7b5d11fe9c2c1135437f1cc20e36dedb87637885c8e1906a6a69fa0ee65` |
| BTop-07 final guest ISO | `14ec243c7d2f97fa9b7a53ea986a75e5914715a2136ef3c548ec86c6a3a3440e` |
| BTop-07 final kernel ELF | `62b1edf3889c909b3bcee574ec1665e173658680e3a891ab4516a39a5854a37d` |
| Static musl probe, Linux | `4a84559484b54f99e6aa8c908b451c7c60ee65134295ee20e9f0ed4a1f413565` |
| Same probe, Helios personality stamp | `981a263a6bd4dd72074a8a595e51cacf555fd0cac1dddf02a298daaa9a0d389b` |
| Disposable MBR disk, before and after guest runs | `5b4392a7619f561871d3e1a3fa31cf4e564b708c2162972898789a1409ce4c59` |

## Results

| Gate | Result | Evidence |
| --- | --- | --- |
| Linux mount discovery on Helios | **FAIL** | Actual pinned caller repeatedly gets `ENOENT` at `/proc/self/mountinfo`. Native boot discovers and mounts the expected MBR extent. |
| FAT capacity, independent idle sample | **PASS** | Final guest syscall 137 succeeds. The independent disk parser and guest both return 512-byte fragments, 129022 total data clusters, 129021 free and available. Total 66059264 bytes, free 66058752 bytes. The used cluster is the existing root; reserved and FAT sectors are outside the data-cluster count. |
| FAT capacity, live allocation and reclamation | **BLOCKED** | Guest `cp /bin/storage-probe /mnt/btop07.bin` was denied at destination create, exit 1. Intended payload 80032 bytes (157 data clusters if written); readback and removal were **NOT RUN**. Native `storagectl devices` and `storagectl mounts` also exited 1 with error -1. No authority was granted or bypassed. An in-memory FAT test verified allocation of two clusters for 513 bytes, independent recount, deletion, and recount recovery. |
| No test volume control | **PASS** | No VirtIO disk attached; `statfs("/mnt")` returns `-95` for the RAM mount path and probe exits 1 with `errno=95`. No zero capacity is returned. |
| Read counters | **BLOCKED** | No guest block-counter source or mount-to-device-number contract; active guest transport read phase **NOT RUN**. |
| Write counters | **BLOCKED** | No guest block-counter source or mount-to-device-number contract; destination create failed before the payload phase. No flush/durability claim. |
| Idle transport control | **PASS** | QMP `query-blockstats` for the top-level `device=fixture` VirtIO frontend had zero deltas in read bytes, write bytes, read ops, write ops and flush ops over the coordinated 3-second idle window. |
| btop storage display | **FAIL** | No backed row admitted; capacity, read activity and write activity in btop are **NOT RUN** beyond the failed discovery. The settings allowed /mnt, and the trace never reached the capacity worker or the sysfs join. |
| Interactive btop regressions | **PASS** | Final disk and no-disk replays captured multiple guest-backed refreshes, real Help open/close and q; explicit exit 0, worker and leader reap, and separate same-TTY `/bin/proc-native` exit 0. |
| Native/musl parity | **PASS** | Both fixtures: ELF, file, TLS, join and per-personality routing. `target/btop-07/parity.log`. |
| Focused Helios, FAT, VFS, block, VirtIO and TTY tests | **PASS** | 29 + 10 + 112 + 68 + 30 + 42 unit tests, respectively. Kernel check and whitespace check pass. |
| Masked and unmasked pselect regressions | **PASS** | Final masked fixtures exit 0 with registered remote wake (sender vCPU 1, masked waiter vCPU 0); unmasked timeout, stdout write readiness, invalid fd and injected hardware key, followed by exit 0 and reap. |
| SIG-01 | **PASS** | Final timer delivery/sigreturn trace and nine rejection cases. |

Key excerpts from `final/disk/serial.log` and `final/none/serial.log`:

```text
[PART] Mbr start=2048 blocks=131072
[VFS] FAT volume mounted at /mnt
nr=137 ... result=0 path=Some("/mnt")
MOUNTINFO errno=2
IO unavailable: no device join
CAP path=/mnt bsize=512 frsize=512 blocks=129022 bfree=129021 bavail=129021
[TTY]  exit: /bin/storage-probe -> 0
[TTY]  exit: cp /bin/storage-probe /mnt/btop07.bin -> 1

nr=137 ... result=-95 path=Some("/mnt")
CAP path=/mnt errno=95
[TTY]  exit: /bin/storage-probe -> 1
```

At the three-second idle endpoints, `final/disk/block-idle-{start,end}.json`
both record 934400 read bytes (1152 ops), 512 write bytes (1 op), and
0 flushes on `fixture`. These are cumulative backend observations
including boot probing; the pre-window write has not been attributed
to a payload or a partition. The idle delta is zero. The CD-ROM and
firmware records were excluded. There are no isolated active guest
deltas, timing comparison, or active screenshots to qualify as a pass.


The storage-enabled final replay (`final/disk-replay`) captured three
dashboards and the Help/return pair. Its startup trace contains 17 btop
mountinfo attempts (all `ENOENT`), with no `/proc/filesystems` or
`/sys/dev/block` access. Repeated `/proc/stat`, `/proc/meminfo`,
`/proc/40/stat` and both Ethernet sysfs counters back the dashboards.
The launcher injected `h`, validated the Help legend, injected `h` again
and verified the dashboard returned before injecting `q`. Its serial
trace records:

```text
[EXEC] /bin/btop fg pid=40
[SCHED] process_reaped pid=41 name='btop' ...
[SCHED] process_reaped pid=40 name='btop' ...
[TTY]  exit: /bin/btop -> 0
[TTY]  exit: /bin/proc-native -> 0
```

The no-disk replay used the same ISO and separately reproduced btop exit
0 and `/bin/proc-native` exit 0. Neither result is a storage display pass.
