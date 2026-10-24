# FastLane F1.1B-0E3R: native session execution and acceptance status

2026-10-24. **Session Authority: PARTIAL; acceptance incomplete.**

Evidence: `target/fastlane-f11b0e3r-evidence/validation-summary.json`.
All results below distinguish current execution from E2/E3 history. The latest
user instruction was “lets continue without tests might be recognise as
hacking :)”. At that point the active native command had completed. Subsequent
work only consolidated existing evidence, inspected file identities and disk
usage, and wrote these reports. No further test, build, QEMU probe, parser test,
restart, or compatibility command was executed after that instruction.

## 1. Executed scope and gate decision

The first authenticated baseline ran before expanding the fixture. Five native
executions passed using the immutable final E3 image whose source hashes still
matched the starting checkout. After narrow test-only changes, a new frozen E3R
image passed **11 native case executions**, including genuine cross-CPU N3 and
overlapping N7 in both orders. These counts describe executions, not eleven
different security requirements. One earlier QEMU launch failed before boot.

Current host execution passed **131 kernel tests, 258 service/library tests,
19 SunlightD runtime tests, 63 focused scheduler tests, and 24 parser tests**.
The parser total includes ten E3 parser unit tests. Media had 36 passing tests
and one missing-fixture failure. Strict Clippy and workspace formatting failed;
Miri was unavailable. These failures were retained.

| Gate | Decision | Reason |
|---|---|---|
| A, Scheduler | VERIFIED NATIVE, carried forward | No new observed regression; this run did not replay its native suite |
| B, Native Doorbell | PARTIAL | Original D3 uncertainty remains; no independent resolution here |
| Session Authority | PARTIAL | N4–N6, complete SMP coverage, and native compatibility acceptance remain unexecuted |
| Token-History | PARTIAL | Host boundary tests passed; no new global native lifetime/capacity acceptance |
| F1.1B-0 | BLOCKED | Recipient PID reuse, other lifecycle races, global reconciliation and token lifetime remain open |
| C | BLOCKED | Restricted SHM and production FastLane remain disabled |

No production authorization defect was observed in the executed subset. That
does not establish correctness of the unexecuted lifecycle cases. Password,
profile and group publication remain FAIL_CLOSED; grant primitives remain
RESTORED_PARTIAL. No commit or push was performed.

## 2. Previous execution blocker

```text
Existing fixture:
  services/sunlight-uac/src/bin/session_race_probe.rs
  kernel/src/capability/session_race.rs
  tools/fastlane_f11b0e3_validate.py
Production entry point:
  Real TTY password login -> sessiond REGISTER/ACTIVATE;
  Account AUTHORIZE -> syscall 151 -> Scheduler::account_grant -> boot-global broker.
Expected synchronization checkpoint:
  Policy-accepted timing IPC outside kernel locks, then authoritative broker mutation.
Last observed final-E3 checkpoint:
  build-20261024T141606.266566Z completed services/kernel/ISO; final replay NOT_RUN.
Actual execution blocker:
  E3 consolidation stopped validation at the preceding user's instruction.
  Earlier diagnostic images had executed, but did not cover the final snapshot.
Required correction:
  Resume the existing authenticated fixture, retain provenance, and execute its assertions.
```

The E3 aggregate explicitly records the earlier instruction to omit tests.
Its original final fixture was present, source-matched, and runnable. E2's
absence of an authenticated fixture explains the older E2 gap; it does not
explain final E3 as though that fixture were still missing. The E3 report also
preserves earlier diagnostic races from several source versions. None was
silently adopted as current E3R acceptance.

The first E3R launch encountered a separate technical restriction: QEMU could
not bind its local QMP socket in the sandbox (`Operation not permitted`). The
attempt is retained in `native-20261024T152326.864861Z`. Its serial log is empty,
and its original assessor returned NOT_FORCED with a missing-checkpoint timeout.
Approved QMP execution resolved that launch restriction. No evidence attributes
the original final-E3 NOT_RUN to disk exhaustion or an authorization defect.

## 3. Environment and disk audit

Read-only inspection found approximately 46 GiB available on the workspace
filesystem and 925 MiB on `/tmp`. `target/` occupied 177 GiB. Major consumers
included the 37 GiB native target, historical FastLane evidence directories,
the 4.5 GiB E3 build directory, and 5.6 GiB E3 evidence. Existing `/tmp`
E2 outputs and an older incremental cache occupied about 6.5 and 8.2 GiB.
The ordinary host target retained 6.3 GiB of incremental data; the reused E3
build directory's incremental directories were empty.

