# BTop-09: Live FAT32 allocation and capacity recovery

Continues [BTop-08](btop-08-mountinfo-capacity.md) and
[BTop-07](btop-07-storage-telemetry.md). The create denial is expected
caller policy on the protected boot attachment. The existing privileged
native write path does not grant an ordinary musl process write access.
No production code or btop source was changed; the rebased service view
still fails closed.

## Create denial

The static musl probe calls x86-64 syscall **257**, `openat(AT_FDCWD,
"/mnt/BTOP09.BIN", O_WRONLY | O_CREAT | O_EXCL, 0600)`. The directory
FD is -100, flags are `0xc1`, and the parent is `/mnt`. The guest
reports real and effective UID and GID all zero. The detector uses
exclusive creation; it never truncates an existing file.

In `target/btop-09/disk/serial.log` the authoritative mount record is at
line 127 and the policy and syscall records start at line 1433:

```text
[BTOP08] VFS mount=2 source=2 parent_device=1 start=2048 blocks=131072 path=/mnt
[SUNLIGHT-FS] request actor=User { uid: 0, name: "root" } op=Create path=/mnt/BTOP09.BIN
[SUNLIGHT-FS] decision actor=User { uid: 0, name: "root" } op=Create path=/mnt/BTOP09.BIN result=deny reason=DeniedImmutableRoot err=Some(ReadOnlyFilesystem)
[LINUX-STARTUP-TRACE] seq=3 pid=39 nr=257 a0=0xffffff9c a1=0x7fffffffebf0 a2=0xc1 a3=0x180 result=-13 path=Some("/mnt/BTOP09.BIN")
```

The raw return is -13 (libc returns -1, errno EACCES). Under the same
root TTY session and mount, native `cp /bin/storage-probe /mnt/BTOP09.BIN`
hits the same Create policy branch and exits 1 with “Permission denied”
(lines 1465–1476). In
`kernel/src/arch/x86_64/syscall.rs`, Linux `sys_linux_openat` and
native `sys_open` converge on `open_resolved_path`. Both check
`can_write` before lookup/create in the FAT backend. The exception
requires **both** an explicit StorageAdmin grant and an installed
runtime mount. The boot mount is protected, so that exception cannot
apply even to an administrative caller. No parent inode permission
check or FAT mutation runs. `DeniedImmutableRoot` is a caller policy
result; its internal `ReadOnlyFilesystem` value does not indicate a
read-only FAT implementation.

On both disk runs guest statfs and statvfs flags were `0` (no
`ST_RDONLY`). The mountinfo formatter emits `rw` for these backend
types; the attachment is protected from detach. The read-only control
sets QEMU's backing device `readonly=on` and still shows the same
caller denial before any write reaches the device. It does not test
backend EROFS or assert that guest mount flags reflect QEMU's read-only
property.

The separate runtime mount route is `storagectl mount ID /mnt[/...]`.
`ipc/src/storage.rs::authorized` requires UID 0 and an explicit
StorageAdmin capability. The embedded root `storagectl` is granted
that capability; native test mode can create and sync a file on its
runtime mount. The grant belongs to the control tool, and ordinary
musl workloads have no supported delegation onto that path. Mounting a
disk would not grant the fixture write authority. The evidence therefore
does not isolate a Helios-only defect to fix. The protected mount was
never remounted and no permissions were bypassed.

The existing native runtime harness used a separate build and fresh
regular-file fixtures. Its first (superfloppy) guest reached login but
timed out before the shell, so it did not exercise native mutation.
MBR and GPT cases were not reached. See
`target/btop-09/native-storage.log` and
`target/btop-09/native-storage/superfloppy.serial.log`.

## Fixture, measurements, and replay

The test used a fresh regular-file copy
`target/btop-09/fixture/disk.img`, byte-identical to the verified
BTop-07 disposable original. The MBR signature and FAT serial are
`b7070001`, label `BTOP07`, the single FAT32 LBA partition starts at
2048 and spans 131072 sectors of 512 bytes. Guest registry parent 1,
partition source 2, mount ID 2 and mountinfo identity `240:2` belong
to this attachment. QEMU's fixture disk is separate from the ISO. The
disk replay uses a temporary QEMU overlay; the read-only replay uses
the original copy as read-only. The independent scanner compares both
FAT copies and counts entries in the data-cluster range, ignoring
FSInfo. The baseline root has one allocated cluster and only a volume
label entry.

| Sample | Fragment bytes | Total | Free | Available |
| --- | ---: | ---: | ---: | ---: |
| Stable image FAT baseline | 512 | 129022 | 129021 | 129021 |
| Guest baseline statfs and statvfs (disk and read-only) | 512 | 129022 | 129021 | 129021 |
| Guest no disk | — | — | — | — |
| After allocation | — | — | — | — |
| After deletion | — | — | — | — |

No-disk statfs and statvfs return errno 95. Linux ran the same probe
against a fresh user-owned host directory, with UID/GID 1000, where the
same create returned FD 3 and the empty file was closed and removed.
Both host capacity APIs returned flags `0x1020`, 4096-byte fragments,
124502016 total, 30924077 free and 30051517 available. This host run
checks the fixture and syscall contract only; it was not a Linux FAT
allocation run. The host EFI volume was not used.

The 4096 MiB, four-vCPU replay captures screenshots and full arguments
under `target/btop-09/{disk,readonly,none}`. In the mounted baseline
66,059,264 total and 66,058,752 available bytes both truncate to
62.9 MiB at btop's IEC precision; the percentages round to 0% used and
100% free. The exact `/mnt` filter and btop's syscall 137 trace tie the
dashboard row to the mounted source. `dashboard-2.png` records the
baseline; `help.png`, `dashboard-return-0.png` and `after.png` cover
the interactive sequence. The no-disk dashboard has no FAT capacity
row; its swap row is independent.

