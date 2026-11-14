# AF_UNIX backing-handle replacement investigation, 2026-11-14

Finding: **B — reachable state transition with correct identity preservation**
for ordinary unconnected AF_UNIX pathname sockets. No production fix is
justified by the measured readiness/ownership behavior. Final execution
matrix is recorded separately in summary/acceptance files below.

## Starting state and baseline verification

HEAD `5b08147001be963e468711d03284b349fa8b2089`, branch main. `head.txt`,
`branch.txt`, `status.txt` (full untracked listing), `incoming.txt` (binary
tracked diff), and `configuration.txt` were captured before editing code.
`production-before.txt` records both actual native archives at
2fb382ef6c2b83b48e034645a078d5e36e40f036d98b00a00eace35d222696c7 and the
ordinary debug kernel at
97d9bb2204549ec7548b14fc6d2c90af0d3986dfdd5292aa5eec8ba21e931594.
`starting-hashes.txt` initially searched a nonexistent libc archive name and
hashed the release kernel instead; the correct production paths were captured
before fixture edits in production-before.txt. Release and debug kernels are
not interchangeable evidence.

The three requested reports/contracts were read, along with the native TCP LT
contract, Helios coverage table, unix_socket module contract, descriptor,
syscall, epoll and pipe implementation and native socket wrapper. Historical
reports were not edited. `historical-independent-audit.json` independently
reads actual stdout and results.json: production wrapper 406 assertions plus
2 summaries, Linux handler 413 plus 2, Linux actor 44 plus 1, raw replacement
50 plus 1, on each CPU configuration; both standard suites have 94 passing
rows. The initial audit counted one raw completion as an assertion; the final
JSON explicitly separates both completion types. This is historical evidence,
not fresh execution. Matching hashes independently verify the reported
production artifact baseline; they do not turn old acceptance into new runs.

## Complete replace_handle inventory and identity table

Repository Rust search finds exactly one definition and one call:

| Location | Purpose | Identity effect |
| --- | --- | --- |
| kernel/src/process/fd_table.rs::FdTable::replace_handle | validates open slot, releases displaced backend, installs handle/rights | preserves flags, offset and shared description; does not allocate a description |
| kernel/src/arch/x86_64/syscall.rs::sys_linux_connect | installs allocated pipe pair endpoint 0 | same logical unconnected socket becomes connected; backing representation changes |

No placeholder-initialization, genuinely new logical-object replacement or
other ownership-transfer caller exists. `dup2`, `dup3`, `install_at` and
`install_from` are different operations and do not call this method.

| Layer | Before connect | After connect | Registration dependency |
| --- | --- | --- | --- |
| Descriptor slot | process table fd | same fd | local interest key |
| Shared description | creation description, status flags | unchanged | flags/description retained; not an INET identity |
| Logical endpoint | caller's unconnected socket | caller's connected socket | interest intentionally follows state |
| Backing handle | UNIX_FLAG + Unix pool index | SOCKET_FLAG + pair index/endpoint 0 | looked up live at collection, not saved in interest |
| Registration | token, mask, 64-bit data, edge/oneshot state | same registration | no external transport observation for local handles |
| Peer owner | none | queue owns pair endpoint 1, then accepted descriptor | independent stream endpoint |

Unix pool indices and pipe pair indices are recyclable storage indices, not
monotonic logical endpoint generations. No generation/token changes during
connect; none is required for this fd-keyed local lookup. Native TCP's optional
external identity/generation validation is a separate branch of collection.
Close and real dup replacement detach registrations before reuse. Adding
TCP-style detach here would make the pre-connect interest disappear and would
fail the new registration-follows-logical-socket assertion.

## Transitions, observability, locking and ownership

1. socket creates a Unix entry with refs=1; fd open consumes it. Open failure
   releases it. bind stores a path; listen publishes it without a handle swap.
2. connect copies/validates sockaddr, takes SCHEDULER, validates the source fd,
   finds a listener, allocates pair refs=[1,1] and enqueues endpoint 1.
