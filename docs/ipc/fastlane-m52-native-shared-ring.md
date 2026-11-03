# M5.2 gated native shared-ring data path

2026-11-03. **Experimental functionality COMPLETE for the archived, validated
source snapshot.** All 51 final-source native guests PASS on 2/4/12 CPUs;
production/release exclusion PASS. Overall production readiness remains PARTIAL
and production FastLane remains **DISABLED**. No benchmark, optimization, general-purpose IPC replacement,
large-message transfer, MPMC support, commit or push is included.

## Implementation and authority

`fastlane_shared_ring_test` (kernel) and `fastlane-shared-ring-test` (Account and
its native fixture) are opt-in descendants of the existing debug-only M1/M2/M4
features. `FL-M52` starts independent native producer and consumer processes.
Each independently makes the ordinary Account request. M1 still binds the exact
PID/incarnation/VM, issuer, active session and epoch, one-use pairing receipt,
channel token/generation, immutable role and one-use attachment. There is no
kernel-installed identity or userspace handle that acts as bearer authority.

M2 maps three independently authenticated, initialized 4096-byte pages per role:

| Page | Producer | Consumer |
|---|---|---|
| Producer cursor and fixed descriptors | RW, NX | RO, NX |
| Consumer acknowledgement cursor | RO, NX | RW, NX |
| Sixteen 256-byte payload slots | RW, NX | RO, NX |

Kernel operation 24 activates each exact role once, only on a fully attached,
live, exclusive native VM with its three authenticated views. It rejects stale,
foreign, wrong-role and duplicate requests, and cannot be combined with a used
M5 channel or enabled M5.1 channel. Once either role activates, the kernel denies
the entire M5/M5.1 word/control backend on that channel. No shared metadata grants
kernel authority. M1 attachment, M2 mapping rights and M4 notification admission
remain separate required checks.

The userspace `shared_ring::Native` backend is private to this experimental
fixture. It uses the **separate hostile-safe `hostile::Producer/Consumer` adapter**,
not cooperative `Ring` or reference-based leases. Reservation, bounded staging,
32-word slot write/clear, descriptor publication, cursor publication, initialized
private-copy acquisition, acknowledgement, cancellation and slot reuse all execute
in userspace. Kernel word/control calls do not transfer any M5.2 payload bytes.
Control-plane syscalls, M4 observation/notification/waiting, ordinary fixture IPC
and diagnostic logging remain. This is a functional data path, not a claim of
zero syscalls per message or a public production transport API.

Fixed geometry is local: sixteen slots, maximum length 256, three pages, 512
aligned words per page and five descriptor words per slot. Shared length, offset,
channel generation, sequence, slot generation and cursors are validated. No
peer-provided pointer, allocation length or layout chooses a native address.
Every slot address comes from checked local geometry; payload storage starts
initialized, is copied in bounded words, and its unexposed suffix is cleared.

## Memory ordering and hostile-peer contract

An aligned raw x86-64 MOV loads/stores one word atomically, without constructing
Rust references to hostile memory. Compiler barriers plus x86 TSO implement
Acquire loads and Release stores. For cooperative peers, payload and descriptor
stores precede publication; observing publication precedes copying; finishing
the private copy precedes acknowledgement; observing acknowledgement precedes
reuse. These ordering guarantees do **not** freeze a malicious producer's alias.

| Property | Contract |
|---|---|
| Memory safety | PROVIDED |
| Stable private snapshots | PROVIDED |
| Coherent publication | NOT PROVIDED |
| Application authenticity | NOT PROVIDED |

A producer can overwrite writable payload or descriptor words after publication.
The native mutation fixture pauses after the first loaded word, lets the actual
producer mutate the backing, completes a bounded mixed private copy, then mutates
the backing again while the private lease remains held. The private bytes stay
stable. Acquire/Release, sequence numbers and valid metadata do not establish
coherence or application authenticity; the API still reports
`Integrity::PRIVATE_COPY`. Applications must validate the private output.

## Native mapping lifetime and retirement

