# BTop-11: Partition I/O stats contract and replay

Continues [BTop-10](btop-10-partition-read-io.md) and
[BTop-09](btop-09-live-storage-capacity.md). The read replay and device
join were reproduced. The stats row remains unavailable: there is no
partition-lifetime request ledger for the required timing and in-flight
fields. No kernel policy, mountinfo behavior, or btop source changed.
The changed paths are this report and `tools/btop-11/write-control.py`.

## Pinned consumer and Linux record

The clean btop v1.4.5 checkout in `target/btop-01/src` is
`a1099d075fa914452366b28c67ff171e21a1f02e`. The binary SHA-256 is
`bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702`.
`src/linux/btop_collect.cpp:2068` reads `/proc/self/mountinfo`. It uses
the third whitespace field, major:minor, to construct
`/sys/dev/block/<major>:<minor>/stat` and checks existence/readability.
The mountpoint selects the display row and provides the statvfs path.
Neither the mount source string nor the disk name forms the stats join.

The non-ZFS parser at line 2277 skips fields using whitespace followed
by a literal space delimiter and extracts three signed decimal integers
with stream extraction. Supply a single line of space-separated decimal
values with a trailing newline; the consumer does not validate every
extraction. The parser stops at field 10. Linux v6.18
`block/genhd.c::part_stat_show` emits all 17 positions:

| Position | Meaning | Units | btop |
| --- | --- | --- | --- |
| 1, 2 | Reads completed, read merges | requests | skips |
| 3 | Sectors read | 512 B sectors | reads |
| 4 | Total read request time | ms | skips |
| 5, 6 | Writes completed, write merges | requests | skips |
| 7 | Sectors written | 512 B sectors | reads |
| 8 | Total write request time | ms | skips |
| 9 | I/O currently in progress | requests | skips |
| 10 | Time device is busy | ms | reads |
| 11 | Weighted I/O time | ms | stops before this |
| 12–15 | Discards completed, merged, sectors, request time | requests, requests, 512 B sectors, ms | — |
| 16–17 | Flushes completed and request time (disk scope) | requests, ms | — |

Linux v6.18 reference: `Documentation/admin-guide/iostats.rst`,
`Documentation/block/stat.rst`, and `block/genhd.c`. For field 10 the
semantic target is elapsed time with at least one outstanding request.
Linux implements a jiffy-based estimate updated at lifecycle events and
can undercount overlap; v6.18 also updates the estimate on a stat read
when I/O is in flight. Fields 4 and 8 sum individual request times;
Linux v6.18 formats field 11 from the sum of operation-class nanoseconds
truncated to milliseconds. Conceptually weighted time accrues at the
in-flight depth; two overlapping requests can add to the latency sum
without doubling wall busy time. Sectors are always 512 bytes regardless
of the logical block size. Partition flush fields are not disk flush
totals; a flush supplies no payload bytes.

btop seeds its first sample with zero and then graphs
`max(0, sector_delta * 512)`. At `src/btop_draw.cpp:1336` and :1362
the label humanizes those bytes directly. Actual throughput requires
division by measured sample elapsed time; the configured 1000 ms cadence
does not enter the byte-delta formula. Utilization is
`clamp(round(busy_ms_delta / uptime_seconds_delta / 10), 0, 100)`;
first sample or equal uptime yields zero.

## Guest ownership and timing boundary

The current mounted replay resolves mount 2 at `/mnt` to source 2,
parent device 1, extent [2048, 133120). The registry-derived mountinfo
identity is `240:2` in this boot; it is not a fixed partition number.
`readonly/attached.json` ties the sole VirtIO attachment to the
read-only fixture. `readonly/serial.log:1136` records the VFS join,
and :2985 records btop PID 41 getting ENOENT on the joined stat path.
There is no sampling open or usable stat snapshot.

