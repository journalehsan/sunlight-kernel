# BTop-14: Partition busy time on the retained read path

Continues [BTop-10](btop-10-partition-read-io.md),
[BTop-11](btop-11-partition-io-stats.md), [BTop-12](btop-12-lba0-write-attribution.md),
and [BTop-13](btop-13-boot-isolated-write-control.md). The retained read
path now measures the union of attributable in-flight intervals. No Linux
stats row is exposed.

## Timing contract

Pinned btop v1.4.5 source revision
`a1099d075fa914452366b28c67ff171e21a1f02e`,
`src/linux/btop_collect.cpp:2008,2277–2309`, samples
`/sys/dev/block/<major>:<minor>/stat` after discovering the device through
mountinfo. Its non-ZFS parser reads field **10** (one-based), `io_ticks`,
in cumulative milliseconds. The first sample and an unchanged uptime
produce zero. Otherwise it rounds
`(current_ms - previous_ms) / (current_uptime_s - previous_uptime_s) / 10`
to the nearest integer (ties away from zero), clamps to 0–100, and
saves the sample. Uptime is read once for the collection; it is not a
timestamp taken atomically with each device's stat read. The configured
update cadence does not substitute for actual elapsed sample time.
Fields 3 and 7 are sector counters; btop does not consume weighted field 11.

Linux's [I/O statistics documentation](https://docs.kernel.org/admin-guide/iostats.html)
describes field 10 as elapsed time doing I/O. Since Linux 5.0 the
jiffy-based update depends on starts and completions and can miss time
under concurrency. The pinned Linux v6.18 `block/genhd.c` snapshot in
`target/btop-11/linux-v6.18-genhd.c` also shows an update when sampling
with nonzero in-flight I/O and conversion with `jiffies_to_msecs`.
Fields 4 and 8 sum request lifetimes by direction from block request
allocation through end request; two overlapping requests contribute
twice during their shared interval. Weighted time instead integrates the
in-flight depth over time; v6.18 reports the combined per-operation
nanosecond accumulators converted to whole milliseconds. Neither
quantity is the elapsed union measured here. For example, intervals
[10,20) and [15,25) give elapsed busy time 15 and summed latencies 20;
the depth integral is also 20 over that fully observed window.

The guest uses the BSP-owned global monotonic tick clock
(`kernel/src/timekeeping.rs`), which is ordered across cores. Times are
stored as nanoseconds with **10 ms resolution**. The clock quantizes
each endpoint down to its tick; two events in one tick contribute zero.
The ledger uses those endpoint differences without additional rounding.
The path includes synchronous polling, validated completion and copying
the read payload; it excludes VFS cache hits, the time waiting for the
retained device mutex, and transport admission before publication.
Linux block-layer queue and merge boundaries, jiffy behavior, operation
coverage and completion accounting are different.

## Ownership and synchronization

The retained VirtIO parent ID and absolute LBA resolve to exactly one
registered child extent. Missing or ambiguous matches are unaccounted.
Only the retained attachment uses the bounded array of partition ordinals
from its fixed registry; there is no ordinal reuse on this path. Early
boot I/O and USB do not use it. The driver observer begins immediately
before publishing the avail index, after validation. The returned guard
ends when the synchronous call returns, including failed status,
malformed completion or timeout. On timeout the existing cancellation
request and queue quarantine still retain pinned DMA; ending the
software interval does not establish that the hardware stopped.

The retained VirtIO mutex serializes submissions. Registry lookup only
holds the registry lock for the lookup. A separate small ledger lock
serializes timestamps, transitions and snapshots; snapshot readers
need no queue mutex. The first begin opens an interval, additional
begins only increment depth, and the last finish banks the interval.
A snapshot while active adds elapsed time since the first begin without
closing or banking it. The opt-in trace emits paired endpoints and two
snapshots at retirement (one just before decrement, one just after).
It prints after both timestamps and snapshots have been acquired, so
serial output is outside the measured request but can affect pacing.

