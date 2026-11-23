# BTop-19: Block timing resolution and Linux accounting

The existing PIT-calibrated TSC resolves early, same-CPU VirtIO request intervals
on the read-only fixture. A faster clock cannot turn those intervals into Linux
request lifetime or its `io_ticks` counter. This milestone adds observations and
a reference model only. Stats export, writes, hotplug, boot mount policy,
fail-closed mountinfo, and btop I/O display are out of scope.

BTop-12 remains historical **FAIL**; BTop-13 remains guest-discovery **BLOCKED**.
BTop-18's exact-ISO replays passed. The historical BTop-16 mounted fault and
CD-only Help timeout were **NOT REPRODUCED**; neither is established as fixed.
The runs here use a separate diagnostic image.

## Pinned contracts

Linux **v6.12**, commit `adc218676eef25575469234709c2d87185ca223a`
(peeled tag; annotated tag object `06090c9b622a7e1f797e775db4c035e0d779b76e`).
Relevant files are `Documentation/admin-guide/iostats.rst`,
`block/blk-mq.c` (request initialization, start, byte retirement, and end),
`block/blk.h` (`blk_time_get_ns`), `block/blk-core.c` (`update_io_ticks`),
`block/genhd.c` (`part_stat_read_all`, `part_stat_show`, `diskstats_show`),
`include/linux/part_stat.h`, and `kernel/time/time.c` (`jiffies_to_msecs`).
The table describes the request-based blk-mq path and the exported snapshot,
using the implementation where the prose documentation is less specific.

| Field | Linux v6.12 measurement and snapshot | Helios gap |
| --- | --- | --- |
| 4: read-request time | Starts at request time initialization, before hardware dispatch. Uses monotonic `ktime_get_ns()` via `blk_time_get_ns`, which may reuse a task plug timestamp. At accounted final completion `blk_account_io_done` adds end minus start to the read-group nanoseconds. In-flight requests contribute no duration until accounted completion. Sum per-CPU nanoseconds first, divide by 1,000,000 (floor), then emit unsigned int ms. | **Resolution and lifecycle/accounting**: the public tick clock hides sub-tick and early durations; the VirtIO publication-to-return boundary omits pre-dispatch time, and timeout return is not proven Linux completion. |
| 10: `io_ticks` | Jiffies, all accounted operations. When `time_after(now, bd_stamp)` and the compare-exchange wins, move the stamp to now. Add the entire jiffy delta on an end event, or on a start only if already in flight before increment. Re-evaluate the whole device independently when updating a partition. Both stats display paths check in-flight and, if active, invoke the end-style update before the per-CPU sum; idle samples do not advance the stamp. Convert the summed jiffies to unsigned int ms (exact ×10 at HZ=100; other configurations follow `jiffies_to_msecs`, including round-up for nonintegral ratios). | **Lifecycle and rules**: elapsed busy union, even measured precisely, is different. Needs the stamp rule, active snapshot side effect, and matching operation and device scope. The 10 ms quantum is sufficient for a HZ=100 jiffy model, assuming a running clock and matching events; fine time alone cannot qualify this field. |
| 11: weighted time | The documentation describes time weighted by concurrency. The pinned exporter actually sums completed read, write, discard and flush durations and floors the total to unsigned int ms. Overlap is counted per request and an unfinished request does not add its partial age at sample time. | **Resolution and lifecycle/accounting**: neither elapsed union nor a live integral that immediately includes unfinished work matches this exporter. |

Snapshots are not atomic across fields or CPUs: in-flight is sampled separately
and the stats are summed across possible CPUs without a global snapshot lock.
The reference model below uses serial event order and does not model races,
plug caching, wraparound, parent propagation or failures. Linux's exported
milliseconds do not promise any particular physical clock precision. Using a
10 ms clock for field 4 can still materially distort totals relative to this
reference: fractional milliseconds from many requests can accumulate into
whole exported milliseconds. For field 10, normal jiffy quantization is part
of the reference, but pre-tick boot stalls and missing events remain distinct
issues.

