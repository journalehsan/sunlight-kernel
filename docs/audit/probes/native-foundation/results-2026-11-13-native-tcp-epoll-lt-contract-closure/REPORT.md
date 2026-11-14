# Native TCP LT epoll contract closure and bridge audit

This is a new evidence directory. It does not rewrite the 2026-11-12
implementation report or its 92 PASS / 1 inet FAIL / 1 cpp-runtime
LINK_FAIL results. The later suite result below is new evidence, not a
correction of history.

## Provenance

Recorded before any edit in `provenance.txt`:

- HEAD `5b08147001be963e468711d03284b349fa8b2089`, branch `main`.
- Toolchain: rustc/cargo `1.98.0-nightly` (2026-06-05/06-01), QEMU 11.1.2,
  clang 23.1.4.
- The reported archive hash was independently confirmed before and after the
  fresh build: `f1fe24486aa0152688408c16cb3ddddb793ef1024d582111f92cd33da4787441`
  in `target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a`,
  `target/native-c/sysroot/lib/libsunlight_native_c.a`, and the retained
  2026-11-12 `final/libsunlight_native_c.a`. The archive is byte-identical
  after `bash tools/build_native_c.sh` and after both full-suite rebuilds.
- Fixture links use the absolute sysroot archive path and `-Wl,-t`; the
  archive members are recorded in `build-inet.log` and `build-companion.log`.

Incoming changes (present before this task) versus changes made here:

- Incoming tracked modifications: `ipc/src/lib.rs`,
  `kernel/src/arch/x86_64/syscall.rs`, `kernel/src/process/epoll.rs`,
  `kernel/src/process/inet_socket.rs`, `sunlight-libc/include/errno.h`,
  `sunlight-libc/src/multiplex.rs`, `sunlight-libc/src/net_bridge.rs`,
  `sunlight-libc/src/socket.rs`, `sunlight-libc/src/sys.rs`,
  `tools/native-c/Cargo.lock`, plus the untracked implementation, fixtures,
  runners, documents and evidence directories listed in
  `worktree-status-before-task.txt` (reconstructed from the initial
  `git status --porcelain` output before the fixture edit;
  `worktree-status.txt` is the later snapshot and additionally shows the
  fixture change).
- Changes made during this task: `tools/native-c/tests/inet-socket.c`
  (maintained fixture updated to the supported contract) and
  `tools/native-c/tests/inet-socket-epoll-lt.c` (retained compatibility
  wrapper that includes the maintained fixture). No production bridge,
  kernel, libc or IPC file was changed during this task.
- `final-diff-stat.txt` and `inet-fixture.diff` record the final diff.

Historical preservation: no file under
`results-2026-11-12-native-tcp-epoll-lt-implementation/` was modified
(`find ... -newermt 2026-11-13` returned zero files). The historical report
hash is recorded in `final-state.txt`. The old failed-run evidence, incoming
pressure work, inventory report, build-reproducibility evidence and archives
remain in place. No commit, checkout switch or destructive clean was made.

## 1. Legacy inet-contract failure

### Exact failure

The fixture as it stood at HEAD before this task's test-only update
(`tools/native-c/tests/inet-socket.c`) fails at assertion 20 (`exit 20`) on
both CPU counts in the 2026-11-12 evidence:

```
if (epoll_wait(ep, &out, 1, 0) != 1 || out.events != EPOLLERR || out.data != 123) return 20;
if (epoll_wait(ep, &out, 1, 0) != 1) return 21;
e.events |= EPOLLET | EPOLLONESHOT;
if (epoll_ctl(ep, EPOLL_CTL_MOD, fd, &e) || epoll_wait(ep, &out, 1, 0) != 1 || epoll_wait(ep, &out, 1, 0)) return 22;
if (epoll_ctl(ep, EPOLL_CTL_MOD, fd, &e) || epoll_wait(ep, &out, 1, 0) != 1) return 23;
```

The fd is a bare `socket(AF_INET, SOCK_STREAM|SOCK_NONBLOCK|SOCK_CLOEXEC,
IPPROTO_TCP)` that is never bound or connected. The old expectation was:

- a synthetic `EPOLLERR` event with `data == 123` on the unattached
  placeholder, repeated by level-triggered `epoll_wait`;
