# F1.1B-0H5: final-source native fixture execution closure

2026-10-28, Asia/Tehran. **VERIFIED NATIVE for the existing authenticated
session-owner contention fixture.** S1 and S2 completed on 2, 4 and 12 CPUs,
including actual competing lock acquisitions, terminal security assertions,
scoped cleanup and independent host acknowledgement. Production FastLane remains
disabled. Wider authority and reaper/endpoint gates remain unchanged.

Evidence root: `target/fastlane-f11b0h5-evidence/`; the aggregate is
`validation-summary.json`. Individual native manifests, serial logs, host receipts
and sealed attempt records are retained there. Paths below are relative to that
root unless otherwise stated. Directory timestamps are UTC; the local report
date is October 28. No old H/H4 execution is counted as H5 acceptance.

## Starting state and execution

The starting worktree was clean at
`02955ae7eee07f5fa453a18e920dd248eb35d488`. H4's report and aggregate were read
directly from `docs/ipc/fastlane-f11b0h4-authorized-controller-contention.md` and
`target/fastlane-f11b0h4-evidence/validation-summary.json`. H4 had implemented and
host-tested controller preparation, bounded coordination, trace observations and
completion receipts, but had not executed its final source. Controller READY,
manager acceptance, contender observations and the 2/4/12 CPU matrix were
NOT_RUN; both winner orders and completion were BLOCKED; contention and
peer-close-first were NOT_FORCED. Earlier H4 partial successes remain historical.

Disk space was checked before both builds: approximately 105 GiB, then 102 GiB
available. H5 built the current instrumented image before running S2 on two CPUs:

```sh
python3 tools/fastlane_f11b0h4_validate.py --section build --evidence-root target/fastlane-f11b0h5-evidence
python3 tools/fastlane_f11b0h4_validate.py --section native --cpus 2 --cases S2 --image-manifest target/fastlane-f11b0h5-evidence/build-20261027T215319.327000Z/validation-summary.json --evidence-root target/fastlane-f11b0h5-evidence
```

The first native attempt, `native-20261027T215424.492479Z`, fixture `661282069`,
completed successfully. **First missing checkpoint: none.** No timeout or later
missing marker was inferred from a partial trace. The opposite winner order S1
then passed on two CPUs before matrix expansion. No fixture, synchronization,
parser, timeout, authorization or production code change was needed. The only
tracked deliverable added in H5 is this report; evidence is under ignored
`target/`. Existing runner manifests retain their H4 phase label because the
existing fixture and driver were executed unchanged; this aggregate identifies
the H5 executions.

## Proven source and image

Build manifest: `build-20261027T215319.327000Z/validation-summary.json`.
Its 1,295 build-input hashes match the live final source and every native attempt.
The aggregate records the source-map fingerprint, Git HEAD, complete map and
relevant source hashes. The image was newly built for H5.

| Artifact | SHA-256 |
|---|---|
| Instrumented unstripped kernel at build | `edb33987caf5b3bbf0fcca1e62ff817628f43253487e7ae0cfb6fdfbdfd3be06` |
| Executed kernel ELF | `57b640f966a1333178d85bb00429bfc9c985da0fbdb0c9a37232bc27a573ce49` |
| Executed ISO | `baab603aa5f6cc10d5dd900357e286ea774c98f611ac3c55b753abab6fb9f048` |
| Actual embedded sunlightctl | `7a4cc9014a140d5276d33fc6c7b973b6c48437774c4dad483922f25c15791b32` |
| Embedded Account/UAC | `1ec3c4888b7377b6715cb6def92cd42846276ffce4bf4de52e30ac200cf08a3f` |
| Embedded sessiond | `9d29183c16b97ca8d484bf32afa1124488b1280807712085fb391797b75a427b` |
| Embedded fixture | `3e42dd73b7e232768de0b7bb09e30229a19d4f2812fb61d72a07b39f38e5a0ce` |