Deterministic host tests cover idle, a single request, overlapping
requests, active snapshots and final idle. The VirtIO host test exercises
the real synchronous observer with injected failed status and timeout
(cancellation requested), and a preflight rejection before publication.
On both errors the read buffer remains unchanged and a later snapshot
shows no phantom activity. The timeout test still expects uncertain DMA.
There is no independent cancellation entry on the synchronous read
path. Guest failure injection and guest overlap were not run.

## Guest evidence

The 4096 MiB, four-vCPU replay uses the original BTop-10 FAT32 image as
a QEMU read-only VirtIO backend and a separate IDE boot CD. The offline
payload map and unchanged musl probe verify both full 65,536-byte reads
and EOF. The stream records 1,302 paired partition events, all within
parent 1 / partition 2 / absolute range [2048,133120). QEMU status and
LBA multisets match each workload window. QMP also shows zero fixture
write bytes. The verifier merges raw guest endpoints and compares each
prefix to the active and retired snapshots. QEMU supplies independent
device/range evidence and no guest clock values.

| Phase | Read requests (payload + other) | Bytes | Reconstructed guest intervals, ns |
| --- | ---: | ---: | --- |
| Idle | 0 | 0 | none |
| First | 128 + 19 | 75,264 | [22,150,000,000, 22,160,000,000) |
| Repeat | 128 + 18 | 74,752 | none at tick resolution |

The only nonzero pair was sequence 1017, LBA 4104, in
`readonly/serial.log:2465`. The cumulative active and retired
snapshots both read 10,000,000 ns there. All other measured pairs
started and ended on the same tick. This does not mean those reads
took zero physical time. No concurrent requests were observed. The CD
accounted for 338,794,496 bytes and QEMU recorded 15 fixture LBA 0
reads before the workload; neither contributes to partition time.
The CD-only boot carries no fixture or partition events. Host marks
in `phases.jsonl` delimit log byte offsets only.

## Reproduction and artifacts

Base revision: `ca178af0ce904e6f4031ccb3499646dfe5a11ca9` with
pre-existing version and report edits. Persistent milestone paths:
`kernel/src/{main,storage}.rs`,
`sunlight-block/src/{busy_time,lib,registry}.rs`,
`sunlight-virtio/src/blk.rs`, `tools/btop-14/{image.sh,verify.py}`,
and this report. The image script reverses its inherited btop fixture
patches. No mount policy, hotplug or fail-closed mountinfo changes.

```sh
out=target/btop-14
mkdir -p "$out"
cp target/btop-10/payload-lbas.txt "$out/"
bash tools/btop-14/image.sh > "$out/image.log" 2>&1
iso=$out/final/guest-ramfs-trace-config.iso
python3 tools/btop-10/replay.py "$iso" "$out/readonly" --disk target/btop-10/disk.img > "$out/readonly.log" 2>&1
python3 tools/btop-10/replay.py "$iso" "$out/none" > "$out/none.log" 2>&1
python3 tools/btop-14/verify.py "$out/readonly" > "$out/verify-readonly.log"
python3 tools/btop-14/verify.py "$out/none" > "$out/verify-none.log"
cargo test --target x86_64-unknown-linux-gnu -p sunlight-fs -p sunlight-fat -p sunlight-block -p sunlight-compat-linux -p sunlight-virtio > "$out/focused-retry.log" 2>&1
(
    set -e
    cp -p tools/native-c/Cargo.lock "$out/native-c.Cargo.lock.before"
    trap 'cp -p "$out/native-c.Cargo.lock.before" tools/native-c/Cargo.lock' EXIT
    cargo generate-lockfile --offline --manifest-path tools/native-c/Cargo.toml > "$out/parity-lock.log" 2>&1
    bash tools/runtime-parity/run.sh "$out/parity-retry" > "$out/parity-retry.log" 2>&1
)
sha256sum target/btop-01/btop target/btop-10/{disk.img,READ10.BIN} "$out"/final/{probe.elf,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso}
git diff --check
```

