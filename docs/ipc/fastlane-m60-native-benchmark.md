# FastLane M6.0 native benchmark

Status: **PARTIAL — supported FastLane measurements qualified; three-way mission incomplete.** Experimental measurement only. Production FastLane DISABLED.
The requested three-way baseline is not established: native register IPC carries
32 inline bytes, below all requested sizes. Only the two existing FastLane
backends are eligible for the corrected pilot. A separately validated bulk IPC
contract would be needed to include conventional IPC without fragmentation or
changing the compared operation.
Historical M1/M4 incident attribution and original SHM ownership remain UNRESOLVED.

Contract frozen before the pilot: 16 warm-up messages followed by 128 measured
messages per configuration, one producer and one consumer restricted to guest CPU
0, maximum one outstanding request, persistent channel per window, no retries.
Any unexpected Full, Empty, IPC timeout, payload mismatch or acknowledgement
mismatch rejects the run. Each byte is `(sequence * 17 + offset * 3) % 251`.
The consumer verifies the entire private payload and acknowledges with a digest
and exact next sequence. Producer verifies reply label, digest and count.

Conventional IPC uses endpoint_create/bind, ipc_call_timeout, ipc_recv_timeout,
and ipc_reply_result: **4 register u64 words (32 bytes)**. The public message
struct has eight storage slots, but raw_syscall_ipc and kernel IPC_REG_WORDS
transmit only four. Passing SHM capabilities is a distinct bulk protocol; this
fixture does not substitute it or fragment a logical payload. M5.1 uses authenticated
channel operations, restricted mappings, transfer_word and the unchanged hostile
ring Producer/Consumer including reserve/acquire/copied/ack admission controls.
M5.2 uses the same bounded private-copy implementation through the existing raw
Native Memory accessor, restricted views and authenticated Doorbell. No new
unsafe block, zero-copy read lease, kernel opcode or production behavior.

| Payload | Conventional | Mediated FastLane | Shared ring FastLane |
|---|---|---|---|
| 64 B | UNSUPPORTED inline | supported | supported |
| 256 B | UNSUPPORTED inline | supported | supported |
| 1024 B | UNSUPPORTED | UNSUPPORTED | UNSUPPORTED |
| 4096 B | UNSUPPORTED | UNSUPPORTED | UNSUPPORTED |

No fragmentation or alternative protocol is substituted. The generic host ring's
4096-byte maximum does not describe the validated native hostile ring, whose
payload limit is 256 bytes.

For the two eligible FastLane backends, the common operation is a **verified request plus acknowledgement**. Aggregate
rate counts 128 acknowledged payloads over an independently qualified producer
window. It includes payload generation, IPC control and integrity work and is
not a claim about unacknowledged streaming or maximum ring throughput.
Round-trip latency covers payload serialization/publication, consumer dispatch,
private verification/release and the checked IPC acknowledgement. For FastLane,
the ordinary IPC request carries only the sequence; payload travels through the
selected ring, with one authenticated Doorbell notification and observation per
message. Conventional inline IPC has no eligible requested payload size. Local FastLane
publication-plus-notification and consumer copy/verification/release intervals
are separately labeled; they are not equivalent to a conventional send syscall.

Setup covers consumer spawn/handshake and data channel authorization, attachment,
mapping and backend construction. Teardown covers reader retirement, revocation,
consumer exit/reap, zero scoped inventory and census. The existing actor accounting selector requires a native-ring role. It is used
on the data channel for the shared-ring path only. Mediated per-actor CPU time,
syscalls and dispatch counts are NOT_QUALIFIED / UNAVAILABLE under that selector.
Whole-system diagnostic counters remain separate concurrent observations. No
wall-derived CPU utilization is computed. Shared-ring CPU accounting windows
include their control calls and end reporting overhead and reconcile
independently of payload windows. An attempted auxiliary diagnostic channel was
refused by the existing one-live-channel-per-producer authority rule; that failed
pilot is preserved. No authority or accounting rule was broadened.