Direct access has an explicit terminal-fault contract. M2 withdrawal removes the
PTE, completes required remote TLB acknowledgements, then permits physical frame
reuse. A MOV racing withdrawal either accesses the still-authorized old frame
before that fence or faults in the actor after withdrawal. It never accesses a
retired/reallocated frame through an unacknowledged old translation. There is no
Rust reference, pointer-based drop action, kernel-held payload borrow, operation
pin, or requirement for an exited process to release a lease.

The fixture has a single actor thread/VM per role. M1's live-membership sharing
exclusion and M2's replacement/protection/unmap restrictions are unchanged.
Successful mappings use the existing checked monotone per-VM VA bump; later
channels do not adopt an old view's address. New backing is sanitized. A fresh
mapping/setup replaces the private endpoint only between completed commands;
there is no reconnect/reset operation on a live endpoint and no pointer carried
across exec. The private backend must not be exported as an arbitrary safe
pointer constructor. Revocation during copying/publication can terminate the
actor with SIGSEGV; graceful per-operation error recovery is not promised.

Kernel retirement does not trust shared slot state or wait for a hostile peer to
acknowledge. It fences authority, wakes/clears M4 waiters and withdraws all views.
Abandoned private reservations/copies disappear with their process. Outstanding
published slots cannot delay retirement or confer rights on a later generation.

`native-acceptance.json` joins each boot, generation, authenticated role,
publication/ack count, abandoned reservation, outstanding publication count,
backing object, mapping, remote TLB request/ACK/completion and physical release.
It additionally checks strictly increasing mapping VAs within each exact VM,
physical reuse events after the corresponding exactly-once free, and flat free,
SHM and page-table frame counts across all four normal channel cycles. Those
steady-cycle counts do not claim whole-system reconciliation across actor
respawns or unrelated services. The M3.1B
`free_shared_page` repair and all memory/TLB/address-space code are byte-identical
to the accepted M5.1R2 baseline.

## Notifications, backpressure and bounded terminal outcomes

Readers observe M4's generation-qualified sequence **before** calling the
hostile-safe consumer adapter. Only its `Error::Empty` result can register a wait;
its existing private monotonic observation, generation validation and checked
exhaustion decide every acquisition. A separate raw-head precheck is forbidden:
it can mistake rollback to the current cursor, or exhaustion, for an empty queue.
An empty read registers against that observation. A notifier
winning registration changes the sequence and the waiter returns `Changed`;
a registration winner owns a completion that is collected exactly once. Every
wake rechecks publication and hostile metadata. Notifications are hints, never
message counts or ownership. Repeated notifications before waiting, a genuine
spurious wake with no publication, both forced registration orders, and both
actual consumer-exit/notify lock orders run in the native ring fixture. The
unchanged M4 hooks require real failed lock acquisition on independent CPUs.

Sixteen outstanding slots return explicit `Full`; the controller schedules the
consumer, then the producer resumes using acknowledged slots. Producer-faster
and consumer-faster cases use the same data plane. There is no reverse Doorbell
or new producer waiter: producers may return `Full` to their caller. Replayed
identical cursor stores create neither extra messages nor capacity credit;
rollback and invented acknowledgement credit have bounded terminal outcomes.

Wait cancellation, closure wakeup and notification after retirement are tested.
Revocation during private copying, closure before acknowledgement and cancellation
mid-publication cause three expected, identity-bound hardware faults, each reaped
with status 139. Killing the consumer during copying is reaped with status 137;
the retained ordinary IPC late reply is discarded, not treated as a ring ack.
Producer exit with an outstanding slot retires the channel and the consumer gets
a terminal control-plane result before reading withdrawn memory. Retries of
hostile metadata are bounded; no snapshot-consistency retry loop exists.

## Native fixture coverage and measurements

Every accepted guest executes one, sixteen, one thousand and ten thousand
ordered messages, sizes 0 through 256, repeated slot reuse, full backpressure,
dropped unpublished reservations, foreign/wrong-role/stale/duplicate admission,
malformed length/offset/channel/slot generation, rollback disguised as an empty
queue (including persistent rejection after metadata restoration), duplicate publication and ack
cursors, invalid shared cursors, producer/consumer exits, cancellation, revocation,
private-copy mutation and all four forced M4 races described above.

