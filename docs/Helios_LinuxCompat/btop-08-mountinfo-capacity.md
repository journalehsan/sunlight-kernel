# BTop-08: Caller-visible mountinfo and FAT32 capacity

This continues [BTop-07](btop-07-storage-telemetry.md) and the interactive
baseline in [BTop-06](btop-06-atomic-masked-pselect.md). The former
image again returned `-ENOENT` from syscall 2 on
`/proc/self/mountinfo` for btop, before capacity sampling:
`target/btop-08/baseline/serial.log` (lines 1613 and 1654).

## Caller contract

The retained upstream btop is commit
`a1099d075fa914452366b28c67ff171e21a1f02e`. In
`target/btop-01/src/src/linux/btop_collect.cpp:2011` the collector
loads the exact mount-point filter (optionally inverted). At line 2071
an input file stream opens `/proc/self/mountinfo`; it extracts
whitespace-separated tokens, ignores to newline, decodes octal escapes
in the mount point and drops ignored or duplicate points. It uses the
filesystem type for the optional physical-filesystem and ZFS filters,
canonicalizes the source on a best-effort basis, and uses field three
for a separate sysfs I/O lookup. `use_fstab` and `only_physical`
would add admission checks; the replay disables both.

The pinned extraction expects eleven tokens even though a valid row
with no optional fields has ten. It still obtains fields three and
five correctly on the first row, misinterprets type and source, and
can consume the next row's ID before discarding the remainder. The
export orders descendants before ancestors (longest paths first);
Linux mountinfo does not require parents to precede children. This
preserves the tested FAT mount as the first row with a root mount
present. There is no invented propagation tag and no claim that this
btop build parses arbitrary mount lists correctly.

The asynchronous capacity worker invokes `statvfs` on the decoded
mount point, not the source. For the replay it multiplies `f_blocks`
and `f_bavail` by `f_frsize`; used is the difference. The linked
Alpine musl 1.2.5-r24 wrapper (disassembly in
`target/btop-07/musl-linked.dis`; musl 1.2.5 `src/stat/statvfs.c`)
invokes x86-64 syscall 137, `statfs(path, buf)`, and copies the
filesystem counts into `statvfs`. The capacity drawing path in
`src/btop_draw.cpp:1364` is independent of the I/O graph condition.
The normal configuration selects `show_disks=True`,
`show_io_stat=False`, `io_mode=False`, `disk_free_priv=False`,
and `disks_filter="/mnt"`. No I/O counters are exported.

## VFS and caller boundary

The global kernel VFS has at most eight live mounts, with stable
incarnation IDs (tag plus one), a source registry ID, an absolute
attachment path and a filesystem backend. It has no stacked mounts,
bind roots, or propagation mode. Longest strict path prefix gives the
parent in this model; the root points to itself. Each backend mounts
its own root `/`. The tmpfs root uses the existing anonymous
`0:0` device identity; any additional tmpfs instance is distinguished
by its mount incarnation. The FAT volume uses the Helios local block
major 240 and the non-recycled, checked 20-bit registry ID as minor.
`helios-block:<id>` names that registry source without claiming that
a devnode exists. There is no sysfs or completion-counter join with this
number, and legacy Linux stat records do not yet provide a corresponding
device number. An absent, non-FAT or out-of-range block identity cannot
produce a row. Both VFS backends implement writes; `rw` describes that
mount property. A protected boot mount cannot be detached, and existing
credentials, pathname policies and file permissions still govern writes.

Only `/proc/self/mountinfo` is admitted, using the existing same-user
leader lookup and an identity check on use. The ordinary global root
remains valid for a caller with routed metadata; routing alone does not
imply a different root. An explicitly service-routed file view or
rebase through the trusted root transaction declines the export. The
root commit records that the root became opaque; spawn inherits the
restriction and group access uses the shared process owner. The
retained service root has no same-incarnation root/pin correspondence
to the global kernel mount table, so it cannot be rebased into a
truthful kernel snapshot. Numeric PID mountinfo access stays absent.
The VFS and device registry are sampled together outside the scheduler
lock, then the open is checked again before publication. Each FD holds
one bounded snapshot; partial reads reuse its bytes.

