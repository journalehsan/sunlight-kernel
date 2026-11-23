# BTop-10: Backed partition reads on disposable read-only FAT32

Continues [BTop-08](btop-08-mountinfo-capacity.md) and
[BTop-09](btop-09-live-storage-capacity.md). The guest completion trace
and QEMU agree sector for sector. btop discovers the correct device
identity, but its I/O display remains blocked. No Linux statistics row
was added. Write I/O and live capacity allocation/recovery are out of scope.

## Caller and device join

The unmodified btop v1.4.5 checkout at
`target/btop-01/src` is clean at
`a1099d075fa914452366b28c67ff171e21a1f02e`.
In `src/linux/btop_collect.cpp:2068`, btop opens
`/proc/self/mountinfo`; it uses whitespace field 3 (major:minor) to
form `/sys/dev/block/<major>:<minor>/stat`. It uses the mountpoint
for its display and filter and uses statvfs for capacity. The canonical
source path and device name are not the stats join. Discovery requires
filesystem existence and read access; sampling rechecks readability
before opening the saved path. The non-ZFS stream parser at line 2277
consumes at least ten positional decimal fields, in Linux order:

| Position | Field | Unit |
| --- | --- | --- |
| 1–4 | Reads completed, reads merged, sectors read, read time | requests, requests, 512-byte sectors, ms |
| 5–8 | Writes completed, writes merged, sectors written, write time | requests, requests, 512-byte sectors, ms |
| 9–10 | I/O in progress, busy time | requests, ms |

Linux also defines weighted I/O time at field 11 and later discard/flush
fields; this parser stops at field 10. It extracts fields 3, 7, and 10
without validating each extraction. Read and write samples are nonnegative
sector deltas times 512, seeded with zero on the first sample. Busy
percentage is rounded and clamped to 0–100 from the busy-time delta
divided by the uptime delta in seconds and by 10. The pinned
`src/btop_draw.cpp:1336` humanizes the sampled bytes directly;
`update_ms = 1000` is a configured cadence, not a divisor. A measured
throughput comparison would need actual elapsed sample time.

On the final image, the VFS mount record is mount 2, source 2,
parent disk 1, absolute LBA extent [2048, 133120), path `/mnt`.
`sunlight-fs/src/vfs/mountinfo.rs` renders source 2 as `240:2`.
QMP `attached.json` identifies the sole VirtIO block attachment as
`fixture`, with `ro: true`, pointing to the disposable image. The
ISO is a separate read-only IDE CD device. `readonly/serial.log:2972`
records btop PID 41 opening its own mountinfo; the lookup at line 2979
for `/sys/dev/block/240:2/stat` returns −2 (ENOENT). The trace also
contains an earlier record for the same syscall path with raw result
4; that is not evidence of a usable stats file. No stats sampling open
was observed. Successful syscall 137 (statfs) calls for `/mnt` start
at line 3006. The no-disk replay has neither the FAT mount nor the
`240:2` stat lookup.

## Completed reads

`tools/btop-10/image.sh` temporarily instruments the retained VirtIO
branch of `KernelBlkDev::read_block` below the VFS block cache and the
partition translation. It sequences only successful returns from
`read_typed(...).legacy()` under the device mutex, recording the parent
ID, absolute LBA and returned buffer length. The typed path validates
the completion and copies the payload before success; errors, timeouts,
and cache hits don't count. A trace limit of 4096 records is enforced and
the verifier rejects truncation. Build patches are reversed on exit.
There is no persistent production instrumentation or new public ABI.

Attribution uses the registry parent and non-overlapping partition
extent, not a name or mountpoint. The measured path submits synchronous
single-sector reads under the lock; there is no guest merging in this
path. The QEMU trace is correlated by device and pending request token
and uses absolute sector ranges with successful request status. Transport
byte lengths, QEMU sector counts, and userspace payload were compared
independently: each measured transfer has 512 bytes, QEMU reports one
sector, and the offline FAT map assigns 128 distinct sectors to the
65,536-byte file. Accounting uses bytes divided by 512 and never scales
by a reported logical block size. Native 4 KiB logical-block operation
was not exercised. This trace does not cover early `VirtioBootDisk`
reads, USB, writes or Linux busy-time semantics. QMP whole-device totals
include early traffic and are a cross-check only.

The payload is written offline with mtools into a fresh regular-file
copy of the verified BTop-07 disk. The offline scanner walks the FAT
chain and compares every payload sector; the files occupy blocks on
the host (disk: 133120, payload: 128, in 512-byte allocation units).
The Linux and guest static musl programs validate every byte and EOF.
The guest uses the ordinary hardware TTY path, with no privileged
login harness or guest mount changes. Inherited informational
`storagectl devices` and `storagectl mounts` both returned 1 and
provided no evidence for the join.

| Phase | Host monotonic ns, start → end | Completion sequence | Payload / other sectors | Bytes |
| --- | --- | --- | --- | ---: |
| Idle | 116415377550223 → 116418379126532 | 1009 → 1009 | 0 / 0 | 0 |
| First | 116418379126532 → 116421282382748 | 1009 → 1156 | 128 / 19 | 75264 |
| Repeat | 116421282382748 → 116424152696334 | 1156 → 1302 | 128 / 18 | 74752 |

Both reads reached all payload LBAs [4099, 4227) once. The other
completions hit FAT sectors 2080/2081 and the root directory at 4098.
The repeat was not a cached payload read: the cache is direct mapped
with 16 slots and cannot hold the sequential 128-sector working set.
One fewer metadata request was observed on repeat; the trace does
not identify the precise cache hit. There is no evidence of additional
payload readahead beyond the file range. Host phase windows include
TTY input and command turnaround and are not device service times.

