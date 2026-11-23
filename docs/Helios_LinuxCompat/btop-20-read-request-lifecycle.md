# BTop-20: Read request lifecycle

Production timing is unchanged. The successful path has a candidate
request boundary, but a published VirtIO request can time out and return
while DMA ownership is unresolved. The current ledger retires on caller
return, and the quarantine path has no later authoritative terminal
accounting event. Charging at return or silently excluding these requests
would not implement lifetime request accounting.

## Pinned source and mapping

Linux v6.12, commit adc218676eef25575469234709c2d87185ca223a, as in
BTop-19. Source is archived in `target/btop-19/ref`, with supplemental
merge and VirtIO sources in `target/btop-20/ref`.

| Phase | Linux v6.12 | Actual Helios VirtIO child |
| --- | --- | --- |
| Before request | In the normal `blk_mq_submit_bio` flow, limits, bio splitting and bio merge checks precede new request creation. Time before request timestamp initialization is not charged; `alloc_time_ns` is separate. | VFS/FAT calls can access metadata and many payload sectors. FAT decomposes work into individual block calls. The partition wrapper checks and translates LBA; VFS/device lock acquisition precedes the candidate interval. Firmware and discovery do not own attributed child requests. |
| Start and attribution | `blk_mq_rq_time_init` stamps `struct request.start_time_ns`, including cached request reuse via `blk_mq_use_cached_rq`. `blk_time_get_ns` takes monotonic time or reuses the task's plug timestamp. `blk_mq_bio_to_request` separately assigns accounting via `blk_account_io_start`, using the bio device as `req->part`. | `VirtioBootDisk` borrows the driver; `KernelBlkDev` holds `VIRTIO_BLK`. Both pass the registered child into `storage::read_virtio`. The candidate starts on the attributed synchronous invocation after exclusive access, before driver admission and descriptor preparation. For a published read there is one logical request, one 512-byte command and one driver `RequestId` owning queue/descriptor generation. An admission refusal is not a published command. |
| Queue and dispatch | Plug/scheduler residence after initialization, dispatch, device work and completion delivery contribute. `blk_mq_start_request` does not reset the timestamp. Requeue of the same request preserves it. `blk-merge.c` transfers the earlier stamp to the surviving request when merging two requests; a merged bio contributes no separately timed request. | No child software request queue, merging or retry/requeue of the read. FAT sector decomposition is upstream of each request. The driver validates, prepares, publishes the avail index, kicks, and polls on the caller stack. The candidate covers preparation and polling; publication to return alone would omit preparation. |
| Completion | `blk_update_request` charges retired bytes in sectors even on error; a partial update can leave the request active. Final `__blk_mq_end_request` samples time and calls `blk_account_io_done`, adding end minus start to the read group and incrementing completions before final cleanup. Batch completion also accounts final durations. | `poll_transfer` and `finish_transfer` check used index, head, status and ownership, retire a validated command, and copy 512 bytes only on success. The candidate end is typed return, including those completion steps and copy. Validated device error is terminal but credits no successful sectors. No partial-success API: used length is diagnostic. Timeout requests cancellation and quarantines; invalid completion can also leave an uncertain outcome. A late used entry does not authorize retirement/reuse by the caller. |
| Snapshot | An active request contributes zero duration until final accounting, even after partial byte retirement. `genhd.c` sums per-CPU nanos first, floors by 1,000,000, then casts to unsigned int for the stats views. Fields and CPUs are not sampled atomically. | `DeviceRegistry::retire_read` publishes a coherent internal update under `DEVICES` for each call that reached the publish observer, including failed returns. Its time starts before avail publication, uses coarse ticks, and ends on return. `ReadBusy` is a separate retained-device busy union. Neither implements Linux `io_ticks`. |

An absent merge, requeue or partial-success path does not require emulation.
The candidate is neither bare device service time nor equivalent to every
Linux queueing scenario. The unresolved retirement on timeout/uncertain
return blocks adopting this boundary for the production lifetime counter.

## Host comparison and clock