Kernel features: `native_session_h4_test,key_inject`; UAC, sunlightd and
sunlightctl: `session-h4-test`. Build environment:
`SUNLIGHT_INJECT_PHASE=desktop_login`, `SUNLIGHT_SESSION_N4_AP=1`.
Account runs on CPU 1; the manager and session-owner exit boundary run on CPU 0.
These are test placements, not inferred CPU participation.

All eight embedded executables are archived, hashed and verified byte-for-byte
inside the executed ELF. Extraction of `/boot/sunlight-kernel.elf` from the actual
ISO matched its recorded ELF hash. Each driver invocation checked ISO, ELF,
service archives and source identity before and after execution. QEMU's recorded
`-cdrom` names this H5 build. The later production build did not replace the
archived test image. Full service hashes, sizes and offsets are in the manifests.

## Controller and observed checkpoints

The ordinary root `sunlightctl` participant uses its existing protected lookup,
Status request and Restart wire request. Its real target and usable route are
resolved before READY. The private prepare/release endpoint carries coordination;
it gives `session-racer` no manager capability. Normal manager admission and
target resolution precede the acceptance observation. No extra privileges or
process/service identities were introduced.

Existing markers implement the requested checkpoints:

| Requested state | Existing evidence |
|---|---|
| Controller started / READY | Actual spawn/reaping identity; `[H4-CTL] checkpoint=ready`, trace 60 |
| Authenticated grant prepared / safe boundary | `authenticated-requester`, `request-binding`, trace arm 1 and boundaries 65/68 |
| Manager submitted / accepted | Trace 61 / 62, distinct from authorized signal 64 |
| Exit contender reached | Trace 70, before lock acquisition 71 |
| Actual lock order / conflict | Grant acquisition/release 3/6; exit acquisition/release 71/74; failed attempts 69/73 |
| Grant / retirement outcomes | Grant result 4/5; authoritative broker retirement 20 |
| IPC terminal / cleanup | Fixture assertions, normal exit, `fixture-ipc-cleaned`, participant reaping, broker inventory |
| Guest receipt / host ACK | Correlated trace 80 plus complete bounded dump; separately written and read back `.host-ack.json` |

The markers remain observations of actual transitions. Manager ACCEPTED is not
used as a contention marker. Both safe boundaries precede either decisive lock
acquisition. No scheduler/broker guard is retained across the bounded gate wait.

## Winner orders and genuine contention

For the first S2/2 run, issuer Account was PID/incarnation/VM `20/32/51`, recipient
`39/51/87`, session/epoch `1/1`, IPC call generation 4, and owner `30/42/71`.
Controller READY was sequence 1, arm 4, grant safe boundary 6, submission 7,
manager acceptance 8, exit-ready 11 and exit-acquired 12. Broker retirement
committed at 14. Grant acquisition failed at 16 while the exit lock interval
12–18 remained active; grant acquired at 19 and rejected at 20, with production
`SessionAuthorityUnavailable` (11). No grant committed. The later fresh request
was denied as `SessionChanged` (5); all participants completed and were reaped.

For S1/2, issuer was `18/30/47`, recipient `39/51/87`, session/epoch `1/1`, call
generation 4, and owner `27/39/65`. Grant acquired at sequence 13 and committed
at 14 while authority was live. The exit contender failed acquisition at 15,
inside the grant's 13–16 lock interval, then acquired at 17. Authoritative
retirement followed at 19. Exactly one mint and zero consumes were recorded;
retirement removed the grant, receipt and pending delivery through production
policy. The Account reply was REPLY; the later old-session request was denied.

These are genuine failed production lock acquisitions on different CPUs, not
just two sequential operations or a manager request. S1 and S2 both recorded
such overlap on every CPU configuration. S3 was NOT_RUN: an additional race
matrix was unnecessary after both existing winner-order cases directly observed
the required contention. No lock was held while waiting for the opposing CPU.