`KernelBlkDev` serializes retained VirtIO calls using its mutex.
At `sunlight-virtio/src/blk.rs::start_transfer`, validation failures
occur before descriptor publication; an already pending request rejects
another admission. Partition-relative offsets have already been
translated for the retained path, and cache hits never reach the
transport. At `finish_transfer`, only a validated used-ring completion
with OK device status authorizes the full fixed-size 512-byte transfer.
A failed status does not authorize a prefix; legacy used length cannot
establish partial success. A successful write receipt establishes
completion; durability needs an independent barrier. A timeout
quarantines the attachment and requests cancellation while retaining
pending DMA ownership. Late ring activity cannot turn that outcome into
a successful receipt. Timing retirement would have to distinguish
terminal guest observation from actual transport quiescence.

A transport ledger would start at queue publication and end at validated
terminal observation; that excludes cache work and caller lock wait and
differs from Linux request-queue residence. An in-flight snapshot must
observe ongoing time without taking the mutex held by the synchronous
transfer: otherwise the sampling path sees only completed requests.
The cross-core public guest monotonic clock in
`kernel/src/timekeeping.rs::monotonic_ns` advances in 10 ms ticks.
An implementation using it must disclose quantization (including
sub-tick transfers and periods with delayed ticks); convert cumulative
time to integer milliseconds at export rather than round each request.
The BTop-10 trace contains only successful retained read completions;
it has neither submission timestamps nor early `VirtioBootDisk` traffic.
The available QEMU event stream establishes device, LBA and order but
has no timestamps. The host phase marks include TTY latency. Even with a
timestamped QEMU trace, its clock and its backend interval differ from
guest queue publication and guest completion observation.

No new busy-time counter was implemented. Fields 4, 9, 10 and 11
therefore lack backing, and fields 1 and 3 are verified phase deltas
rather than a partition-lifetime ledger. No truncated or padded row is
exported. The read-only QEMU backend and the negotiated VirtIO RO guard
provide a route to justify write zeros over an immutable attachment
lifetime, after verifying that binding for the stats source. Mount flags
and zero observed write traffic alone do not establish that binding.

## Disposable raw write control

The existing `sfs5b4-test` route checks the 8192-sector device capacity
and the disposable marker at LBA 0 before attempting its test writes
at LBAs 62 and 63. The runner creates a fresh exclusive regular file,
checks its size, contents and test address range, and attaches it as
the sole VirtIO disk, separate from the read-only boot ISO. It does not
use the mounted replay disk. This raw device has no partitions.

The first run panicked for lack of a framebuffer; the image remained
byte-identical. The retry with a standard VGA framebuffer reached
`NORMAL_COMPLETION` and `READY`. QEMU shows successful one-sector
writes at 62 and 63; a whole-file comparison found exactly those two
512-byte sectors changed to 0x62 and 0xb4 respectively. Both traces
also contain an unexpected successful LBA 0 write, which left the
marker sector byte-identical. Its origin was not established. The runner
rejects both traces on that unexpected request. These observations
show the intended raw writes occurred but do not pass the range control
or establish partition write counters, failed/partial operation deltas,
or partition isolation. The pre-readback flush is separate from the
1024 bytes of intended payload.

```sh
cargo build -p sunlight-kernel --offline --locked --features sfs5b4-test
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/btop-11/write.iso
python3 tools/btop-11/write-control.py target/btop-11/write.iso target/btop-11/write-retry
```

See `write{,-retry}/{command.json,serial.log,qemu.log}`. The write ISO
SHA-256 is `187be09182a894494a94f6e454f1372e15d11113d3671d976717b83eeb2b92d3`;
the initial image is
`e56d69a36098d85c525133521a6a1772902eb1812363b06a92d1f4d5b360016f`
and the retry image is
`9a3f4eb4aee18baa1c7303c2b59278f4268c2f2c9296ccec3864161c198a63d9`.

## Evidence and commands