Raw stamps retain CPU, epoch, frequency, uncertainty, flags and stability spread.
Only positive, stable-epoch, CPU-0 pairs accepted independently by the parser
produce durations. Missing, nonresident, backward, stale or calibration-changing
pairs remain visible as rejected observations. No cross-CPU subtraction.
Every sample is retained; p50/p95/p99 use nearest-rank order statistics with at
least 100 accepted observations. No tail population is sampled away. At this
small population p99 is descriptive, not a precise rare-tail estimate.
Scale uncertainty is ceil(duration * ppm / 1e6) + 1 ns and does not bound host
scheduler pauses, read/syscall jitter, PIT absolute accuracy or execution noise.

Pilot: one 2-vCPU guest, 300-second host wall budget. Proceed only after exact
integrity, counts, receipt/parser, qualified windows, available CPU reconciliation and
retirement PASS. Baseline: three new guests each at 2, 4, 12 vCPUs, the same image,
300 seconds per guest, no concurrent QEMU guests. Preserve every failed,
interrupted or rejected attempt. Host monotonic time controls deadlines only.

Observer effects: two endpoint stamps per local sample; extra producer stamp
for FastLane publication timing; actor-private fixed arrays; serial reporting
outside payload windows. M5.1's unchanged kernel emits admission/publication
trace records inside its path, which is a material asymmetry. Actor-private CPU counters run only for the shared-ring backend; this adds
an observer asymmetry. Each configuration makes 646 clock-stamp syscalls
(640 paired stage/round-trip reads plus six setup/window/teardown endpoints).
Instrumentation time overhead is NOT_ISOLATED: no matched timing control or
overhead subtraction is claimed. Native and conventional copies and
Doorbell notification semantics differ as described above. QEMU TCG host
scheduling, concurrent guest services, cache state and fixed workload order can
affect results. No universal superiority or bare-metal projection is justified.
Memory safety and stable private snapshots PROVIDED. Coherent publication and
application authenticity NOT PROVIDED for hostile shared payloads. Capability
and process authentication do not turn mutable shared bytes into authenticated
application content. Heap peaks, blocked-time accuracy, user/kernel separation,
CPU utilization and cross-CPU latency remain NOT_QUALIFIED.

Results and retained attempts follow below.

The initial contract incorrectly inferred a 64-byte native inline limit from
the message storage size. The first real guest rejected its truncated setup
command before payload measurement. Both that failed receipt and the initial
contract are preserved under `.fastlane-evidence/m60/`; the corrected contract
packs its setup fields into four words and includes no conventional result.
The sandbox socket failure before that guest is also retained separately.

Executable identity before M6 edits is
`5b20cba2627952f162263b72be359a8c314ebf963874b0095793d10a36991f74`
(1,386 canonical files), HEAD `e5ed4849e57cb729252ba7ad0c25efcfe7fe5ae3`.
`initial-source.json` and `initial.patch` record the inherited dirty tree.
The measured image source identity is
`5c09e705fbc6c3f1d4aef27efea19bedfa5abb6d3f2622250a540e624bc78cd7`.
Canonical identity is SHA-256 of the sorted `sources()` map encoded with JSON
`separators=(',', ':')`. Each build/run retains its complete map, source snapshot,
dirty patch, embedded service hashes, ELF/ISO hashes and artifact index. Kernel,
IPC, hostile ring and Doorbell executable inputs match the accepted R2 source
byte for byte. M6 changes are in the opt-in fixture, its feature and host tools.

Reused accepted safety manifests are exactly:

- `.fastlane-evidence/m54r2/matrix-20261103T183322.818190Z/matrix.json`.
- `.fastlane-evidence/m54r2/safety-20261103T192044.990473Z/orchestration.json`.
- `.fastlane-evidence/m54r2/final-readiness.json` (COMPLETE, zero unmet criteria).

Phase A independently replayed the retained readiness predicates, checked 80
referenced serial hashes across 62 manifests, four expected native leak FAILs,
five sets of 18 adversarial controls, measurement qualification, exact lifecycle
and remote ACK witnesses, and debug/release production exclusion. No completed
security suite was rerun. `safety-audit-*.json` records the initial exact-source
acceptance; `verify_reused_safety.py` rechecks those immutable receipts against
the captured initial source map. This does not certify the M6 fixture by itself:
its native receipts, focused host checks and parser controls are separate.
Historical M1/M4 failures and the original untraced SHM frame ownership remain
UNRESOLVED; current passing evidence supplies no retrospective causal attribution.