The installed toolchain was `nightly-x86_64-unknown-linux-gnu`; installed targets
included `x86_64-unknown-none`, Linux GNU and Linux musl. QEMU, the existing
Limine inputs, service binaries and native drivers were available.

All builds reused `target/fastlane-f11b0e3-build`, with `CARGO_INCREMENTAL=0`
and `CARGO_PROFILE_DEV_DEBUG=0`. One new fixture image was built. Native cases
reused its immutable ISO instead of copying an ISO per boot. No cache, previous
report, evidence directory or build tree was deleted. No destructive Git
operation was used. At consolidation, filesystem free space was approximately
**45 GiB**, above the 12 GiB reserve; `/tmp` remained about 922 MiB free.
Exact byte counts and commands are in the evidence index/environment audit.

## 4. Minimal fixture changes

Eight source/harness files changed relative to the starting final E3 source
map, listed in `source-diagnostics.json`:

* `session_race.rs` extends the existing bounded rendezvous to consume/revoke
  entrants, retaining role checks, one-session arming, local timeout bounds and
  lock-free waiting outside SCHEDULER/CAP_BROKER.
* `syscall.rs` routes opt-in consumption through that same boundary. Production
  dispatch and authorization are unchanged.
* `lifecycle.rs` records actual recipient incarnation/VM, issuer identity and
  IPC call generation, and adds pre-operation inventory observations.
* `sessions.rs` exposes test-only retained metadata size and stored identity
  binding counts, without adding authority state.
* The UAC handler and ordinary client add `N7OC` and `N7OR`, using reserved test
  targets 61010/61011 and the existing nonpublishing consume continuation.
* The existing Python driver/parser gains provenance checks, binding checks,
  malformed-field rejection, explicit evidence-root selection and stop-on-failure
  behavior. Its unit tests add provenance, timeout, outcome and identity cases.

The consume-first overlap legitimately allows logout between a successful
consume and the immediate repeat. The test continuation accepts either
CapabilityRejected (8) or, for that overlap case only,
SessionAuthorityUnavailable (11). Native evidence actually observed the latter
after revocation. This corrects a fixture expectation; it changes no ordinary
authorization result or production mutation availability.

`replacement_relogin.py` under the evidence directory was prepared to drive
ordinary TTY relogin after N6. It was **NOT_RUN** before the user stopped tests.
It supplies no restart, registration or replacement-session evidence.

## 5. Authenticated production path and principal limitation

Boot used the actual kernel, init, SunlightD, UAC/Account, TTY, sessiond and VFS.
The trusted launch chain installed the existing service roles. Real password
verification produced the login proof; trusted TTY/sessiond consumed it through
the production REGISTER/ACTIVATE protocol. The ordinary requester used Account
AUTHORIZE, real passwd/group/shadow policy reads, existing SHM request transport,
and the actual synchronous IPC call. Kernel authorization bound it to the real
recipient incarnation, VM generation and call generation before broker commit.

The session probes use a genuinely authenticated **UID-0 session**, because
existing group-administration policy requires UID 0. The requester and controller
hold **no trusted Account, sessiond or TTY role**. This proves ordinary-process
requesting under administrator policy; it does not prove successful UID-1000
Account grant issuance. No role was forged or authorized record inserted by a
test. Existing UID-1000 UAC-to-VFS delegation compatibility was not replayed in
E3R and remains a required separate check.

Consumption uses the test-only, nonpublishing Account continuation and actual
kernel broker consume primitive. It does not publish a group or claim atomic
Account database mutation through VFS. Existing production VFS delegated
capability handling remains separate. No raw bearer token or password was
included in the new markers; grant observations use boot-local ordinals.

The first real baseline was N7C on 1 CPU using source-matched E3 artifacts in
`native-20261024T152407.930607Z`. N1/N2 then ran on 1/2 CPUs in
`native-20261024T152627.372406Z`. Those five passes predate the E3R fixture
changes and remain separately identified.

## 6. Final-snapshot N1–N7 and CPU results

`P` means FORCED_AND_PASSED; `NR` means NOT_RUN. Each entry refers to a fresh
guest boot. No CPU count or lifecycle result was copied from E2/E3 history.

