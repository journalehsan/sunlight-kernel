# Linux dup3 and direct Rust dup2 runtime coverage

Final result: **PASS on 2 and 4 CPUs** for direct production Rust dup2,
the actual Linux dup3 handler with native TCP bridge state, and the Linux
personality syscall-292 actor. Standard suite: **94/94 PASS on each CPU
configuration**. Focused counts are separate from those 94 rows. No new
production defect was reproduced; harness failures below are preserved.

New evidence only. Historical reports, failures, archives and incoming work
are preserved. No production implementation fix was made.

## Starting provenance and independently checked baseline

`starting-head.txt`, `starting-status.txt` (full tracked/untracked status),
`incoming.diff` (full tracked binary diff), `toolchain.txt`,
`config-before.txt`, and `native-hashes-before.txt` were recorded before
editing source. HEAD is `5b08147001be963e468711d03284b349fa8b2089`.
Rust is 1.98.0-nightly, revision 8954863c8 (2026-06-05). The kernel target is
x86_64-unknown-none; native artifacts use tools/targets/x86_64-sunlightos.json
and the pinned staged native std sysroot. `starting-hashes.txt` used two
nonexistent candidate paths and is empty; the correct actual artifact paths
were identified and captured in `native-hashes-before.txt` before edits.

The archive hash in both release and sysroot locations was
`2fb382ef6c2b83b48e034645a078d5e36e40f036d98b00a00eace35d222696c7`.
The ordinary kernel ELF hash was
`97d9bb2204549ec7548b14fc6d2c90af0d3986dfdd5292aa5eec8ba21e931594`.
These agree with the prior evidence, independently recomputed here.

The required three documents, fixture, wrapper, bridge, syscall paths and
native build/runtime infrastructure were inspected. `baseline-audit.json`
independently parses the historical evidence: both standard suites contain
94 rows with 94 pass_=true; both raw replacement logs contain 50 individual
PASS assertions plus one PASS summary; the pre-fix raw run contains the six
reported failures plus its failure summary. Direct wrapper bookkeeping and
Linux dup3 were indeed only code-reviewed there. Native unistd.h and archive
symbols still expose no public C dup-family operation. The separate AF_UNIX
replace_handle lead still exists and has not been changed.

## Call-path / layer coverage matrix

See also `docs/networking/2026-11-13_NATIVE_DUP_RUNTIME_COVERAGE.md`.

| Layer | Raw native dup2 fixture | Linux dup3 handler fixture | Direct Rust dup2 fixture |
| --- | --- | --- | --- |
| Dispatch | syscall 46 | dispatcher 1031, exact sys_linux_dup3 handler; separate Linux actor proves 292 translation | adapter calls production sunlight_libc::dup2 → syscall 46 |
| Descriptor validation | Kernel | Flags/same-fd, then kernel table | Kernel, result mapped to Rust Errno |
| Target registration invalidation | Kernel shared helper | Kernel shared helper | Kernel shared helper |
| Descriptor replacement | FdTable::dup2 | FdTable::dup3 | FdTable::dup2 |
| TCP bridge detach before replacement | Not applicable: bypassed | Not applicable: bypassed | Executed |
| Capture / validate generation | Real C socket attachment + lookup | Real C socket attachment + lookup | Same state observed through actual Rust lookup and generation query |
| Detached attachment release | Deferred to C close | Deferred to C close | Executed on success; forwarding observer counts real release routine |
| Failed-replacement restoration | No bridge mutation to restore | No bridge mutation to restore | Executed for attached targets, and for a naturally stale record |

The Linux-personality actor is a distinct raw ELF (OSABI=3), uses Linux
syscall 292 in RAX, old/new/flags in RDI/RSI/RDX, and Linux packed epoll data.
Kernel translation maps 292 via -32 to dispatcher 1031. This actor directly
executes translation and local descriptor/epoll cases. The TCP handler fixture
is native so that its C socket/read/write operations exercise the native TCP
bridge; it does not claim Linux-personality TCP libc integration.

Linux errors are -9 EBADF and -22 EINVAL. Only 0 and O_CLOEXEC=0x80000 are
accepted; same-fd is EINVAL. FdTable stores the close-on-exec bit per descriptor;
F_GETFD returns FD_CLOEXEC=1, and flags=0 clears it. The base shared fixture
expects native raw BadFd=-5 and dup2 same-fd success: its Linux adapter checks
actual dup3 EINVAL before normalizing this one return for the base fixture and
normalizes -9 to -5. Dedicated checks use unnormalized Linux results. No error
or same-fd behavior is hidden by normalization.