- TCP `EPOLLET`/`EPOLLONESHOT` accepted by `epoll_ctl`, with edge/oneshot
  suppression and MOD rearming.

New observed behavior: the unattached placeholder is inert (`epoll_wait`
returns 0), and TCP `EPOLLET`/`EPOLLONESHOT` are rejected with `EINVAL`.

### Was the old expectation an explicit unsupported-feature contract?

No. It was the documented behavior of the pre-transport placeholder phase,
not an intentional rejection of an unsupported feature. `docs/audit/std-progress.md`
lines 3301-3307 state that native poll/epoll shared `descriptor_events`
“which reports POLLERR because I/O can immediately report an error; it does
not advertise normal POLLOUT or readable stream data”, that existing epoll
level/edge/one-shot rules applied, and that “There is no connected stream
EOF, shutdown, peer-close or asynchronous network transition to test.” The
ERR was the only observable state of a socket that could not be attached.

The current contract, `docs/networking/2026-11-12_NATIVE_TCP_EPOLL_LT.md`,
explicitly supersedes that: “A registration can precede bind/connect: an
unbound placeholder is inert, and its first matching observation binds its
transport identity in the kernel. Merely being an inet placeholder never
produces an epoll POLLERR.” It also states: “TCP EPOLLET and EPOLLONESHOT are
rejected with EINVAL on ADD/MOD, including before attachment.”

The new behavior is intentional, documented, and validated by
`tcp-epoll-lt.c` (pre-bind registration returns 0; ET/oneshot ADD returns
EINVAL) and `tcp-epoll-revalidation.c` (identity/generation/stale-token
rejection) on 2/4 CPUs.

### Classification

**A. An intentionally obsolete expectation requiring a justified test
update.** It is not B: the production behavior is the documented supported
contract, and the old fixture's ERR/ET/oneshot expectations contradict it.
It is not C: the mismatch is fully explained and independently reproduced by
the native API, not merely inferred from the companion passing.

### Assertion-by-assertion change

`inet-fixture.diff` is the exact diff. The unrelated assertions are retained
unchanged: AF/type/protocol/flag errors (2-7), I/O errors (8-17), poll
`POLLERR` on the unattached placeholder (18), ADD (19), close/EBADF (24),
128-socket exhaustion/ENOBUFS (25-26), dup/EMFILE (27-29), pthread
ENOTCONN/close (31), 128-socket rollback (33-34), select (35), and
zero-count/flag errors (36-38).

| Old assertion | New assertion | Why |
| --- | --- | --- |
| 20: one `EPOLLERR` event, `data == 123` | 20: `epoll_wait == 0` | An unattached placeholder has no transport readiness; the old ERR was the placeholder stand-in. |
| 21: second level-triggered `EPOLLERR` | 21: second `epoll_wait == 0` | Same contract, repeated to prove no synthetic level event. |
| 22: MOD with ET+ONESHOT succeeds; one event then none | 22: MOD returns `-1`/`EINVAL` | TCP ET/oneshot are explicitly unsupported and must not be silently armed. |
| 23: MOD with ET+ONESHOT succeeds; one event | 23: MOD with ET only returns `-1`/`EINVAL` | Separates the ET rejection from the oneshot rejection. |
| (none) | 39: MOD with ONESHOT only returns `-1`/`EINVAL` | Covers the remaining unsupported mode independently. |
| (none) | 40: `epoll_wait == 0` after the rejected MOD | Proves the rejected MOD did not arm or corrupt the registration. |

The old `data == 123` full-width check is not lost: `tcp-epoll-lt.c` checks
full-width `event.data` (`0x8877665544332211`) on a real TCP stream, and the
pressure adapter checks full-width data on every event. Local ET/oneshot
delivery, suppression and rearm remain covered by `epoll.c` and
`epoll-lifecycle.c`, which pass on 2/4 CPUs.

### Companion consolidation

`tools/native-c/tests/inet-socket-epoll-lt.c` is now a retained compatibility
wrapper that includes the maintained original. This removes duplicated
coverage without deleting the companion path, its historical ELF/logs, or
the 2026-11-12 `final/inet-companion-source.diff`. The wrapper builds and
passes on 2 CPUs (`companion-2`).

### Behavior unrelated to TCP LT epoll that also changed