3. replace_handle validates the live fd again, releases its old Unix reference
   via release_endpoint_socket and assigns pair endpoint 0 without retaining
   it. It preserves the shared description and flags. This is the backing
   swap; observers protected by SCHEDULER see it after unlock. Queue polling
   via syscall also needs SCHEDULER, so it cannot collect the interim state.
4. accept dequeues endpoint 1 and installs it in a fresh descriptor without
   retaining again. Listener close drains queued endpoint-1 references.
   Stream close consumes the descriptor and releases its endpoint/ring refs.
5. Missing listener allocates no pair. Queue rejection closes both newly
   allocated endpoints, keeping the original source. Already-connected retry
   returns EISCONN before allocation. Replacement failure would withdraw and
   close the pair and release the Unix index, but is unreachable after source
   validation while SCHEDULER is continuously held: close/reuse/dup cannot
   change the source slot. No public fault injection is added for that branch.

Order: SCHEDULER -> short Unix/socketpair/pipe pool critical sections and epoll
pool where applicable. Unix release unlocks its pool before draining pairs;
socketpair close unlocks its pool before closing rings. No blocking IPC occurs
in connect or replace_handle. Binding/listening resolve their handle and drop
SCHEDULER before pool mutation; general races in those other operations are
not proved atomic here. Concurrent source close either precedes connect's
lookup or follows its completed swap; it cannot target a different incarnation
inside this critical section. No claim is made about already-resolved I/O.

## Reachability matrix

| Operation | Native public C | Native raw dispatcher actor | Linux-personality actor |
| --- | --- | --- | --- |
| pathname socket/bind/listen/connect/accept | socket creation wrapper supports AF_INET, not AF_UNIX pathname creation | 1050/1052/1053/1051/1054 | 41/49/50/42/43 translated to same handlers |
| pre-transition poll/epoll | existing local-fd wrappers; creation limitation above | existing Linux poll/epoll handlers 1006/1020-22, packed ABI | 7/291/233/232, packed ABI |
| pre-connect register then post-connect data | no new C export | executed | executed |
| close/reuse, rejected/repeated connect | consuming close plus same handlers | executed | executed |
| source slot changes inside replacement | impossible under SCHEDULER | unreachable | unreachable |

An ordinary unconnected socket has **zero readiness**, not POLLOUT. Therefore
"make client readable before connect, collect afterward" is not reachable for
that documented state. A listener has pending-accept IN, not stream bytes;
fixture tests listener registration through bind/listen/queue/accept separately.
Connected streams support writable readiness, traffic, buffered bytes plus HUP,
and EOF. A second connect on a connected stream is rejected. No test treats
these three states as interchangeable. Poll has no persistent registration;
epoll does. Event collection recomputes readiness instead of carrying a queued
snapshot of an obsolete Unix backing handle.

## Executable evidence and failure preservation

`tools/native-c/tests/unix-replacement.c` uses real production syscall handlers,
with native-dispatch and Linux-personality builds. No production adapter or
export is added. Both are static freestanding -nostdlib ELFs linked with
services/user-space.ld, no host or native libc startup. Linux OSABI byte 3
selects real translation; native OSABI 0 uses existing dispatcher numbers.
The guest runner performs ordinary locked isolated kernel builds. Maps, ELF
hashes, per-run kernels, ISOs, build and QEMU logs are retained.

Initial run: `run.py`, root linux/native-{2,4}, `baseline-unix-replacement.c`.
Expanded coverage and final runner are separate output directories. Initial
coverage has 200 individual checks plus one completion per actor/CPU. It tests
eight lifecycles without sleeps; all waits are zero-timeout, sockets are
nonblocking, data writes/accept/close establish deterministic ordering and
watchdogs bound guest and host. Expanded coverage adds listener interests,
flags preservation, per-round distinct data markers and a full 128-entry queue
with four allocation/enqueue rejections, successful retry on the same socket,
and EOF for every queued peer after listener close.

Cleanup observations are actual peer EOF/HUP, repeated pathname binding,
explicit successful closes, fd-number reuse and guest clean=1/fds=0 reaping.
These are not a live Unix pool allocation counter. There is no exported Unix
pool census, so exact global Unix allocation/release balance is NOT_RUN;
code ownership tracing plus observable EOF does not prove a numeric census.
Local pipe cleanup is synchronous; TCP regression cleanup is asynchronous and
uses the existing bounded reclamation observations unchanged.

