# BTop-16: Lifetime partition read accounting

Continues [BTop-10](btop-10-partition-read-io.md), [BTop-14](btop-14-partition-busy-time.md)
and [BTop-15](btop-15-read-only-io-display.md). Scope: the registered
VirtIO child on the proven immutable read-only attachment.

## Ownership and units

The registry zeros a slot before publishing the new runtime ID, while
holding the registry mutex. The child becomes eligible after successful
partition registration. Discovery runs on the whole-disk adapter with
no child ID. At view construction the validated parent and extent bind
to a registered child; the boot and retained adapters carry that identity
through the partition offset translation. They use the same read
accounting helper. Runtime FAT construction carries the registry identity
directly. USB is outside this accounting path. No identity is inferred
from an incoming physical LBA or a mount path.

The counters belong to the registered child through cache/view
reconstruction, file close/reopen, mount changes and repeated queries.
Reboot creates a new registry. No hot removal exists; any future slot
reuse gets a new ID and zero totals. Each metric saturates at u64 max.
Updates and copies of all three fields use the same registry lock.

One logical read is an admitted synchronous VirtIO block read. The
partition view translates once, and each cache miss reaches one
512-byte driver request. Cache hits, filesystem calls, preadmission
failures, and whole-disk reads are outside this count. FAT can issue
many such reads for one file call. The driver emits one descriptor chain
per admitted read, with no block request merge, split or retry; the
descriptors in that chain are not separate requests. The queue lock
serializes the retained path, and the boot path uses the same locked device.
QEMU's host I/O implementation is below this boundary.

Completion advances once when the admitted operation returns to
software, including bad status or an uncertain timeout. Timeout does not
prove hardware completion or release DMA ownership. Successful sectors
advance only after a validated status and full payload copy; the driver
cannot certify a successful partial prefix. The conversion is successful
bytes shifted right by nine, a fixed 512-byte reporting unit independent
of device logical geometry. The host test also charges 4096 successful
bytes as eight sectors. Elapsed time starts in the prepublication callback
after admission and ends at synchronous return. It includes software
polling and validated buffer copy, excludes cache lookup and device mutex
wait, and sums all admitted intervals including failures. Two overlapping
intervals can contribute independently. BTop-14 separately measures the
union for retained reads. Neither is mapped to Linux io_ticks.

The BSP tick clock is monotonic at 100 Hz (10 ms increments). It reads
zero before the timekeeper advances during boot. A zero difference
means no sampled tick crossed, not zero physical latency. Sum raw
nanoseconds first; conversion to integer ms would divide the sum by
1,000,000 once. Boot reads before clock progress have no resolved
duration. This clock and boundary do not establish a Linux-equivalent
lifetime latency.

Linux v6.18 reference: inspected the source copies
`target/btop-15/linux-v6.18-blk-mq.c` and
`target/btop-11/linux-v6.18-{genhd.c,iostats.rst}`. In
`blk_account_io_completion`, Linux accounts retired bytes with
`bytes >> 9`; `blk_update_request` can retire partial or failed bytes.
`blk_account_io_done` increments operations and accumulates elapsed
nanoseconds at request end for eligible requests, without a success
filter. Its request start timestamp predates hardware dispatch;
`part_stat_show` divides the accumulated read nanoseconds into ms.
The guest count agrees with the successful serialized subset, but
software timeout retirement is not proven equivalent to Linux end
accounting. The successful-sector metric intentionally excludes failed
bytes and therefore differs from Linux retired-sector semantics.
No Linux ABI field is qualified here.

## Reproduction

Base revision `ca178af0ce904e6f4031ccb3499646dfe5a11ca9`.
Changed paths: `kernel/src/main.rs`, `kernel/src/storage.rs`,
`kernel/src/storage_partition_test.rs`, `sunlight-block/src/registry.rs`,
`tools/btop-16/{replay,verify}.py`, and this report.