- Native local-fd `epoll_wait` now uses the libc bridge loop (20 ms bounded
  slices) instead of the kernel blocking wait. This is documented in the
  contract and is a latency bound, not a correctness change; `epoll.c` and
  `epoll-lifecycle.c` pass on 2/4 CPUs.
- Native `epoll_ctl` errors now carry proper `EEXIST`/`ENOENT`/`EMFILE`
  instead of the old generic native result, and DEL on a closed fd now
  returns `EBADF` (generation query) rather than `ENOENT`. This is
  Linux-aligned and is exercised by `epoll.c`, `tcp-epoll-lt.c` and
  `tcp-epoll-revalidation.c`.
- Linux-compat epoll on an `inet_socket` handle (only reachable from a native
  process invoking the Linux epoll syscall numbers; `sys_linux_socket` does
  not create AF_INET handles) also loses the placeholder ERR and rejects
  ET/oneshot. This is outside the documented native C contract and is
  recorded as a boundary, not claimed as tested.

## 2. Bridge invariant audit

All line references are to the current working tree.

| Invariant | Evidence | Conclusion |
| --- | --- | --- |
| Kernel registrations authoritative | `kernel/src/process/epoll.rs:45-61` stores token, external identity, mask, raw 8-byte data, ET/oneshot state and cursor. `external_snapshot` (`:199-226`) only copies; `collect_ready_external` (`:228-333`) uses the kernel's mask/data. | PASS. Libc owns no registration table. |
| Token scoped to epoll object, registration, endpoint | `NEXT_TOKEN` (`epoll.rs:13`) is globally monotonic; ADD/MOD assign a fresh token (`:146-172`); collect matches `o.fd == fd && o.token == interest.token` within the selected instance (`:288-296`). | PASS. A token from another instance/registration cannot match. |
| MOD/DEL/close/reuse invalidate stale observations | MOD assigns a new token (`:161-166`); DEL removes the entry (`:174-178`); `sys_close` calls `detach_fd` under `SCHEDULER` before `take(fd)` (`kernel/src/arch/x86_64/syscall.rs:9666-9690`); `detach_fd` removes the fd from every epoll in that descriptor table (`epoll.rs:338-357`). | PASS. `tcp-epoll-revalidation.c:43-66` covers stale MOD, DEL/re-ADD, stale generation and close/reuse. |
| Integer fd reuse cannot redirect readiness | Close+reuse requires close; close detaches the old interest under `SCHEDULER`; the new ADD gets a new token. The ctl path also captures the kernel endpoint generation before reading the libc map and rechecks it under `SCHEDULER` (`syscall.rs:7515-7544`; `inet_socket.rs:8-24`). | PASS for the native C surface. The `dup2` stale-bridge-map case is outside the C surface and is listed under limitations. |
| Full-width `event.data` preserved | `Interest.data: [u8;8]` (`epoll.rs:53-54`); ctl copies 8 bytes at native offset 8 (`syscall.rs:7504-7512`); collect copies 8 bytes at offset 8 (`syscall.rs:7644-7649`). | PASS. LT and pressure fixtures check non-trivial 64-bit data. |
| Stale submissions cannot mutate/deliver unrelated entries | Collect only mutates `interest.external` when fd+token+identity match (`epoll.rs:288-300`); it never uses observation-supplied events/data for the mask or copy-out. | PASS. `tcp-epoll-revalidation.c:46-59` submits stale observations and gets zero events. |
| Counts, masks, buffer bounds, token inputs validated | Mode/count checks and reserved/ready-mask validation (`syscall.rs:7593-7605`); `maxevents` 1..=128; snapshot count exactly 64; collect count <=64; kernel masks readiness (`epoll.rs:305-306`). | PASS. |
| No user pointer retained across blocking work | Mode 0/1 copy from user before taking `SCHEDULER` and copy out after dropping it (`syscall.rs:7598-7649`). Libc passes stack buffers to a non-blocking syscall and sleeps only in libc. | PASS. |
| No spinlock across blocking IPC | `net_bridge::lookup` is an atomic load (`net_bridge.rs:80-84`); `wait_any_per_interest` does not hold `TABLE_LOCK` across `ipc_call_timeout` (`:235-285`); kernel collect performs no IPC. | PASS. |
| Error paths release locks/resources | Rust RAII drops `SCHEDULER`/`EPOLL_POOL`; shm token is freed after the IPC (`net_bridge.rs:269`); kernel vecs drop on every return. | PASS. |
| Positive waits share one monotonic deadline | `socket.rs:190` creates one deadline; every slice uses it (`:252-259`). | PASS with a documented latency caveat: the deadline is checked between scans, not inside a scan; per-identity zero-time NetOp IPC has a 100 ms watchdog. |
| `timeout -1` never returns zero on an internal slice | `deadline = None`; `remaining = 20`; the loop continues (`socket.rs:247-259`). | PASS. |
| Local/network batching and maxevents fairness | One rotating cursor covers both domains (`epoll.rs:255-259`, `:323`); per-identity NetOp WAIT avoids the 32-identity batch limit (`socket.rs:203-230`); LT proves 40 TCP registrations, 9 ready, maxevents=1, all 9 observed. | PASS. |