## Actual wrapper harness and proof

There was no existing supported native guest mechanism that directly called
this Rust function. `dup2-wrapper-adapter.rs` is a test-only no_std object
compiled against the exact target-built libc rlib identified from the native
archive's wrapper member. It references the production mangled Rust symbol;
it contains no copied implementation or TCP map. It provides narrowly named
coverage entry points only in this fixture, never in the shipped archive.

The C executable uses cc.py's ordinary native startup, linker script,
-nostdlib and static archive. `driver-expanded.log` captures clang's expanded
commands. `validated/wrapper.map`, `wrapper.elf-symbols.log`,
`adapter.o-symbols.log`, disassembly, ELF headers and empty undefined-symbol
lists prove resolution. `wrapper-proof.json` and `wrapper-proof-excerpts.txt`
show the adapter calling the actual sunlight_libc::dup2, one production
bridge lookup implementation, and the observer forwarding to the real
release_detached routine. No host library path occurs in the link map.

The test-only linker observer increments a count then always calls the real
release routine. It does not suppress a missing symbol, stub success or alter
production behavior. Runtime checks prove C-created sockets are visible in
the same Rust map, successful replacement removes the mapping and calls the
real release once, rejection makes no release call and restores the exact
original id/generation, the original peer remains bidirectionally usable,
and a stale record restored after rejection still fails generation validation.
The raw replacement used only to construct that stale record is separate
from the calls testing the wrapper.

## Executed cases and cleanup oracles

Both focused TCP modes reuse all original raw fixture assertions:
TCP→local, local→TCP, TCP→TCP; registered-target invalidation; readiness
observed before and collected after replacement; local I/O and TCP fail
closed/source preservation; same-fd and invalid source/target; eight fresh
registrations and replacements; eight worker-thread replacements with
release/acquire arm/done handshakes and readiness collection; final cleanup.
The worker executes the selected wrapper or Linux handler, rather than raw
native dup2. No atomicity is asserted for already-resolved operations.

Both add four rounds of eight attached-target rejection cycles. Each cycle
checks pending readiness, errors, retained registration and full-width data,
original-byte receive and bidirectional routing. Wrapper cycles additionally
check original bridge socket id and endpoint generation, and same-fd no-op.
Linux cycles check unsupported and mixed flags and same-fd EINVAL. Each
Linux round tests O_CLOEXEC acceptance, target-only FD_CLOEXEC, and clearing
that bit on a flags=0 replacement. Each wrapper round then replaces an
attached TCP target with a local source, counts exactly one real release,
and checks local routing and no second release on target close. The base
wrapper phase separately counts the two displaced TCP releases.

The wrapper fixture also creates a naturally stale mapping with raw dup2,
then rejects a real wrapper replacement and proves restoring the stale
record does not authorize it. This reaches the branch without a production
hook. Each mode checks descriptor count before exit, zero network sockets,
listeners, streams, RX/TX buffers and reserved bytes, allocation/release
balance, and kernel endpoint-reference/handle cleanup from serial output.

The separate Linux-personality actor executes pipe replacement, pending
readiness, packed full-width event.data, invalid source/target, same-fd,
unsupported flags, preserved registration/content after rejection, target-only
CLOEXEC, flags=0 clearing, eight fresh registration/replacement cycles,
explicit closed-fd-number reuse, absence of old registration, and cleanup.

## Failed attempts preserved, not product fixes

- `adapter-build.log`: BUILD_FAIL, rustc does not accept Cargo's
  -Zjson-target-spec. Attempts 2 and 3 are BUILD_FAIL for missing
  -Zunstable-options and explicit target core rlib respectively. Attempt 4
  builds successfully. Corresponding link attempts 1–3 cannot compile
  without adapter.o; attempt 4 is BUILD_FAIL for a C declaration with block
  scope. Link attempt 5 succeeds after a file-scope extern declaration.
- `wrapper-2/`: runtime FAIL, two immediate SOCKET_RELEASE_TOTAL assertions.
  All functional checks and final counters passed. TCP close initiates
  asynchronous shutdown (`sunlight-net/src/tcp.rs::close`); release totals
  count eventual reaping, not calls to release_detached. The preserved
  `first-attempt-dup-runtime.c` contains the erroneous immediate oracle.
  The forwarding release observer measures the actual once-only call;
  eventual resource-zero assertions remain mandatory.