```sh
out=target/btop-16
mkdir -p "$out/final"
sed 's@target/btop-14/final@target/btop-16/final@g' tools/btop-14/image.sh > "$out/image.sh"
SUNLIGHT_BTOP16_TRACE=1 bash "$out/image.sh" > "$out/image.log" 2>&1
iso=$out/final/guest-ramfs-trace-config.iso
cp target/btop-10/payload-lbas.txt "$out/"
sha256sum target/btop-01/btop "$out"/final/{probe.elf,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso} target/btop-10/{disk.img,READ10.BIN} > "$out/hashes.txt"
python3 tools/btop-10/replay.py "$iso" "$out/readonly" --disk target/btop-10/disk.img > "$out/readonly.log" 2>&1
python3 tools/btop-10/replay.py "$iso" "$out/none" > "$out/none.log" 2>&1
python3 tools/btop-10/replay.py "$iso" "$out/readonly-retry" --disk target/btop-10/disk.img > "$out/readonly-retry.log" 2>&1
python3 tools/btop-10/replay.py "$iso" "$out/readonly-final" --disk target/btop-10/disk.img > "$out/readonly-final.log" 2>&1
python3 tools/btop-10/replay.py "$iso" "$out/none-final" > "$out/none-final.log" 2>&1
python3 tools/btop-16/replay.py "$iso" "$out/readonly-paced" --disk target/btop-10/disk.img > "$out/readonly-paced.log" 2>&1
python3 tools/btop-16/replay.py "$iso" "$out/none-paced" > "$out/none-paced.log" 2>&1
python3 tools/btop-16/verify.py "$out/readonly" --boot-only > "$out/verify-boot.log"
python3 tools/btop-16/verify.py "$out/readonly-final" > "$out/verify-readonly.log"
python3 tools/btop-16/verify.py "$out/readonly-paced" > "$out/verify-paced.log"
python3 tools/btop-16/verify.py "$out/none-paced" > "$out/verify-none.log"
cargo test --target x86_64-unknown-linux-gnu -p sunlight-fs -p sunlight-fat -p sunlight-block -p sunlight-compat-linux -p sunlight-virtio > "$out/focused.log" 2>&1
(
    set -e
    cp -p tools/native-c/Cargo.lock "$out/native-c.lock.before"
    trap 'cp -p "$out/native-c.lock.before" tools/native-c/Cargo.lock' EXIT
    cargo generate-lockfile --offline --manifest-path tools/native-c/Cargo.toml > "$out/parity-lock.log" 2>&1
    bash tools/runtime-parity/run.sh "$out/parity" > "$out/parity.log" 2>&1
)
sha256sum "$out/parity/guest/guest.iso" target/btop-10/disk.img
git diff --check
```

| Artifact | SHA-256 |
| --- | --- |
| Pinned btop (checked by image script) | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Probe ELF | `7a325dfdf08e493157bcf98ca2e035a8b71f95b8278960177b9f2a7c8bfd32fb` |
| Kernel ELF | `e80c32fa3f1f7bd20e59fd9d72fc1596fa42bb5f43b353bf08f779394b0d444f` |
| Final read-only and CD-only ISO | `b15e3726cd5535ff61e7c154aac5fedddb6d4dc39aea33efae15ddf432b4ccc9` |
| Disk fixture | `4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f` |
| Payload | `21ee0c5a4f5a4ce80bab81105d1fb73d857e6e7464b915a02890b130013de699` |
| Separate native/musl parity ISO | `6a98b4a0ae50256723d840e916ca0743f1a19d3f5a0c91c91d09f2d5d797c9e8` |

The initial mounted run timed out before the workload: serial lines
4462–4463 show `storagectl mouns` received instead of `storagectl mounts`.
The original serial and QEMU traces remain under `target/btop-16/readonly`.

## Reconciled guest evidence

The ISO runs with 4096 MiB and four vCPUs. The mounted replay used the
read-only fixture backend and verified all 65,536 payload bytes and EOF
twice. Raw serial, QEMU request issue/completion trace, phase byte
offsets, QMP byte counters, attachment state and emulator arguments
are in `target/btop-16/readonly-final/{serial.log,qemu.log,phases.jsonl,block-*.json,attached.json,network-command.json}`.
Selected raw serial events:

```text
[BTOP16] register parent=1 id=2 start=2048 blocks=131072
[BTOP16] totals id=2 completions=0 sectors=0 ns=0
[BTOP16] read parent=1 id=2 lba=2048 start=0 end=0 ok=1 bytes=512
[BTOP16] totals id=2 completions=1 sectors=1 ns=0
[PART] Mbr start=2048 blocks=131072
[BTOP16] read parent=1 id=2 lba=2048 start=0 end=0 ok=1 bytes=512
[BTOP16] totals id=2 completions=2 sectors=2 ns=0
...
[BTOP16] totals id=2 completions=1011 sectors=1011 ns=0
[BTOP08] VFS mount=2 source=2 parent_device=1 start=2048 blocks=131072 path=/mnt
```