Fresh, authenticated channels additionally seed **test-only** local/shared cursors
at `2^32 - 1` and `u64::MAX - 2`. Real native sends/receives cross the 32-bit
boundary and reach the last representable cursor. Subsequent producer and
consumer operations fail with checked exhaustion, including the ordinary
receive command rather than only a boundary-check helper. This is isolated seeded native
arithmetic evidence, not a claim of naturally transferring 2^64 messages or
allowing sequence wrap. The `fixture` Cargo feature exposes safe constructors for
this arithmetic fixture only; normal construction still starts at zero, and these
constructors grant no native mapping or role authority.

| CPUs | Publications | Private-copy acknowledgements | Full events | Channels | Backing objects / views | Remote / zero-target unmaps |
|---:|---:|---:|---:|---:|---:|---:|
| 2 | 11,244 | 11,219 | 688 | 27 | 81 / 162 | 18 / 144 |
| 4 | 11,244 | 11,219 | 688 | 27 | 81 / 162 | 24 / 138 |
| 12 | 11,244 | 11,219 | 688 | 27 | 81 / 162 | 24 / 138 |

The deliberate difference between publications and acknowledgements belongs to
identified terminal/adversarial cases, not lost normal messages. Per-channel
joins preserve the exact outstanding state. All normal count/order assertions
pass in each native process, independently of host parser checks.

Twenty-five bounded kernel samples per guest record system-wide syscall entries,
attempted mediated word calls, notification calls and actual scheduler context
switch counters for the selected scheduler mode. They are cumulative system
observations, include unrelated services and fixture logging, and do not cover
the final producer-exit lifecycle after the last sample. They are not actor-specific
or benchmark counters. The first four cleanup samples have precisely 2/4/6/8
mediated word attempts: one deliberate denial per role/channel and **no per-message
payload word calls**, despite increasing message counts. Exact notification call
samples and syscall/context-switch counters are retained in the native receipt.

Each backing allocation is counted by its real trace (81 per guest). The data
plane allocates no heap storage: fixed private arrays/leases and mapped pages
supply its storage. `data_plane_allocations=0` is that structural observation;
it does not count diagnostic formatting, IPC or system allocator activity.
Twenty-five cleanup receipts report elapsed guest monotonic milliseconds for
revoke plus resource reconciliation. These are correctness diagnostics only;
no throughput, latency comparison or tuning is claimed.

## Evidence and regression assessment

Evidence is under `.fastlane-evidence/m52/`, outside Cargo target. **Final source**
is under `v3/`: native 2 CPUs at
`v3/matrix-20261103T084121.535404Z/m52/native-20261103T084121.659071Z/`, and 4/12 at
`v3/matrix-20261103T084121.535404Z/m52/native-20261103T084206.157131Z/`.
`v3/native-acceptance.json` independently reassesses all original serials and joins
retirement/outstanding state. The image build is
`v3/build-20261103T083821.215588Z/validation-summary.json`. Every native receipt
retains full source maps and copies, ELF/ISO/service identities, command, topology,
serial hash and assessment. Generations/PIDs are scoped to their original boot.
The 1,374-file final source map has canonical SHA-256
`ec10f529dfbd21e627b59e65f8e0cfed8bc2dd20b6530269f38e71f4d205c7f4`.

The final 2/4/12 regression matrix and production checks **PASS**. The original
parent receipt is `v3/matrix-20261103T084121.535404Z/matrix.json`; the original
broader receipt is `v3/broad/regressions-20261103T085708.509456Z/orchestration.json`.
`v3/exact-native-matrix.json` joins all **51 distinct passing guests**, seventeen
on each topology, to original classifications, manifest/serial hashes, exact
commands and immutable images. Nested M3/M2 receipts are one guest, not two.
No earlier-source pass substitutes for a final-source guest.