### Check-then-use trace

`snapshot -> NetOp readiness -> kernel submission -> delivery` is protected
as follows:

1. The kernel snapshot reads the fd table under `SCHEDULER` and the interest
   table under `EPOLL_POOL` (`epoll.rs:199-226`). The token and external
   identity are copied, not the user data.
2. Libc reads the bridge map with an atomic load, takes a zero-time NetOp
   WAIT for that identity, then rechecks the map before setting `ready`
   (`socket.rs:203-230`). A close/reuse between those steps leaves the
   observation at zero.
3. The kernel copies the observations into kernel memory before taking
   `SCHEDULER` (`syscall.rs:7598-7610`), so the user buffer cannot change
   during validation.
4. `collect_ready_external` runs under `SCHEDULER` and `EPOLL_POOL` and
   matches fd, token and identity (`epoll.rs:284-300`). MOD/DEL/close must
   take the same `SCHEDULER`/pool locks, so a completed mutation is visible
   before selection; an event already selected before a racing DEL may still
   finish copy-out (documented, not retracted).
5. Copy-out happens after dropping `SCHEDULER` (`syscall.rs:7644-7649`).

The generation check does not by itself eliminate races. It protects the ctl
window: libc captures the kernel endpoint generation before reading the
bridge map, and the kernel rechecks it under `SCHEDULER` with registration
mutation. The collect window is protected by the token plus the fact that
close detaches under the same `SCHEDULER` lock. The libc bridge map is
per-fd userspace state; the kernel cannot re-read it, so the identity recheck
is necessarily a userspace recheck followed by a token match.

### What remains unproven / limitations

- `dup2` over a bridged inet fd does not update the libc bridge map (the
  native C surface does not declare `dup2`; `dup` is documented as not
  copying the mapping). A raw-syscall `dup2` could leave a stale identity for
  the replacement fd. This is a pre-existing bridge limitation, not a new
  epoll defect, and is not claimed as protected.
- Duplicated/fork-inherited epoll handles still lack a reference-counted
  queue lifetime; `close_file_handle` frees the pool instance. Not claimed
  as Linux OFD-equivalent.
- Positive timeout enforcement is per-iteration, not per-IPC; a pathological
  scan can exceed the deadline by the per-identity watchdog.
- Local epoll readiness latency is bounded by the 20 ms sleep slice.
- No RST-to-POLLERR mapping; no UDP, TCP PRI/RDHUP, nonblocking
  connect/accept, shutdown, signal interruption, or Linux-compat TCP epoll.
- Linux-compat epoll on inet handles is outside the native C contract.

No new reproducible defect was found in the supported native C surface, so no
new production code or debug backdoor was added. The existing
`tcp-epoll-revalidation.c` already covers the critical stale-token,
generation, close/reuse, input-validation, capacity and maxevents cases.

## 3. Pressure retry accounting

The previous loopback runs reported `unstable=16` per run. The maintained
fixture counts a retry in two places (`tcp-poll-pressure-loopback-client.c`
lines 252 and 262): a zero-timeout POLLOUT snapshot that still reports
capacity after send EAGAIN, and a 150 ms withholding wait that returns
POLLOUT before the timeout. A retry is followed by `continue`, so the fill
resumes from the same byte offset; it is not a failed assertion and is not
counted as a stable cycle. `g_cycles` increments only after the snapshot is
zero, the withholding wait times out, the peer drains, and POLLOUT recovers.
`REQUIRED_CYCLES` is 3; the observed runs had 12-13 stable cycles.

