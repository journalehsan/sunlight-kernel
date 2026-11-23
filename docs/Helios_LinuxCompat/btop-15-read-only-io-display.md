# BTop-15: Read-only partition stats and btop replay

**The Linux stats record stays unavailable.** The final read-only image
has an attributable retained read trace and an internal busy ledger.
It lacks authoritative lifetime read completions, sectors, and request
latency, and its busy clock and accounting boundaries do not reproduce
the selected Linux ABI. Only this report is a persistent change.
Continues [BTop-10](btop-10-partition-read-io.md),
[BTop-11](btop-11-partition-io-stats.md) and
[BTop-14](btop-14-partition-busy-time.md).

## Record and join

The v1.4.5 checkout is clean at
`a1099d075fa914452366b28c67ff171e21a1f02e`.
In `src/linux/btop_collect.cpp:2068`, the third mountinfo field determines
`/sys/dev/block/<major>:<minor>/stat`; the mountpoint supplies the display
row and capacity path. The non-ZFS parser at line 2277 skips fields by
space delimiters and extracts decimal fields 3, 7 and 10. Select the
Linux v6.18 `block/genhd.c::part_stat_show` ABI: one newline-terminated
row of all 17 space-separated fields in order. Earlier partition
formats documented in `Documentation/admin-guide/iostats.rst` include
a historical four-field record; it has different event semantics and
cannot satisfy this collector. Eleven-field historical records also
omit the discard and flush positions of the selected reference.
`/proc/diskstats` adds identity columns, which do not belong here.
The references inspected are the v6.18 source in
`target/btop-11/linux-v6.18-{genhd.c,iostats.rst}` and
`target/btop-15/linux-v6.18-{blk-core,blk-mq,blk-merge}.c`.

Mounted serial line 1136 joins `/mnt` to source 2, parent 1,
extent [2048,133120). Line 3002 shows PID 42 looking up
`/sys/dev/block/240:2/stat` and getting ENOENT. That number is derived
from this boot's registry; no number was hard-coded in the kernel.
The discovery and lookup join passes; a join to an exported row
was not exercised.

## Backing by field

Linux v6.18 `blk-mq.c` accounts sectors as request bytes are retired
(`bytes >> 9`) and counts/times requests at the accounted end path.
Neither hook is simply a success-status filter. Merges have their own
hooks in `blk-merge.c`. The guest equivalents below are scoped to the
verified VirtIO attachment and its child, not every block transport.

| Field | Unit and Linux event boundary | Guest source and validation |
| --- | --- | --- |
| 1 Reads | requests, accounted read end | No lifetime counter. Retained trace pairs mark retirement including errors, exclude the early boot adapter, and do not by themselves indicate a successful read. QEMU corroborates success in the measured windows only. |
| 2 Read merges | merges, bio/request coalescing | Fixed 512-byte calls, no block request coalescer in these guest adapters or VirtIO submission. Cache hits bypass the device and are not merges. |
| 3 Read sectors (btop) | 512-byte sectors, accounted byte retirement | Validated read completion can substantiate sectors for the retained path; QEMU and payload establish window deltas below. No authoritative partition-lifetime byte ledger. |
| 4 Read time | ms, sum of accounted request lifetimes at end | No duration sum starting at an equivalent request-accounting boundary. BTop-14 measures publication through software return, with coarse timestamps. |
| 5 Writes | requests, accounted write end | RO rejection precedes device request publication for all traced source routes described below; no admitted write can reach an accounted device completion, including a failed completion. Rejected caller operations are not such requests. |
| 6 Write merges | merges, bio/request coalescing | No guest write request merge machinery on this path; this conclusion does not depend on completion or success. Cached dirty data is not a merged device request. |
| 7 Written sectors (btop) | 512-byte sectors, accounted byte retirement | No admitted write can retire bytes because the immutable driver guard rejects before publication. QMP reports zero fixture bytes written only as corroboration. |
| 8 Write time | ms, sum of accounted write request lifetimes | There is no admitted write interval at the block device boundary. Caller work and elapsed time returning a validation rejection are not completed block request lifetimes. |
| 9 In flight | requests, instantaneous accounted depth | The ledger has internal depth for retained reads after admission; it omits earlier boot paths and has no exported coherent depth snapshot. Timeout retirement is software state. |
| 10 Busy (btop) | ms, jiffy stamp accounting at start/end/active readout | Retained read union only; see timing comparison. |
| 11 Weighted time | ms, cumulative sum of operation-class request nanoseconds at completion, converted once | No authoritative duration sum. The BTop-14 union does not implement the per-request sum. |
| 12 Discards | requests, accounted discard end | VirtIO accepts no discard feature and does not implement a discard request opcode here. No discard can reach retirement. |
| 13 Discard merges | merges, discard coalescing | No discard admission or merge queue exists; the event is absent independently of discard success. |
| 14 Discard sectors | 512-byte sectors, accounted discard progress | No discard operation can retire sectors on this device path. |
| 15 Discard time | ms, sum of accounted discard lifetimes | No discard request interval exists at the device boundary. |
| 16 Flushes | requests, accounted disk flush end | Linux reports flush accounting at disk scope and documents that it is not tracked for partitions. Partition zero follows from scope, not from RO or no payload. |
| 17 Flush time | ms, sum of accounted disk flush lifetimes | Same partition scope exclusion. A parent flush may still take time; none of that is a partition flush duration. |

