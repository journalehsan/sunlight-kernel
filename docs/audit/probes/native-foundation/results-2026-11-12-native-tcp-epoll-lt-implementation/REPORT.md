# Native TCP epoll LT implementation and fresh acceptance

The bounded LT implementation and the listed fresh execution matrix are
delivered. Validation is not all-green: both unchanged full suites retain the
legacy inet epoll contract mismatch and historical cpp-runtime LINK_FAIL.
Their exact classifications, updated-contract companion results and limitations
are recorded below. No environmental blocker was established.

## Baseline and preservation

HEAD was rechecked before edits: `5b08147001be963e468711d03284b349fa8b2089`.
The only incoming tracked modification was `tools/native-c/Cargo.lock`.
Incoming untracked build-reproducibility, poll-pressure and epoll-inventory
reports, pressure scripts/peer/fixtures and pressure networking document were
retained. `incoming-lock.diff` records the incoming tracked change;
`incoming-preservation-hashes.json` records the preserved files for final
integrity comparison. No commit, checkout switch, destructive clean, privileged
disk operation or historical-fixture rewrite was performed.

The separate inventory-only report is preserved at
`../results-2026-11-12-native-tcp-epoll-lt-inventory/REPORT.md`; none of its
historical results are counted as fresh acceptance. The original earlier
milestone text is not present in the accessible conversation. The detailed
continuation requirements were used without dropping its listed cases.

## Decision and implementation

Selected A: kernel registration ownership plus a narrow snapshot/observation
protocol (native syscall 177). Rejected B: a libc registration authority would
need to duplicate lifecycle, data, ET/oneshot state and raw-close synchronization.
Libc owns no second epoll registration table. TCP remains in net_server.

`docs/networking/2026-11-12_NATIVE_TCP_EPOLL_LT.md` specifies the public contract,
timeout, identity, ownership, batching and fairness design and its exclusions.
`ipc/src/native_epoll.rs` defines the shared 32-byte observation wire format.
The kernel owns registrations and raw eight-byte event.data, allocates fresh
ADD/MOD tokens, removes them on DEL/close, validates current tokens/transport
identities, and selects local and network events in one rotating cursor.
Libc snapshots every inet registration, obtains real NetOp readiness without
holding registration locks, rechecks the bridge attachment, and submits
observations for kernel selection. Pre-bind registrations acquire their later
transport identity without MOD. Inet placeholders alone do not produce ERR.

A separate kernel endpoint generation prevents a racing ctl from attaching an
old TCP identity to a reused endpoint. Net_server identities already include
transport generation. Selection under SCHEDULER and the epoll pool lock is the
concurrent ctl/delivery linearization point; an event selected before a racing
DEL can finish copy-out. MOD/DEL completed before selection reject old tokens.

Positive waits have one absolute monotonic deadline; zero waits scan once;
infinite waits repeat bounded slices without returning false zero. Each empty
iteration sleeps at most 20 ms, with scan/IPC overhead additional. All interests
are visited individually, so NetWait's actual 32-identity batch limit does not
truncate the kernel's 64-registration capacity. TCP ET/oneshot are explicitly
EINVAL; existing local modes stay in the kernel.

Changed production files: `ipc/src/lib.rs`, `ipc/src/native_epoll.rs`,
`kernel/src/arch/x86_64/syscall.rs`, `kernel/src/process/epoll.rs`,
`kernel/src/process/inet_socket.rs`, `sunlight-libc/src/socket.rs`,
`sunlight-libc/src/multiplex.rs`, `sunlight-libc/src/net_bridge.rs`,
`sunlight-libc/src/sys.rs`, and `sunlight-libc/include/errno.h` (EEXIST).
Added fixtures/runners: `tcp-epoll-lt.c`, `tcp-epoll-revalidation.c`,
`epoll-observe.h`, three `tcp-epoll-*client.c` include adapters,
`run_epoll_tcp.py`, `run_epoll_pressure.py`, and the
`inet-socket-epoll-lt.c` updated-contract companion. Original fixtures/runners remain
intact, including the incoming pressure work.

## Fresh build and linked artifact

`final/build-archive.log` records the ordinary `bash tools/build_native_c.sh`
locked release native-C build, exit 0, including actual recompilation.
Fresh archive SHA-256:
`f1fe24486aa0152688408c16cb3ddddb793ef1024d582111f92cd33da4787441`.
The release and sysroot archives match; `final/libsunlight_native_c.a` retains
that exact archive. New fixture links use cc.py with `-Wl,-t`, recording actual
archive members from the absolute sysroot archive path in their build logs.
`final/cc-driver.txt` records that driver's explicit static archive selection.
The final execution ledgers record archive hashes before/after each matrix job;
ABI/full-suite rebuilds must not silently change it. ELF hashes are retained in
runner results and the final artifact manifest. `final/source-hashes.json` and
`final/source-state.patch` pin the implementation and fixture sources.

## Support boundaries