Pinned btop **1.4.5**, source revision
`a1099d075fa914452366b28c67ff171e21a1f02e`,
`src/linux/btop_collect.cpp:2115` and `:2277`: resolves the mounted device
via major/minor, reads fields 3 and 7 and scales sector differences by 512,
then skips field 8 and in-flight field 9 to read field 10. Its activity is the
field 10 delta divided by elapsed uptime seconds and by 10, rounded and
clamped to 0–100 (zero for initial sample or unchanged uptime). It consumes
neither field 4 nor field 11 in this path. A parser accepting a row is not
proof of counter equivalence.

## Deterministic host model

`tools/btop-19/timing.py` compares the serial reference, HZ=100, against
expected samples. Starts align allocation and accounting start in this model;
the guest does not prove that alignment. Each sample below is
`(read_ms, io_ticks_ms, in_flight, exact_busy_union_us)`.

| Sequence, time in µs | Samples |
| --- | --- |
| Early: start 100, finish 1600, sample 1800 | (1, 0, 0, 1500); a frozen boot tick clock would give zero read time |
| Sub-ms: 100–700 and 800–1400, sample 1500 | (1, 0, 0, 1200); flooring each completion would give zero |
| Tick edge: 9999–10001, sample 10002 | (0, 10, 0, 2) |
| Long: 19000–51000, sample 60000 | (32, 40, 0, 32000); completion adds four jiffies |
| Overlap: 11000–42000 and 23000–48000, sample at 35000 and 59000 | (0, 20, 2, 24000), then (56, 30, 0, 37000); read-only weighted output is 0 then 56 ms, while the live weighted integral is already 36 ms at the active sample |

## Existing clock and guest evidence

- `kernel/src/timekeeping.rs`: one shared BSP-maintained 100 Hz counter;
  nanoseconds are ticks × 10,000,000. Reads are atomic and usable across CPUs,
  but boot requests are unresolvable before the BSP begins advancing it.
  RTC is wall time, with steps; PIT channel 0 is a wrapping hardware
  calibration reference and LAPIC timers are per-CPU periodic timers. Neither
  offers a separate published fine monotonic clock. No HPET clock driver was
  found under `kernel/src`.
- `kernel/src/arch/x86_64/interrupts.rs`: `interrupts::init` calibrates TSC
  against PIT before VFS initialization and AP bring-up. `now_ns` reads a
  common BSP origin and fixed point multiplier, computes
  `((TSC.saturating_sub(origin) * floor((10^9 << 32) / hz)) >> 32)`,
  and falls back to ticks if uncalibrated. This read has no lock, allocation,
  or sleep. Bare RDTSC supplies no instruction serialization guarantee, and
  the shared origin and scale do not establish offset, drift, or frequency
  stability across CPUs. The opt-in `measurement_clock.rs` source uses the
  same PIT calibration with stability checks, per-CPU origins and an explicit
  same-CPU contract; it is a fixture clock, not a cross-CPU alternative.
- `kernel/src/storage.rs` and `sunlight-virtio/src/blk.rs`: the observed
  retained device path holds the device mutex, publishes one request, polls
  for the used entry synchronously and returns on the caller's stack. Boot
  executes on BSP before APs start; the payload read path uses the synchronous
  kernel VFS with IF masked at syscall entry and no blocking handoff inside
  the driver loop. Start and completion on different CPUs are not a supported
  path here. Different requests do run on different CPUs; coverage across
  CPUs is not a cross-CPU offset/drift test.

The diagnostic patch takes two consecutive `now_ns` readings at each endpoint,
with CPU identity and calibrated Hz, while locally masking IRQs during capture.
Begin is in the observer before avail-index publication, end is after driver
return. It logs after the pair, without feeding the stamps into any metric.
These timings include software at the observed boundary; serial logging
between completed requests changes pacing. It is not a measurement of
bare device service time or of arbitrary sub-nanosecond resolution.

Raw excerpt from `target/btop-19/mounted/serial.log` (stamp tuple:
CPU, first ns, second ns, calibrated Hz):