- `final/linux-entry-link.log`: LINK_FAIL duplicate _start. The native C
  linker script's explicit allocator exports pull native startup even when
  a raw actor supplies its own entry. Corrected by using the established raw
  Linux actor model: static freestanding -nostdlib and services/user-space.ld,
  without native startup. Nothing was suppressed and no host libc was used.
- `run-2/linux-entry-{2,4}`: NOT_RUN at guest runtime (runner invocation
  FAIL before boot): native-only ELF guard rejects OSABI=3. Added explicit
  single-actor --linux-actor opt-in; the native default still rejects Linux
  images. `focused-final/linux-entry-{2,4}` subsequently execute and pass.
- `focused-final/` wrapper/handler: runtime FAIL only on global
  rejections-release-no-attachments totals. Previous rounds' sockets were
  still being reaped. All release-call, identity, routing and final cleanup
  checks passed. `second-attempt-dup-runtime.c` preserves this version.
  Added a bounded observable reclamation barrier at each round's end, keeping
  the counter equality assertion intact. No arbitrary scheduling sleep or
  production behavior change was introduced.
- One apply_patch tried a parser.add_argument line where the source uses
  choice.add_argument; it failed without mutation, then was applied to the
  actual line. Initial read-only searches for nonexistent build.py/run.py
  and elf/src/lib.rs were corrected to the actual files.

`run-2` contains the fresh adjacent regressions and standard suites, as well
as superseded focused attempts. `validated` is authoritative for the final
focused fixture. Names such as `final` and `focused-final` on earlier attempt
directories do not make them acceptance evidence. All are retained.

## Artifact-aware scope and exclusions

Only tests, harness and new documentation/evidence changed. There is no
production syscall/libc/bridge edit. Native archive hash matching is checked
before/after ordinary locked rebuilds. The exact prior production acceptance
at this same archive hash remains in
`results-2026-11-13-native-dup2-replacement-identity/final/`: external epoll
2/4 CPUs, loopback and external poll/epoll pressure, with zero counters.
Those are matching historical executions, not new executions in this task.
Fresh LT/revalidation, replacement, lifecycle, descriptor, C++ runtime, ABI
and full suite runs are identified separately in the result tables.

Remaining NOT_RUN: Linux-personality native TCP libc integration; exec
lifecycle proving CLOEXEC consumption (bit representation is directly run);
arbitrary races in attachment install/restore, and already-resolved I/O
atomicity. Existing exclusions persist: public dup-family C ABI, TCP
attachment transfer through dup/fork/exec, Linux OFD epoll compatibility,
AF_UNIX replace_handle replacement semantics and stale-entry install races.
No remaining naturally reachable sequential wrapper branch required a new
production test hook. No blocking IPC under a spinlock was introduced.

## Files and preservation

- tools/native-c/tests/dup2-replacement.c: optional test adapter selector;
  default raw behavior is unchanged. Incoming source reconstructed without
  that selector is preserved in incoming-dup2-replacement.c.
- tools/native-c/tests/dup-runtime.c: shared focused fixture and cleanup checks.
- tools/native-c/tests/dup2-wrapper-adapter.rs: calls actual Rust API/state.
- tools/native-c/tests/dup3-linux-entry.c: raw Linux personality actor.
- tools/native-c/tests/run_dup_runtime.py: locked build, symbol-selected rlib,
  bounded commands, maps, per-assertion and per-suite-row evaluation.
- tools/native-c/guest.py: explicit Linux-actor harness opt-in only; incoming
  modifications preserved in starting diff.
- docs/networking/2026-11-13_NATIVE_DUP_RUNTIME_COVERAGE.md and this evidence.

No commit, reset, clean, stash or branch switch. All incoming tracked and
untracked work, archives, logs and historical failed attempts remain.

## Final build, link, runtime and individual assertion results

`acceptance-summary.json` is the authoritative independently re-evaluated
matrix. `summarize.py` checks per-assertion output, every CSV build/run/
acceptance cell, every JSON pass_ value, every kernel endpoint reference and
cleanup record. `summary-validation.log` is PASS. It does not infer success
from the orchestration process's return code. All commands use external
`timeout` watchdogs; guest.py also bounds guest runtime. Exact commands and
exits are in each run's commands.json and guest invocation/build/QEMU logs.
No accepted command timed out.