`kernel/src/main.rs::init_block_and_fat` registers capabilities from
the driver instance it retains; the child binding is by parent and
extent. `sunlight-virtio/src/blk.rs` captures offered RO bit 5 at
initialization, accepts that bit and never changes the instance's
`read_only`. The early `VirtioBootDisk` and retained `KernelBlkDev`
both call `write_typed`. The alternate staged write entry
also funnels into `start_transfer`. Capacity, FAT and storage
calls use those adapters; USB is a separate attachment. The
write guard at line 546 precedes descriptor construction and
publication, regardless of mount flags. Validation and device
availability rejections do not publish requests. A failed device
status would be a different event from an admission rejection.
The sole fixture is confirmed read-only by QMP in `attached.json`;
that corroborates which attachment the source audit applies to.
Flush capability is separately negotiated; both adapters can call
`flush_typed` and the guard checks flush support rather than RO.
Disk-level flush count and timing therefore cannot be inferred zero.
RO does not close the missing read lifetime and timing fields.

## Busy time and collector behavior

In v6.18 `blk-core.c::update_io_ticks`, a successful advance of
`bd_stamp` charges the jiffy difference if called as an end or if
there is in-flight work. `blk-mq.c` calls it before incrementing
in-flight on start and at accounted completion. `part_stat_show`
also calls it as an end-style update if the sampled depth is nonzero.
Same-jiffy calls do not advance the stamp. The exported value uses
`jiffies_to_msecs`; tick size depends on Linux HZ. Charging depends
on the sampled depth, event order and successful stamp advance.
Concurrent updates need not account all elapsed busy time exactly;
the estimate is neither a sum of latency nor a guarantee of the
precise union.

The guest banks elapsed monotonic time between first admission
and last retirement with 10 ms endpoints. Consecutive active
snapshots are nonmutating: for an isolated request beginning at
tick 100 ms and active at 110 and 120 ms the ledger reports +10
then +10 ms; retirement at 130 ms banks +30 ms total, and a later
idle sample adds zero. A serialized Linux request with matching
boundaries and aligned equal ticks can yield those same deltas:
the stat reads advance the stamp, and completion charges only
the remainder. That conditional correspondence does not resolve
queue residence, tick granularity, operation coverage, or a timed-out
request still owned by the transport. Sub-tick work contributes zero
when both endpoints fall in the same tick; no activity is padded.

Host `BusyTime::idle_single_and_overlap` exercises active and
idle snapshots and overlap: starts at 30 and 33, stops at 38 and
43, with the busy contribution from the union [30,43) rather than
the sum of lifetimes. VirtIO tests cover preflight rejection and
retirement on bad status and timeout. The guest mutex serialized
every captured request. Its paired retirement snapshots are not
independent stat reads across an active request. Independent active
guest sampling and overlap are NOT RUN. BTop-14's original 10 ms
interval remains historical evidence; the new replay observed
zero ticks for all 1,302 intervals, not zero physical duration.

btop seeds the first read sample at zero. Later read samples
are `max(0, (cur_sectors - prev_sectors) * 512)`; the draw code
humanizes the bytes directly for the rate label. Actual bytes/sec
would also require the elapsed sample time. Utilization is
`clamp(round(delta_ms / delta_uptime_seconds / 10), 0, 100)`
with ties away from zero and zero for the first sample or unchanged
uptime. With ENOENT there are no sampled stat times or raw stat
deltas; the phase markers below are workload delimiters, not
collector timestamps.

## Fresh read-only and no-disk evidence

Both replays use the final BTop-14 ISO at 4096 MiB and four vCPUs.
The disk replay verifies the full 65,536-byte payload and EOF twice
(`readonly/serial.log:2618,2788`). The verifier rebuilds the interval
union from raw guest begin/end timestamps and agrees with the
prefix snapshots. QEMU supplies independent LBA and completion status
and QMP supplies byte totals, without guest timestamps.

| Workload window | Guest/QEMU sectors | Bytes | Guest reconstructed busy |
| --- | ---: | ---: | ---: |
| Idle | 0 | 0 | 0 ms |
| First read | 147 (128 payload) | 75,264 | 0 ms |
| Repeat read | 146 (128 payload) | 74,752 | 0 ms |