```text
[TIME] PIT calibration mode=2 input_hz=1193182 reload=11932 start_count=11906 end_count=7900 elapsed_counts=4006 tsc_delta=8270634 tsc_hz=2463397807
[BTOP19] registration id=2 stamp=(0, 140504935, 140560317, 2463397807)
[BTOP19] id=2 lba=2048 first=(0, 143852560, 143852706, 2463397807) last=(0, 144206557, 144206600, 2463397807)
[BTOP16] read parent=1 id=2 lba=2048 start=0 end=0 ok=1 bytes=512
[BTOP19] mount path=/mnt stamp=(0, 1321853004, 1321853047, 2463397807)
```

The guest calibration implies about 0.406 ns per TSC count and integer-ns
converted timestamps. The smallest observed nonzero consecutive sample
step was **25 ns** (maximum 37,401 ns): empirical granularity including
sampling overhead, not a hardware precision or accuracy bound. All 1,304
attributed intervals are positive and shorter than 10 ms. Reconstruction
subtracts endpoints within each CPU and sums durations, never subtracting
stamps from different CPUs. The coarse lifetime duration was zero throughout.
No guest overlap, active stats sample or long single request was exercised.

| Window | Endpoint CPU | Requests | Sum of endpoint deltas (ns) | Min–max (ns) |
| --- | --- | ---: | ---: | ---: |
| Registration through mount | 0 | 1011 | 90,964,317 | 30,702–353,997 |
| Mounted idle | — | 0 | 0 | — |
| First payload probe | 2 | 147 | 16,029,393 | 36,194–2,324,445 |
| Second payload probe | 1 | 146 | 11,647,611 | 22,717–352,067 |

Both probes verified 65,536 bytes and EOF. Each includes 128 payload-sector
reads; the remaining 19 and 18 requests are outside the payload LBA set.
BTop-16's absolute **1,304 completions / 1,304 successful sectors** reproduced
from registration: 1,011 at mount and after idle, 1,158 after the first probe.
The existing verifier matched request/completion pairs and phase LBA multisets
in the QEMU trace and checked QMP bytes, separately excluding 814 pre-registration
sectors (15 covering LBA 0; 128 within the later child extent). No fixture
writes were recorded and QMP reports zero write bytes. CD-only had no fixture
events. QEMU traces establish ranges and attribution, not guest time. Success
sectors still do not establish Linux retired-byte semantics.

## Reproduction and results

Base HEAD `ca178af0ce904e6f4031ccb3499646dfe5a11ca9` plus the pre-existing
dirty tree. New paths: this report, `tools/btop-19/timing.py` and
`tools/btop-19/observe.patch`. The last temporarily instruments
`kernel/src/{main,storage}.rs`; it was removed after the build. The inherited
image scripts also temporarily apply their existing packaging and traces.
No production accounting change was justified.

```sh
out=target/btop-19
mkdir -p "$out"/{final,ref}
git ls-remote https://github.com/torvalds/linux.git refs/tags/v6.12 'refs/tags/v6.12^{}'
for file in block/{blk-mq.c,blk.h,blk-core.c,genhd.c} include/linux/part_stat.h \
    kernel/time/time.c Documentation/admin-guide/iostats.rst; do
    curl -fsSL "https://raw.githubusercontent.com/torvalds/linux/adc218676eef25575469234709c2d87185ca223a/$file" \
      -o "$out/ref/${file##*/}"
done
(
    set -e
    git apply tools/btop-19/observe.patch
    trap 'git apply -R tools/btop-19/observe.patch' EXIT
    sed 's@target/btop-14/final@target/btop-19/final@g' tools/btop-14/image.sh > "$out/image.sh"
    SUNLIGHT_BTOP16_TRACE=1 bash "$out/image.sh"
) > "$out/image.log" 2>&1
iso=$out/final/guest-ramfs-trace-config.iso
cp target/btop-10/payload-lbas.txt "$out/"
python3 target/btop-18/replay.py "$iso" "$out/mounted" --disk target/btop-10/disk.img
python3 target/btop-18/replay.py "$iso" "$out/cd"
python3 tools/btop-19/timing.py "$out/mounted"
python3 tools/btop-16/verify.py "$out/mounted"
python3 tools/btop-16/verify.py "$out/cd"
cargo test --target x86_64-unknown-linux-gnu -p sunlight-block -p sunlight-virtio \
    -p sunlight-fat -p sunlight-fs -p sunlight-compat-linux
sha256sum "$iso" "$out"/final/{probe.elf,kernel-ramfs-trace-config.elf} \
    target/btop-01/btop target/btop-10/{disk.img,READ10.BIN} \
    target/btop-16/parity/guest/guest.iso tools/btop-19/observe.patch target/btop-18/replay.py
git diff --check
```