The replay used the unchanged BTop-10 traced ISO, 4096 MiB and four
vCPUs. Logs and screenshots are under `target/btop-11`. In the mounted
run the idle interval showed 0 receipts, the first read showed 147
sectors (128 payload + 19 metadata, 75,264 B), and the repeat showed
146 sectors (128 + 18, 74,752 B). Each guest completion multiset matched
the QEMU LBA/status events. LBA 0 MBR reads (15) and 339,023,872 bytes
of boot ISO reads remained outside the partition ledger. See
`readonly/phases.jsonl` for host monotonic marks and byte offsets,
`readonly/block-*.json` for QMP totals and `verify-readonly.log` for the
reconciliation. No btop stats sample times or deltas are available.

The dashboard, Help, return and shell captures are
`dashboard-2.png`, `help.png`, `dashboard-return-0.png`, and
`after.png` in `readonly/` and `none-retry/`. The first no-disk run
(`none/`) captured a dashboard instead of Help and failed the Help
assertion. The unchanged replay passed on retry. The mounted capacity is 62.9 MiB
and the no-disk run has no FAT row. The replays check repeated CPU,
memory, process and Ethernet queries, real Help open/close, explicit
btop exit 0 and leader/worker reaping, then independent same-TTY
`/bin/proc-native` exit 0.

```sh
out=target/btop-11
mkdir -p "$out"
cp target/btop-10/payload-lbas.txt "$out/"
iso=target/btop-10/final/guest-ramfs-trace-config.iso
python3 tools/btop-10/replay.py "$iso" "$out/readonly" --disk target/btop-10/disk.img
python3 tools/btop-10/verify.py "$out/readonly"
python3 tools/btop-10/replay.py "$iso" "$out/none-retry"
python3 tools/btop-10/verify.py "$out/none-retry"
cargo test --target x86_64-unknown-linux-gnu -p sunlight-fs -p sunlight-fat -p sunlight-block -p sunlight-compat-linux -p sunlight-virtio
bash tools/runtime-parity/run.sh "$out/parity"
sha256sum target/btop-01/btop target/btop-10/disk.img "$iso"
git diff --check
```

| Replay artifact | SHA-256 |
| --- | --- |
| Disposable read-only disk | `4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f` |
| BTop-10 traced ISO | `3b46fa5f88780a4d1edb2dc5fc95a46329146f5db2a648a79405c9d41c744bbd` |

## Gates

| Gate | Result | Evidence |
| --- | --- | --- |
| Read regression | **PASS** | Bounded read, repeat, idle and LBA isolation with independent QEMU trace. |
| Busy-time accounting | **BLOCKED** | No timestamped guest submission/retirement ledger or independent live snapshot. Busy-time single/overlap, in-flight snapshot and error cleanup accounting tests **NOT RUN**. Same-queue overlap cannot occur in the retained transport. |
| Active partition write accounting | **BLOCKED** | Existing raw fixture is unpartitioned and has no partition ledger; no authorized btop-visible write fixture. |
| Disposable raw write range control | **FAIL** | QEMU reports unexpected LBA 0 writes; intended writes to 62 and 63 independently confirmed on retry. |
| Stats ABI and device join | **BLOCKED** | Device join **PASS**; full ABI row withheld for unbacked timing and lifetime fields. |
| btop read display | **BLOCKED** | Stat ENOENT; rate/utilization reconciliation and activity screenshots **NOT RUN**. |
| btop write display | **NOT RUN** | No sampled writable fixture or stats ABI. |
| Interactive regressions and no-disk control | **PASS** | Hardware TTY replays, exit and reaping evidence. |
| Focused storage and Helios tests | **PASS** | 68 block, 29 Helios, 10 FAT, 113 VFS and 30 VirtIO. VirtIO host tests include failed status, stale token, timeout and cancellation ownership; they do not verify busy accounting. |
| Native/musl parity | **PASS** | Separate guest file, TLS, join and syscall routing checks. |
| Rebased service view replay | **NOT RUN** | Existing fail-closed path unchanged. |
