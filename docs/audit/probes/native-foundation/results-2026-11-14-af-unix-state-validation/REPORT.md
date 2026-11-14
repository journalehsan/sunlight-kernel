# AF_UNIX connect state validation and alias coherence, 2026-11-14

Finding: **one demonstrated, reachable contract defect** in listening-source
validation, fixed. **Alias coherence shows no ownership or registration
defect**; the observed dup-alias divergence from Linux OFD semantics is
reachable-but-unspecified and was not changed. All other reported baselines
were independently re-verified from the current tree.

## Classification

| Area | Classification | Basis |
| --- | --- | --- |
| Listening-source `connect` | **Proven contract violation** (Linux compat) → fixed | `unix(7)`: LISTEN source returns `EINVAL`; pre-fix handler returned success and destroyed the listener |
| Alias ownership / refcounts / close order | **Correct supported behavior** | refs released exactly once; no double free; fd-number reuse isolated |
| Alias shared-description transition | **Reachable but unspecified; contract decision unresolved** | connecting one `dup` alias leaves the other a valid unconnected endpoint; local contract is fd-keyed and excludes OFD/alias attachment |
| Same-logical-socket registration continuity across connect | **Correct supported behavior** | fd-keyed lookup follows the live handle; token/data preserved; no invalidation needed |
| Concurrent blocking-connect wake selection | **NOT_RUN** | out of scope; no direct dependency demonstrated |
| Numeric Unix pool census | **NOT_RUN** | no exported counter; ownership tracing + clean reaping only |

## Provenance

Starting HEAD `5b08147001be963e468711d03284b349fa8b2089`, branch `main`.
`starting-head.txt`, `starting-branch.txt`, `starting-status.txt` (full
tracked/untracked status), `starting-status-long.txt`,
`incoming-tracked.diff` (1087 lines of preserved incoming worktree changes),
`toolchain.txt`, `configuration.txt` and `production-before.txt` were captured
before any edit. Rust 1.98.0-nightly (8954863c8, 2026-06-05); kernel target
`x86_64-unknown-none`. No commit/reset/clean/stash/branch switch.

Production artifacts before edits:

| Artifact | SHA-256 |
| --- | --- |
| `target/native-c/sysroot/lib/libsunlight_native_c.a` | `2fb382ef6c2b83b48e034645a078d5e36e40f036d98b00a00eace35d222696c7` |
| `target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a` | `2fb382ef…` (same) |
| `target/x86_64-unknown-none/debug/sunlight-kernel` | `97d9bb2204549ec7548b14fc6d2c90af0d3986dfdd5292aa5eec8ba21e931594` |

After the kernel-only fix the native archive is **unchanged** at `2fb382ef…`
(verified after `tools/build_native_c.sh`), and the ordinary debug kernel is
`d3c50f196e46573a30ac061f228c8a3cef2462c42dbe0d26cc18a43971990407`
(`production-after.txt`, `regression/kernel-build.log`). The libc archive is
not rebuilt by a kernel change, so its identity matching is expected, not a
claim of a fresh userspace rebuild.

## Contract and call paths

State model, entry-point contract and alias ownership matrix:
`docs/networking/2026-11-14_AF_UNIX_STATE_VALIDATION.md`.

Exact paths:

- `kernel/src/arch/x86_64/syscall.rs:1206` dispatcher `1051 => sys_linux_connect`;
  Linux syscall 42 translates to 1051 via the `-32` mapping.
- `sys_linux_connect` at `kernel/src/arch/x86_64/syscall.rs:8020`.
- New source-state check at `kernel/src/arch/x86_64/syscall.rs:8046`
  (`unix_socket::is_listening`), before `find_listener`, `alloc_socketpair`,
  `enqueue` and `FdTable::replace_handle`.
- `kernel/src/process/unix_socket.rs:206` `is_listening`;
  `kernel/src/process/fd_table.rs:642` `replace_handle`.
