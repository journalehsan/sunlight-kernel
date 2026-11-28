# VFS-C2F2 generic cross-authority cwd handoff

**PARTIAL.** The opt-in `vfs_c2e_candidate` routes ordinary cwd changes
across the `/root/Downloads` boundary. Both directions pass the host
integration test. Ordinary boot is **BLOCKED** by the ownership guard, so
native and Linux guest qualification is **NOT_RUN**.

## Entry and exit transactions

Native chdir (63) and Linux chdir (80, translated to 63) share the generic
adapter. The C2F1 adapter explicitly rejected Downloads entry with error
95 after joining the legacy cwd and pathname. That source rejection was
confirmed before editing. Its pre-change syscall reproduction is
**NOT_RUN** because the ordinary boot gate stops before guest launch.

For legacy entry, the legacy component walker checks traversal and pathname
authorization up to the reserved mountpoint and passes the canonical
boundary plus the unwalked suffix to the existing copied continuation.
This supports absolute entry and relative entry from the legacy parent.
The service requires its Downloads mount, checks search access, and pins
the resulting directory and root. This works with a new or an existing
caller scope and uses the regular caller identity and credentials.

For exit, the service checks the old retained root and cwd in its mounted
namespace. A relative walk follows the pinned object through renames and
stops at the first legacy component. An absolute legacy pathname is
anchored at the authenticated, unrebased root. The service supplies a
distinct boundary result alongside a provisional root-only receipt. The
kernel resolves the boundary path in the reserved legacy namespace, checks
restrictions and search access, and validates a temporary directory pin.
The kernel rejects any traversal that reaches the migrated reservation
again. Service lookup errors, stale epochs, missing authority and retired
callers all take the error path.

For both directions the finish authenticates the service registration and
exact Retain, checks caller and waiter incarnations, credentials and prior
scope, and installs the replacement binding. It commits the cwd and routing
state to the scheduler's shared owner, then reconciles the old lease.
During exit the temporary legacy pin spans that commit under the legacy
namespace lock. The legacy cwd uses its canonical-path representation
after commit. The service keeps its root-only pair for existing directory
descriptions and independent absolute Downloads operations. That pair
holds two root handles and drains at caller retirement. The old pair
drops its Downloads directory pin through the existing release exchange
after the last local user drains. The parked wait holds no scheduler,
capability broker, registration or filesystem guard.

A rejected publication restores the service's saved binding and settles the
provisional receipt with the C2F1 compensation path. Failure to resolve the
legacy destination follows that same path and preserves the old service
cwd. Missing service admission leaves a service-owned cwd routed to the
service. Timeout, caller retirement and registration retirement retain
their existing cancellation and settlement paths.

## Source changes

| Source | Change |
| --- | --- |
| `kernel/src/arch/x86_64/syscall.rs` | Generic native and translated Linux chdir dispatch, legacy authorization and pins, and raw relative open, stat and namespace mutation routing. |
| `kernel/src/ipc/vfs_registration/chdir.rs`, `kernel/src/ipc/vfs_registration.rs`, `kernel/src/ipc/mod.rs` | Entry and exit in the existing authenticated continuation, exact prior-scope validation, receipt compensation and shared cwd publication. |
| `kernel/src/process/mod.rs`, `kernel/src/process/spawn.rs`, `kernel/src/ipc/vfs_registration/spawn.rs` | Downloads cwd routing state, including exact inherited namespace launch. Thread creation carries the fail-closed route if its shared owner goes away. |
| `ipc/src/vfs_rpc/chdir_transport.rs` | Copied entry and retained-cwd operation kinds and an explicit legacy boundary status. |
| `sunlight-fs/src/vfs/{lookup,anchored,attachment}.rs`, `sunlight-fs/src/vfs.rs` | Boundary handling in the existing component walkers and pin validation at commit. The attachment walker uses the same callback interface. |
| `services/vfs_server/src/{directory,policy}.rs`, `services/vfs_server/src/runtime/control.rs` | Mounted Downloads validation, scoped directory replacement and root-only retention on exit. Relative operations stay within the mounted subtree while that cwd is active. |
| `kernel/src/ipc/vfs_registration/*_tests.rs`, `tools/test_vfs_registration_host.py` | Separate host namespace fixtures, shared-owner process records and copied-channel integration coverage. |

## Qualification

The host matrix uses the same route selection, registration continuation,
service control loop and legacy lookup as the kernel adapter. It checks the
Linux syscall-number translation and passes paths to the shared routing
function with host scheduler records. It covers absolute and relative entry,
dot-dot and absolute exit, repetition, relative create and stat across a
retained rename, post-exit directory-relative stat, rejected entry and exit,
service loss, shared cwd commit, and caller retirement while the service
remains live. The test ends with zero host backend handles and no outstanding
retains. The shared worker test qualifies cwd publication and resume. Relative
file and metadata operations still require the existing per-caller service
admission, so a sharing worker without its own admission fails closed.

The inventory is **PASS** at 71 references before and after. Comparing the
multisets of file, category and exact source text yields zero added, removed,
or relocated references. The unchanged no-growth and final-removal checks
are **FAIL**. The four historical unmatched sites are the FAT constructor in
`storage::mount` (dd00fa775), the legacy topology lookup in
`linux_procfs::mountinfo_snapshot` (dd00fa775), legacy capacity in
`sys_linux_statfs` (dd00fa775), and the private mmap lookup in
`process::mmap::sys_mmap` (5b8579f1d).

The C2C GPT fixture passed its existing SHA-256 check. The guarded boot
attempt stopped at the ownership check before QEMU launch. The native
and Linux syscalls, guest exit and exec settlement, and the historical
storage, mmap and descriptor guest replays are **NOT_RUN**.

See the [machine-readable results](vfs-generic-cross-authority-cwd-results.json).
