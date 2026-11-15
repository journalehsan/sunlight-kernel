# SFS-5B.6 — Real BlockBackend drain and allocator reuse boundary

**PARTIAL. Not COMPLETE. No guest DMA release or allocator reuse is authorized.**

This experiment extends the isolated VirtIO fixture into QEMU's real
BlockBackend. It does not complete the retirement, allocator reuse, device
removal or epoch qualification required by SFS-5B.6. The machine-readable
ledger is `docs/audit/sunfs/sfs-5b6/validation-summary.json`; individual raw
attempts and failures remain in that evidence namespace.

## Baseline and preservation

The initial revision is `22a5947f5fbd4ffe810aee5efee57b0ec18f8915`, branch
`main`. The incoming dirty checkout includes many prior evidence deletions,
tracked changes and untracked SFS work. The complete incoming status, binary
diff, production/harness hashes and prior SFS evidence hashes were captured
before fixture modifications. No incoming change was reverted.

Focused baseline host tests completed: 68 block tests and 30 VirtIO tests.
The baseline recorder subsequently encountered a relative-path ledger error;
the test log is retained and the recorder corrected. See
`development-attempts.md` for all development failures and source revisions.

Free space was checked before the isolated QEMU build: approximately 52 GiB
on the repository filesystem and 13 GiB on `/tmp`. No cache or prior evidence
was deleted. QEMU source/build directories are `/tmp/sfs5b6-qemu-src` and
`/tmp/sfs5b6-qemu-build`. Earlier QEMU trees and the system emulator remain
unchanged. Incremental Rust compilation was disabled for new validation.
Subsequent ENOSPC attempts were preserved. After the user freed Docker image
space, approximately 143 GiB was available and final builds were rerun.

## Actual backend path

The guest sends a genuine 512-byte write to sector 63 of an exclusively
created marked disposable image. Existing descriptor parsing, bounds checks
and accounting precede the hook. The original request qiov is passed to
`blk_aio_pwritev`. The final coroutine hook is inside
`blk_co_do_pwritev_part`, after admission gating and the BlockDriverState
in-flight increment. It uses supported coroutine timers on the main
AioContext, without invoking BlockBackend from a private thread.

Device logs identify the request pointer, descriptor head and qiov. Backend
logs identify that same qiov and the actual backend in-flight value. Guest
logs record submission, timeout, uncertainty, reset and pinned ownership.
Host reads of the disposable sector independently distinguish a completed
write from the injected pre-write EIO case. Readback is not durability proof.

The retaining completion callback records the terminal result without
publishing a used entry. Reset drains, verifies that callback terminal state,
then detaches the element and frees the emulator request. Thus this fixture
qualifies an instrumented reset cleanup path; it does not qualify the stock
used-ring publication race.

## Drain failure that changed the fixture

The first hook delayed before `blk_wait_while_drained`. When resumed during
drain, that function parked the coroutine on `queued_requests` and temporarily
removed its in-flight count. Drain returned with the callback unresolved.
The fixture assertion stopped the process before mapping release.

This preserved negative observation is essential: earlier in-flight admission
plus a drain call is insufficient evidence of callback closure. The corrected
hook holds after admission with the node's in-flight reference acquired.
No production QEMU drain behavior was changed to obtain this result.

## Cancellation and bounded failure

`blk_aio_em_aiocb_info` has no `cancel_async` function. Asynchronous
cancellation therefore leaves the request to complete normally; it does not
establish cancellation acceptance or terminal cancellation. Cancellation-wins
cases remain unsupported by this AIOCB and unqualified for the milestone.

The indefinitely held case has an external process supervisor. It permits
45 seconds for scenario execution, then termination and a 15-second grace
period before killing that specific process. Process destruction is a
terminal test failure, never proof that the guest attachment became safe.
There is no guest mapping release or epoch transition in that failure path.

## Separate private mapping lifetime

The `private` case keeps a joinable device worker reading the original mapped
payload while genuine backend I/O executes. After backend drain returns,
the reset owner publishes a mutex-protected marker. A subsequent real payload
read witnesses that marker before worker stop/join and mapping detach.
The 2-second observation bound fails closed if that witness is absent.

This directly distinguishes backend drain from private-user quiescence. The
old `async` worker-only case is retained separately as a control; it cannot
pass genuine backend admission. The combined device-side closure still does
not reconcile guest allocator ownership or authorize release of its pages.

## Race and interrupt boundaries