- Descriptor replacement (`dup2`/`dup3`) invalidation is separate at
  `kernel/src/arch/x86_64/syscall.rs:10496` `invalidate_replaced_fd`; connect
  intentionally does **not** call it.

`bind`/`listen` resolve the fd under `SCHEDULER` and mutate the Unix pool
after dropping it; `connect` holds `SCHEDULER` from source lookup through
enqueue, `replace_handle` and wakeup. Lock order remains
`SCHEDULER -> Unix pool`; no spinlock is held across blocking IPC.

## Baseline reproduction (pre-fix)

`tools/native-c/tests/unix-state-validation.c` + `run_unix_state_validation.py`
run the real production path with no test-only kernel adapter. `baseline-final/`
is the clean pre-fix run with the final fixture. `baseline/` and
`baseline-corrected/` preserve two earlier attempts (a fixture fd-reuse
ordering bug and a `&&`-short-circuit cleanup leak, both fixture-only, corrected
before the production fix).

| Cell | Build | Link | Runtime | Assertions | Failures |
| --- | --- | --- | --- | --- | --- |
| linux-2 | PASS | PASS | FAIL | 137 | 40 |
| linux-4 | PASS | PASS | FAIL | 137 | 40 |
| native-2 | PASS | PASS | FAIL | 137 | 40 |
| native-4 | PASS | PASS | FAIL | 137 | 40 |

All 40 failures per cell trace to the same defect: listening-source connect
returned success (not `EINVAL`), which unregistered the source pathname,
dropped its pending queue (queued peer observed EOF), lost the source
registration, and installed an unexpected connection at the destination.
`full-listening-source-EINVAL-precedes-EAGAIN` failed because the full-queue
path returned `EAGAIN` before any source validation. The alias-connect and
alias-reuse sections passed in baseline, i.e. no alias ownership defect.

## Fix

`kernel/src/arch/x86_64/syscall.rs` `sys_linux_connect`: after resolving the
source index and before any allocation or mutation:

```rust
if crate::process::unix_socket::is_listening(index) {
    return linux_errno(sunlight_compat_linux::abi::EINVAL as u64);
}
```

Rationale: `unix(7)` states a LISTEN source must return `EINVAL`; the check
precedes endpoint allocation, destination enqueue and `replace_handle`, so a
rejected attempt preserves the listener, its pathname, pending connections,
registrations and destination capacity. No registration is cleared, no lock
order changes, no public export added, no success stub.

## Post-fix validation

`validated/` (same fixture source, fresh isolated kernel build, 2/4 CPUs):

| Cell | Build | Link | Runtime | Assertions | Failures | Cleanup |
| --- | --- | --- | --- | --- | --- | --- |
| linux-2 | PASS | PASS | PASS | 176 | 0 | clean=1, fds=0, handles 0/0 |
| linux-4 | PASS | PASS | PASS | 176 | 0 | clean=1, fds=0, handles 0/0 |
| native-2 | PASS | PASS | PASS | 176 | 0 | clean=1, fds=0, handles 0/0 |
| native-4 | PASS | PASS | PASS | 176 | 0 | clean=1, fds=0, handles 0/0 |

The fixture has 177 CHECK sites; `176` excludes the terminal `complete`
record. Covered: empty-queue listening source, pending-queue source, full
destination queue, missing/bound-not-listening destination, self-target,
pre-attempt registrations with full-width `event.data`, post-rejection
usability and routing with distinct payloads, dup-alias connect, dup-alias of
a listening source (both aliases rejected, queue/registration preserved),
close/reuse one alias, and four repeated rejected lifecycles.

## Regression matrix (fresh, 2 and 4 CPUs)