| Artifact | SHA-256 |
| --- | --- |
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Disposable read-only disk | `4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f` |
| Payload | `21ee0c5a4f5a4ce80bab81105d1fb73d857e6e7464b915a02890b130013de699` |
| Guest probe | `7a325dfdf08e493157bcf98ca2e035a8b71f95b8278960177b9f2a7c8bfd32fb` |
| Kernel | `34024bfcffeb81b2f2b9f17999a3745b7834667a955496d6a1ffd9796a6837f2` |
| btop ISO | `4df853e5b65f8ccdec95b9e51d11d8bd714fc98e839e010733ec7774fe903c80` |

Raw evidence under `target/btop-14/{readonly,none}/`:
`network-command.json` (actual QEMU args), `attached.json`,
`serial.log`, `qemu.log`, `phases.jsonl`, `block-*.json`,
and dashboard, Help, return and shell captures. `verify-*.log`
contains the reconstructed intervals. The mounted dashboard shows
62.9 MiB capacity. Mounted btop leader 42 and worker 44 were reaped;
serial lines 4475, 4487, 4491 and 4515 record the two reaps, btop
exit 0 after q, and separate same-TTY `/bin/proc-native` exit 0.
The unchanged replay also checks repeated CPU, memory, process and
Ethernet sampling and opens and closes Help.
The CD-only replay has no FAT capacity row; the probe returned ENOENT
and exit 3 twice. Its btop leader 40 and worker 41 were reaped (serial
lines 2632 and 2646), btop exited 0 after q at line 2650, and the
separate same-TTY native probe exited 0 at line 2674.

The first focused build rejected never-type inference in the added
test; the explicit unit type fixed it. The first parity attempt failed
before boot on the standalone native C lockfile's mismatch with
incoming versions. Retrying with a temporary regenerated lockfile
passed ELF, file, TLS, join and ABI routing checks in the standard
separate parity guest built from this source. The original lockfile
was restored byte for byte. The parity ISO digest is
`05750ffefb32b8266045d399a67a9f0467b3534be7b2d7228dc0f4f9917dfdac`.
The CD-only verifier initially rejected the plain QEMU shutdown
message. It now checks for VirtIO events rather than requiring an empty
stderr file. Both raw replays were retained.

| Gate | Result |
| --- | --- |
| Internal busy time | **PASS**. Host deterministic accounting and failed/timeout cleanup; guest reconstruction agrees with snapshots on the retained read path. |
| Linux ABI suitability | **BLOCKED**. Full row and semantic mapping remain unsupported. No write metrics inferred. |
| Guest overlap | **NOT RUN**. The retained queue serialized the captured requests; overlap proved only with timestamps in a host unit test. |
| Mounted read, idle and isolation regressions | **PASS**. Payload/EOF twice, exact QEMU LBA multisets, idle and excluded boot traffic. |
| Focused regressions | **PASS**. 69 block, 29 Helios, 10 FAT, 113 filesystem, 31 VirtIO. |
| Native/musl parity | **PASS** on the separate parity image. Exact btop ISO parity **NOT RUN**. |
| CD-only read and isolation control | **PASS**. ENOENT twice, no VirtIO events or partition samples. |
| Pinned btop capacity and interactive baseline | **PASS** on both boots. Help, q, reaping and same-TTY native probe. |
| Active partition writes and btop I/O display | **NOT RUN**; outside this milestone. |
| BTop-12 historical write range control | **FAIL**. The successful out-of-range LBA 0 write is still a failed range control. |
| BTop-13 guest discovery | **BLOCKED**. Boot isolation passed at QEMU's boundary; guest binding was not proven. The boot-isolated writable control was not run. |