QMP fixture read bytes at the four marks were 934400, 934400, 1009664,
1084416 (read operations 1152, 1152, 1299, 1445); write bytes were zero
at all marks. Boot isolation: QEMU reports 339023872 bytes read from
the distinct IDE CD backend in both boots, and 15 successful fixture
MBR reads at LBA 0 on the disk boot. Neither enters the partition
ledger. The CD-only guest runs the same probe twice, with ENOENT and
exit 3 both times, no VirtIO read events, and no guest read receipts.
A second simultaneously mounted partition and a deliberate
concurrent unrelated-device workload were not tested.

## Replay and evidence

Both runs use the same 4096 MiB, four-vCPU ISO on QEMU 11.1.3 TCG.
See `target/btop-10/{readonly,none}/network-command.json` for actual
QEMU arguments, `attached.json` for backing, `qemu.log` for sector
requests/completions, `phases.jsonl` for timestamped byte offsets,
and `block-*.json` for raw QMP totals. The mounted dashboard
(`readonly/dashboard-2.png`) shows 62.9 MiB total and free, with
used/free rounded to 0%/100%. Offline FAT: 129022 total and 128893
free/available fragments of 512 bytes. The CD-only dashboard has
no FAT capacity row. Both show repeated CPU, memory, process and
Ethernet sampling. `help.png`, `dashboard-return-0.png` and
`after.png` cover Help, close, and return to the shell. Explicit btop
exit 0 and leader/worker reaping are in each serial log (disk: leader
41, worker 43; no disk: leader 39, worker 40). A separate same-TTY
`/bin/proc-native` exited 0 in both.

btop is missing a truthful block-stat record: the measured retained
read ledger cannot supply Linux-compatible write totals and busy
time or the intervening required fields. A read-only backend and zero
observed writes during this experiment do not define those ABI
counters. Coordinated btop sampling, an activity screenshot and rate
reconciliation were therefore not run.

```sh
set -euo pipefail
out=target/btop-10
mkdir -p "$out/final"
python3 tools/btop-10/fixture.py | tee "$out/fixture.log"
musl-gcc -Wall -Wextra -Werror -O2 -static tools/btop-10/read.c -o "$out/read.elf"
"$out/read.elf" "$out/READ10.BIN" | tee "$out/linux.log"
cp "$out/read.elf" "$out/final/probe.elf"
bash tools/stamp_helios_elf.sh "$out/final/probe.elf"
bash tools/btop-10/image.sh > "$out/image.log" 2>&1
iso=$out/final/guest-ramfs-trace-config.iso
python3 tools/btop-10/replay.py "$iso" "$out/readonly" --disk "$out/disk.img" > "$out/readonly.log" 2>&1
python3 tools/btop-10/replay.py "$iso" "$out/none" > "$out/none.log" 2>&1
python3 tools/btop-10/verify.py "$out/readonly" | tee "$out/verify-readonly.log"
python3 tools/btop-10/verify.py "$out/none" | tee "$out/verify-none.log"
cargo test --target x86_64-unknown-linux-gnu -p sunlight-fs -p sunlight-fat -p sunlight-block -p sunlight-compat-linux > "$out/focused.log" 2>&1
bash tools/runtime-parity/run.sh "$out/parity" > "$out/parity.log" 2>&1
sha256sum target/btop-01/btop "$out"/{disk.img,READ10.BIN,read.elf,final/probe.elf,final/kernel-ramfs-trace-config.elf} "$iso"
git diff --check
```

New paths: this report and `tools/btop-10/{read.c,fixture.py,image.sh,replay.py,verify.py}`.
The captured hashes below identify this run; file timestamps and image
build metadata may change fresh artifact hashes.

| Artifact | SHA-256 |
| --- | --- |
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Disposable disk | `4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f` |
| Payload | `21ee0c5a4f5a4ce80bab81105d1fb73d857e6e7464b915a02890b130013de699` |
| Host probe | `03cecc2da8b046287c26fd0637587e0b8c517e30fca28935bd132da47aee625b` |
| Guest probe | `7a325dfdf08e493157bcf98ca2e035a8b71f95b8278960177b9f2a7c8bfd32fb` |
| Kernel ELF | `60de57e471e5d8777a4f7ed3c44136d02c1f348e3cb01c6ea0d48ed46303b1d8` |
| Final ISO | `3b46fa5f88780a4d1edb2dc5fc95a46329146f5db2a648a79405c9d41c744bbd` |

| Gate | Result | Evidence |
| --- | --- | --- |
| Device identity join | **PASS** | Mount source 2 maps to parent 1 and btop actually looks up `240:2`. |
| Partition stat endpoint | **BLOCKED** | No truthful record for all fields consumed by btop. |
| Completed-read accounting | **PASS** | Runtime retained VirtIO path; exact LBA multiset and bytes agree with QEMU, including metadata. |
| Unrelated-device and outside-extent controls | **PASS** | ISO boot traffic and fixture MBR reads stay outside the partition ledger. |
| No test disk | **PASS** | ENOENT twice, no receipts or FAT row. |
| Concurrent unrelated workload / second partition / 4 KiB logical device | **NOT RUN** | Outside the exercised configurations. |
| btop read-I/O display | **BLOCKED** | No row published; activity screenshots and rate reconciliation **NOT RUN**. |
| Capacity and BTop-08 interactive regressions | **PASS** | Both replays, including explicit exit statuses and reaping. |
| Focused regressions and native/musl parity | **PASS** | 68 block, 29 Helios, 10 FAT, 113 VFS tests; separate parity guest passed file, TLS, join and syscall routing. |
| Rebased service view replay | **NOT RUN** | Existing fail-closed implementation was untouched. |