## Native ledger and matrix

Every row has validated guest completion, a host ACK and matching final identity.
Full commands, source/image/service hashes, trace sequences and receipts are in
each immutable attempt record. No H5 native attempt failed or required retry.

| Directory suffix | Case / CPUs | Fixture | Actual trace CPUs | Result |
|---|---|---|---|---|
| `215424.492479Z` | S2 / 2 | 661282069 | 0,1 | FORCED_AND_PASSED |
| `215526.066384Z` | S1 / 2 | 495742562 | 0,1 | FORCED_AND_PASSED |
| `215624.644987Z` | S1 / 4 | 287423969 | 0,1,2 | FORCED_AND_PASSED |
| `215624.644987Z` | S2 / 4 | 250761132 | 0,1 | FORCED_AND_PASSED |
| `215823.040349Z` | S1 / 12 | 426736967 | 0,1,2,3 | FORCED_AND_PASSED |
| `215823.040349Z` | S2 / 12 | 535638811 | 0,1,2,3,6 | FORCED_AND_PASSED |
| `215836.147695Z` | N4 / 2 | 960110059 | 0,1 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT |
| `215836.147695Z` | N6 / 2 | 654123267 | 0,1 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT |

Each suffix belongs to `native-20261027T...`. The grant/exit contenders themselves
used CPUs 1/0 in all six S cases; additional trace CPUs are not extra race
participants. All boots completed fresh authenticated login before the fixture.
Post-retirement replacement login and stale-endpoint adoption attempts were
NOT_RUN; no wider recovery/session-authority claim is made.

N4 and N6 are focused end-to-end regressions using the same final image. Their
unchanged parser's classification is retained verbatim. N4 observed grant-first
and the execution-ownership serialization contract; N6 verified replacement,
old-call closure and stale-session denial. Neither contributes additional S-case
contention proof or closes the peer-close-first gap.

## Completion and scoped cleanup

All security terminal observations arrived, followed by normal fixture exit and
resource cleanup. The host consumed the serial evidence, verified its identity
and only then wrote/read the hash-bound ACK. It did not require a retired TTY
owner to emit a new foreground-exit message. QEMU was deliberately terminated by
the driver after the required guest cleanup observations; its signal exit is
recorded separately from the fixture's exit code zero. Serial hashes were taken
after process shutdown and validated against ACKs, not while evidence was moving.

`completion-negatives.json` records 30 PASS host checks using mutated copies of
the captured two-CPU traces. Missing security, cleanup, manager acceptance,
lock acquisition, retirement or grant outcome; wrong/stale fixture; truncated
trace; timeout; failed authorization; mismatched source; missing ACK; missing ACK
cleanup; and mismatched serial hash were rejected. These are parser checks,
not additional native executions. The existing 49-method parser suite also passed.

`resource-audit.json` independently correlates all six S cases: authenticated
issuer incarnation and VM, recipient incarnation and VM binding, session epoch,
call generation, retiring owner, exactly one S1 mint or no S2 mint, zero consumes,
endpoint removal and terminal inventories. All finish with zero live sessions,
receipts, Account grants, pending Account deliveries, session identity bindings
and delivered-unconsumed grants. Each fixture/controller/sunlightctl cleanup
receipt has zero owned endpoints and no pending call, reply target, deferred
targets or pending commits. Their processes and the old owner are reaped.

Account and replacement sessiond remain legitimate services. Session capacity 4,
one historical registration/retirement, token history and reusable metadata
capacity persist; they are not live authority or unexplained retained grants.
Allocator heaps and a global Account/sessiond/UAC/VFS census are UNMEASURED.
No full global reconciliation or stale-grant consumption test is claimed here.

## Reaper path and remaining endpoint limitation