Both FastLane producers initialize a private 256-byte staging array and clear
all 32 words of the reused shared payload slot, even for a 64-byte payload.
Consumers copy `ceil(payload_bytes / 8)` words into initialized private storage
before full-byte comparison. M5.1 performs each shared access via the authenticated
word syscall plus private admission controls. M5.2 performs the same bounded
accesses through restricted mappings. Reported payload MiB/s counts logical
payload bytes, excluding slot padding, descriptors, control requests and replies.
Doorbells are authenticated hints; the sequence, private copy and acknowledgement
establish completion. No zero-copy advantage is claimed.

The host is an Intel Core i7-1260P with 16 online logical CPUs; full topology is
in `host-lscpu.json`. QEMU is 11.1.2, `tcg,thread=multi`, `-cpu max`, `-smp N`,
2 GiB RAM, display/network disabled and virtio RNG enabled. Host threads are
unpinned and host scheduling is uncontrolled. Guest online CPU IDs and both
actors' observed sample endpoints are checked; hardware socket/SMT hierarchy is
not separately measured. Affinity mask 1 restricts each measured actor to CPU 0.
A pin request can return before residency changes, so the parser still checks
actual stamp CPU identity instead of assuming pinning succeeded. Guest timer
calibration uses six PIT reference windows, discards the first, and derives its
scale from the median of five with observed spread plus boot bounds. Raw
calibration windows and final uncertainty remain attached to each run.

To reproduce the supported experiment (no privilege inside guest fixture):

```sh
python3 -m unittest discover -s tools -p test_fastlane_m60.py
cargo clean -p sunlight-kernel --target-dir target/fastlane-f11b0e3-build
python3 tools/fastlane_m60_validate.py --section build --measurement \
  --evidence-root .fastlane-evidence/m60/rebuild
python3 tools/fastlane_m60_validate.py --section native --measurement \
  --cpus 2 --timeout 300 --image-manifest <build>/validation-summary.json \
  --evidence-root .fastlane-evidence/m60/new-pilot
# Inspect a complete PASS before continuing. Reuse that same image for all guests.
python3 tools/fastlane_m60_validate.py --section native --measurement \
  --cpus 2 4 12 --timeout 300 --image-manifest <build>/validation-summary.json \
  --evidence-root .fastlane-evidence/m60/new-repetition-1
# Repeat with distinct evidence roots for repetitions 2 and 3.
python3 .fastlane-evidence/m60/verify_reused_safety.py
python3 .fastlane-evidence/m60/receipt_controls.py
python3 .fastlane-evidence/m60/summarize.py
```

The driver saves a native receipt after each guest and preserves failed and
interrupted attempts. A NOT_QUALIFIED metric stops that invocation; already
accepted guests stay accepted in their original row, while the invocation's
failure verdict is preserved. An unattempted topology is not a failed guest.
No QEMU run is repeated solely to improve its performance score.

The supported baseline has **nine fully accepted guests**, three at each CPU
count, and 36 accepted configuration windows. Each window has 128 measured
messages and 16 warm-up messages: 4,608 measured payloads, 5,184 including warm-up,
with exact private-byte verification and matching acknowledgements. The producer
and consumer are separate native VMs on CPU 0. No payload-window migration was
observed. All published timing pairs have independent parser acceptance and full
clock metadata. This is a single-outstanding acknowledged exchange baseline;
independent maximum streaming transfer throughput remains NOT_QUALIFIED.

The table reports medians over three accepted independent guests. Brackets show
the full rate range, and RTT columns are medians of per-run nearest-rank
percentiles, not pooled distributions. The machine summary also includes every
individual value and median absolute deviation, local publication/notification
and consumer copy/verification/release percentiles, CPU observations and raw
measurement references. Clock ppm is a frequency-scale bound, not a bound on
scheduling noise. Rejected counts in this table refer to fully accepted guests;
the additional rejected run is disclosed immediately below. Setup and teardown
columns are elapsed latency, with CPU setup/teardown cost NOT_QUALIFIED.