Read-only exploratory searches included nonexistent docs/helios and a socket.c
candidate; corrected to docs/Helios_LinuxCompat and actual suite sources. No
files were removed. Any fixture/harness failure remains in its attempt folder;
final classifications never replace or silently erase failed attempts.

## Scope and separate follow-ups

No production behavior changed. No external/pressure acceptance is newly
required by a production dependency change. Matching historical external and
pressure acceptance remains in the prior replacement closure, not fresh here.
No Linux OFD lifetime, dup/fork/exec attachment transfer, exec CLOEXEC consumption,
TCP personality integration or already-resolved-I/O atomicity is claimed.

Separate findings (not demonstrated stale-readiness defects): connect
does not reject a source Unix handle merely because it is listening. The final
fixture exercises a pending IN event before that replacement, then collects
afterward: the old queue is no longer delivered, its queued peer sees EOF,
and traffic from the new accepted peer arrives with the retained data marker.
This verifies current backing lookup even across that reachable transition;
it does not endorse accepting a listening source as Linux-compatible. Aliases
of an unconnected Unix handle can retain an older backing representation while
one fd connects. General alias transition semantics and connect-on-listener
validation deserve independent investigation; this task does not redefine
those contracts or manufacture a failure with invalidation. Bind/listen
resolve-then-mutate race behavior is likewise not validated by these serial
state transition fixtures. ET/oneshot transition-specific coverage is NOT_RUN;
the existing lifecycle regressions still cover their supported local behavior.

## Native public API coverage

`unix-replacement-public.c`, `run-public.py` and `public/` additionally execute
production C poll/epoll/read/write/close wrappers against AF_UNIX descriptors
created through the existing raw pathname dispatcher. This tests the native
16-byte aligned epoll ABI and real native bridge fallback without adding a
public socket creation operation or C export. Eight cycles cover pre-connect
ADD, connected poll, readiness/data, repeated-connect rejection, MOD/DEL/re-ADD,
buffered HUP/EOF, close and fresh registration on explicit fd reuse. The
ordinary cc.py link uses native startup, -nostdlib and the exact staged native
archive. `public.map`, `public-elf.txt` and empty `public-undefined.txt` retain
link provenance. This is separate from the freestanding raw actors and adds
zero standard suite rows.

Expanded attempt: Linux actor passes; native actor fails exactly eight flag
assertions because the test mapped Linux fcntl(72) to native fstat(48) instead
of native fcntl(49). `expanded-fixture.c`, expanded maps/ELFs/stdout/builds and
logs are preserved. Corrected the test selector only; no production change.
`validated/fixture.c` is the final actor source and adds the listening-source
observation. No functional or cleanup assertion was weakened.

## Additional exclusions from scheduler review

`wake_linux_unix_socket` selects blocked pollers whose shared table contains
the listener Unix index. Already-blocked poll/epoll waiting on a source while
a different thread connects it to a listener in another process is not
executed here; source-only connect wake selection is a separate code-review
lead, not a reproduced stale identity or ownership finding. These fixtures
prove readiness collection after synchronous transition, not every blocking
wakeup interleaving. No scheduler changes were made.

## Final independently evaluated results

`acceptance-summary.json` is authoritative for per-assertion and per-row
results; `summary-validation-2.log` is PASS. It checks stdout, completion
records, JSON pass_, every CSV build/run/acceptance cell, kernel reaping,
INET before/after records, production hashes and unchanged incoming tracked
diffs. No accepted command timed out. Exact watchdog commands/exits are in
root, expanded, validated, public and regression commands.json files.