The idle control is the three-second interval between
`block-idle-start.json` and `block-idle-end.json`. QEMU recorded no
fixture write delta in either disk run. The overlay already had one
512-byte write before the idle window; the read-only backing had zero.
The overlay write was not classified by sector, so it cannot establish
a metadata allocation. The stable base image hash and FAT scan were
unchanged after both guests stopped; the discarded overlay was not
scanned. Per-partition I/O remains separate. The QEMU snapshots are
harness observations and no Linux I/O counters were added or validated.

## Reproduction

```sh
out=target/btop-09
disk=$out/fixture/disk.img
mkdir -p "$out/final" "$out/fixture"
test ! -e "$disk" && test ! -L "$disk"
test ! -L target/btop-07/fixture/disk.img
printf '%s  %s\n' 5b4392a7619f561871d3e1a3fa31cf4e564b708c2162972898789a1409ce4c59 target/btop-07/fixture/disk.img | sha256sum -c
cp --reflink=auto target/btop-07/fixture/disk.img "$disk"
python3 tools/btop-07/fat-reference.py "$disk" --empty | tee "$out/fat-before.log"
musl-gcc -Wall -Wextra -Werror -O2 -static tools/btop-09/deny-create.c -o "$out/probe.elf"
tmp=$(mktemp -d "$out/host.XXXXXX")
"$out/probe.elf" "$tmp" > "$out/linux.log"  # exit 1: create succeeded; file removed
rmdir "$tmp"
cp "$out/probe.elf" "$out/final/probe.elf"
bash tools/stamp_helios_elf.sh "$out/final/probe.elf"
SUNLIGHT_BTOP06_WAIT_TRACE=1 SUNLIGHT_HELIOS_SIG01_TRACE=1 bash tools/btop-09/image.sh > "$out/image.log" 2>&1
iso=$out/final/guest-ramfs-trace-config.iso
sha256sum "$disk" target/btop-01/btop "$out/probe.elf" "$out"/final/{probe.elf,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso} > "$out/artifacts.sha256"
python3 tools/btop-07/replay.py "$iso" "$out/disk" --deny-create --disk "$disk" > "$out/disk.log" 2>&1
python3 tools/btop-07/replay.py "$iso" "$out/readonly" --deny-create --disk "$disk" --readonly > "$out/readonly.log" 2>&1
python3 tools/btop-07/replay.py "$iso" "$out/none" --deny-create > "$out/none.log" 2>&1
cargo test --target x86_64-unknown-linux-gnu -p sunlight-fs -p sunlight-fat -p sunlight-compat-linux > "$out/focused.log" 2>&1
bash tools/runtime-parity/run.sh "$out/parity" > "$out/parity.log" 2>&1
PYTHONPATH=tools python3 -c 'import test_runtime_storage as t; t.OUT = t.ROOT / "target/btop-09/native-storage"; t.OUT.mkdir(exist_ok=False); t.main()' > "$out/native-storage.log" 2>&1
python3 tools/btop-07/fat-reference.py "$disk" --empty | tee "$out/fat-after.log"
sha256sum -c "$out/artifacts.sha256"
git diff --check
```

| Artifact | SHA-256 |
| --- | --- |
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Disposable disk | `5b4392a7619f561871d3e1a3fa31cf4e564b708c2162972898789a1409ce4c59` |
| Static musl probe | `8853076aeb5fe1993873934e1ca88235b2174a4e999a69e4084c2b0a4d5c05a6` |
| Guest-stamped probe | `35e5c44155cd2e03ae72f68da35ca360ee85d441cd1f8c00a4ec0a0e2ebd8902` |
| Traced kernel ELF | `8cb44cbdf151ac390ff85a9d127cbaa6b60bb647eb7eb30f205390f1ac55c8ba` |
| Final replay ISO | `4070c13674c28a540c8515f5bdcdee8b22dcdea0dd1bfcc2fd35450db9b61082` |

| Gate | Result | Evidence |
| --- | --- | --- |
| Create-denial diagnosis | **PASS** | Linux openat and native cp agree on the protected attachment; kernel logs show why the backend was not reached. |
| Authorized write path for musl | **BLOCKED** | The runtime exception is StorageAdmin-only; ordinary musl has no grant. |
| Privileged native runtime harness | **FAIL** | Login timeout; native runtime mutation was **NOT RUN**. |
| Baseline capacity | **PASS** | Both capacity APIs match the offline FAT scan. |
| Allocation | **NOT RUN** | No authorized musl create; no data clusters written. |
| Recovery | **NOT RUN** | No allocated file to unlink; no metadata allocation delta measured. |
| Live btop capacity | **NOT RUN** | Baseline display only; no allocation/deletion transitions to observe. |
| Read-only negative control | **PASS** | Read-only backing, expected caller denial and readable baseline. |
| No-disk negative control | **PASS** | No FAT mount, errno 95 on capacity, and no FAT row. |
| Interactive regressions | **PASS** | All three replays observed repeated CPU, memory, process and Ethernet sampling, Help open/close, q, explicit btop exit 0, leader and worker reaping, and separate same-TTY proc-native exit 0. |
| Focused tests and ABI parity | **PASS** | 113 VFS, 10 FAT, 29 Helios tests; musl and native file/TLS/join and routing checks passed on the separate parity image. |
| Rebased service view replay | **NOT RUN** | Fail-closed implementation untouched. |
| Per-partition I/O | **NOT RUN** | Separate milestone. |
