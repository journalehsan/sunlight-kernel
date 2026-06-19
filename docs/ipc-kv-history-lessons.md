# IPC / KV / Calculator History — Lessons Learned

This document records bugs found and fixed during the IPC/KV/calculator-history
debugging work. Read this before touching IPC, KV, or shell history code.

---

## 1. Shell first receive: ipc_recv before reply_wait

**What broke:** sshl used `ipc_reply_and_wait()` as its very first receive call.
At startup there is no current caller to reply to, so the kernel had nothing to
deliver the reply to and the call wedged.

**Symptom:** Shell and TTY handshake froze after sshl registered. The entire
terminal was silent — no prompt, no output, keyboard did nothing.

**Root cause:** `ipc_reply_and_wait(ep, reply)` requires an *active caller* in
the current reply slot. At process start there is none.

**Fix:** The first receive must always be plain `ipc_recv(ep)`. After that first
message arrives, `ipc_reply_and_wait()` is safe because each iteration is a
reply to the caller from the *previous* loop iteration.

**Rule:** Never replace the first `ipc_recv` with `ipc_reply_and_wait`. Code
review must check this when touching server entry points.

---

## 2. Multi-caller IPC endpoint queue

**What broke:** Multiple clients (TLS, shell, timer) called the same KV endpoint
at the same time. The kernel stored only one pending caller slot, so a second
caller silently clobbered the first. Occasionally both sides ended up blocked:
the caller waiting for a reply that was never sent; KV waiting for a new message
that never came.

**Symptom:** Sporadic shell hangs. `[IPC-DIAG] stuck rendezvous` in the serial
log. All processes reported `BlockedOnIpc` with no ready queue entries.

**Root cause:** Single-slot pending-caller storage. A new caller overwrote the
slot before the previous caller got its reply.

**Fix:** A multi-caller queue was added to the endpoint bus. Each endpoint now
holds a FIFO of pending callers. If the receiver is waiting, deliver immediately;
otherwise enqueue the caller and dequeue when the receiver later calls `ipc_recv`.

**Rule:** Endpoints are many-to-one. Never store only one pending caller per
endpoint. The reply path must wake the *original* caller, not a freshly queued one.

---

## 3. Blocking ipc_call in the interactive shell path

**What broke:** Calculator history called plain `ipc_call()` to reach KV. If KV
was slow, missing, or momentarily busy, `ipc_call()` looped forever with no
timeout. tty_server calls sshl synchronously (KBD_LABEL handler), so a stalled
`ipc_call` inside sshl froze tty_server, which froze all keyboard input.

**Symptom:** Typing `= 1 + 1` caused the terminal to freeze. All subsequent
keystrokes were dropped. The shell eventually resumed if KV responded, but the
freeze duration was unbounded.

**Root cause:** `ipc_call` has no timeout. Any slow callee can freeze the caller
indefinitely. This is catastrophic when the caller is on the synchronous
tty_server → sshl call chain.

**Fix:** Calculator history now uses `ipc_call_timeout(cap, msg, KV_TIMEOUT_MS)`
with a 50 ms deadline. On timeout the history save fails silently but the
calculator result is still printed. KV unavailability never prevents the result
from appearing.

**Rule:** Any IPC call from an interactive or user-facing path MUST use
`ipc_call_timeout`. Plain `ipc_call` is only safe for boot-time server-to-server
calls where the peer is known to be alive and prompt.

---

## 4. Register IPC only transports words[0..3]

**What broke:** KV key names were encoded starting at word 2 of an IpcMsg.
`IpcMsg` has 8 logical words, but `raw_syscall_ipc` only carries words[0..4]
via r8/r9/r10/r12. words[4..7] are silently dropped by the syscall ABI.

Since keys start at word 2, only 2 words × 8 bytes = **16 bytes** of key are
actually transmitted. A key like `calc.history.0000000000001ebe` (32 bytes)
was silently truncated to 16 bytes, causing every KV lookup to fail with a
truncated key that matched nothing.