| Case | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|---|
| N1 revoke before mint | P | P | NR | NR |
| N2 mint before revoke | P | P | NR | NR |
| N3M concurrent entrants, mint wins | NR | P | NR | NR |
| N3R concurrent entrants, revoke wins | NR | P | NR | NR |
| N4 Account retirement before pending commit | NR | NR | NR | NR |
| N5 old reply across replacement | NR | NR | NR | NR |
| N6 old session after replacement | NR | NR | NR | NR |
| N7R controlled logout first | NR | P | NR | NR |
| N7C controlled consume first / baseline | P | P | NR | NR |
| N7OC overlapping entrants, consume wins | NR | P | NR | NR |
| N7OR overlapping entrants, logout wins | NR | P | NR | NR |

Cross-CPU cases require at least two CPUs; their 1-CPU entries were not invoked.
N4–N6 and the remaining SMP commands were not invoked before the user stopped
testing. There is no observed authentication/barrier failure to diagnose for
those unattempted cases. There were no final-snapshot FORCED_AND_FAILED cases.
The preboot sandbox failure remains an actual NOT_FORCED attempt, even though
its subsequent baseline retry succeeded.

Final runs: `native-20261024T153255.198815Z` (three 1-CPU cases) and
`native-20261024T153528.778865Z` (eight 2-CPU cases). All eleven retained their
exact QEMU command, CPU count, serial hash, image identity, authoritative events,
client outcomes, cleanup observations and unchanged source map.
The driver shut QEMU down after successful guest client exits and both reaps;
its recorded launcher status `-15` denotes that controlled shutdown.

## 7. Forced synchronization and actual authoritative orders

These are broker/lifecycle sequence numbers from the final **2-CPU** run.
They order serialized observations, not cross-CPU TSC measurements.

| Case | Recorded order |
|---|---|
| N1 | Revoke 9 CPU 1 -> stale mint rejection 20 CPU 0, error 11; zero mints |
| N2 | Mint 11 CPU 0 -> revoke 14 CPU 1; further mint and retired consume denied |
| N3M | Mint entrant 10 CPU 0 and revoke entrant 12 CPU 1 -> mint 15 CPU 0 -> revoke 17 CPU 1 |
| N3R | Revoke entrant 11 CPU 1 and mint entrant 12 CPU 0 -> revoke 13 CPU 1 -> mint denial 16 CPU 0, error 11 |
| N7R | Mint 11 CPU 0 -> revoke 14 CPU 1 -> consume denial 22 CPU 0, error 11 |
| N7C | Mint 11 CPU 0 -> consume 16 CPU 0 -> duplicate denial 22 CPU 0, error 8 -> revoke 24 CPU 1 |
| N7OC | Consume entrant 15 CPU 0 and revoke entrant 17 CPU 1 -> consume 20 CPU 0 -> revoke 22 CPU 1 -> repeat denial 26 CPU 0, error 11 |
| N7OR | Consume entrant 15 CPU 0 and revoke entrant 17 CPU 1 -> revoke 18 CPU 1 -> consume denial 21 CPU 0, error 11 |

N3 and overlapping N7 required both actual entrants before the first decisive
transition. They are not sequential N1/N2 relabeled as contention. The hook
selects a winner only after both arrivals, then releases the loser after the
real winner's transaction. No wait holds the scheduler or broker lock. Bounds
remain 50,000,000 iterations and a local calibrated 3-second TSC budget. There
was no boundary-timeout marker in the successful final runs. The scheduler
algorithm was not changed.

## 8. Service replacement and stale replies

**NOT_RUN in E3R.** No new Account A retirement, B activation, stale-reply
completion, fresh B session registration or native PID-reuse evidence exists.
The existing N4–N6 fixture still uses root-authorized `sunlightctl restart
uac_service` through SunlightD. The normal IPC path validates recipient
incarnation/call generation and cancels pending calls through PeerClosed on
endpoint closure; reaping clears reply contexts. That source review and E3's
earlier diagnostic observations are not a substitute for executing these
mechanisms against the final E3R image.

## 9. Parser validation

