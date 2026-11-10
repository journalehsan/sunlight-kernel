# M5.3R5 lifecycle evidence status

Production FastLane, production restricted-channel admission and hostile-process
zero-copy remain disabled. Quota limits remain 512 MiB per process and 1 GiB per
authenticated session. M5.3R5 is **PARTIAL**, not a production admission milestone.

The fresh evidence root is `.fastlane-evidence/m53r5-20261109T204407Z/`.
Its `REPORT.md`, `validation-summary.json`, and `original-28-results.json` are the
qualification record. Earlier attempts remain in place with their original
results. Successful boots and host tests do not qualify native lifecycle races.

The debug-only `quota_lifecycle_test` feature adds passive resource-bound receipts
at reservation, publication, unmap, process termination/reaping, exec replacement,
TLB completion, physical PMM free and final receipt consumption. It provides
read-only Account observations through the existing debug syscall. It neither
selects a cleanup routine nor grants allocation/release authority. Release builds
reject the feature. The `restricted_remote_test` feature additionally permits an
existing creator to request ordinary restricted revocation only after the exact
consumer VM is actually Running on a distinct CPU and appears in its active mask.
The NMI/IPI hook records sequence observations using atomics without logging or
locking. Production synchronization and authority checks remain in use.

Native clients use the R4.1 authenticated SessionD launch, ordinary child spawn,
aligned shared control words, bounded scheduler yields and real lifecycle syscalls.
Normal exit, SIGKILL, genuine `ud2` termination, exec replacement and final child
reaping are distinct evidence. A surviving mapped peer keeps the original payer's
charge through owner death and exec. Final PMM free precedes one receipt refund.
Test-owned resource closure, session accounting closure and system-wide PMM
observations are reported separately. Global zero drift is not established.

The legacy SHM API intentionally preserves transferable bearer capabilities across
owner exit and exec. Owner death by itself does not retire a surviving backing or
invalidate all bearer capabilities. A first experimental assertion assuming the
opposite is retained as a failed attempt, with its diagnosis. Restricted channel
VM-bound authorization is a separate boundary. Session retirement, retained peers
and denial of new admission still need their own coordinated proof.

The original 28 identifiers are preserved from the R2 lifecycle matrix. Sequential
handle rejection does not qualify a concurrent double-release race; observing a
new VM generation does not qualify independently forged/stale VM access. Internal
cancellation/completion, construction rollback during death, and competing final
attach/retirement winner orders remain unqualified. No host model or compiler
ownership control is substituted for a missing native operation.

Shared-service delegated charging remains unresolved scope. No operation is added
that selects a foreign payer or silently migrates an Account charge to system
ownership. A general shared-service redesign is outside this milestone.

The next milestone should coordinate bounded competing native attempts at the
existing lock boundaries, execute both reachable winner orders, add authenticated
session retirement with a surviving peer, and exercise supported native stale
identity boundaries. The current PID allocator does not offer numeric reuse as a
routine fixture operation; do not fabricate reuse or weaken the production API.

The final scheduled subset passes on 1/2/4/12 CPUs: 14 original identities are
qualified, two concurrent identities have sequential controls only, and 12 are
NOT_RUN. Each lifecycle run closes 13 tracked backing frames, 26 views and all
tested Account charges. Whole-system free frames decrease by 1006 and page-table
frames increase by six; attribution and global closure remain unqualified.

The remote fixture observes exact request-specific ACK-before-free with a real
CPU-1 consumer. Fixed CPU-0 placement is not established because the existing
`fastlane_coord_diag` test feature overrides its affinity mask. Actual initiators
are recorded separately. No same-remote-frame reuse receipt is captured, so full
remote TLB closure and historical causal resolution remain unqualified. The
original combined FAIL is preserved; standalone M2 assessment receives its own
interval because earlier intentional R5 invalid-opcode crashes are outside M2.

## M5.3R5.2 deterministic exit/unmap qualification