Native libc AF_INET TCP LT: listener readiness; buffered data and drain; EOF
without invented HUP; can_send-based OUT including saturation/recovery; existing
ERR/HUP; ADD/MOD/DEL; multiple independent queues; full-width data; timeout
0/positive/-1; shared-native-thread ctl; registration before bind/connect;
registration sets above the network batch size; fair bounded delivery.

The guest ABI is explicitly 16-byte events, alignment 8, data offset 8. Linux's
packed 12-byte ABI is separate. No RST mapping was introduced. No claims for
UDP, TCP PRI/RDHUP, nonblocking connect/accept, shutdown, signals interrupting
wait, Linux-compat TCP epoll, or poll/select of nested TCP epoll queues.

Dup/fork inspection found shared fd open descriptions for offset/status but no
TCP bridge-map transfer and per-PID net_server ownership. Closing an fd removes
fd-keyed registrations from queues held in that table; this is not Linux OFD
epoll lifetime. Duplicated/fork-inherited epoll instance aliases also lack a
reference-counted queue lifetime: close_file_handle frees their pool instance.
These existing unsupported aliases are not claimed as Linux-equivalent. Native
std shared-descriptor workers are exercised. Concurrent connect/bind/close on
one fd is not newly specified by this readiness milestone.

## Evidence layout and exact execution

`run-matrix.py` and `final/matrix-ledger.json` contain exact final commands,
exit statuses, durations, watchdogs, archive hashes and full-log locations.
`run-revalidation.py`, `final/revalidation-ledger.json`, and
`final/revalidation-build.log` cover the additional negative cases. Native API
outputs are in each guest's results.json and serial.log. Per-run summary JSON
contains peer exits, integrity verdicts, guest cleanup and resource counters.
Every guest uses 2048 MiB; CPU counts are explicit in the commands.

All pre-final attempts are retained outside `final/`. Initial C compilation
failed for a missing public EEXIST definition, then an unused fixture helper;
both logs remain. The first capacity topology attempted 40 simultaneous
listener/client/pending triples and hit ENOMEM at the existing net_server
1 MiB heap. The corrected topology retains 40 TCP registrations, eight pending
connections and one local registration, nine ready at once with maxevents=1.
No assertion or required capacity was weakened. A later fixture assumed the
closed stream was the lowest free fd; it now allocates through lower holes and
requires reuse of the exact old number. Initial guest.py output classified
exit=1 as an unspecified expected exit; the actual FAIL stdout is retained and
not counted as acceptance. Final focused runs specify --expect-exit 0.

The pre-attachment matrix was stopped at a completed-run boundary to close the
pre-bind registration gap. Its child loopback-4 run completed and exited 0;
the paused coordinator was killed after that child was already a zombie. No
live guest or peer was abandoned. All ten main epoll runs were then repeated
in `final/` against the final archive; pre-final passes are not substituted.

## Final results

All following results are fresh and under `final/`. Every QEMU uses 2048 MiB.
The complete matrix has 42 command records; only the two full-suite commands
exit nonzero. Six focused runs (LT, protocol revalidation and inet companion on both
CPU counts) are represented in their own ledgers or the matrix; the two LT
runs are already part of the 42 matrix commands. `acceptance-summary.json` joins the individual per-case/per-run results.

| Acceptance run | CPUs | Classification |
| --- | --- | --- |
| `lt-2-final`, `lt-4-final` | 2 / 4 | PASS |
| `epoll-external-2`, `epoll-external-4`, `epoll-external-2-repeat` | 2 / 4 / 2 | PASS; host peer exit 0 each |
| `epoll-pressure-external-2`, `-4`, `-2-repeat` | 2 / 4 / 2 | PASS; five stable pressure cycles each, unstable=0; host peer exit 0 each |
| `epoll-pressure-loopback-2` | 2 | PASS; 13 stable cycles, unstable=16; support peer exit 0 |
| `epoll-pressure-loopback-4` | 4 | PASS; 12 stable cycles, unstable=16; support peer exit 0 |
| `revalidation-2`, `revalidation-4` | 2 / 4 | PASS |
| `inet-lt-2`, `inet-lt-4` | 2 / 4 | PASS; updated contract plus retained socket/exhaustion/rollback/thread checks |

Pressure proves fill to native send EAGAIN, zero OUT snapshot, finite timeout
while reads are withheld, coordinated peer drain, OUT recovery, and resumed
writes. External peers validated all 16,777,224 framed data bytes with FNV
`5105c7d8c3e47d34`; separate reset traffic is also included in the clients'
reported aggregate byte count. Loopback peers validated all 262,152 framed
bytes. Unstable counts are recorded retries where capacity reappeared after
EAGAIN or during the withholding wait; they are not counted as stable cycles
and were not hidden. Stable transitions still meet the unchanged assertions.

External RST is separate from orderly FIN. All three final reset runs observed
`revents=0x10` (HUP) and subsequent write errno 111. No new ERR/reset mapping was
introduced, and no claim of observed RST POLLERR is made. EOF/FIN tests assert
IN remains level-triggered after receive returns zero, with no fabricated HUP.
The pressure adapter additionally MODs to events=0 on actual terminal events
and requires only ERR/HUP, preserving full-width data.