`completion-races.json` preserves every requested race identifier. Backend
completion before/during drain and asynchronous cancellation returning before
normal completion have explicit logs. Those cases do not automatically
qualify timeout/used-ring completion, removal, reset retry or pressure races.

R4/R5 are NOT_APPLICABLE to the current polling-only driver, and execution of
the hypothetical interrupt path is NOT_RUN. No interrupt handler was added.
The architectural assessment is `interrupt-path-assessment.md`.

## Retirement and epoch decision

Emulator request detach/free is distinct from guest physical page release.
All guest attachment pages remain pinned. There is no fresh allocator
release, replacement allocation, controlled replacement I/O, ABA qualification
or native premature-reuse negative test. These are NOT_RUN, not inferred
from free-page counters or host `g_free` calls.

The original five historical pages remain excluded: 20,480 bytes retained,
zero released or assigned to a replacement queue. Exact historical physical
ownership provenance remains incomplete; no addresses were invented.

The guest coordinator stays in terminal quarantine. No new epoch,
reinitialization or writable SunFS integration is authorized. Device reset,
backend terminal completion and observed file contents cannot erase the
original write/flush uncertainty. Exact short counts remain unknown where
the typed interface reports `Amount::Unknown`.

## Evidence and validation

The final evidence index separates final emulator/guest hashes from earlier
development executions and startup failures. It records 1/2/4 CPU profiles,
both inherited cache settings, the unchanged `0x5b5001` deterministic fixture
seed and exact launch arguments. Guest READY output contains per-CPU online,
timer and task progress; no boot-only attempt is counted as scenario proof.
The machine is q35/TCG with 1,024 MiB RAM, no explicit CPU model override,
legacy VirtIO PCI (`disable-modern=on`, `ioeventfd=off`) and polling completion.
The observed accepted feature word is 576 (`0x240`). No IOThread, migration,
IOMMU or physical-device memory model is qualified by these runs.
The final series completed 30 genuine backend runs: completion before drain,
completion during drain, asynchronous cancellation request, backend EIO and
private mapping-user closure, each with 1/2/4 CPUs and both cache settings.
Six separate old-worker controls completed. One additional indefinitely held
backend operation reached drain and required supervisor process termination;
it supplies no quiescence or retirement proof.

The event graph records observed device/backend order and explicit private
worker synchronization. Full identity-bound guest/allocator happens-before
closure remains PARTIAL. An acyclic device log alone is insufficient.

Fresh host/storage, no_std, formatting, Clippy, kernel builds and hosted SunFS
results are indexed in `regression-results.json`: 208 storage tests, 212 kernel
host tests, no_std checks, scoped formatting and Clippy, default and diagnostic
kernel builds passed. Clippy retains existing warnings. Each hosted SunFS
profile freshly passed 54 tests and 13,142 schedules (13,044 crash schedules
plus 98 reordering schedules). Earlier packaging failures do not invalidate
the retained test output or erase their failed wrapper outcomes. Historical
schedules are not counted as fresh execution. The prior UART THRE stall remains
UNQUALIFIED; the serial driver was not changed.

G1 and G2 pass for the declared inventory and genuine instrumented backend
scope. G10 covers evidence integrity, not missing behavioral qualification.
G3–G6, G8 and G9 remain PARTIAL; G7 is NOT_RUN. Successful regressions do not
complete the missing native fault, retirement or allocator-reuse matrix.

`tools/sunfs5b6_verify.py` independently checks identity-bound final runs,
mandatory synchronization edges, rejected missing-edge/cyclic graphs, raw
schedule archive hashes and the explicit absence of guest frees/reuse. Its
parser negatives are evidence checks, not native stale-DMA or allocator tests.
`verification.json` records these checks; `artifact-manifest.json` binds the
final artifacts and scripts. Reproduce aggregation with
`python3 tools/sunfs5b6_report.py`, then verification with
`python3 tools/sunfs5b6_verify.py`; the former additionally needs the pinned
local QEMU source/build directories to recheck their hashes.

## Remaining work

Continue SFS-5B.6 with a complete identity-bound cross-layer event graph,
deterministic completion/removal/retry/pressure races and a fresh allocation
manifest. Only after all six retirement prerequisites are observed should a
guest allocator retirement permit be implemented and exercised with controlled
replacement I/O and safe stale-completion negatives. No historical quarantine
recovery is included in that continuation.

No native SunFS mount, physical formatting, root/boot storage change,
CPython/SunApp implementation, scheduler redesign, commit or push occurred.