To determine the cause without modifying the incoming fixture, this audit
built a throwaway copy (`retry-probe-client.c`, `retry-probe-epoll.c`) with
separate counters and ran it against the unchanged service:

| Run | Stable cycles | `unstable` | snapshot retries | held retries |
| --- | --- | --- | --- | --- |
| `retry-probe-2` (2 CPUs) | 13 | 17 | 0 | 17 |
| `retry-probe-4` (4 CPUs) | 13 | 16 | 1 | 15 |
| 2026-11-12 loopback-2 | 13 | 16 | not split | not split |
| 2026-11-12 loopback-4 | 12 | 16 | not split | not split |
| 2026-11-12 external-2/4/repeat | 5 | 0 | not split | not split |

The retries are observed capacity reappearance, predominantly during the
150 ms withholding wait. The observed condition is that POLLOUT becomes true
again after the client already saw send EAGAIN and after a zero-timeout
snapshot reported no capacity. This is consistent with the peer's TCP stack
ACKing already-received data after the client sees EAGAIN, freeing TX space
before the application read is withheld; the probe does not instrument the
ACK path, so that cause is inferred, not directly measured. The count varies
(16/17, and the split varies), so it is not a configured limit;
`MAX_ITERATIONS` is 4096 and `TOTAL_DEADLINE_MS` is 120000. The external
topology reports zero retries because its host peer and larger buffers do
not produce the same small-window ACK race.

Retries are bounded by `MAX_ITERATIONS` and the total deadline, are reported
separately, preserve the same byte offset, and are covered by the peer's
full-stream integrity check and the zeroed socket/listener/stream
diagnostics. No failed assertion is converted into a retry. The acceptance
criterion (at least 3 stable saturation -> no readiness -> timeout -> drain
-> readiness cycles, with retries reported but not counted as stable) is
sound; no threshold was changed.

## 4. Fresh validation

All commands and exits are in `commands-ledger.json`; per-run guest output is
in the named directories and logs.

| Check | 2 CPUs | 4 CPUs |
| --- | --- | --- |
| Maintained `inet-socket.c` | PASS, exit 0, clean=1 | PASS, exit 0, clean=1 |
| Companion wrapper | PASS, exit 0, clean=1 | not run (wrapper is the same TU) |
| `epoll.c` local-fd | PASS | PASS |
| `epoll-lifecycle.c` | PASS | PASS |
| `tcp-epoll-lt.c` (`--net`) | PASS | PASS |
| `tcp-epoll-revalidation.c` (`--net`) | PASS | PASS |
| Full native suite | 94 cases: 93 PASS, 1 LINK_FAIL/NOT_RUN | same |

Other checks: `run_poll.py` PASS (12 ABI checks, unique exports, unchanged
historical link); `cargo check -p sunlight-kernel` exit 0;
`cargo check -p sunlight-net -p sunlight-ipc -p sunlight-net-server` exit 0;
`cargo check -p sunlight-libc --features native-c` exit 0 (existing warnings
retained); `git diff --check` exit 0.

The full-suite command exits 0 on both CPU counts because the only remaining
non-PASS case is the historical `cpp-runtime` LINK_FAIL, whose runtime is
NOT_RUN; `guest.py` does not treat that as an infrastructure failure. The
suite is **not** all-green: 93 PASS, 1 LINK_FAIL, 1 runtime NOT_RUN.

Historical versus new:

- 2026-11-12 evidence (verified from `final/suite-2` and `final/suite-4`
  `results.csv`): 94 cases, 92 PASS, 1 `inet-socket-contract` FAIL (exit 20),
  1 `cpp-runtime` LINK_FAIL/NOT_RUN. That report remains unchanged.
- 2026-11-13 evidence: 94 cases, 93 PASS, 0 inet FAIL, 1 `cpp-runtime`
  LINK_FAIL/NOT_RUN. This is new evidence, not a correction of the old run.