**Symptom:** `= history` always showed an empty history even though
`= 1 + 1` appeared to succeed. KV returned KV_ERROR for every GET.

**Root cause:** `IPC_MAX_WORDS = 8` but `IPC_REGISTER_WORDS = 4`. Code that
treated them as equal and used word-packed keys beyond word 4 silently lost data.

**Fix:** History keys were shortened to fit the 16-byte budget:
- `calc.hist.idx` (13 bytes) — index key
- `calc.h.<8hex>` (15 bytes) — per-record key

**Rule:** Any string packed into register words starting at offset W has
`(4 - W) * 8` bytes available, not `(8 - W) * 8`. Do not extend keys beyond
this budget without adding SHM-key opcodes (KV_PUT_SHM2 / KV_GET_SHM2).

---

## 5. KV volatile mode and best-effort persistence

**What broke:** KV failed to open `kv.store` (sunlight-sm not running, VFS not
mounted, path wrong). In some configurations KV panicked or stopped responding
instead of continuing in volatile mode.

**Symptom:** KV IPC requests timed out at boot even though the KV binary was
running.

**Root cause:** Store open failure was treated as fatal.

**Fix:** KV now logs a warning and enters volatile mode when the store is
unavailable. GET/PUT/DELETE all work from the in-memory BTreeMap. Persistence
failures (sunlight-sm errors) are logged and queued records are dropped, but the
in-memory state remains correct. Clients see no difference between persistent
and volatile mode at the IPC level.

**Rule:** KV must always reply promptly. Persistence failures must not delay or
suppress client-visible replies. The in-memory store is the source of truth for
running sessions; the log is just a durability aid.

---

## 6. SHM cleanup on timeout and error

**What broke:** If `ipc_call_timeout` returned `Timeout` after a KV_PUT_SHM
call, the SHM token allocated by the caller was not freed. Over many timeouts
this leaked SHM pages until the kernel ran out.

**Symptom:** Eventually KV_PUT returned `shm alloc failed` for all callers after
enough timeout events accumulated.

**Root cause:** SHM token free (`shm_free(tok)`) was only called on the success
path, not on the timeout/error path.

**Fix:** Every `kv_put` / `kv_get` call now frees the SHM token on all exit
paths (success, error, timeout) using an unconditional `shm_free` after the IPC
attempt. On the receiver side, KV always maps + copies + frees the page before
replying, so double-free cannot happen.

**Rule:** Every `shm_alloc` must have a matching `shm_free` on all paths.
SHM tokens received in IPC replies must be mapped, copied out, and freed before
the reply message is processed further.

---

## 7. KV IPC reply invariant

Every opcode in the KV request loop must produce exactly one reply:
- KV_REPLY (success) or KV_ERROR (failure) for mutations.
- KV_VALUE for successful GET.
- KV_ERROR for missing keys, access denied, and unsupported opcodes.

Silence (falling through without replying) leaves the caller blocked forever.
This applies equally in volatile mode and persistent mode.

---

## Regression checklist

Run these checks after any change to IPC, KV, shell, or tty_server:

- [ ] Boot reaches login prompt (no hang).
- [ ] Login as root works.
- [ ] sshl starts, prompt appears, keystrokes echo.
- [ ] `= 9 - 1` prints `8`.
- [ ] `= history` prints at least the last calculation.
- [ ] TLS can seed/load roots from KV (if TLS service is enabled).
- [ ] KV with missing store falls back to volatile mode (boots cleanly).
- [ ] No all-process `BlockedOnIpc` deadlock (check serial log for `[IPC-DIAG]`).
- [ ] No truncated calculator history keys (check KV GET returns non-empty).
- [ ] No SHM leak on timeout/error (run calc history repeatedly under load).
- [ ] No plain `ipc_call()` in calculator history (grep `ipc_call\b` in calc.rs).
- [ ] `cargo build --package sunlight-kernel` passes.
- [ ] `./tools/test.sh sunlightd` passes.