| Fresh execution | Build | Link | Runtime 2 CPUs | Runtime 4 CPUs | Individual assertions / checks |
| --- | --- | --- | --- | --- | --- |
| `validated/wrapper-*` | PASS | PASS | PASS | PASS | 406/406 each + 2 completion PASS records |
| `validated/linux-handler-*` | PASS | PASS | PASS | PASS | 413/413 each + 2 completion PASS records |
| `validated/linux-entry-*` | PASS | PASS | PASS | PASS | 44/44 each + 1 completion PASS record |
| `run-2/dup2-replacement-*` | PASS | PASS | PASS | PASS | original 50/50 each + 1 completion record |
| `run-2/epoll-*`, `epoll-lifecycle-*`, `inet-socket-*` | PASS | PASS | PASS | PASS | fixture exit/output contracts + kernel resource records |
| `run-2/tcp-epoll-lt-*`, `tcp-epoll-revalidation-*` | PASS | PASS | PASS | PASS | production readiness acceptance markers, cleanup, no failure records |
| `run-2/suite-*` | PASS | PASS | 94/94 PASS | 94/94 PASS | all 94 JSON and CSV rows checked individually |
| `run-2/cxx` checker | PASS | PASS | n/a | n/a | runtime also covered by both full suites |
| `run-2/poll-abi`, production export/header scan | PASS | PASS | n/a | n/a | ABI checks; no dup-family C declaration/export |
| cargo kernel/libc/network checks, final diff check | PASS | n/a | n/a | n/a | locked cargo commands |

Focused fixtures add **zero** standard-suite rows. They execute 863
individual checks per CPU configuration in three separately executed ELFs,
plus five completion records; the existing raw fixture is another 50 checks.
The standard suites remain 94 rows. Each suite has 15 expected nonzero-exit
rows; all match their expected exit and other contracts. Their names/codes
are explicitly retained under expected_nonzero_tests in acceptance-summary.
The deliberate public-dup2 negative link remains expected LINK_FAIL,
confirmed `undefined symbol: dup2` in public-dup2-negative.log. The C++
checker's preserved historical bare link is also an expected LINK_FAIL,
separate from actual failure attempts above.

Per CPU, wrapper allocated=24/released=24; handler allocated=21/released=21.
Both report descriptor counts 3→3 before process exit; socket/listener/
stream/buffer/reserved-byte counters are zero. Kernel INET references are
0→0 and handle counts 0→0 after each focused case. Round reclamation and
exact release-call assertions pass. No resource leak was observed.

## Final hashes, source provenance and preservation

`validated/hashes.json`, `run-2/hashes.json`, and
`all-artifact-hashes.json` record native archives, exact target rlib/core,
fixture ELFs/objects, per-run kernel images and ISOs. Every new evidence
executable/object/ISO is hashed, including failed attempts. Final production
archive remains `2fb382ef6c2b83b48e034645a078d5e36e40f036d98b00a00eace35d222696c7`
in both locations; ordinary kernel remains
`97d9bb2204549ec7548b14fc6d2c90af0d3986dfdd5292aa5eec8ba21e931594`.
No production dependency changed. Guest kernels differ because their embedded
fixture manifests differ; each is saved with build provenance and its own
hash, rather than treated as the ordinary production kernel.

Final focused ELFs:

- wrapper: `d644760e590fd03c0ca26af90b45d186e2af9282ed73537ef874763396aed067`
- Linux handler: `520612b09a8d699802dac7a7ed517004bac1ebd82f4a7dcac0380c9875bb94e7`
- Linux personality actor: `f5239234150cb7ca71446ed23bc0b22c6623a9cda522a5edf517949b5b2d2a58`

`validated-source-hashes.json` and validated-* source snapshots capture the
sources at execution. Two subsequent tooling-only refinements are explicit:
rustfmt sorted the adapter imports; `format-proof/hash-comparison.json`
proves recompilation produces a byte-identical adapter object. The runner
now returns nonzero when its already-recorded runtime classifications
include a failure; the same expression returns zero for validated and
nonzero for the preserved mixed run-2 attempt. No runtime instructions or
fixture assertions changed after final execution. Final source hashes are
recorded separately. Python syntax, Rust formatting and final diff checks
pass.

`incoming-tracked-preservation.json` confirms all 14 incoming tracked diffs
other than guest.py remain exactly unchanged. The starting state had 15
tracked modifications; guest.py contains only the documented additional
opt-in alongside its preserved incoming edits. `task-only.diff` separates
our harness additions from incoming tracked work. Full final status and HEAD
are retained. No commit, branch change, reset, stash, clean, historical-report
rewrite, archive removal or failed-attempt deletion occurred.