Production acceptance was not repeated because no production bridge,
readiness, kernel, libc or IPC file changed during this task and the archive
is byte-identical (`f1fe...`). The existing 2026-11-12 external TCP epoll
acceptance (2/4/repeat), five epoll pressure configurations, loopback
lifecycle/mixed/capacity/fairness, and TCP poll regressions remain the
applicable production evidence at that exact archive hash. The LT and
revalidation fixtures were rerun here against the same archive to confirm
the test-side change did not disturb them.

## 5. Remaining classifications and limitations

- `cpp-runtime`: LINK_FAIL, runtime NOT_RUN. Historical and explicit; not
  reclassified as PASS.
- No new TIMEOUT, BUILD_FAIL or production FAIL was observed.
- The four no-`--net` LT/revalidation invocations in `tcp-epoll-lt-2`,
  `tcp-epoll-lt-4`, `tcp-epoll-revalidation-2` and
  `tcp-epoll-revalidation-4` are preserved as FAIL (ENOTCONN at bind/listen)
  because net_server was not started. They are invalid invocations, not
  product failures; the corrected `*-net-2/4` runs PASS.
- Unsupported boundaries remain: UDP, TCP PRI/RDHUP, RST-to-POLLERR,
  nonblocking connect/accept, shutdown, signal interruption, Linux-compat
  TCP epoll, dup/fork epoll aliasing, and the `dup2` stale-map case above.

## Final report answers

1. **The inet failure was an obsolete expectation (A), not a regression or
   unresolved mismatch.** The old synthetic ERR and ET/oneshot acceptance
   were the pre-transport placeholder contract; the new inert-placeholder
   and EINVAL behavior is the documented TCP LT contract and is validated by
   native API fixtures.
2. **Changed:** the maintained `inet-socket.c` assertions 20-23 plus new
   39/40, and the companion became an include wrapper. This is not test
   weakening: unrelated negative/error assertions are retained, unsupported
   TCP ET/oneshot are now tested as explicit rejections, and full-width
   data/local ET/oneshot coverage remains in the LT, pressure, `epoll.c` and
   `epoll-lifecycle.c` fixtures.
3. **Bridge audit:** kernel registrations are authoritative; tokens, fd,
   identity and generation are checked; MOD/DEL/close/reuse invalidate stale
   observations; full-width data is preserved; counts/masks/bounds are
   validated; no user pointer or spinlock spans blocking work; the deadline
   is shared and -1 does not return a false zero; the rotating cursor and
   per-identity scan provide fairness. Unproven: `dup2` stale mapping,
   dup/fork epoll aliasing, per-IPC deadline enforcement, 20 ms local
   latency, and the excluded protocol features.
4. **The reported 16 retries are real observed capacity reappearance**,
   mostly during the 150 ms withholding wait (probe: 17 total, 0 snapshot +
   17 held on 2 CPUs; 16 total, 1 snapshot + 15 held on 4 CPUs). They are
   bounded by 4096 iterations and the 120 s deadline, reported separately,
   never counted as stable cycles, and covered by stream integrity and
   cleanup checks. No threshold changed.
5. **Provenance:** HEAD `5b0814700...`, archive
   `f1fe24486aa0152688408c16cb3ddddb793ef1024d582111f92cd33da4787441`
   before/after build and suite rebuilds, fixtures linked against that
   absolute sysroot archive. Fresh 2/4-CPU suite: 93 PASS, 1 LINK_FAIL, 1
   runtime NOT_RUN each. Maintained inet fixture, companion, `epoll.c`,
   `epoll-lifecycle.c`, LT and revalidation all PASS on 2/4.
6. **Remaining:** `cpp-runtime` LINK_FAIL/runtime NOT_RUN. No other fresh
   FAIL/BUILD_FAIL/TIMEOUT.
7. **Evidence paths:** this directory (`REPORT.md`,
   `commands-ledger.json`, `provenance.txt`, `artifact-hashes.txt`,
   `inet-fixture.diff`, `companion-source.txt`, `retry-probe-*`,
   `suite-2/`, `suite-4/`, `inet-2/`, `inet-4/`, `companion-2/`,
   `epoll-*`, `tcp-epoll-*-net-*`), plus the preserved 2026-11-12
   `results-2026-11-12-native-tcp-epoll-lt-implementation/`.
8. **Preservation:** no commit, no destructive clean, no historical report
   or evidence rewrite; incoming tracked/untracked work and the four
   preserved invalid-invocation runs remain in place.