The focused continuation selects the original
`concurrent/exit-unmap-with-backing` scenario. The native fixture places an
independently authenticated owner and its mapped peer at shared-word release
boundaries. Kernel Account snapshots record both ready identities. Fresh backing
identities are used for exit-first and peer-unmap-first runs; bounded reaping
confirms owner cleanup before the second release in the exit-first run.

A passive `reference-committed` receipt follows the existing broker reference
count decrement while the existing locks remain held. Its resource, process and
remaining reference count identify the actual retirement winner. The existing
`termination-committed` event identifies process exit independently. No production
linearization point or synchronization rule changes. The new receipt is compiled
only with debug assertions and `quota_lifecycle_test`; the existing release-build
feature rejection remains in force. The user fixture is selected explicitly by
`quota-lifecycle-test`, including for optimized guest service payloads.

Evidence and final qualification status live in
`.fastlane-evidence/m53r52-20261109T231507Z/REPORT.md` and
`validation-summary.json`. This focused scenario does not qualify close/completion,
cancellation/completion, attach/revoke, native private identity misuse, session
retirement ordering, or same-frame reuse after remote TLB retirement. Production
FastLane and restricted admission remain disabled.

## M5.3R5.3 authenticated session retirement / owner cleanup

The focused continuation qualifies both independent backing-owner cleanup orders
around real SessionD retirement, with a surviving mapped peer. The existing
registry/ledger separation, original payer, normal PTE/TLB/PMM retirement and
post-retirement admission checks are unchanged. Details and authority limitations:
`fastlane-m53r53-session-retirement.md`; qualification record:
`.fastlane-evidence/m53r53-20261109T233500Z/REPORT.md`.

The complete original `concurrent/session-termination-peer-retention` case remains
UNQUALIFIED because its denied-attachment boundary is not proven by legacy bearer
SHM. This focused subset does not advance production admission or the original
28-case counts. No simultaneous lock-held contention or forced identity reuse is
claimed. Final-source regressions must pass independently of the R52 archive.

## M5.3R5.4 restricted retirement diagnostic

An opt-in native diagnostic now exercises actual restricted proposal/consent,
role attachment, protected peer mapping and normal reclamation around SessionD
retirement. Its implementation and qualification boundary are documented in
`fastlane-m53r54-restricted-retirement.md`. Restricted channels currently drain
all views eagerly on session or participant retirement. This differs from the
retained bearer backing used by R53. Diagnostic execution cannot qualify the
required retained restricted lifetime or advance the original 28-case counts.
See `.fastlane-evidence/m53r54-20261110T073847Z/REPORT.md` for actual native results.

## M5.3R5.5 restricted lifetime clarification

The contract in `fastlane-m53r55-restricted-lifetime.md` separates authentication,
mapping and physical lifetimes. Restricted channels forcibly withdraw mappings;
backing is reclaimable after the last actual view and its exact TLB invalidation
retire, even if process cleanup is still pending. Legacy bearer peer retention
remains a separate policy. The R54 retained-restricted expectation is a fixture
mismatch; it is not a demonstrated live-mapping use-after-free.

The new opt-in isolated-guest fixtures select one restricted ordering per real
Account login. Cleanup-first uses a fully reaped dedicated peer and ordinary IPC
coordination; retirement-first releases held cleanup after forced revocation.
Post-retirement stale session/channel checks use real kernel validation. The R53
strict default oracle remains intact; an explicitly selected extra-backing topology
requires independent restricted lifetime proof before accounting for its 4 KiB.

M5.3R5.5 remains PARTIAL because same-kernel distinct SessionRef turnover and an
already pending retirement request held before commit are not established.
Original scenario definitions and 15/3/10 counts remain unchanged. Exact-source
results: `.fastlane-evidence/m53r55-20261110T080804Z/validation-summary.json`.
Production FastLane and restricted admission remain disabled.