`tools/btop-20/lifecycle.py` compares pinned-source event rules and
candidate boundary rules against expected snapshots
(completions, sectors, ns, floor ms). Two sequential 600,000 ns requests
give (1, 1, 600000, 0) with the second active; two overlapping requests
give (0, 0, 0, 0) with both active. Both end at (2, 2, 1200000, 1).
A Linux error can retire 512 bytes even though a validated Helios
device error returns zero bytes; both terminal events can charge time.
Partial Linux retirement publishes sectors before duration. A Helios
timeout cannot claim terminal duration. This is a serial source model,
not live Linux or a model of snapshot races. Host VirtIO tests include
validated failure, no partial prefix, timeout and late completion;
overlap and active timing samples are host-only. No guest error,
overlap or partial completion was exercised.

`interrupts::init` calibrates TSC against PIT before child registration,
and both boot probing and the mount run on the BSP before
`smp::start_aps`. For the payload path, x86 SYSCALL masks IF
(FMASK 0x44700); the synchronous VFS/FAT read, device spin lock and
poll loop do not enable IRQs, block in the scheduler or hand off the
stack within the interval. Hence timer preemption and scheduler
migration cannot move these request endpoints. The device lock itself
only enforces exclusive access. The observer masks IRQs during each stamp,
not across the interval, and adds no pinning. This invariant is specific
to the boot and synchronous syscall paths traced here.

Every stamp has CPU identity, two integer ns samples and calibrated Hz.
The same BTop-19 TSC origin and fixed-point conversion were used; bare
RDTSC is not an architectural serialization barrier. All 1,304 intervals
were positive and same-CPU; range 25,205–1,842,187 ns. The calibrated
source progressed from the first attributable request while the coarse
counter recorded zero elapsed ns. No isolated offset/drift bracket or
actual crossing request was exercised, and the reconstruction never
subtracts endpoints from different CPUs.

## Final diagnostic guest

QEMU TCG, 4 GiB, four vCPUs, CPU max, fixture attached read-only.
Raw serial/events, QEMU trace, QMP attachment and byte totals, phase byte
offsets, exact QEMU argv and interactive captures are under
`target/btop-20/{mounted,cd}`. The first matching events are:

```text
[TIME] PIT calibration mode=2 input_hz=1193182 reload=11932 start_count=11911 end_count=7911 elapsed_counts=4000 tsc_delta=8316494 tsc_hz=2480772735
[BTOP20] registration id=2 stamp=(0, 108452179, 108474915, 2480772735)
[BTOP20] parent=1 id=2 lba=2048 retained=0 published=1 ok=1 first=(0, 110149025, 110149062, 2480772735) last=(0, 110763641, 110763669, 2480772735)
```

| Checkpoint | Absolute completions / sectors | Candidate absolute ns | Delta ns | Floor ms | CPU |
| --- | ---: | ---: | ---: | ---: | --- |
| Registration | 0 / 0 | 0 | 0 | 0 | 0 |
| Mount / before idle | 1011 / 1011 | 90605133 | 90605133 | 90 | 0 |
| After idle | 1011 / 1011 | 90605133 | 0 | 90 | — |
| First payload | 1158 / 1158 | 104023301 | 13418168 | 104 | 1 |
| Repeat payload | 1304 / 1304 | 116269307 | 12246006 | 116 | 1 |

Two requests use the boot adapter and 1,302 the retained adapter,
including most mount traffic. Both probes verified 65,536 bytes and EOF;
each read 128 payload sectors plus 19 and 18 non-payload sectors,
respectively. The BTop-16 verifier checks every coarse ledger snapshot,
QEMU request/completion LBA multisets and QMP byte totals. It excludes
814 pre-registration sectors (15 touching LBA 0, 128 inside the
future child extent); IDE CD is a separate transport. Fixture write
bytes were zero. This reproduces BTop-19's 1,304 / 1,304 under equivalent
conditions. QEMU corroborates range and bytes; timing comes from guest
endpoints. Guest sibling isolation was not exercised.

## Reproduction

Base HEAD ca178af0ce904e6f4031ccb3499646dfe5a11ca9 plus the incoming dirty
tree. New paths: this report and `tools/btop-20/{observe.patch,lifecycle.py}`.
The observer was removed after building; both affected kernel files
matched their incoming bytes. Boot mount policy and fail-closed mountinfo
were untouched. No commit or push.