| Suite | Result | Notes |
| --- | --- | --- |
| AF_UNIX replacement actor (`run_unix_replacement.py`) | 507/507 each cell | was 508; revised listening section is 8 checks vs 9 (−1), expectation now `EINVAL` + preservation |
| Native public AF_UNIX API (`unix-replacement-public.c`) | 137 PASS each cell | 136 assertions + completion; unchanged |
| Production Rust `dup2` wrapper | 408 PASS each cell | 406 assertions + 2 completion records |
| Linux `dup3` handler | 415 PASS each cell | 413 assertions + 2 completion records |
| Linux syscall actor (292) | 45 PASS each cell | 44 assertions + completion |
| Raw `dup2` replacement fixture | 51 PASS each cell | 50 assertions + completion |
| epoll / epoll-lifecycle / inet-socket | PASS each cell | output/exit/resource contracts |
| TCP LT + revalidation | PASS each cell | existing readiness/identity assertions |
| Full native suite | 94/94 each cell | 94 rows, 94 pass |
| C++ runtime checker, poll ABI | PASS | host-side checks |
| `cargo check` kernel/libc/network | PASS | `--locked` |
| `git diff --check` | PASS | `final-diff-check.log` |

All from `regression/dup-runtime/` (single runner, `--suite`), plus
`regression/unix-replacement/` and `regression/unix-public/`. Focused counts
are separate from the 94-row suite; the new fixture adds **zero** standard
suite rows. The revised `unix-replacement.c` listening section changes that
focused count from 508 to 507 and is documented with contract rationale; its
pre-edit baseline is preserved as
`results-2026-11-14-af-unix-replacement/validated/fixture.c` (SHA-256
`9772e86b5f5e918e32c4571a439032c4b74dbde00f3166b8299e085362d1ba0d`).

## Cleanup and resource observations

Every guest cell reports `clean=1`, `file_before=after=0`, stable private/page
counts, `handles 0/0` and `names 2/2`. Rejected attempts are followed by
successful accept/routing and repeated lifecycles. This is ownership tracing
plus observable EOF/close, **not** a numeric Unix pool census; no exported
counter exists, so exact global allocation/release balance is NOT_RUN.

## Fresh versus historical

All rows above are fresh executions in this task except where noted. The
previous replacement investigation's 2/4-CPU acceptance, external/pressure
acceptance and matching archive hashes remain historical evidence and are not
re-claimed as fresh. No external or pressure acceptance is newly required by
this kernel-only change; the affected networking regressions (TCP LT,
revalidation, inet socket) were re-run fresh.

## Limitations and follow-ups

- Alias divergence from Linux OFD semantics (connect through one `dup` alias
  does not connect the others) is a contract decision, not fixed here.
- `listen` mutates the Unix entry without `SCHEDULER`; a concurrent
  cross-thread `listen` could race the source-state check. Pre-existing
  bind/listen atomicity gap; serial transitions fully validated.
- Concurrent blocking-connect wake selection, ET/oneshot transition coverage,
  fork/exec attachment transfer, Linux OFD epoll lifetime and numeric Unix
  pool census remain NOT_RUN.
- `bind`/`listen` resolve-then-mutate races are not proved atomic here.

## Files changed

- `kernel/src/arch/x86_64/syscall.rs`: listening-source `EINVAL` check.
- `tools/native-c/tests/unix-state-validation.c`: new deterministic fixture.
- `tools/native-c/tests/run_unix_state_validation.py`: new bounded runner.
- `tools/native-c/tests/unix-replacement.c`: listening section revised to the
  corrected contract (baseline preserved).
- `docs/networking/2026-11-14_AF_UNIX_STATE_VALIDATION.md`: new contract.
- `docs/networking/2026-11-14_AF_UNIX_REPLACEMENT_IDENTITY.md`: cross-reference.
- `docs/Helios_LinuxCompat/coverage.md`: connect row notes LISTEN `EINVAL`.
- This evidence directory (new).

No commit, reset, clean, stash or branch switch. Historical reports, incoming
tracked/untracked changes, fixtures, archives and failed attempts preserved.