Exact LBA multisets match including additional filesystem reads.
Fifteen fixture LBA 0 boot reads and the separate IDE CD traffic
are outside the partition accounting. CD-only has zero VirtIO
events and ENOENT twice for the payload. This is fixture/boot
isolation; no sibling stats-row isolation could be probed.

Raw evidence: `target/btop-15/{readonly,none}/` contains actual
QEMU arguments, `serial.log`, `qemu.log`, `phases.jsonl`,
`attached.json`, and QMP `block-*.json`. Dashboard screenshots
are `dashboard-{0,1,2}.png`, Help is `help.png`, return is
`dashboard-return-0.png`, and the final shell is `after.png`.
Mounted dashboard text shows 62.9 MiB FAT capacity; the CD-only
dashboard has no FAT capacity row. Both replays assert repeated
CPU, memory, process and Ethernet reads and real Help open/close.
Mounted leader 42 and worker 44 are reaped at lines 4441 and 4429;
btop exits 0 after q at 4445, and separate same-TTY proc-native exits
0 at 4473. CD-only leader 40 and worker 41 are reaped at 2694 and
2682; the respective exits are 2698 and 2722.

## Commands and hashes

Base source revision `ca178af0ce904e6f4031ccb3499646dfe5a11ca9`
with incoming worktree changes preserved. No rebuild of the btop
ISO was needed. The native C lockfile was temporarily regenerated
for parity and restored byte for byte.

```sh
out=target/btop-15
iso=target/btop-14/final/guest-ramfs-trace-config.iso
mkdir -p "$out"
cp target/btop-10/payload-lbas.txt "$out/"
echo 'bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702  target/btop-01/btop' | sha256sum -c
sha256sum target/btop-01/btop "$iso" target/btop-10/{disk.img,READ10.BIN}
python3 tools/btop-10/replay.py "$iso" "$out/readonly" --disk target/btop-10/disk.img > "$out/readonly.log" 2>&1
python3 tools/btop-10/replay.py "$iso" "$out/none" > "$out/none.log" 2>&1
python3 tools/btop-14/verify.py "$out/readonly" > "$out/verify-readonly.log"
python3 tools/btop-14/verify.py "$out/none" > "$out/verify-none.log"
cargo test --target x86_64-unknown-linux-gnu -p sunlight-fs -p sunlight-fat -p sunlight-block -p sunlight-compat-linux -p sunlight-virtio > "$out/focused.log" 2>&1
(
    set -e
    cp -p tools/native-c/Cargo.lock "$out/native-c.lock.before"
    trap 'cp -p "$out/native-c.lock.before" tools/native-c/Cargo.lock' EXIT
    cargo generate-lockfile --offline --manifest-path tools/native-c/Cargo.toml > "$out/parity-lock.log" 2>&1
    bash tools/runtime-parity/run.sh "$out/parity" > "$out/parity.log" 2>&1
)
sha256sum "$out/parity/guest/guest.iso" target/btop-10/disk.img
```

| Artifact | SHA-256 |
| --- | --- |
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Final btop ISO | `4df853e5b65f8ccdec95b9e51d11d8bd714fc98e839e010733ec7774fe903c80` |
| Fixture, before and after replay | `4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f` |
| Payload | `21ee0c5a4f5a4ce80bab81105d1fb73d857e6e7464b915a02890b130013de699` |
| Separate parity ISO | `7129a0f55edb3bb0701d170799afb97510e216d22b7bb7ffe28f882eb7ff548d` |

| Gate | Result |
| --- | --- |
| Linux ABI suitability | **BLOCKED**. No complete authoritative record. |
| Device discovery and lookup join | **PASS**. Bound mount to partition and observed the matching lookup. Exported record join **NOT RUN**. |
| Controlled read deltas | **PASS** for payload, retained guest events and QEMU. Exported sector delta **NOT RUN**. |
| Utilization mapping | **BLOCKED**. The internal union verified; the Linux boundary and granularity mapping remains unsupported. Active guest stat sampling and overlap **NOT RUN**. |
| Independent Linux/guest stat probe | **NOT RUN**. No ABI-valid guest row. |
| btop read display | **BLOCKED**. No stat samples to reconcile; coordinated collector replay **NOT RUN**. |
| Capacity, CD-only and interactive regressions | **PASS**. Both replays completed with explicit exits and reaping. |
| Focused regressions | **PASS**. 69 block, 29 Helios, 10 FAT, 113 filesystem and 31 VirtIO tests. |
| Native/musl parity | **PASS** on the separate ISO above: ELF, file, TLS, join and ABI routing. Same-image parity **NOT RUN**. |
| BTop-12 historical write range | **FAIL**. Successful out-of-range LBA 0 write remains unexplained by a passing range control. |
| BTop-13 guest discovery | **BLOCKED**. QEMU boot isolation did not establish guest binding. |
| Writable guest and btop write display | **NOT RUN**. No writable replay resumed. |