```sh
out=target/btop-20
mkdir -p "$out/ref"
cp target/btop-10/payload-lbas.txt "$out/"
ref=https://raw.githubusercontent.com/torvalds/linux/adc218676eef25575469234709c2d87185ca223a
curl -fsSL "$ref/block/blk-merge.c" -o "$out/ref/blk-merge.c"
curl -fsSL "$ref/drivers/block/virtio_blk.c" -o "$out/ref/virtio_blk.c"
sed 's@target/btop-14/final@target/btop-20/final@g' tools/btop-14/image.sh > "$out/image.sh"
(
    set -e
    git apply tools/btop-20/observe.patch
    trap 'git apply -R tools/btop-20/observe.patch' EXIT
    SUNLIGHT_BTOP16_TRACE=1 bash "$out/image.sh"
) > "$out/image.log" 2>&1
iso=$out/final/guest-ramfs-trace-config.iso
sed 's@target/btop-18/replay.generated.py@target/btop-20/replay.generated.py@g' target/btop-18/replay.py > "$out/replay.py"
python3 "$out/replay.py" "$iso" "$out/mounted" --disk target/btop-10/disk.img
python3 "$out/replay.py" "$iso" "$out/cd"
python3 tools/btop-20/lifecycle.py
for mode in mounted cd; do
    python3 tools/btop-20/lifecycle.py "$out/$mode"
    python3 tools/btop-16/verify.py "$out/$mode"
done
cargo test --target x86_64-unknown-linux-gnu -p sunlight-block -p sunlight-virtio -p sunlight-fat -p sunlight-fs -p sunlight-compat-linux
sha256sum -c "$out/hashes.sha256"
git diff --check
```

| Artifact | SHA-256 |
| --- | --- |
| Final diagnostic ISO | e94c8df4f240db0a0af5ee75a40962a6a0722c3bc44b295943c10a203b78b4b0 |
| Diagnostic ELF | d6e90cfae72b5781225a099c2773ef69fdf407d68028f174d5255b54154a0eba |
| Storage probe ELF | 7a325dfdf08e493157bcf98ca2e035a8b71f95b8278960177b9f2a7c8bfd32fb |
| btop (complete binary verified inside diagnostic ELF) | bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 |
| Read-only fixture | 4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f |
| READ10.BIN | 21ee0c5a4f5a4ce80bab81105d1fb73d857e6e7464b915a02890b130013de699 |
| Historical parity ISO, `target/btop-16/parity/guest/guest.iso` | 6a98b4a0ae50256723d840e916ca0743f1a19d3f5a0c91c91d09f2d5d797c9e8 |

| Gate | Result |
| --- | --- |
| Lifecycle equivalence | Bounded successful mapping and source model **PASS**; lifetime equivalence **BLOCKED** by missing authoritative terminal retirement on timeout/uncertain return. |
| Early timestamp availability | **PASS** from the first attributed request on the diagnostic guest. |
| Cross-vCPU coverage | **NOT RUN** for actual crossing requests and isolated offset/drift; same-vCPU invariant established for exercised paths. |
| Read-request time | Candidate guest reconstruction and host accumulation/rounding **PASS**; production timing **BLOCKED**, unchanged. Guest active snapshot, overlap, error and partial completion **NOT RUN**. |
| Read-only counts, idle and isolation | **PASS** for mounted phase totals, payload, and disk-versus-CD/discovery isolation; guest sibling isolation **NOT RUN**. |
| Focused host regressions | **PASS**, 253 tests. |
| Mounted interaction | **PASS**: Help open/return, q, worker 44 and leader 42 reaped (serial lines 8198 and 8210), btop exit 0 (8214), separate same-TTY proc-native exit 0 (8239). |
| CD-only storage and interaction | **PASS**: both probes returned ENOENT and exit 3; no attributed requests or VirtIO fixture traffic. Help open/return, q, worker 41 and leader 40 reaped (2742 and 2754), btop exit 0 (2758), separate same-TTY proc-native exit 0 (2786). |
| Same-image native/musl parity | **NOT RUN**. Historical parity passed on the separate ISO above. |
| BTop-12 / BTop-13 | Historical **FAIL** / discovery **BLOCKED**, respectively. |
| Linux io_ticks, full stats ABI, active writes and btop I/O display | **NOT RUN**. |