| Fresh execution | Build | Link | Runtime 2 CPUs | Runtime 4 CPUs | Assertions / suite rows |
| --- | --- | --- | --- | --- | --- |
| Initial raw Linux + native actors | PASS | PASS | PASS | PASS | 200 each actor, plus completion |
| Expanded Linux actor | PASS | PASS | PASS | PASS | retained intermediate coverage |
| Expanded native actor (test selector error) | PASS | PASS | FAIL | FAIL | 8 flag assertions + completion FAIL each |
| Final raw Linux actor | PASS | PASS | PASS | PASS | 508/508 each, plus completion |
| Final raw native dispatcher actor | PASS | PASS | PASS | PASS | 508/508 each, plus completion |
| Native production C local-fd APIs | PASS | PASS | PASS | PASS | 136/136 each, plus completion |
| Production Rust dup2 wrapper | PASS | PASS | PASS | PASS | 406/406 each, plus 2 completion records |
| Linux dup3 compatibility handler | PASS | PASS | PASS | PASS | 413/413 each, plus 2 completion records |
| Linux-personality syscall actor | PASS | PASS | PASS | PASS | 44/44 each, plus completion |
| Existing raw dup2 replacement fixture | PASS | PASS | PASS | PASS | 50/50 each, plus completion |
| Local epoll, epoll lifecycle, inet descriptor/close/reuse | PASS | PASS | PASS | PASS | output/exit/resource contracts independently checked |
| TCP LT + revalidation | PASS | PASS | PASS | PASS | existing readiness/identity assertions and cleanup |
| Full native suite | PASS | PASS | PASS | PASS | 94/94 rows each |
| C++ runtime checker + poll ABI/export checker | PASS | PASS | n/a | n/a | runtime C++ also executed in both suites |
| Locked kernel/libc/network cargo checks | PASS | n/a | n/a | n/a | separate logs under regression |
| git diff --check, Python syntax checks | PASS | n/a | n/a | n/a | no whitespace/syntax failure |

New final AF_UNIX focused checks total **1,152 per CPU** plus 3 completion
records, adding **zero standard-suite rows**. Suite rows retain their original
expected exits, including expected nonzero tests. Assertions are not inferred
from exit zero. The first independent summarizer expected only eight failure
lines in the known bad native attempt and omitted its failure completion;
`first-summarize.py`, `first-acceptance-summary.json` and
`summary-validation.log` preserve that failed audit. The corrected evaluator
requires exactly eight flag failures AND the failure completion. It does not
change any guest assertion or acceptance criterion.

## Final provenance, artifacts and changed files

Native build ran through the ordinary locked tools/build_native_c.sh recipe.
All regression fixtures were relinked using the staged production archive;
wrapper object selection/provenance is retained by the existing runner.
Guest-specific kernel and ISO hashes differ with embedded manifests and are
not reported as the ordinary kernel. `production-after.txt` and the independent
summary verify archive **2fb382ef...** in both locations and ordinary debug
kernel **97d9bb22...**, unchanged. Final ELF/maps/guest kernels/ISOs, including
failed attempts, are listed in `all-artifact-hashes.json`; validated/hashes.json
also records final raw source and actor artifacts. Tool versions, target/linker
configuration and reviewed source snapshots/hashes are retained separately.

Only additions were made: tools/native-c/tests/unix-replacement.c (raw actors),
run_unix_replacement.py (bounded reusable runner with individual-result checks),
unix-replacement-public.c (production native API fixture),
docs/networking/2026-11-14_AF_UNIX_REPLACEMENT_IDENTITY.md (current contract),
and this evidence directory. No incoming tracked file changed; independent
binary diff comparison proves exact preservation. No production fix, new
production export, dependency change, global registration clearing or lock
change. No commit/reset/clean/stash/branch switch. Historical reports, archives,
logs, incoming fixtures and all attempts remain in place. `final-head.txt`,
`final-branch.txt`, `final-status.txt` and preservation checks retain final state.
Fresh external/pressure acceptance: **NOT_RUN**; matching historical acceptance
is explicitly distinguished above. Numeric Unix pool census and concurrent
blocking-connect wake selection remain NOT_RUN as described, rather than
being inferred from generic clean=1 or matching hashes.

The fresh C++ checker explicitly reports the original bare command as expected
LINK_FAIL and the configured historical native ELF as linked; both guest
suites provide runtime coverage. This expected negative link is separate from
the two native flag-selector runtime failures. Its logs are under
regression/cxx/ and regression/cxx-checker.log (there is no cxx/summary.json;
an exploratory search for that candidate failed without modification).