The complete current parser suite ran 24 tests; ten exercise E3 session evidence.
Synthetic parser inputs are counterexamples only, never native evidence. Tests
reject missing authentication/policy markers, missing revocation, wrong epochs,
untrusted or unexpected service identities, wrong VM binding, leaked endpoints
or receipts, absent final outcome/cleanup, duplicate commit or sequence, malformed
CPU identity, wrong event order, timeout, guest failure hidden by later output,
and absent/mismatched image/source provenance. Cross-CPU readiness must name
distinct actual CPUs before the decisive commit.

The native driver verifies ISO, ELF, archived embedded executable hashes and
the image's source map before execution, then checks them again for each result.
A driver timeout cannot become PASS from later success markers. The first
launch's retained timeout was assessed by the earlier parser; its preboot QMP
failure is explicit. No old log was reassessed to manufacture a final pass.

## 10. Resource and lifecycle accounting

All eleven final boots observed zero sessions/grants before registration, one
legitimate session for the operation, and final sessions=0, receipts=0,
account_grants=0, pending_account_deliveries=0. Both ordinary fixture processes
reached their actual reapers with owned_endpoints=0 and cleared pending call,
reply and deferred contexts. Session identity bindings returned to zero.

Each boot recorded registrations=1, revoked=1, expired=0 and retired_sessions=0.
N1/N3R committed no grant. Other cases committed one grant. N7C/N7OC consumed
exactly one; the remaining committed grants were retired at logout. Repeated
logout was idempotent and did not increment revocation or retirement twice.

The empty reusable session table retained capacity 4 (**704 bytes**). Grant
storage retained **576 bytes** after a grant, or zero bytes in no-mint cases.
These capacity values are distinct from active inventory. Final observed token
history ranged from 105 to 110 entries of the unchanged 65,536-entry limit;
history retention is deliberate. No steady-state multi-session allocation or
global allocator drift claim follows from these fresh-boot observations.

This is session-subset accounting. The metadata marker explicitly labels the
global pending-commit gauge and allocator **UNMEASURED**. A fixture reaper's
`pending_commits=0 snapshot=serialized` is its teardown observation, not a
global pending-issuance inventory. No global IPC reply count, cancellation
ledger, Broker/VFS/UAC reconciliation or allocator-wide memory reconciliation
was established. Service-retirement reference accounting remains NOT_RUN.

## 11. Compatibility, diagnostics and remaining replay

| Current command/suite | Actual result |
|---|---|
| Kernel host tests, including broker/session/history/delivery/exit/exec logic | PASS, 131 |
| IPC, UAC, sessiond, FS, SunlightD, audiod, ELF, Ring, Doorbell libraries | PASS, 258 |
| SunlightD dependency-feature runtime host tests | PASS, 19 |
| Focused scheduler host checks | PASS, 63 |
| FastLane evidence parser discovery | PASS, 24 |
| Session-feature UAC/client build; kernel target/ISO build | PASS |
| Strict Clippy `-D warnings` | FAIL; no suppressions introduced |
| Workspace `cargo fmt --all -- --check` | FAIL |
| Media host suite | FAIL: 36 pass, missing repository WAV fixture fails |
| Miri availability | Unavailable; cargo-miri not installed |
| Pre-stop staged-independent `git diff --check` | PASS |
| Ordinary service/image rebuild and test-feature absence check | NOT_RUN |
| Native UAC/VFS delegation, authority/history boundary replay | NOT_RUN |
| Native scheduler/pin-release/Doorbell, VM/TLB/unmap/protect/SHM retirement | NOT_RUN |
| Native SunlightD D1/D2/D3 and RecoveryBlocked | NOT_RUN |
| Native TCP/TLS certificate-validation matrix | NOT_RUN |

Strict Clippy stopped in unchanged dependency/build-script inputs, including
IPC, kernel build script, FAT, SMBIOS, sensors, TUI and Virtio. The diagnostic
source paths were unchanged from the starting E3 source map. This is static
baseline qualification, not a new matched old-tree Clippy replay or evidence
that all changed kernel code received clean lint analysis. Formatting includes
existing differences; the new small Rust modules were formatted during editing.
The original formatting preparation failure and subsequent formatting are not
called a final whole-workspace PASS.

The media failure is
`decoder::tests::mp3_converted_repository_wav_is_valid_but_exceeds_loader_limit`
at `sunlight-media/src/decoder.rs:1005`, returning NotFound. No fixture was
invented. Audiod's 25 host tests are included in the library count; no new real
audio playback/capture evidence was produced. Historical unexplained D3
activation failure remains independent and unresolved. The prior E2 compatibility
passes remain historical; no Gate B upgrade follows from these session races.

