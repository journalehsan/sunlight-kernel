# M5.3R5.3 authenticated retirement and process cleanup

Production FastLane, restricted production admission and hostile-process zero-copy
remain disabled. This milestone qualifies a focused process-cleanup ordering
subset of original `concurrent/session-termination-peer-retention`. It does not
close the original 28-case matrix. Evidence is under
`.fastlane-evidence/m53r53-20261109T233500Z/REPORT.md`.

## Existing authority semantics

`kernel/src/capability/sessions.rs` has Active and Suspended registry states.
Registration begins suspended with one-use bootstrap eligibility. Active,
unexpired membership permits allocation and authorized inheritance. Suspension
blocks ordinary admission, removes grants and advances the launch revision, so
an outstanding launch attachment cannot become valid merely by resuming.

There is no additional Retiring enum value or separate retired registry. Under
SCHEDULER -> CAP_BROKER, `remove_session` removes the exact SessionRef row with
`swap_remove`; this is the authority retirement linearization point. The passive
`[R51-SESSION] event=retired` receipt occurs in that same locked operation.
Grant withdrawal completes before the operation returns. Removal, expiry and
authority-owner/issuer death use the same removal path. Existing process records
may still carry that SessionRef, but its absence prevents new Account allocation
and inherited/SessionD launch. Zero quota cannot recreate a registry row.

Session IDs and broker epochs are checked monotonic allocations. Removing a row
never refunds an identity. Numeric process IDs alone cannot confer membership;
launch authorization binds the actual process incarnation and VM identity.
This milestone does not force session ID, epoch or numeric PID reuse, and makes
no identity persistence claim across reboot.

`shared_quota.rs` is an independent ledger, not the authentication registry.
Its move-only backing receipt retains the original process incarnation and
session ID/epoch through retirement, owner death and exec. A retired authority
can therefore have outstanding charges. Fully reclaimed means the backing,
views and receipts have closed; it does not mean authority becomes active again.
Quota limits remain 512 MiB per process and 1 GiB per session.

`shared.rs` checks active membership before reserving new backing. Legacy bearer
attachment follows the bearer API's existing authority contract and does not
check Account liveness. Account retirement does not directly unmap legacy views
or reclaim their frames. The regular unmap path removes the PTE, completes the
specific synchronous TLB request, removes mapping metadata and decrements the
reference count. Only the final reference transfers backing to PMM reclamation;
the original quota receipt is consumed after the actual frame free.

## Native fixture and observed boundaries

The explicit `quota-lifecycle-test` guest payload uses the real authenticated
SessionD shell launch. Its surviving peer spawns two ordinary, inherited members,
each allocating a fresh backing, and maps both before either owner is released.
The shared control backing is allocated by the managed shell. No raw bearer or
grant token is logged.

| Boundary | Cleanup-first backing | Retirement-first backing | Session charge |
| --- | --- | --- | --- |
| Both owners and peer ready | Owner + peer views | Owner + peer views | 12288 |
| First owner exits and is fully reaped | Peer view survives | Owner waits at release word | 12288 |
| Real SessionD REVOKE | Peer view survives | Owner view + peer view survive | 12288 |
| Retired admission attempts | Denied from surviving member | Owner allocation/launch denied before exit | 12288 |
| Second owner exits and is fully reaped | Peer view survives | Peer view survives | 12288 |
| First final peer release | One free and original refund | Still live and readable | 8192 |
| Second final peer release | Reclaimed | One free and original refund | 4096 |
| Managed shell reaped, control finally released | Reclaimed | Reclaimed | 0 |

Aligned shared-word handshakes and bounded yields release ordinary syscalls;
there are no synchronization sleeps or waits inside kernel locks. The assessor
requires actual termination, reference-commit and reaped receipts on either side
of real session retirement. Both owners are prepared before retirement, but
simultaneous kernel-lock contention is not claimed. SessionD itself always
revokes before stopping its managed shell; a shell-cleanup-first order is not
reachable through that normal path and is not manufactured by the fixture.

The peer reads both backings after both owner cleanups and retirement, tests
duplicate release rejection and allocation/launch denial after zero quota, and
then exits. Runner completion requires the peer's actual kernel reap. Each view
is joined by PID/incarnation, VM generation, root, VA and resource ID to its PTE
retirement and matching request-specific TLB completion before physical free.
Zero-target shootdowns are valid completions; they are not remote-flush proof.
An ACK from a CPU that has switched away is distinguished from an actual flush.

## Qualification boundary

The native runner reruns the six published R5 lifecycle fixtures, authenticated
launch/suspension/exec/inheritance regressions, and both R52 exit/unmap orders.
Missing or contradictory R53 receipts fail the aggregate qualification. Retained
real-serial destructive assessor controls are separate from native evidence.

The original selected scenario also names denied attachment. That boundary is
not established by legacy bearer SHM, so its full original classification remains
UNQUALIFIED even when this focused cleanup subset passes. The original matrix
remains 15 PASS, 3 UNQUALIFIED and 10 NOT_RUN. Restricted attachment, identity
reuse/forgery, internal cancellation/attach/revoke contention and same-frame reuse
remain separate gates. Test-owned backing closure does not establish whole-system
physical-frame or full-session memory attribution; no fixed display offset is
subtracted. The evidence report records final native and host results and hashes.