| Native gate | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| M5.2 authenticated shared ring | PASS | PASS | PASS |
| M5.1 repeated mediated transfer | PASS | PASS | PASS |
| M5 single-message mediated transfer | PASS | PASS | PASS |
| M4 authenticated Doorbell | PASS | PASS | PASS |
| M1 authority and bounded coordination | PASS | PASS | PASS |
| M1/M2/M3 mapping, authority and word probes | PASS | PASS | PASS |
| M3.1B reclamation, eight crossings | PASS | PASS | PASS |
| IPC deferred/reply, VM and SHM retirement | PASS | PASS | PASS |
| Authenticated authority S1/S2 | PASS 2/2 | PASS 2/2 | PASS 2/2 |
| Successful/failed exec, three cycles | PASS | PASS | PASS |
| Actual exit modes 11/12/13, three cycles each | PASS 3/3 | PASS 3/3 | PASS 3/3 |
| Scheduler lifecycle | PASS | PASS | PASS |
| R2D remote wake and physical handoff | PASS | PASS | PASS |
| Existing kernel-native Doorbell | PASS | PASS | PASS |

All three legacy direct runners additionally have contemporaneous equality
attestations between the just-built default Cargo ELF, archived ELF and
ISO-extracted ELF (`v3/audit/direct-images/audit.json`). The parent explicitly
unsets `CARGO_TARGET_DIR`; this prevents the documented earlier image-selection
error. Their original schemas remain unchanged.

Ordinary debug features are empty; its extracted ISO ELF matches its archived
ELF. Debug symbol inspection excludes the new control/diagnostic/fixture helpers.
Both ordinary debug and full release ELFs exclude experimental authority,
mapping, transfer, Doorbell and M5.2 markers. The full release ELF contains the
exact normal embedded-service bytes. Its SHA-256 is
`f16d0148f94cca3e300ff60275836a73965a071ebde44829939ae6950b08ac67`.
The explicit experimental release check returns 101 with the existing debug-only
compile error; that original rejection is retained, never called a passing build.
Stripped release symbol inspection remains NOT_RUN; marker and exact embedded
service byte checks provide the recorded release exclusion evidence.

Host checks pass: kernel 157, ring 39 plus 5 doctests, Doorbell 9, IPC 62 and
Account 18 tests; scheduler/boundary checks and 12 M5.2 retained-serial parser
controls pass. M3 ASan and TSan each pass 27 hostile tests outside the sandbox.
Miri remains **UNAVAILABLE / NOT_RUN**. Parser mutation/replay tests are validator
controls, not additional native guests.

Retained development failures include the initial missing assessor import,
sandbox QMP socket denial (no booted guest), a mistaken image-manifest path
(no booted guest), and the expanded native run's unused foreign actor reaching
its ordinary ten-second command deadline. The latter is a real failed fixture
receipt, not converted into a pass; foreign actors now exit immediately after
their authority checks. The first sandbox sanitizer attempt runs all 27 ASan
checks successfully but LeakSanitizer terminates under ptrace; its original FAIL
remains alongside the passing unsandboxed attempt. Earlier source builds and
passing development guests remain distinct from frozen final-source acceptance.

### Retained pre-final M2 remote-flush coverage failure

The broader mapping-only 4-CPU guest
`broad/regressions-20261103T074453.523161Z/compatibility-4/native-20261103T074600.427024Z/mapping/native-20261103T074600.546233Z/`
remains **FAIL** with the sole error `required still-running remote VM was not
flushed`. Its designated generation 28 records two zero-target TLB requests,
no remote ACK/flush in that case, the expected subsequent stale-access fault,
and exactly-once retirement. Restricted resources are zero and SHM/tables are
flat at checkpoints 100–105. All six real M3 word witnesses pass when assessed
independently. This guest supplies no required remote-flush coverage for that
case; a zero-target receipt is never promoted to a remote flush.

