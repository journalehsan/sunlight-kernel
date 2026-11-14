# Direct descriptor replacement runtime coverage

This supplement validates the bounded contract in
`2026-11-13_NATIVE_DUP2_REPLACEMENT_IDENTITY.md`. Earlier reports and logs
remain historical evidence. New evidence is under
`docs/audit/probes/native-foundation/results-2026-11-13-dup-runtime-gaps/`.

## Paths and semantics

| Path | Dispatch | Kernel validation / registration invalidation / replacement | Userspace attachment handling |
| --- | --- | --- | --- |
| Raw native guest dup2 | syscall 46 → `sys_dup2` | All three; invalid source/target and same-fd preserve registration | No immediate detach; subsequent `lookup` validates endpoint generation; C close detaches/releases even a stale record |
| Linux dup3 | Linux personality syscall 292 → translation -32 → internal dispatcher 1031 → `sys_linux_dup3`; native test can also invoke dispatcher 1031 | Flags and same-fd checked first, then shared invalidation and `FdTable::dup3` | No wrapper bookkeeping; native TCP fixture's existing mapping is generation-checked on later operations and released by C close |
| Rust `sunlight_libc::dup2` | Test adapter → production Rust wrapper → syscall 46 | Same kernel path | Same-fd forwards untouched; otherwise detach target first, release captured attachment on success, restore original socket id + generation on rejection |

Generation is captured before attachment's NetOp request. Lookup revalidates
against kernel endpoint generation. Restoration reinstalls the original
generation, including a stale generation, rather than blessing the current
endpoint. No path copies the source's TCP attachment.

Linux dup3 accepts flags 0 and `O_CLOEXEC=0x80000`; any other bit returns
Linux `-EINVAL` (-22). Same-fd returns -22, including with CLOEXEC. Invalid
source/out-of-range target returns Linux `-EBADF` (-9). The native dup2 raw
error is -5; Rust maps it to `Errno::BadFd`. FdTable stores O_CLOEXEC in
per-descriptor flags, independently of the shared description. F_GETFD
reports FD_CLOEXEC=1. Flags=0 clears the target's close-on-exec bit. These
bit transitions are executed; an exec lifecycle is not newly tested here.

## Harness boundaries

`tools/native-c/tests/run_dup_runtime.py` uses the ordinary locked native
archive build and target-built rlib from the same archive member. A no_std
Rust object calls the actual production wrapper and bridge lookup; it owns
no copy of the bridge. The C fixture uses ordinary `cc.py` startup, linker
script, `-nostdlib`, static native archive, and no host libc. Link maps,
disassembly, undefined-symbol checks and archive hashes are retained.

A test-only linker observer counts calls to production `release_detached`
and always forwards to that routine. It does not change the production
archive or create a public dup C symbol. Checks include C-created sockets
visible through Rust lookup, original attachment id and generation after
rejection, bidirectional routing to the original peer, exact release-call
counts, stale-generation restoration failing closed, and pre-exit descriptor
counts. TCP reaping is asynchronous; each repeated round awaits observable
reclamation with a bounded deadline before sampling the next round's global
counters. Kernel endpoint-reference and handle cleanup records are checked
separately from process exit.

The TCP dup3 fixture invokes the actual compatibility handler at dispatcher
1031 from a native process, so it can exercise the real native TCP bridge.
A separate raw Linux ELF (OSABI=3) executes syscall 292 with Linux registers
and packed Linux epoll events. It verifies personality translation and local
replacement semantics without importing native libc into a Linux process.
`guest.py --linux-actor` explicitly opts into this single non-network actor;
the standard suite continues to reject Linux images. Linux-personality TCP
libc/epoll integration is not claimed.

The focused fixtures run separately; they add no rows to the 94-row native
suite. See the evidence report for per-CPU classifications and assertion
counts. Expected errors are successful negative assertions, not product
failures. Failed harness attempts are retained and explained there.

## Continuing exclusions

No public dup/dup2/dup3 C ABI is added. No attachment transfer through dup,
fork or exec, Linux OFD epoll lifetime, or atomicity against already-resolved
bridged operations is claimed. Worker tests synchronize after replacement
returns and before checking readiness. No generation/token validation,
event.data width, lock ordering, or production behavior was changed.

AF_UNIX Linux connect's `FdTable::replace_handle` remains a separate audit
lead: it replaces a handle without this invalidation helper. It has no
reproduced dependency on these TCP tests and remains unchanged. Concurrent
attachment installation over a stale record is also outside this work.