| Checkpoint | Absolute completions / sectors | Delta completions / sectors | Delta bytes | Absolute / delta sampled ns |
| --- | ---: | ---: | ---: | ---: |
| Registration | 0 / 0 | 0 / 0 | 0 | 0 / 0 |
| Mount and before idle | 1011 / 1011 | 1011 / 1011 | 517632 | 0 / 0 |
| After idle | 1011 / 1011 | 0 / 0 | 0 | 0 / 0 |
| After first read | 1158 / 1158 | 147 / 147 | 75264 | 0 / 0 |
| After repeat read | 1304 / 1304 | 146 / 146 | 74752 | 0 / 0 |

The verifier reconstructs count, successful sectors and interval sums from
the request event stream, comparing each prefix to the independently
queried registry snapshot. It matches successful QEMU issue/completion
pairs by request token, requires each child command to span one sector
and matches the exact read LBA multisets and QMP byte totals. The first
two reads of LBA 2048 are boot FAT probes, before the retained adapter.
The QEMU prefix ends after the LBA 0 and 1 table discovery reads and
accounts for another 814 sectors, including firmware reads of 64 sectors
each at 2048 and 4096. Those 128 sectors lie in the future child's
extent and are deliberately excluded. The IDE CD reads are a separate
transport. The full trace has no fixture write events and QMP reports
zero fixture bytes written.

Focused host suites passed: 70 block, 29 Helios, 10 FAT, 113 filesystem
and 31 VirtIO tests. Host coverage includes synthetic overlapping
intervals, concurrent update/snapshot consistency, errors and timeouts
at VirtIO, and sibling/slot isolation at the registry. Guest coverage
is serialized; no error, partial completion, overlap, sibling device
or actual unmount/remount was exercised. The same live mount was opened
and closed by two separate probe invocations. No partial-success API
exists on this path.

The first mounted retry timed out at the first storage command after a
shell key IPC timeout. `readonly-final` completed storage validation,
then timed out waiting for btop to launch. The paced replay changed
only the host key spacing; it repeated both successful workload reads
and the same totals. The pinned btop launched as leader 42, reported
that it could not determine its configuration path, then took a
general-protection fault at RIP `0x60a3ef` (serial line 5554).
Leader 42 was reaped at line 5561 and the TTY reported exit 139 at
line 5565. The Help assertion failed. The config warning and fault
are observations; their cause has not been established. The original
BTop-15 acceptance remains historical evidence on its own ISO.

The paced CD-only run returned ENOENT and exit 3 twice. The verifier
found no VirtIO traffic, child registration, lifetime events or totals.
btop leader 40 launched, worker 41 was reaped (serial line 1712),
and Help opened. The dashboard return assertion timed out. The replay
did not reach q or the separate native probe; no leader reap or normal
exit was observed. The captures and serial remain in
`target/btop-16/none-paced`.

| Gate | Result |
| --- | --- |
| Lifetime read completion count | **PASS** for the serialized successful VirtIO workload, including both early FAT probes and mount reads. Linux field equivalence **BLOCKED**. |
| Lifetime successful read sectors | **PASS** against payload, QEMU command ranges and QMP byte totals. Linux field equivalence **BLOCKED** by retired-byte semantics. |
| Summed read-request time | **PASS** for host arithmetic and matching guest tick samples. **BLOCKED** for resolved boot and sub-tick latency and Linux field equivalence. |
| Idle and isolation | **PASS** for zero idle delta and separation from firmware, discovery and IDE CD; guest sibling isolation **NOT RUN**. |
| Guest overlap, error, partial and remount | **NOT RUN**. Partial success is unsupported on the path; host coverage is listed above. |
| Focused block/Helios regression suites and pinned hash | **PASS**. |
| Mounted btop interactive regression | **FAIL**: exit 139 on paced replay. Dashboard capacity, normal exit 0, worker reap and separate same-TTY `/bin/proc-native` exit 0 were not established. |
| CD-only control | **PASS** for storage absence and probe exits. Interactive regression **FAIL** at Help return; btop exit 0 and same-TTY native exit 0 **NOT RUN**. |
| Native/musl parity | **PASS** on the separate ISO listed above: ELF, file, TLS, join and ABI routing. Same-image parity **NOT RUN**. |
| BTop-12 historical write range | **FAIL**. The successful out-of-range LBA 0 write remains unresolved. |
| BTop-13 guest discovery | **BLOCKED**. QEMU boot isolation did not establish guest binding. |
| Linux io_ticks mapping, stats export, active writes and btop I/O display | **NOT RUN**. |
