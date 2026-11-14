# Native TCP readiness identity across dup2 descriptor replacement

This directory is new evidence. It does not rewrite the 2026-11-12
implementation report, the 2026-11-12 inventory report, or the 2026-11-13
contract-closure report. The limitation recorded there ("a raw-syscall
`dup2` could leave a stale identity for the replacement fd ... not claimed as
protected") is now measured, reproduced, fixed and revalidated.

## 1. Starting provenance

`provenance-before.txt` (written before any edit) and `toolchain.txt`:

- HEAD `5b08147001be963e468711d03284b349fa8b2089`, branch `main`.
- Toolchain: rustc/cargo `1.98.0-nightly` (2026-06-05 / 06-01), QEMU 11.1.2,
  clang 23.1.4; target `x86_64-unknown-none`, kernel linker script from
  `.cargo/config.toml`.
- Native archive at start (`archive-hashes-before.txt`):
  `f1fe24486aa0152688408c16cb3ddddb793ef1024d582111f92cd33da4787441` in
  `target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a` and in
  `target/native-c/sysroot/lib/libsunlight_native_c.a`. This matches the
  reported baseline hash, independently recomputed here.
- Incoming worktree: 14 tracked modifications and the untracked vendor,
  fixture, runner, document and evidence files listed in
  `provenance-before.txt`. Nothing was committed, reset, stashed, cleaned or
  checked out during this task.
- Reported baseline (94/94 native suite, cpp-runtime runtime PASS) was
  re-measured here rather than trusted: `final/suite-2` and `final/suite-4`
  contain 94/94 PASS at the final archive hash, including
  `cxx-runtime-execution` and `cpp-runtime-native`.

Supported commands used (exact strings in `commands-ledger.json`):

- `bash tools/build_native_c.sh` (native archive + sysroot copy)
- `python3 tools/native-c/cc.py ...` (fixture link against the sysroot archive)
- `python3 tools/native-c/guest.py <elf> [--net] --cpus {2,4} ...`
- `python3 tools/native-c/{run_epoll_tcp,run_poll_tcp,run_epoll_pressure,run_poll_pressure}.py`
- `python3 tools/native-c/tests/run_poll.py`, `.../run_cxx_runtime.py`
- `cargo build --package sunlight-kernel --locked`, `cargo check ...`,
  `git diff --check`

## 2. Ownership and lifetime trace

All references are to the final working tree.

| Element | Code | Identity / lifetime |
| --- | --- | --- |
| Guest fd number | `kernel/src/process/fd_table.rs` | index into a 256-slot table |
| Table entry | `fd_table.rs` `FileDescriptor` | handle + shared offset/status description |
| Inet endpoint | `kernel/src/process/inet_socket.rs:8-40` | pool slot + monotonic generation; `release` only drops a reference |
| Epoll registration | `kernel/src/process/epoll.rs:45-61` | instance + fd key + token + `external: Option<u64>` + raw `data: [u8;8]` |
| Readiness token | `epoll.rs:146-172`, `:288-300` | fresh token per ADD/MOD; observation must match fd, token and identity |
| Bridge mapping | `sunlight-libc/src/net_bridge.rs:56-160` | fd key → (`net_server` socket id + 1, kernel endpoint generation) |

Contract: `docs/networking/2026-11-13_NATIVE_DUP2_REPLACEMENT_IDENTITY.md`.

### Reachable duplication interfaces (measured, not assumed)

- `sunlight-libc/include/unistd.h` declares neither `dup` nor `dup2`
  (`abi-boundary.txt`).
- The archive exports no `dup`/`dup2`/`dup3` symbol
  (`nm -g --defined-only`, `abi-boundary.txt`).
- `probe-dup2-abi.c` declares `dup2` and calls it; the link fails with
  `undefined symbol: dup2` (`probe-dup2-abi.log`, exit 1). **The C ABI does
  not expose the operation.**
- The guest syscall table still exposes `dup2` (46) and Linux `dup3`
  (`sys_dup2` at `kernel/src/arch/x86_64/syscall.rs:10464`,
  `sys_linux_dup3` at `:6594`), and `sunlight_libc::dup2` exists at
  `sunlight-libc/src/lib.rs:176`. Both reach the replacement.

So the operation is not a C-ABI operation, but it is a reachable ABI
operation; the unsupported part is transport attachment transfer, not the
descriptor replacement itself.

## 3. Baseline reproducer (before the fix)

`tools/native-c/tests/dup2-replacement.c` (now 50 checks) issues syscall 46
directly, like the shipped `open-unlink.c` fixture does for syscall 45. The
baseline binary was built before any production edit, against archive
`f1fe2448...`, and run with `--net --cpus 2`
(`dup2-2-before/`, `dup2-2-before.log`). Runner exit 1; 6 failures:

```
dup2-replacement: FAIL local-target-registration-invalidated (errno=0)
dup2-replacement: FAIL tcp-target-registration-invalidated (errno=0)
dup2-replacement: FAIL replaced-tcp-target-fails-closed (errno=0)
dup2-replacement: FAIL tcp-target-invalidated-by-local-source (errno=107)
dup2-replacement: FAIL local-source-keeps-local-semantics (errno=107)
dup2-replacement: FAIL repeated-replacement-invalidates-each-registration (errno=107)
```

What each failure means concretely:

- A local read end was registered, made readable, then replaced by another
  readable pipe read end. `epoll_wait` returned one event carrying the old
  data: the fd-keyed registration followed the number and reported the new
  object's readiness as if it were the old registration.
- A registered TCP stream with pending data was replaced by another
  connected TCP stream. `epoll_wait` returned the old registration's event.
- `read()` on the replaced TCP fd returned the replaced endpoint's pending
  byte (`errno` unchanged, count 1) instead of failing: the stale bridge
  mapping routed transport I/O to the endpoint the descriptor no longer
  names.
- A local source replaced a registered TCP target. The registration survived
  and fired, and `read()` on the now-local fd was routed through the bridge
  to the replaced TCP endpoint instead of reading the pipe.
- Re-adding the replaced number failed (registration still present), so the
  repeated lifecycle could not complete.

This is the reported `dup2` gap, reproduced deterministically with no sleeps:
readiness is created before the replacement, and the post-replacement
observation is a single zero-timeout `epoll_wait`.

Preservation note: the failing observation is preserved in `dup2-2-before/`
(`elf.stdout.txt`, `serial.log`, `results.json`, `guest.iso`, `kernel.elf`).
The baseline ELF itself was relinked at the same path during the later
post-fix builds, so it is no longer a distinct file. The baseline fixture
source is the final source minus section 7 (the concurrent phase was added
after the fix); all later builds are kept under their own paths.

## 4. Root cause

Two layers each kept an identity that the replacement invalidated, and
neither was told about the replacement:

1. **Kernel registrations were fd-keyed but not replacement-aware.**
   `sys_dup2`/`sys_linux_dup3` swapped the table entry and released the
   displaced handle, but only `close` called `epoll::detach_fd`. The
   registration survived with its old token, mask and data, and
   `collect_ready_external` then either (a) observed the new object directly
   for a local handle, or (b) accepted an observation whose identity matched
   the stale `interest.external`. Both deliver readiness and `event.data`
   across an identity boundary.
2. **The libc transport map was keyed only by fd number.**
   `IDS[fd]` had no binding to the kernel endpoint generation and no
   replacement hook, so a rebound number kept sending `NetOp` traffic to the
   `net_server` socket of the replaced endpoint. `close` released it, which
   is why closing alone did not reveal a counter leak, but every operation in
   between used the wrong endpoint.

The defect is real, not a test artefact: it is the direct cause of the
wrong-byte reads and the stale readiness events above.

## 5. Patch

Kernel (`kernel/src/arch/x86_64/syscall.rs`):

- new `invalidate_replaced_fd` (`:10490`) called from `sys_dup2` and
  `sys_linux_dup3` under `SCHEDULER`, after the same validation the fd table
  performs and before the entry is replaced. It detaches the target fd key
  from every epoll instance held by the descriptor table.
- `old_fd == new_fd`, an invalid source, or an out-of-range target return
  before the detach, so a rejected call destroys nothing.

Libc bridge (`sunlight-libc/src/net_bridge.rs`):

- per-slot endpoint generation (`:57`), captured by `endpoint_identity`
  (`:106`) **before** the attach `NetOp` and stored by `install` (`:124`),
  outside the table lock;
- `lookup` (`:118`) returns the attachment only while the fd still names the
  recorded endpoint, so stale mappings fail closed for read/write/send/recv/
  poll/epoll paths, including callers that bypassed this crate;
- `detach` (`:133`) returns the recorded (socket id, generation) pair and
  lets `close` (`:843`) release the attachment even when the identity no
  longer matches;
- `release_detached` (`:145`) and `reattach` (`:154`) support the
  replacement path; a restore re-installs the original generation, so it only
  becomes valid again if the descriptor still names the same endpoint.

Libc entry point (`sunlight-libc/src/lib.rs:176`):

- `dup2(fd, fd)` is forwarded untouched;
- otherwise the target attachment is detached, the kernel replacement runs,
  and the attachment is released on success or restored on rejection. The
  duplicate is left unbridged, consistent with the documented rule that
  attachments are not copied.

Why this is the smallest justified fix:

- kernel registrations, tokens, full-width `event.data`, ET/oneshot
  rejection, lock order (`SCHEDULER` → `EPOLL_POOL`), non-blocking critical
  sections and the fairness cursor are unchanged;
- no registration is cleared globally: only the replaced fd key is removed,
  and only when the replacement will actually happen;
- no delay, no descriptor-number heuristic, no blocking IPC under a lock;
- no new C symbol and no new syscall.

Cost note: `lookup` now performs one non-blocking generation query, and only
when a mapping exists. Every such path already performs at least one
`NetOp` IPC for the same descriptor.

## 6. Files changed by this task

- `kernel/src/arch/x86_64/syscall.rs` (invalidation on replacement)
- `sunlight-libc/src/net_bridge.rs` (generation-bound mapping, detach/close)
- `sunlight-libc/src/lib.rs` (`dup2` attachment bookkeeping)
- `tools/native-c/tests/dup2-replacement.c` (new fixture)
- `docs/networking/2026-11-13_NATIVE_DUP2_REPLACEMENT_IDENTITY.md` (new
  contract)
- `docs/networking/2026-11-12_NATIVE_TCP_EPOLL_LT.md` (one cross-reference
  line in the limitations list; no result or historical claim edited)
- this evidence directory

No other file was edited. No commit, branch switch, reset or clean occurred.

## 7. Validation

Final artifact: archive
`2fb382ef6c2b83b48e034645a078d5e36e40f036d98b00a00eace35d222696c7`
(release + sysroot copy, `archive-hashes-final2.txt`), unchanged after both
full-suite rebuilds. Kernel ELF from
`cargo build --package sunlight-kernel --locked`:
`97d9bb2204549ec7548b14fc6d2c90af0d3986dfdd5292aa5eec8ba21e931594`
(the kernel image embeds native consumers built from the same sources).
All `final/*.elf` fixtures were relinked against this archive.

Superseded runs are preserved, not deleted: `264d2ac0...` was a
formatting-intermediate archive (runs in this directory's root) and
`4c9bcea4...` was the archive before the attach-time generation refinement
(`runs-archive-4c9bcea4/`, same PASS matrix). `final/` is the only matrix
quoted below and is the one that matches the final sources.

| Run | CPUs | Classification | Result |
| --- | --- | --- | --- |
| `dup2-replacement` pre-fix (`dup2-2-before`) | 2 | FAIL (product, reproduced) | 6/47 assertions failed |
| `final/dup2-replacement-net-*` | 2, 4 | PASS | 50/50 checks, zero FAIL |
| `final/tcp-epoll-lt-net-*` | 2, 4 | PASS | 40 registrations, 9 ready, maxevents=1, `socket_live=0` |
| `final/tcp-epoll-revalidation-net-*` | 2, 4 | PASS | stale MOD/DEL/re-ADD, identity, generation, close/reuse, capacity 64 |
| `final/tcp-epoll-external-*` | 2, 4 | PASS | 5 exchanges, 1371 bytes, counters zero |
| `final/tcp-poll-loopback-2` | 2 | PASS | counters zero |
| `final/poll-pressure-loopback-2` | 2 | PASS | 15 cycles, saturated 261820/262152 bytes, counters zero |
| `final/poll-pressure-loopback-4` | 4 | PASS | 16 cycles, saturated 255696/262152 bytes, counters zero |
| `final/poll-pressure-external-2` | 2 | PASS | 5 cycles, 16777228 bytes, counters zero |
| `final/epoll-pressure-loopback-2` | 2 | PASS | 12 cycles, saturated 259508/262152 bytes, counters zero |
| `final/epoll-pressure-loopback-4` | 4 | PASS | 11 cycles, saturated 262144/262152 bytes, counters zero |
| `final/epoll-pressure-external-2` | 2 | PASS | 5 cycles, 16777228 bytes, counters zero |
| `final/epoll-{2,4}` | 2, 4 | PASS | local epoll contract |
| `final/epoll-lifecycle-{2,4}` | 2, 4 | PASS | local lifecycle/reuse |
| `final/inet-socket-{2,4}` | 2, 4 | PASS | socket/dup/select/limits contract |
| `final/suite-*` | 2, 4 | PASS | 94/94 cases each |
| `final/abi-poll` | n/a | PASS | 12 ABI checks, unique exports, historical link |
| `final/cxx-runtime` | n/a | PASS | ABI/unwind ownership, vendored integrity |
| `cargo build -p sunlight-kernel --locked` | n/a | PASS | kernel ELF recorded |
| `cargo check -p sunlight-net -p sunlight-ipc -p sunlight-net-server` | n/a | PASS | |
| `cargo check -p sunlight-libc --features native-c` | n/a | PASS | pre-existing warnings retained |
| `git diff --check` | n/a | PASS | exit 0 |

Concurrency and lifecycle coverage in the new fixture:

- readiness created before replacement, collected after (local and TCP);
- TCP source over local target, local source over TCP target, TCP over TCP;
- `dup2(fd, fd)`, invalid source, out-of-range target;
- repeated replacement with a fresh registration per iteration;
- a worker thread performing eight replacements with an atomic
  arm/publish handshake, while the main thread collects readiness with
  zero-timeout `epoll_wait`; no timing assumption is used;
- final `net_server` socket/listener/stream counters return to zero.

## 8. Remaining classifications and exclusions

- `NOT_RUN`: direct execution of `sunlight_libc::dup2`. No shipped guest
  harness reaches the internal Rust entry point and the C ABI deliberately
  has no symbol for it. Its observable guarantees (fail closed, attachment
  released exactly once) are exercised through the raw-syscall path by the
  generation guard and `close`; the bookkeeping itself is code-reviewed only.
- `NOT_RUN`: Linux-personality `dup3` runtime. It shares
  `invalidate_replaced_fd` and compiles, but no Linux fixture performs
  descriptor replacement over an epoll registration.
- Audit finding, unchanged: `FdTable::replace_handle`
  (`kernel/src/arch/x86_64/syscall.rs:8064`, Linux AF_UNIX `connect`) swaps
  the handle under a live fd number and does not invalidate registrations.
  It is not TCP, has no userspace transport map, and needs a `Process`
  reference at the fd-table layer; it is recorded here rather than changed.
- Audit finding, unchanged: `install_at`/`install_from` during spawn target a
  freshly built child table whose only entries are the standard streams, so
  no registration exists to invalidate.
- Semantic exclusions (not claimed): Linux OFD epoll lifetime, `dup`/`fork`/
  `exec` attachment transfer, non-atomicity of a concurrent replacement
  versus an already-resolved userspace operation, and the pre-existing
  `install`-over-a-stale-entry overwrite path.
- Historical invalid invocations and failed attempts from earlier evidence
  directories were not touched.

## 9. Final state

`provenance-after.txt` records HEAD, the final `git status --porcelain` and
the diff stat. All incoming work is present; the only additions are the
files listed in section 6. No commit, no branch change, no reset, no
destructive clean.

## Final answers

1. **Reachable and reproduced.** Not through the C ABI, which has no
   `dup2`, but through the guest syscall ABI and the internal Rust entry
   point. The focused fixture failed six assertions before the fix,
   including a `read()` that returned the replaced endpoint's byte.
2. **Contract.** Kernel replacement under `SCHEDULER` is the linearization
   point: validation first, then detach of the target fd key from every
   epoll instance, then the handle swap and displaced-handle release. The
   libc map binds fd → (`net_server` id, kernel endpoint generation),
   `lookup` revalidates the generation, `close` releases the recorded
   attachment, and `dup2` releases the target's attachment and does not copy
   the source's.
3. **Root cause.** Replacement was invisible to both fd-keyed epoll
   registrations and the fd-keyed transport map, so the new identity
   inherited the old registration and the old endpoint.
4. **Files.** `kernel/src/arch/x86_64/syscall.rs`,
   `sunlight-libc/src/net_bridge.rs`, `sunlight-libc/src/lib.rs`, the new
   fixture, the new contract document, one cross-reference line, and this
   directory.
5. **Before/after.** 6 deterministic failures (2 CPUs, archive
   `f1fe2448...`) → 50/50 checks PASS on 2 and 4 CPUs (archive
   `2fb382ef...`), with counters at zero.
6. **Provenance.** Locked kernel build PASS, kernel ELF SHA-256
   `97d9bb22...`, archive SHA-256 `2fb382ef...` in both locations and stable
   across suite rebuilds.
7. **Acceptance.** 94/94 native suite on 2 and 4 CPUs; loopback,
   mixed-domain, capacity/fairness and concurrent-ctl LT fixtures on 2 and 4;
   external epoll on 2 and 4; poll/epoll pressure loopback and external;
   ABI and C++ runtime checkers.
8. **Remaining.** `sunlight_libc::dup2` direct execution and Linux `dup3`
   runtime are NOT_RUN; AF_UNIX `replace_handle` and `dup`/`fork`/`exec`
   attachment transfer and Linux OFD semantics remain excluded.
9. **Preservation.** No commit, no reset, no clean; all incoming tracked and
   untracked work, archives, fixtures, historical logs and failed attempts
   remain in place.