## Reproduction

Commands from the repository root, using fresh replay directories:

```sh
out=target/btop-08
disk=target/btop-07/fixture/disk.img
mkdir -p "$out/final"
printf '%s  %s\n' bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 target/btop-01/btop | sha256sum -c
git -C target/btop-01/src rev-parse HEAD
python3 tools/btop-07/replay.py target/btop-07/guest-ramfs-trace-config.iso "$out/baseline" --disk "$disk"
python3 tools/btop-07/fat-reference.py "$disk"
musl-gcc -Wall -Wextra -Werror -O2 -static tools/btop-08/probe.c -o "$out/probe.elf"
"$out/probe.elf" /boot/efi
cp "$out/probe.elf" "$out/final/probe.elf"
bash tools/stamp_helios_elf.sh "$out/final/probe.elf"
SUNLIGHT_BTOP06_WAIT_TRACE=1 SUNLIGHT_HELIOS_SIG01_TRACE=1 bash tools/btop-08/image.sh
iso="$out/final/guest-ramfs-trace-config.iso"
python3 tools/btop-07/replay.py "$iso" "$out/final/disk" --disk "$disk" --probe
python3 tools/btop-07/replay.py "$iso" "$out/final/none" --probe
cargo test --target x86_64-unknown-linux-gnu -p sunlight-fs -p sunlight-fat -p sunlight-compat-linux
bash tools/runtime-parity/run.sh "$out/parity"
cargo check --package sunlight-kernel --features native_document_viewer
sha256sum -c "$out/final/artifacts.sha256"
git diff --check
```

The image recipe temporarily adds the probe, btop configuration and
read/VFS traces, and reverses its patches. The test disk is a regular file
attached with QEMU `snapshot=on`. Production changes are in
`sunlight-fs/src/vfs.rs`, `sunlight-fs/src/vfs/mountinfo.rs`,
`kernel/src/linux_procfs.rs`, `kernel/src/arch/x86_64/syscall_procfs.rs`,
`kernel/src/arch/x86_64/syscall.rs`, `kernel/src/process/{mod,spawn}.rs`,
and `kernel/src/ipc/vfs_registration/chdir.rs`. The BTop-08 probe,
configuration and image helper are under `tools/btop-08/`.

Linux probe evidence is in `target/btop-08/linux.log`: it read in
37-byte fragments, found its root row and the selected `/boot/efi`
target (`259:1`, vfat), opened numeric self mountinfo, and returned
4096-byte fragments with 523262 total and 523089 free and available.
These are host reference observations only.

## Final traces and display

Both replays use the same 4096 MiB, four-vCPU image. See
`target/btop-08/final/{disk,none}/network-command.json` for full QEMU
arguments and the adjacent `serial.log` files for the complete traces.
The initial replays in `target/btop-08/first-{disk,none}` are retained:
the first visibility check incorrectly treated routed metadata as
proof of a restricted root, returning ENOENT to the probe. Neither
initial replay counts toward final acceptance.

```text
[BTOP08] VFS mount=2 source=2 parent_device=1 start=2048 blocks=131072 path=/mnt
MOUNT 2 1 240:2 / /mnt rw - vfat helios-block:2 rw
MOUNT 1 1 0:0 / / rw - tmpfs tmpfs rw
DISCOVERY root=1 target=1
NUMERIC 2
CAP path=/mnt bsize=512 frsize=512 blocks=129022 bfree=129021 bavail=129021

[BTOP08] read pid=40 fd=3 offset=0 bytes=77
[LINUX-STARTUP-TRACE] seq=26 pid=41 nr=137 ... result=0 path=Some("/mnt")
[BTOP08] read pid=42 fd=3 offset=0 bytes=77
[LINUX-STARTUP-TRACE] seq=37 pid=43 nr=137 ... result=0 path=Some("/mnt")
[SCHED] process_reaped pid=42 name='btop' ...
[SCHED] process_reaped pid=40 name='btop' ...
[TTY]  exit: /bin/btop -> 0
[TTY]  exit: /bin/proc-native -> 0

MOUNT 1 1 0:0 / / rw - tmpfs tmpfs rw
DISCOVERY root=1 target=0
NUMERIC 2
CAP path=/mnt errno=95
```