| CPUs | Path | B | Reps | Ack msg/s median [min–max] | Payload MiB/s | Window ms | RTT p50/p95/p99 µs | Setup / teardown ms | Max ppm | Rejected |
|---|---|---|---|---|---|---|---|---|---|---|
| 2 | mediated | 64 | 3 | 1013.9 [862.5–1038.4] | 0.062 | 126.25 | 899.9/1402.7/1770.4 | 18.62 / 10.97 | 738 | 0 |
| 2 | shared ring | 64 | 3 | 7713.0 [7279.3–7770.4] | 0.471 | 16.60 | 115.3/132.8/326.2 | 9.65 / 9.75 | 738 | 0 |
| 2 | mediated | 256 | 3 | 794.6 [774.5–920.8] | 0.194 | 161.09 | 1057.7/1583.5/4762.7 | 11.12 / 10.47 | 738 | 0 |
| 2 | shared ring | 256 | 3 | 7237.1 [6388.3–7982.6] | 1.767 | 17.69 | 132.9/155.3/331.2 | 11.63 / 9.58 | 738 | 0 |
| 4 | mediated | 64 | 3 | 916.2 [896.7–917.7] | 0.056 | 139.70 | 979.2/1798.3/2621.9 | 18.50 / 12.67 | 859 | 0 |
| 4 | shared ring | 64 | 3 | 7737.2 [7470.0–7824.3] | 0.472 | 16.54 | 108.9/176.4/351.0 | 11.50 / 10.35 | 859 | 0 |
| 4 | mediated | 256 | 3 | 856.9 [822.1–866.5] | 0.209 | 149.37 | 1098.9/1741.0/1955.4 | 10.97 / 10.52 | 859 | 0 |
| 4 | shared ring | 256 | 3 | 6351.1 [6039.0–8082.9] | 1.551 | 20.15 | 132.0/266.1/395.9 | 11.23 / 10.39 | 859 | 0 |
| 12 | mediated | 64 | 3 | 815.2 [781.5–831.8] | 0.050 | 157.03 | 1125.0/1850.7/2168.9 | 21.25 / 18.80 | 708 | 0 |
| 12 | shared ring | 64 | 3 | 6495.5 [5815.5–7580.1] | 0.396 | 19.71 | 131.1/167.1/520.0 | 17.37 / 12.94 | 708 | 0 |
| 12 | mediated | 256 | 3 | 672.8 [654.3–710.7] | 0.164 | 190.25 | 1328.6/2411.8/3222.2 | 13.97 / 12.16 | 708 | 0 |
| 12 | shared ring | 256 | 3 | 6711.1 [6044.3–6926.6] | 1.638 | 19.07 | 128.6/157.4/484.3 | 15.25 / 12.74 | 708 | 0 |

Ten baseline guests were attempted: nine accepted and one NOT_QUALIFIED. In
repetition 2 at 4 CPUs, the first setup pair began on CPU 1 and ended on CPU 0.
No subtraction or setup duration is published for that pair. This is the only
rejected timing observation across completed baseline guests; all integrity and
retirement checks passed. The invocation stopped before attempting its 12-CPU
guest. One bounded replacement batch supplied the missing accepted 4-CPU and
12-CPU repetitions. The original NOT_QUALIFIED row and invocation FAIL remain
unchanged. Its otherwise qualified window rates are retained, including slower
values: mediated 64 B 891.7, shared 64 B 7,783.3, mediated 256 B 842.7, shared
256 B 6,626.5 acknowledged messages/s. They are excluded from the three-complete-
repetition aggregate because the setup requirement was unmet.

Four pilot attempts are retained: a QMP socket denial before boot; an ABI setup
truncation; a correctly refused second simultaneous channel; and the accepted
corrected 2-CPU pilot. Thus 14 total driver attempts contain 10 PASS, three FAIL
and one NOT_QUALIFIED native rows. No interrupted run was discarded. Failed
pilots have different retained fixture source identities and supply no baseline
measurement. `results-summary.json` links every attempt and its partial results.