QEMU TCG, 4 GiB, four vCPUs, `-cpu max`; exact argv in
`{mounted,cd}/network-command.json` under the evidence directory
`target/btop-19`. The mounted fixture's `ro: true` is in
`mounted/attached.json`. Both runners and the focused command returned
0. Serial, QEMU traces, QMP phase and input logs, and Help/dashboard captures
remain there alongside the timing reconstruction and verifier receipts.

| Artifact | SHA-256 |
| --- | --- |
| Pinned btop (complete bytes also verified in the packaged kernel) | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Diagnostic ISO, mounted and CD-only | `a3670881273b02a8d3cca0b1163dfebe052865168c01520c9271c8a2587c9ca4` |
| Diagnostic ELF | `ce6d358be75c5cc85bfa59a641f5c6749d54d190c4dde7a6b2e576e0ba4479c6` |
| Storage probe ELF | `7a325dfdf08e493157bcf98ca2e035a8b71f95b8278960177b9f2a7c8bfd32fb` |
| Read-only disk | `4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f` |
| READ10.BIN | `21ee0c5a4f5a4ce80bab81105d1fb73d857e6e7464b915a02890b130013de699` |
| Observer patch | `62b44d716aaab53980310a69f60e8b70b111b14e626387cb75774488c9db8a12` |
| BTop-18 replay wrapper | `bd059602f65267d46eb9e458964e3f9c63026f1abd0665737df2945ecbf84456` |
| Separate historical native/musl parity image, `target/btop-16/parity/guest/guest.iso` | `6a98b4a0ae50256723d840e916ca0743f1a19d3f5a0c91c91d09f2d5d797c9e8` |

| Gate | Result |
| --- | --- |
| Early clock availability | **PASS** on this diagnostic guest: calibration and progressing same-CPU stamps before the first attributable request; through mount and both payload probes. |
| Cross-vCPU comparability | **NOT RUN**; no start/completion pair crossed CPUs and no ordered inter-CPU bracket was sampled. General cross-CPU fine-clock adoption remains **BLOCKED**; would require a shared clock contract or measured offset/drift bounds with ordered capture. |
| Read-request time | Same-CPU observed interval reconstruction **PASS**. Linux field 4 mapping **BLOCKED** pending equivalent request lifetime boundaries and retirement semantics. |
| Linux `io_ticks` mapping | Serial reference model **PASS**. Direct busy-union substitution **FAIL** by deterministic counterexample. Guest field mapping **BLOCKED** pending equivalent events and snapshot rules. |
| Weighted time | Reference semantics established; guest mapping **NOT RUN**. |
| Read-only lifetime regression | **PASS**, 1,304 completions and 1,304 sectors; fixture absence in CD-only **PASS**. |
| Focused host regressions | **PASS**, 253 tests across the five selected crates. |
| Mounted interactive baseline | **PASS**: `help.png`, `dashboard-return-0.png`; worker 44 and leader 42 reaped at serial lines 8357 and 8369, btop exit 0 at 8373, separate same-TTY `/bin/proc-native` exit 0 at 8399. |
| CD-only interactive baseline | **PASS**: same capture names; worker 41 and leader 40 reaped at 2725 and 2737, btop exit 0 at 2741, separate same-TTY `/bin/proc-native` exit 0 at 2766. |
| Native/musl parity | Historical **PASS** on the separate image above; diagnostic same-image parity **NOT RUN**. |