The disk serial places the authoritative VFS record at line 127, the
independent probe at 1434–1446 and the first btop read/statfs at
1653/1688. The latter uses the selected mount point `/mnt` from the
configured exact filter. The independent FAT scan
(`target/btop-08/fat-reference.log`) agrees on the partition extent and
all three cluster counts. The used 512 bytes are the FAT root cluster.
Total 66,059,264 bytes is 62.9990 MiB and available 66,058,752 bytes is
62.9985 MiB. The pinned binary's IEC formatter truncates at one
decimal in this range, so both render as **62.9 MiB**. The percent
calculation rounds to **0% used, 100% free**.

`target/btop-08/final/disk/dashboard-2.png` shows both 62.9 MiB
figures and the corresponding Used/Free percentages. The guest
terminal does not legibly render every label in this capture; mount
association comes from the exact filter and btop's own syscall 137
trace. The independent no-disk dashboard
`target/btop-08/final/none/dashboard-2.png` shows no corresponding
capacity row. The existing swap display is separate. Each directory
also retains `dashboard-{0,1}.png`, `help.png`,
`dashboard-return-0.png`, `after.png`, OCR text and Ethernet PCAP.

| Gate | Result | Evidence |
| --- | --- | --- |
| Mountinfo ABI, global view | **PASS** | 37-byte reads through EOF, Linux field order and escaping, real root and child IDs; numeric mountinfo returns ENOENT. The VFS unit test also checks nearest nested parent, component boundary and output bound. |
| Rebased service view | **BLOCKED** | Export fails closed; no kernel-to-service root identity join. Live narrowed-root replay **NOT RUN**. The legacy stat device field is outside this export. |
| FAT32 discovery | **PASS** | Guest VFS source 2, parent device 1, extent 2048/131072; mountinfo ID 2, parent 1 and local block identity 240:2 agree. |
| Raw capacity | **PASS** | 512-byte fragments, 129022 total, 129021 free and available, independently scanned. |
| btop capacity display | **PASS** | Pinned btop opens and reads mountinfo, samples `/mnt` successfully and displays the expected IEC capacity and free percentages. |
| No test disk | **PASS** | Root tmpfs still appears; no FAT mount or corresponding btop row; `statfs("/mnt")` returns EOPNOTSUPP. |
| Live allocation and reclamation | **BLOCKED** | BTop-07 create was denied; no authorized writable test mount was identified. Live writes and reclamation **NOT RUN** for BTop-08. |
| Per-partition I/O | **BLOCKED** | No backed completion counters or proven Linux device-stat join. Counters remain unavailable. |
| Interactive regressions | **PASS** | Both replays: multiple CPU, memory, process and Ethernet samples; real Help open and return; q, explicit zero exit, leader and worker reaping, and separate zero exit from proc-native on the same TTY. |
| Parity and focused tests | **PASS** | `parity.log`: both ELF personalities, file/TLS/join and routing. `focused.log`: 113 VFS, 10 FAT, 29 compat tests. `check.log`: kernel check. Separate signal and pselect matrix **NOT RUN** for this milestone. |

| Artifact | SHA-256 |
| --- | --- |
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| BTop-07 baseline replay ISO | `a0a8b7a65be9fa6c0b953844d441a9e6f896a6f7e0aba5a429e4d9eb8fab0e0c` |
| Final BTop-08 ISO | `eac6ba6169a040d32b8cd8343fd84911a454d33dfa9388c0042c8c859d2387a4` |
| Final traced kernel ELF | `3f6352efec6e8b4906fdf56de31ae1281890e7b5d23ab10924fb1c5ce44427e2` |
| Static musl probe | `62da3c0659d01590095ff6908b3aa50962bb5b0dea0a542c768912e186ea9d10` |
| Guest-stamped probe | `ce9038f0058676604605454f9ffe9083da56eab43317b8fa9f575f4bec1207f3` |
| Disposable disk, unchanged | `5b4392a7619f561871d3e1a3fa31cf4e564b708c2162972898789a1409ce4c59` |