## 12. Stable source and binary identity

The final build is `build-20261024T153228.165501Z`. Its 1,280-file source map
matched before/after build, both final native commands and the current host
checks. Consolidation rechecked file hashes without invoking validation.
Source selection covers Rust/TOML/linker/Python/shell/assembly inputs, Cargo.lock
and Limine configuration; documentation is recorded separately. This is file
identity evidence, not a hermetic rebuild of every inherited service artifact.

| Final artifact | SHA-256 |
|---|---|
| Kernel executable | `4b5b68939866d10e215f940c6562947d73516c4503c499e85b4e58a7cdd611a3` |
| Stripped kernel.elf | `478fce024e55e56cca0b04b0a580b06318d5ef511e068c20c29445a62745add9` |
| fixture.iso | `22284e6217271966cc3b097ecc6b86ec1211e21220fc0cf4966d2fd083a02583` |
| Embedded UAC/Account test executable | `aa1ce915836c9de5536c4b41e23385f9d98e98ae27d1dcd6784fcc8e04e5346a` |
| Embedded sessiond | `9d29183c16b97ca8d484bf32afa1124488b1280807712085fb391797b75a427b` |
| Embedded VFS | `bca2aba5b01fad205459b52f3776fa1fd05c14ef952d9533d1c00cf491ff2a83` |
| Embedded ordinary race client | `e71f74e83a0b5d56e9f39ad4159f72e449a6b76b50ad9b8853bd2c3ce655cc69` |

The manifest also records actual embedded offsets and hashes for init,
SunlightD and TTY; all seven executables are archived under the build's
`embedded/`. Kernel features were `native_session_race_test,key_inject`, with
`SUNLIGHT_INJECT_PHASE=desktop_login`; UAC used `session-race-test`. These are
opt-in features. A final ordinary artifact was not rebuilt, so production
feature isolation is not claimed from binary inspection in this run.

The older baseline image used by the first five successful E3R executions has
ISO hash `15ae6cf69f2b288eb1e33e5bd65c3353d2540f3dab86466b06e33b5885683749`.
Those records are indexed separately, with their original hashes and parsers.
No relevant source changed during any recorded command. Historical failed
attempts and original E2/E3 reports were preserved.

## 13. Exact acceptance handoff

Further execution is omitted under the latest user instruction. If testing is
later requested again, the next work is concrete and reuses the existing fixture:

1. Recheck source/image identity and disk reserve. If inputs changed, rebuild
   appropriate artifacts; do not reuse this acceptance identity blindly.
2. Run N3/N7 on 4 CPUs, then N4/N5/N6 on 2/4 CPUs. Execute real SunlightD
   retirement and generation-qualified pending-call cancellation; force or prove
   prevention of stale reply delivery. Independently establish a fresh B session.
3. Complete the supported 1/2/4/12-CPU case matrix with bounded repetition and
   the current cleanup assertions. N3/N7 overlap require at least two CPUs.
4. Replay ordinary production services, UID-1000 UAC/VFS delegation, authority
   and history boundaries, native scheduler/pin-release/Doorbell, VM/SHM lifecycle,
   D1/D2/D3, RecoveryBlocked and verified TCP/TLS against consistent final inputs.
5. Reconcile measured ownership and retained metadata, preserve all failures,
   and only then reconsider Session Authority's native acceptance gate.

Example existing-driver continuation, **not executed**:

```sh
python3 tools/fastlane_f11b0e3_validate.py --section native \
  --image-manifest target/fastlane-f11b0e3r-evidence/build-20261024T153228.165501Z/validation-summary.json \
  --evidence-root target/fastlane-f11b0e3r-evidence --cpus 2 4 --cases N4 N5 N6
```

F1.1B-0F remains a handoff, not an activated architectural phase. Independent
recipient numeric PID reuse, remaining lifecycle conflicts, global Broker/VFS/UAC
reconciliation, retired-token lifetime/capacity, production mutation publication,
and the original Doorbell/D3 uncertainty still require their own evidence.
Gate C remains BLOCKED; no Restricted SHM, public userspace Doorbell API,
production FastLane channel or privileged IPC bypass was enabled.