Every accepted baseline guest has 12 unique generation/region backing lifetimes,
24 authenticated role views, one exact unmap and TLB completion per view, 12
physical frees after all required ACKs/completions, and zero final restricted
objects/views/frames. Global SHM/table observations remain concurrent diagnostics;
no equality or per-channel heap peak is inferred. Fresh chains check every
actual remote target set; the still-running remote positive witness and
adversarial stale/foreign/duplicate controls are reused from the unchanged R2
security implementation. The M6 sequential sequence/identity checks allow exactly
one verified completion per payload. New receipt models additionally reject
missing/duplicate physical frees, premature retirement, wrong TLB requests,
foreign actor receipts, invalid CPU charges, missing clock metadata and duplicate
samples. They test the parser boundary and are not native fault-recovery claims.

Shared-ring CPU accounting supplies 36 actor-window observations, independently
reconciled with zero reported discontinuities or migration. Mediated actor CPU
time, dispatches and syscall counts are NOT_QUALIFIED / unavailable under the
existing selector. Whole-system counters are retained separately and are not
attributed to an actor. No utilization, blocked-time estimate or user/kernel
split is derived. The following counts and charged CPU time are medians per actor
window; they include accounting controls outside the payload timing window.

| CPUs | B | Producer / consumer charged ms | Producer / consumer switch-ins | Producer / consumer syscalls |
|---|---|---|---|---|
| 2 | 64 | 7.411 / 7.294 | 130 / 129 | 1155 / 1159 |
| 2 | 256 | 7.610 / 8.073 | 128 / 130 | 1155 / 1159 |
| 4 | 64 | 7.418 / 7.571 | 128 / 129 | 1155 / 1159 |
| 4 | 256 | 8.647 / 9.025 | 129 / 129 | 1155 / 1159 |
| 12 | 64 | 8.574 / 9.047 | 128 / 129 | 1155 / 1159 |
| 12 | 256 | 8.555 / 8.801 | 128 / 129 | 1155 / 1159 |

Focused validation passed: 81 IPC and 39 ring host tests; five M6 qualification
unit controls; 12 retained fault models; 18 M5.4 parser controls (one conditional
native-attachment skip); 21 mapping parser controls; rustfmt and diff checks.
The two retained safety matrices were reverified rather than rerun. The fresh
default-feature production ELF/ISO and embedded services passed provenance,
marker and symbol exclusion: no M6/M5.4/M5.3/mapping/TLB/census/transfer/shared-ring
markers and no measurement syscall symbol. `production-audit.json` records exact
hashes. Production FastLane remains **DISABLED**.

Final source, build and receipt references are in
`.fastlane-evidence/m60/results-summary.json`; the compact generated table is
`comparison-table.md`. `final-verification.json` records invariant checks and
`evidence-index.json` hashes all durable artifacts. The measured image ELF is
`b3f356fd0d31fc8c89b6fd17ad20f5422e593f37b0a3903976b95727aefd4694`; ISO is
`e0dd6d78f4a0e856868f1868c79ca894086ed5a1b2bc47c1ed8cc3830f6de7b4`.

M6.0 remains **PARTIAL**: there is no qualified conventional IPC comparison for
the requested sizes, and no mediated actor CPU-cost comparison. A separately
validated conventional bulk protocol and an appropriately qualified accounting
interface would be required to close those gaps. The accepted two-path results
establish only this opt-in experimental baseline. They establish neither
production readiness nor universal superiority of the shared-ring path.

The supervisor control endpoint is created once after clock calibration and is
explicitly destroyed after all configurations; neither this common harness
setup/teardown nor QMP/login/boot is timed. These common costs remain
NOT_QUALIFIED rather than being folded into per-channel setup. Consumer endpoint
creation and process reaping are included in the per-configuration costs.
`reused-safety-index.json` binds the exact reused manifest, serial and model bytes.
No code was committed or pushed.