S cases follow the genuine session-owner exit path: path event 10/a=1, broker
retirement, endpoint removal (zero queued calls), then resource reap path 10/a=2.
N4/N6 follow Account self-exit, broker retirement, endpoint removal and matching
pending-call peer-close, then resource reaping. Both regressions are also
retirement-first. **Peer-close-first remains NOT_FORCED.** The required separate
trigger is the genuine resource-reap endpoint-removal path with a still-pending,
generation-correlated call before broker session retirement. No path selection,
log order or existing parser predicate was changed to manufacture that result.

## Focused regression and production build

Commands and logs: `checks-20261027T215624.647964Z/validation-summary.json`,
`host-focused/validation-summary.json`, and
`normal-20261027T215719.350979Z/validation-summary.json`.

| Check | Classification |
|---|---|
| Kernel capability/IPC host library, 133 tests | PASS |
| IPC/UAC/sessiond/FS and related service libraries, 258 tests | PASS |
| SunlightD runtime/manager, 19 tests | PASS |
| Scheduler production state/lifecycle/accounting/policy checks | PASS |
| Native harness/parser suite, 49 methods | PASS |
| Controller / bounded gate / trace buffer, 2 / 3 / 1 tests | PASS |
| Captured-trace completion negatives, 30 checks | PASS |
| Native S1/S2 on 2/4/12 CPUs; N4/N6 on 2 CPUs | PASS within the scopes above |
| Ordinary production services/kernel/ISO; test marker exclusion | PASS |
| Existing focused fixture-module formatting; diff whitespace | PASS |
| Strict Clippy | FAIL_CURRENT; dependency/build-script/IPC diagnostics retained |
| Media/WAV fixture | FAIL_CURRENT; 36 passed, 1 missing-file failure |
| Workspace formatting | FAIL_CURRENT |
| Miri | UNAVAILABLE; cargo-miri is not installed |

Strict Clippy diagnostics include virtio safety documentation, TUI build script,
sensors/SMBIOS, compat-linux, FAT, kernel build script and IPC result type checks.
The WAV failure is
`decoder::tests::mp3_converted_repository_wav_is_valid_but_exceeds_loader_limit`
at `sunlight-media/src/decoder.rs:1005` (missing file). No unrelated remediation
was attempted and no FAIL_BASELINE_MATCHED classification is asserted.

Production ELF SHA-256:
`6655d19022f05ea2d25703ae9349b02299a293d27db234f4e15df3afb29ecf2b`;
production ISO:
`f45537259d41e333fedd13db4e1cc698d50c9104773a4686fbd7efe83157d0f8`.
Production feature and test-environment lists are empty, with the same source map.

Historical H peer-close-first, N3M endpoint lookup, D3 failures, earlier N4 timeout,
Clippy, WAV, formatting and Miri limitations remain recorded in the H4 aggregate.
New successful N4 execution does not erase the old timeout. H/N3M/D3 historical
cases were NOT_RUN in H5. All original evidence remains untouched.

## Decisions and next executable work

Controller readiness, ordinary manager acceptance, both safe contenders, both
winner orders, genuine contention, terminal security/cleanup and guest/host
completion are **PASS on the final-source native image**. H5 execution closure
is **VERIFIED NATIVE only for this fixture scope**. There is no remaining H5
controller/contention/completion blocker.

Scheduler Gate A remains VERIFIED NATIVE and Token-History VERIFIED BOUNDED
(65,536 identities per boot). Reaper/Endpoint Ordering, Global Authority, Grant
Lifecycle, Session Authority and Doorbell Gate B remain PARTIAL. F1.1B-0 and
production Gate C remain BLOCKED. Restricted SHM, public Doorbell APIs and
production FastLane remain disabled.

The smallest separate next executable blocker is the peer-close-first native
resource-reap trigger described above, with matching endpoint and pending-call
generation. Post-retirement fresh login remains a separate unexecuted recovery
check. No new broad matrix or architecture framework was added. No commit,
push, destructive Git operation, credential logging or unrelated source change
was performed.