| Required case | Fresh proof |
| --- | --- |
| Guest size/alignment/data offset, full-width data, ctl errno, invalid maxevents | LT, revalidation, existing epoll fixtures and ABI gates |
| Listener LT, registration before bind, multiple ready listeners | LT on 2/4 |
| Receive LT, partial/full drain, EOF, no invented HUP | LT on 2/4; external receive and pressure FIN |
| ADD/MOD/DEL, changed masks/data, multiple instances, close/exact-fd reuse | LT on 2/4 |
| Concurrent MOD/DEL, stale MOD/DEL/re-ADD observations, identity/generation mismatch | LT and revalidation on 2/4 |
| Positive deadline across slices/concurrent MOD, -1 across slices then coordinated local wake | LT and revalidation on 2/4 |
| More than 32 TCP registrations, ready entries beyond the batch boundary | 40 TCP registrations in LT on 2/4 |
| More ready entries than maxevents, eventual delivery, mixed domains | 41 total registrations, nine ready, maxevents=1; all nine observed on 2/4 |
| Actual registration capacity and overflow | 64 accepted / 65th EMFILE in revalidation on 2/4 |
| Empty ordinary mask with actual terminal events | External epoll pressure adapter; EOF-only empty mask stays zero in LT |
| Unsupported TCP ET/oneshot, including before attachment | LT ADD tests and companion MOD tests on 2/4 |
| Pressure repetition, byte integrity, external RST distinct from FIN | Five final epoll pressure runs and their peer logs |

### Regressions

| Regression | Fresh classification |
| --- | --- |
| Existing `epoll.c` and `epoll-lifecycle.c` | PASS on 2/4 |
| Poll ABI / socket ABI C/C++ gates and historical links | PASS |
| Existing loopback/external TCP poll | PASS on 2/4 |
| Existing loopback/external POLLOUT pressure | PASS on 2/4 |
| Existing loopback echo, external echo, external stall | PASS on 2/4 |
| Kernel cargo check | PASS, exit 0 |
| sunlight-net / sunlight-ipc / sunlight-net-server cargo check | PASS, exit 0 |
| sunlight-libc cargo check --features native-c | PASS, exit 0; existing warnings retained |
| git diff --check | PASS, exit 0 |
| Full native suite, 2 CPUs | 94 cases: 92 PASS, one runtime FAIL, one LINK_FAIL/runtime NOT_RUN; command exit 1 |
| Full native suite, 4 CPUs | same classifications; command exit 1 |

The full-suite runtime failure is `inet-socket-contract`, exit 20. The unchanged
historical test requires synthetic ERR on an unattached inet placeholder, then
success for TCP ET/oneshot (later assertions 22/23, not reached after exit 20).
Those expectations conflict with this milestone's explicit contract. This is
an intentional behavior change with a retained test failure, not an unrelated
baseline failure or an all-green regression suite. The original file's HEAD
bytes were verified unchanged. The separate `inet-socket-epoll-lt.c` companion
requires zero readiness before attachment and EINVAL for unsupported modes,
while retaining the original 300-cycle I/O/resource, exhaustion, rollback,
dup and thread assertions. It passes on both CPU counts. Its exact changes
are in `final/inet-companion-source.diff`; historical assertions were not
rewritten or silently disabled in the full suite.

`cpp-runtime` remains the historical LINK_FAIL; no executable exists, so its
runtime is NOT_RUN. The separate `cpp-runtime-native` case passes. No known
hanging host secret_store suite was launched; historical unrelated host
alloc/dirent failures were not rerun or relabeled. No requested matrix row was
left unexecuted. Broader excluded APIs and unavailable original-spec text are
not represented as tested.

### Cleanup, integrity, final state

`final/cleanup-audit.json` records 34 final guest runs and 218 per-case resource
records, all `clean=1`. All eight guest support-service exits were 0. All host
peer exit checks passed. No QEMU, guest runner or TCP peer process remained.
The network fixtures reported socket/listener/stream counters back at zero;
serial records retain fd, frame, page-table, handle and namespace balances.
Captures support external topology, not readiness ordering.

The final archive is
`f1fe24486aa0152688408c16cb3ddddb793ef1024d582111f92cd33da4787441`
in the release path, sysroot and retained snapshot. Every matrix command
following the initial build saw that same before/after hash, including ABI
and suite rebuilds. `final/artifact-hashes.json` records 200+ linked kernel/guest
ELFs and archive copies; linker traces and linked epoll disassembly are retained.

The final audit found no incoming-file hash mismatch and no pinned-source
mismatch. The incoming Cargo.lock diff is byte-for-byte unchanged. HEAD remains
`5b08147001be963e468711d03284b349fa8b2089`. `final/final-status.txt`,
`final/final-diff-stat.txt` and `final/summary-audit.log` record repository state
and checks. Historical evidence, inventory, incoming artifacts and all failed
attempts remain intact. No commit was created.
