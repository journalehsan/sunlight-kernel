# FastLane M5.3: identity-qualified backing quotas

Status: **BLOCKED for milestone acceptance; implementation PARTIAL**.
Evidence: `.fastlane-evidence/m53-quota-20261108T230044Z/REPORT.md` and
`validation-summary.json`. Directory timestamps use UTC; this work was performed
on November 9, 2026 in Asia/Tehran. This quota milestone is separate from the
older M5.3 measurement-readiness document.

## Implemented boundary

Legacy SHM backing is charged to `(PID, process incarnation)`. Native workers
capture the process owner's exact identity at creation, independently of their
numeric descriptor-sharing owner. Process constructors reserve incarnation
before resource setup; scheduler publication preserves it. Exec preserves the
process identity while replacing the VM. The existing address-space identity,
region/PTE validation and synchronous TLB protocol continue to govern mappings.
There is one backing byte charge, rather than charging backing again per view.

Authenticated foreground PTY members additionally charge the existing
`(broker incarnation, session ID, session generation)` tuple. Restricted-channel
backing charges its immutable creator and authenticated Account `SessionRef`.
Callers with no PTY tuple use the process ceiling. Incomplete or zero-generation
identities fail admission. Existing outstanding receipts keep their original
owner and session through exec, membership changes, and peer retention.

Both process and session reservations are serialized in one bounded ledger.
Reservation preflights arithmetic, both limits and metadata capacity before
mutation. Existing keys are found before empty slots, closing the old fragmented
ledger bypass. Overflow and exhaustion reject allocation. The allocation
receipt moves into the backing object; there is no caller-supplied PID/byte
refund interface. Verified final unmap transfers backing and its receipt to a
reclamation result. That result refunds accounting after PMM frees all backing
frames. Dropping an unreclaimed result retains its charge. Revocation with live
views retains backing and charges. Kernel invariant corruption fails closed.

## Initial limits and compatibility

Process: **512 MiB**. Authenticated session: **1 GiB**. Ledger: 512 combined
process/session entries; metadata exhaustion also denies allocation.

Display accepts 8192 x 8192 x 4-byte surfaces (256 MiB each), and resize allocates
a replacement before releasing the prior surface. The process ceiling admits
two such surfaces. Document transport accepts a 64 MiB + page response alongside
a request. Wise Owl insert batches are at most 64 KiB; bulk PCM, PTY, accounts and
clipboard use page-sized buffers. These consumers make the former 16 MiB limit
inadequate. The session ceiling admits two complete process budgets. These are
ceilings, not guarantees of available RAM or support for unlimited windows.

`shm_alloc`, `shm_create`, and the kernel single-page wrapper converge on the
same reservation path. The internal already-backed `mint_shared_page` security
fixture remains explicitly uncharged and has no exposed allocation syscall.
Legacy bearer-token authorization is preserved. Restricted admission and payload
transfer remain debug gated; hostile receiving continues to use private copies.

## Dependencies and remaining gates

General Account/desktop/legacy-login sessions do not provide authenticated
membership for every process. This implementation does not infer it from UID or
bearer grants. Services and processes without the foreground tuple retain the
process fallback. Propagating and qualifying membership, including workers that
predate binding and membership transitions with outstanding backing, is a
separate lifecycle dependency. A comprehensive login-session budget remains
BLOCKED. The native tests assign a PTY tuple inside a kernel fixture; they do not
prove the trusted PTY publication/inheritance path or independent live sessions.

The existing allocation path still serializes VM mutation under scheduler,
PMM and broker locks. The quota ledger is held only for bounded bookkeeping.
Moving long allocations outside the existing lifetime locks requires explicit
VM/process and broker publication pins; this phase preserves the lock order.
No locks were added to the steady-state payload path.

Final-source host checks pass. Native kernel prerequisites pass on 1/2/4/12 CPUs,
including physical quota boundaries on one CPU and real remote invalidation,
guarded physical reuse and zero frame/quota drift on SMP. The fresh restricted
userspace gate panics in unchanged `channel_race.rs:165` before M2 mapping tests.
The failed combined gate remains failed; isolated quota results do not supersede
it. The assertion's runtime provenance has not been established with a pristine
baseline. Mandatory scheduled crash/kill/exec quota counters, stale incarnation
reuse, VM mismatch, concurrent allocation/release races and full mapping
permission evidence remain unvalidated.

Production FastLane and normal restricted-channel admission remain disabled.
Next milestone: **M5.3R — qualify the M1 private-stack continuation boundary,
then close the source-matched native quota/session and lifecycle matrix**.

## M5.3R follow-up, November 9, 2026

The original private-stack failure is now classified and repaired as a
pre-existing debug fixture/entry-contract mismatch, reproduced on the exact
pre-M5.3 baseline. Fresh source-matched M1/M2 userspace runs pass on 2/4/12 CPUs;
new restricted backing receipts reconcile process/session quota after PMM
reclamation. Kernel quota prerequisites freshly pass on 1/2/4/12 CPUs.

Overall acceptance remains BLOCKED on complete authenticated session membership
and outstanding scheduled quota lifecycle/identity/race coverage. These results
do not supersede the preserved original M5.3 failure or validate its source as
a passing combined run. See [M5.3R continuation qualification](fastlane-m53r-continuations.md)
for the repair, ownership proof and remaining native evidence limits.

## M5.3R2 scope correction, November 9, 2026

The foreground PTY key is authenticated by the trusted terminal broker, but it
does not prove membership in an authenticated login. The Account session registry
and general spawn paths lack a process-membership handoff. Thus a 1 GiB PTY-group
ceiling must not be described as complete enforcement of the authenticated login
budget. General launches, detached/unbound processes, pre-binding workers and
shared multi-user services remain outside that claim. Limits and existing receipt
semantics are unchanged. See
[M5.3R2 membership boundary](fastlane-m53r2-membership-lifecycle.md).