One bounded replay uses the **identical ELF/ISO/services/features** on 4 CPUs:
`m2-remote-replay/native-20261103T075022.238344Z/`. It passes the unchanged M1/M2/M3
assessors with a real remote flush. `audit/m2-remote-coverage.json` joins both
original serials, exact image identity and TLB records. No source, timeout,
placement policy or assessor requirement changed. The cause of the original
coverage outcome remains UNRESOLVED; no missing nonzero-target ACK or premature
release is reported by its other checks. The replay supplies separate passing
coverage and **does not discharge or relabel the original FAIL**. The original
broad and parent orchestration results must retain their failure independently
of accepted coverage. This is current evidence, separate from historical census
drift and M4 failures; no historical causal investigation was reopened.

### Receive review and preserved intermediate validation

The first implementation used a separate raw-head availability check. Final
review identified that it could hide a rollback to the current consumer cursor
and could park at cursor exhaustion before calling the adapter. The intermediate
`v2/` implementation added duplicate local head tracking; its completed native
results remain retained and its orchestration was deliberately ABORTED before
further validation. The final `v3/` implementation removes the precheck entirely
and uses the existing `Consumer::try_read` for all decisions. It preserves the
adapter's first terminal error even after shared metadata is restored, and tests
ordinary receive exhaustion. No new unsafe code or kernel change accompanies
this correction. The final-source matrix is independent of both earlier source
maps; their passes are not substituted into it.

The pre-final source has 51 passing native guests and one retained failing M2
coverage guest, with its independent identical-image replay joined separately.
Its original broad and parent orchestration remain FAIL. An earlier production
audit wrapper also retained an incorrect expected-diagnostic literal failure;
the compiler's actual experimental-release rejection and the independent corrected
audit remain available. No original receipt is overwritten.

## Security and unsafe-code audit

`v3/audit/source-audit.json` compares all 1,374 validated executable/configuration
inputs to the source-bound M5.1R2 baseline, retains the sixteen changed/new files
and exact diffs, and verifies fourteen critical files unchanged. In particular,
M1 authority/coordination, M2 mapping, M4 Doorbell, M5.1 admission, interrupt/saved-IF
repair, cooperative ring/leases, hostile tests and memory/TLB/reclamation code
are unchanged. The M5 backend adds only native-mode exclusion. Scheduler code
adds a cfg-gated diagnostic read through its existing guards, without scheduling
policy or queue changes.

The only new unsafe blocks are two bounded aligned MOV blocks in the private
userspace backend. They create no references to shared memory and use no
peer-derived address. The kernel control plane adds no unsafe code or lock.
`hostile.rs` retains `forbid(unsafe_code)`; its additional fixture constructors
use safe checked arithmetic. Three fixed kernel atomics count diagnostics only
in the experimental image. No new global hot-path lock, allocation, token rights,
secret logging or production path is introduced. Existing mapping/VM withdrawal
and remote TLB fences provide lifetime safety; no shared terminal flag substitutes
for those fences.

Historical M1 timeout **attribution remains UNRESOLVED**, despite the accepted
M5.1R2 bounded coordination repair. Historical M4 failures and global census drift
remain **UNRESOLVED**. Fresh passing guests do not reconstruct missing historical
artifacts or upgrade those classifications. Production readiness remains PARTIAL
and production FastLane remains DISABLED.

## Checkout changes after validation

The completed matrix records identical source maps before and after its run.
On resuming the session, the checkout had additional changes outside this task,
including package version/lock updates and interrupt, VM/mapping and scheduler
source edits. These are preserved. `v3/audit/checkout-divergence.json` records
the exact validated and subsequently observed source maps and differences.
The PASS/completion statement applies to the archived source identity above,
**not the later checkout**. The later integration requires its own validation;
this report does not infer that changed VM/interrupt code preserves the proven
retirement guarantees. Original passing receipts are unchanged.

## Next smallest milestone toward M6

First revalidate the later VM/interrupt integration against the same gates; those
changes are outside the archived completion above. Then M5.3 measurement
readiness: retain this fixed protocol and production exclusion,
introduce bounded per-actor diagnostic counters and compact native count/order
receipts that can disable per-message debug printing, define exact warm-up and
measurement windows, and rerun the same safety/retirement gates. Establish an
honest measurement envelope before M6 performance validation; no batching,
throughput optimization or comparative tuning is part of M5.2.
