# Userspace VFS migration

Status: request/response codecs, operation-bound pending-operation model and service
negotiation implemented. Namespace context framing and trusted, epoch-bound kernel
service registration, kernel admission-identity capture/revalidation, kernel
namespace-record/buffer preparation, a bounded resource-owning pending table with
lifecycle retirement/explicit reap, and boot-lived remote retention bookkeeping
are implemented; authenticated retention transport, file dispatch and continuations
remain open.
Normal file syscalls still use KERNEL_VFS. No runtime file routing or namespace
ownership has changed. FS-1 kernel integration and its native IPC gate remain open.

## Roadmap phase tracking

The eight migration phases are ordered acceptance gates. Existing FS-0..FS-5
milestone names below group related work; they do not imply completion of a phase.

| Phase | Work | Current state |
| --- | --- | --- |
| 1 | Namespace-service contract | Bounded stat context and envelope defined; isolated tmpfs directory pins implemented; runtime object retention remains open |
| 2 | Authenticated kernel dispatch | Trusted registration, identity revalidation, kernel namespace-record/buffer preparation and userspace injection fence implemented; restricted mappings, publication/completion unwired |
| 3 | Syscall continuations | Bounded pending table owns adapter-provided resources; kernel continuation and scheduler integration unimplemented |
| 4 | One isolated pathname syscall | NOT RUN; no syscall selects the userspace namespace |
| 5 | Open-file lifecycle | Unimplemented |
| 6 | Production root/mount namespace | Unimplemented; current service namespace remains separate |
| 7 | Remaining namespace operations | Codecs only; execution unsupported |
| 8 | Failure handling and kernel retirement | Local admission/pending retirement and explicit reap implemented; bounded remote ownership survives replacement with lifecycle abandonment; remote drain and release reconciliation remain open |

The next native acceptance gate is a deliberately selected test process/root,
authenticated stat dispatch, and completion without KERNEL_VFS traversal. A
registration or negotiation pass cannot satisfy that gate. Root cutover follows
the open-object, boot-dependency, and failure-handling gates; never fallback from a
selected userspace namespace to the bootstrap resolver after an operation fails.

## Namespace-service v1 context contract

`ipc/src/vfs_rpc/context.rs` defines the initial **stat-only** dispatch profile.
The wider operation codecs remain available for future profiles, but context
admission rejects them with `ProtocolError::Operation`. Negotiation continues to
advertise zero executable features until kernel dispatch and server execution are
wired and qualified. Decoding a context is structural validation, not authority.

Every integer uses little-endian fixed-width encoding. The immutable input frame
starts with 14 u64 fields in order: version, request ID, service endpoint, service
owner PID, service owner incarnation, service registration epoch, caller PID,
caller incarnation, caller address-space generation, namespace ID, namespace
generation, root object reference, directory object reference, and attenuated
grant bits. Then come uid u32, gid u32, supplementary-group count u16, reserved
u16 zero, exact operation-input bytes u32, exact output capacity u32, and reserved
u32 zero. This header is 136 bytes. It is followed by at most 32 group u32s and
exactly one existing request encoding. Truncation, trailing input, unknown grant
bits, unknown versions, zero identities, and non-exact operation lengths fail.
PID zero is excluded from this userspace-service profile. Endpoint zero is a valid
allocation: native boot can create VFS's endpoint before init's endpoint. The
nameserver's zero selector convention does not reserve the numeric endpoint ID.
UID/GID zero and an empty group list are valid and convey no implicit grant bits.
Group overflow fails admission; truncating the caller's groups is forbidden.

| Boundary field/resource | Producer | Validator and owner |
| --- | --- | --- |
| Endpoint/owner/incarnation/registration epoch | Kernel trusted-service registration | Kernel checks boot role, live owned endpoint, process incarnation and VM generation; retains registration |
| Caller identity and uid/gid/groups | Kernel process records under lifecycle serialization | Kernel snapshots; server enforces search/ownership using snapshot, never an application identity claim |
| Namespace ID/generation and root/directory objects | Server allocation, explicitly selected by kernel process binding | Server checks namespace membership and service epoch; kernel retains references for the request |
| Path | Caller bytes captured once by kernel | Codec checks UTF-8, NUL, length; server traverses without kernel pre-resolution |
| Grants | Kernel admission and attenuation | Server intersects grant ceiling with permissions; uid zero does not bypass ceiling |
| Request ID | Kernel boot-scoped checked allocator | Pending table selects exact request before validating completion; never reuse after exhaustion |
| Input/output lengths | Kernel derives from operation | Frame decoder compares exact lengths with ReplyContract; service and kernel check mapping bounds |
| Input capability | Kernel owns immutable captured frame | Service read-only mapping; no application writable alias; retained until terminal cleanup |
| Output capability | Kernel owns bounded result allocation | Service write mapping, kernel immutable completion snapshot, then caller mapping revalidation |
| Reply | Bound service endpoint and registration | Kernel authenticates source first, validates envelope/body, delivers once |

Required REQUEST envelope: REQUEST label, exactly four transfer words and exactly
two capabilities. Capability 0 is immutable input; capability 1 is writable output.
The four words retain Transfer semantics: version, nonzero request ID, output
allocation offset, output allocation length. Frame and transfer request IDs must
agree. Output offset/length use checked arithmetic against retained output buffer
size. Allocation length covers `max(operation_input_bytes, output_capacity)` for
compatibility with Pending admission; frame header/group storage is separate.
The actual writable result is limited to the exact frame output capacity, even
when the allocation is larger. The receiver derives the input frame length from
the bounded header/group count and exact operation-input length, then validates
that slice, excluding any page padding. Each frame/allocation is bounded by 64 KiB.
The kernel must charge retained memory and mapping/reference quotas before send.
The current SHM transport has **not** been qualified for these attachment rules;
no executable feature may be advertised on the strength of this codec alone.

Directory references are opaque retained objects scoped to `(service epoch,
namespace ID, namespace generation)`. Root is the traversal confinement boundary;
directory is the relative-path starting object. Absolute paths start at root.
`..` stops at root, `.` retains the current directory, repeated separators are
accepted, and a trailing separator requires a directory. Every traversed parent
requires search permission, including components later removed by `..`. Mount
crossing belongs entirely to the server namespace. Missing components return
ENOENT; traversal through a non-directory returns ENOTDIR; denied search or grants
return EACCES; unsupported operation/backend semantics return EOPNOTSUPP; exhausted
admission returns an explicit resource error. Stale objects/generations return
ESTALE. Kernel pointer or textual cwd substitution is forbidden.

Stat observes metadata under one server operation, without exposing resolve then
act as a separately callable authority boundary. Native backends currently expose
no symlink objects: symlink traversal is deferred, not approximated lexically.
A later symlink-capable profile must define follow/no-follow and a finite traversal
limit before advertising support. This profile permits UTF-8 paths of 1..4095
bytes; the existing native walker has a 255-byte bound, so initial runtime
integration must explicitly reject unsupported lengths or extend the server walker
before enabling them. No existing syscall path limit changes in this slice.

Initial namespace/root/directory reference acquisition, path-scoped grant
attenuation (including private-secret/KV restrictions), backend metadata provenance
availability, and server retained-object storage are outstanding runtime gates.
In particular, the grant bit mask is an operation ceiling, not a replacement for
existing path restrictions. An isolated test namespace must contain no production
secrets and must be explicitly selected; it cannot silently become the root for
ordinary processes. Do not synthesize unavailable link counts or object identity.

Every pending request owns its retained caller/VM, namespace/directory and buffer
references until one terminal transition. A reply from another endpoint/epoch or
with an unknown request ID leaves the operation waiting. A malformed body/envelope
for the correctly identified authenticated request fails that request once. Exit,
exec, cancellation, timeout and service disconnect race through the same terminal
transition. Reply-before-suspend must be handled atomically with caller parking.
After terminal publication, detach queue/continuation references and release
input/output mappings exactly once; delivery failure must release returned remote
objects. Stat returns no remote object. Mutations are unsupported here and must
never be automatically replayed in later profiles after an uncertain outcome.

Service registration is now kernel-gated independently of the nameserver. Only
the boot-assigned `trusted_vfs_service` role may register its live owned endpoint.
The binding retains owner incarnation, address-space generation and a checked
non-reusing epoch. Re-registration of the same live identity is idempotent; another
live candidate is rejected. Exit, successful live exec and authenticated endpoint
destruction now eagerly retire the matching local registration and namespace
bindings. Liveness checks remain a backstop for a dead/execed/closed incumbent;
a replacement gets a new epoch. Ordinary
IPC calls with REQUEST are rejected even from the trusted service. File dispatch
must use a separate kernel publication path. This registration scaffold does not
yet publish file requests, authenticate file replies, or clean up pending file
resources on service death; those paths do not exist yet.

## Destination and ownership

```text
Applications
    | native and Linux-compatible file syscalls
Kernel: validate buffers, track descriptors, dispatch authenticated IPC
    |
vfs_server: namespace, mounts, permissions, filesystem operations
    |
tmpfs / FAT server -> restricted block-device interface
```

There must be one authoritative runtime namespace and one owner of each writable
filesystem. Initially tmpfs and FAT may be modules within vfs_server. Moving them
into separate processes is a later isolation step, not a prerequisite for removing
filesystem engines from the kernel.

RAMFS ownership: RamFs currently aliases Tmpfs. The authoritative writable tmpfs
will live inside the ring-3 vfs_server, not in the kernel. The service already owns
a separate instance today; FS-2 migrates the syscall-visible state and removes the
kernel instance. A separate tmpfs process is optional, not required. Bootstrap seed
bytes are read-only input, not a second runtime filesystem.

The kernel retains address-space validation/copying, authenticated process
identity and capability attenuation, descriptor/open-description lifetime,
IPC scheduling, and restricted device transport. Namespace traversal, ownership
and mode evaluation, mount policy, filesystem parsing, allocation, and filesystem
caches belong in userspace. Kernel descriptor rights remain an independent ceiling
on server permissions. Existing private-secret and KV restrictions must survive.

## Current evidence

- kernel/src/main.rs: KERNEL_VFS owns tmpfs root and FAT mounts; KernelDisk owns
  block caching and privileged VirtIO/USB transport.
- kernel/src/arch/x86_64/syscall.rs: file syscalls call KERNEL_VFS directly.
- services/vfs_server/src/main.rs: a separate ring-3 Vfs serves legacy IPC clients;
  BootFs reads a kernel-populated FAT share page, not live storage.
- sunlight-fs/src/vfs.rs: FsNode has only Ram and Fat; direct synchronous dispatch.
- sunlight-fs/src/ramfs.rs: RamFs aliases Tmpfs.
- ipc/src/lib.rs: IPC register transport carries four words, despite the larger
  in-memory message array. Existing IPC call completion can park/wake callers.

The duplicate paths must converge; merely adding a third path increases drift.
Existing SunFS requirements in sunfs/sunfs-vfs-integration.md remain applicable,
especially retained object identity, mount retirement, and storage fencing.

## Milestones and acceptance gates

### FS-0: contract foundation (started)

Added ipc/src/vfs_rpc.rs: versioned four-word bounded shared-buffer transfer and
reply validation. Reject overflow, unsupported versions, zero request identity,
stale request IDs, oversized output, malformed errno, and incomplete replies.
This is a transport primitive, not an operation ABI or a functioning RPC service.

Implemented in ipc/src/vfs_rpc/codec.rs: explicit little-endian request codecs for
open, close, read/write, stat/fstat, directory enumeration, mkdir/unlink/rename,
chmod/chown, sync and truncate. Paths are length-prefixed UTF-8 and reject embedded
NUL, empty paths and excess length. Decode rejects trailing bytes, truncation,
zero handles, unknown operations and offset overflow. Read/write offsets are
explicit; seek is managed by shared open descriptions in the kernel.

NEGOTIATE/NEGOTIATE_REPLY and REQUEST/REPLY occupy a separate label namespace.
vfs_server handles read-only negotiation and advertises **zero file-dispatch
features**. Incompatible versions and malformed negotiation fail explicitly.
The phase3.0 server test exercises this handler and requires a serial marker;
this is not a cross-process IPC test.

Implemented in ipc/src/vfs_rpc/response.rs: successful reply codecs and a retained
ReplyContract derived from the original request. Pending admission validates the
request and requires space for both the exact encoded input and maximum output.
Completion validates the REPLY label, exactly four words, zero attached capabilities,
request identity, errno, initialized snapshot length, and operation-specific body.
An authenticated malformed reply is terminal; a mismatched service identity cannot
terminate the bound operation. These checks run in the shared model, not kernel IPC.

| Operation | Successful output |
| --- | --- |
| Open | Nonzero opaque handle u64, then 56-byte metadata (64 bytes total) |
| Read | Raw initialized bytes, from zero through the requested count |
| Write | u32 bytes written, at most input data length (four output bytes) |
| Stat / Fstat | Exactly 56-byte metadata |
| ReadDir | 16-byte page header and complete variable-length entries within requested count |
| Close / Mkdir / Unlink / Rename / Chmod / Chown / Sync / Truncate | Empty body |

All integers are explicitly little endian. Metadata contains four nonzero u64
provenance fields (mount ID/generation and object ID/generation), kind u16,
permission bits u16, uid/gid/link count u32, and size u64. Zero links is valid for
an unlinked open object. These provenance fields are structurally checked only;
kernel retained-object matching and server generation allocation remain FS-1/FS-2
integration work. No timestamps or durability claims are synthesized by this ABI.

Directory headers contain next cookie u64, EOF u16 (0/1), entry count u16, and
reserved u32 (zero). Each entry contains provenance, cookie u64, kind u16, byte
length u16, and a UTF-8 name of 1..255 bytes without NUL or slash. Cookies are
monotonic opaque positions scoped to the open directory handle, not inode IDs or
byte offsets. Each exceeds its predecessor; the final cookie equals the header's
next cookie. An empty page is EOF and retains the input cookie. Invalidated
cursors require ESTALE, never silent restart. If the next entry cannot fit, return
an error (EMSGSIZE), not an empty non-EOF page. Actual server cookie state is still
unimplemented. Requests with less than 16 bytes of directory output capacity fail
admission. Dot entries remain permitted.

Open accepts the currently supported native/Linux flag bit set, including
O_LARGEFILE, and rejects unknown flags, O_DIRECT, access mode 3, and mode bits
outside 07777. Mkdir/Chmod also reject bits outside 07777. These are provisional
protocol checks; no existing syscall behavior changes in this slice.

The 64 KiB transfer ceiling includes request encoding: Write data is at most
65514 bytes because its opcode/handle/offset/length consume 22 bytes. Read output
can use the entire 65536 bytes. Input and output occupy the same bounded range in
successive phases, so payload storage needs max(input_bytes, output_capacity),
not their sum. A nonzero transport offset remains relative to the retained buffer.
The future authenticated dispatch frame must convey the exact input length and
output capacity separately from this allocation length; no frame is wired today.
The kernel must copy service output into an immutable, quota-charged snapshot
before validation, then copy only validated initialized bytes into a revalidated
caller mapping. Passing a shared service-writable slice to validation is forbidden.
Error replies contain zero output bytes. Replies carry zero capabilities; they
cannot replace the transfer capability retained by the pending kernel operation.

The namespace context above now specifies request attachment rules and dispatch
framing for stat. Remaining: runtime enforcement of those rules, authenticated
credential/grant capture, retained namespace references, supported semantic flag
combinations for future operations, and executable operation handling. Wider
request codecs remain provisional until their profiles are specified and tested.
The planned path ceiling is 4095 bytes; do not silently change existing syscall
limits. Payloads must use fixed-width fields and explicit endianness, never Rust
struct layout or kernel pointers. Separate input and output lengths per operation.
Define negotiation before enabling dispatch; legacy VfsMsg stays distinct.

### FS-1: authenticated service and pending-operation lifecycle

Implemented prerequisite in ipc/src/vfs_rpc/lifecycle.rs: allocation-free transition
model binding caller PID/incarnation/address-space generation and service endpoint,
owner PID/incarnation/mount epoch to a transfer. First terminal transition wins;
request IDs exhaust permanently instead of wrapping. Tests cover cancellation,
timeout, retirement, duplicate replies, malformed replies and stale identities.
The pending model now retains the operation's ReplyContract, rather than only a
transport byte ceiling; it holds no borrowed input path or output mapping.
This is a shared model only: it is **not yet connected to kernel process records,
resource ownership, registration, IPC dispatch, or scheduler continuations**. Its
identity arguments must eventually come from authenticated kernel records.

Source integration points inspected for the next slice: kernel/src/main.rs marks
the boot-loaded VFS process trusted_vfs_service; process/exec_authority.rs clears
that role at exec. ipc/mod.rs currently mediates nameserver registration, enqueues
generation-tagged calls, and completes authenticated reply targets into ipc_reply.
That ordinary IPC result is not a file-syscall continuation. Service names and the
existing nameserver registration alone do not establish an FS-1 root authority.
Bind registration to the trusted role, current process/VM incarnation, live owned
endpoint, and a non-reusing registration epoch. The dispatch context must snapshot
kernel uid/gid/supplementary groups and attenuated operation grants with the
request; it must not decode credentials supplied by a client into authority.

The kernel registration scaffold now binds endpoint owner incarnation, VM identity
and registration epoch; nameserver strings or process names alone grant no authority. Kernel
supplies authenticated credentials, supplementary groups and filesystem grants,
bound to the operation; clients cannot forge credentials in message words.

Add kernel pending-file-operation records retaining caller/address-space
generation, open-description reference, server/mount incarnation, request ID,
bounded transfer capability and completion state. IDs never wrap into reuse.
Snapshot input before dispatch. On completion, revalidate output mapping and caller
lifetime; never retain a raw userspace pointer across a wait. Copy out only
validated initialized bytes. Define pinning/copying and SHM quotas explicitly.

Dispatch, park, resume and cancel without holding KERNEL_VFS, scheduler, broker,
PMM or descriptor locks across a wait. Existing IPC mechanics are reusable, but
file syscall completion needs its own continuation and errno/return translation.
No recursive invocation of the user-facing IPC syscall handler.

Gate: real ring-3 echo/metadata probe, forged identity rejection, malformed reply,
timeout, server exit, caller exit/exec, output unmap and cancellation/reply races.
This gate must pass before routing ordinary syscalls.

### FS-2: shared root and file descriptions

Move mutable root ownership into vfs_server and route both native and Linux file
syscalls plus legacy IPC clients to that same state. Use an isolated boot profile
for development; choose authority before publishing root, never dual-write or
fallback to a separate filesystem after a request fails.

Keep kernel descriptor numbers separate from opaque server handles. Dup/fork
share open descriptions and offsets; last close releases the server object.
In-flight requests retain descriptions independently of descriptor-number reuse.
Preserve open flags, append atomicity, truncation, unlink/rename semantics,
permissions, directory cookies, cwd and relative paths. Inventory all direct
KERNEL_VFS consumers, including exec, dynamic loading, /proc-style synthetic data,
secret storage, capability grants and process teardown; no bypass may remain.

Gate: create/write through syscalls then read through legacy IPC and vice versa;
dup/fork offsets; concurrent append; unlink while open; permissions and capability
attenuation; directory enumeration; cancellation; application/runtime regressions.
Remove writable kernel root only after this gate passes.

### FS-3: bootstrap, exec and recovery

Load the initial VFS service from embedded read-only bootstrap bytes without
depending on its own filesystem. Supply root seed data once; no second writable
kernel mount. Define init/VFS startup dependency explicitly and reject cycles.
Subsequent executable and interpreter reads use retained server objects; snapshot
or pin bytes consistently during ELF validation/loading. File mmap and demand
paging need explicit pager/lifetime handling before removing their old path.

On service death, invalidate the service incarnation and terminate pending calls
with an appropriate I/O error. Existing descriptors remain revoked unless a
documented recovery protocol restores identity and state; restart does not make
old handles valid. No automatic resend of possibly committed mutations.

Gate: cold boot, VFS launch failure, exec during server failure, stale replies
after restart, inherited descriptors and retained mappings.

### FS-4: live FAT and restricted block access

Replace BootFs snapshot with live userspace FAT backed by capability-scoped block
requests: device/media generation, partition extent, read/write/flush rights,
checked LBA/count arithmetic, bounded DMA buffers and quotas. Server never receives
unrestricted PCI/port access. Preserve storage-admin admission and /state trust
policy. Kernel may initially retain hardware drivers and block transport.
Filesystem cache ownership moves with FAT; transport buffers stay kernel-owned.

Gate: FAT read/write/remount, flush failures, partition bounds, denied device
access, USB removal, media generation replacement and outstanding I/O retirement.
Coordinate with existing SunFS fencing work rather than bypassing it.

### FS-5: removal and optional backend isolation

Delete unused kernel tmpfs/FAT engines, namespace and filesystem caches, legacy
share-page population, and transitional routing once all consumers migrate.
Keep only the narrowly scoped bootstrap source and kernel boundary. Compare kernel
dependencies, linked image size, runtime memory, syscall latency and throughput.
Run normal boot and application gates. Then optionally separate tmpfs and FAT
servers behind authenticated mount/backend capabilities and lifecycle contracts.

## Validation and progress reporting

Each milestone reports source changes, host tests, native QEMU evidence, remaining
bypasses, and whether normal boots use the new path. Host codecs cannot establish
ring-3 execution or IPC lifecycle correctness. Keep default boot behavior until an
isolated native gate proves the replacement. Do not call the migration complete
until the old runtime ownership is removed.

Reproducible focused validation:

```sh
cargo test -p sunlight-ipc --lib vfs_rpc --target x86_64-unknown-linux-gnu --offline
cargo test -p sunlight-ipc --lib --target x86_64-unknown-linux-gnu --offline
python3 tools/test_vfs_registration_host.py
cargo check -p sunlight-kernel -p sunlight-ipc -p sunlight-vfs-server --target x86_64-unknown-none --offline
rustfmt --check --edition 2021 ipc/src/vfs_rpc.rs ipc/src/vfs_rpc/codec.rs ipc/src/vfs_rpc/context.rs ipc/src/vfs_rpc/lifecycle.rs ipc/src/vfs_rpc/registration.rs ipc/src/vfs_rpc/response.rs kernel/src/ipc/vfs_registration.rs services/vfs_server/src/main.rs
CARGO_NET_OFFLINE=true SUNLIGHT_TEST_SERIAL_LOG=target/vfs-rpc-response-phase30-serial.log bash tools/test.sh phase3.0
```

The response-contract slice passes 21 focused VFS RPC tests and all 118 IPC host
tests, plus both crate checks. Host coverage includes all reply shapes, every
fixed-size response truncation, provenance, short I/O, oversized operation output
within a larger transfer, envelope/capability rejection, malformed directory pages,
cookie non-progress/wrap, and input/output admission. Existing warnings remain in
unrelated IPC naming, libc cfg/static handling and the startup-fixture manifest.
The Phase 3.0 QEMU boot gate passed again for this response-contract slice. Serial
evidence is at target/vfs-rpc-response-phase30-serial.log, including:

```text
[VFS]  Registered as 'vfs'
[VFS-RPC] negotiation handler OK dispatch=disabled
[SunlightOS] Phase 3.0 OK
```

The negotiation marker establishes ring-3 handler execution only, never file
syscall migration, authenticated kernel dispatch, or native lifecycle race safety.
At that response-contract snapshot, the next work was FS-0 dispatch framing and
FS-1 registration/resource ownership/continuations. The context/registration slice
below advances the first two items. Resource-owning dispatch and continuations,
followed by the real IPC lifecycle gate, remain required. FS-2 root ownership
cutover remains prohibited until that gate passes.

### Namespace context and registration slice (2026-11-15)

The earlier 118-test and Phase 3.0 results above describe the response-contract
snapshot. This slice adds context framing, envelope checks and a boot-scoped
registration model, kernel trusted-role/endpoint/VM checks, ordinary IPC injection
rejection, and the unknown-request-ID non-interference fix. Unknown IDs no longer
terminally fail an otherwise unrelated pending operation; a later valid reply can
still complete it.

All 127 IPC host tests pass. Context tests cover every truncation, trailing bytes,
zero identities, unsupported operations, unknown grant/reserved bits, wrong input
and output lengths, group overflow, root UID without implicit grants, request-ID
agreement and capability/output-range envelope checks. Registration-model tests
cover idempotence, live conflicts, owner/VM/incarnation differences, replacement
epochs, delayed retirement, and malformed identities. The existing checked-ID
allocator tests cover exhaustion without wrap.

`tools/test_vfs_registration_host.py` additionally passes one production-adapter
test using mock process/broker records. It imports the actual kernel registration
adapter, exercising role/endpoint ownership rejection, live incumbent conflict,
exit, closed-endpoint and exec replacement. This is host decision coverage, not
native scheduler, locking, teardown, or IPC evidence.

The kernel, IPC and VFS service type-check; focused formatting passes. Whole-file
kernel IPC formatting retains pre-existing differences in unrelated session-race
instrumentation and existing registration/test code. Those are outside this slice.
The added Phase 3.0 service probe uses a real bounded IPC syscall to its valid
owned endpoint and requires InvalidCapability for REQUEST. Its serial marker is
`[VFS-RPC] userspace dispatch injection rejected`; this proves rejection of a
userspace publication attempt, not successful kernel file dispatch.

Native attempts are preserved separately:

1. The build preceding the injection probe passed VFS registration and existing
   filesystem checks but failed the complete gate because `timezone: zone=` was
   missing. Serial: `target/vfs-namespace-contract-phase30-serial.log`; harness:
   `target/vfs-namespace-contract/phase30-attempt1.log`. Its timezone failure's
   cause is not established by a later pass.
2. The first probe build exposed an incorrect new assumption that endpoint zero
   was invalid. The actual boot allocated endpoint zero to VFS before init
   allocated endpoint one. Registration rejected VFS and the gate failed. Serial:
   `target/vfs-namespace-contract/phase30-attempt2-serial.log`; harness:
   `target/vfs-namespace-contract/phase30-attempt2.log`. The implementation now
   accepts endpoint zero and both context/model and production-adapter host tests
   cover it. This correction does not weaken endpoint ownership checks.
3. The corrected final source passed the **complete Phase 3.0 gate**, including
   the timezone assertion, existing filesystem checks, and new injection probe.
   Serial: `target/vfs-namespace-contract-phase30-verified-serial.log`; harness:
   `target/vfs-namespace-contract/phase30-verified.log`. This run again allocated
   endpoint zero to VFS and endpoint one to init, exercising the corrected case.

Final native excerpts:

```text
[CAP] Created endpoint 0 for pid=3
[CAP] Created endpoint 1 for pid=1
[VFS-RPC] userspace dispatch injection rejected
[VFS-RPC] negotiation handler OK dispatch=disabled
[SunlightOS] Phase 3.0 OK
timezone: zone=UTC offset_seconds=0 dst=false application_point=timezone_service
```

Final implementation hashes and host/type-check logs are under
`target/vfs-namespace-contract/`. Hashes were checked after the native gate; the
earlier pre-correction manifest is retained as `attempt2-source.sha256`.
Successful authenticated file publication/completion, kernel-owned buffers and
continuations, caller/service teardown, and isolated end-to-end stat remain
**unimplemented**. No native stale-reply/lifecycle race gate has run and no ordinary
pathname syscall has migrated. The immediate end-to-end milestone remains open.

### Bounded pending-resource table slice (2026-11-15)

`ipc/src/vfs_rpc/pending_table.rs` adds an allocation-free, fixed-capacity table
around the existing operation-specific `Pending` validator. It owns an
adapter-provided resource bundle without requiring Clone or Copy. Admission
returns the bundle on failure and allocates a checked, non-reusing request ID.
The future kernel adapter must keep one table for the entire boot; rebuilding it
on service restart would reset IDs and violate this contract.

Replies select the exact request ID and validate the authenticated service
identity against that entry. Unknown IDs and wrong service identities leave
pending calls unchanged. A malformed reply from the bound service fails only
the selected operation. Caller retirement matches PID, incarnation and VM
generation; service retirement matches endpoint, owner, incarnation and epoch.
Completed operations remain terminal when retirement arrives later. Submission
failure has an explicit terminal reason and never triggers replay.

Terminal entries retain capacity and resource ownership until `reap` transfers
the bundle exactly once to result delivery or cleanup. `reap_next` drains terminal
entries without a second request-ID index and skips waiting entries. Reaping a waiting entry
is forbidden. Old replies cannot complete a new operation after slot reuse.
The adapter must reap under its lifecycle lock, then perform copy-out/resource
destruction outside that lock. Immutable output snapshots must remain owned by
the adapter through delivery; the table validates a borrowed snapshot and does
not itself capture shared-memory output.

Seven new host tests cover bounded admission (including zero capacity), ownership
return on failure, drop counts, terminal capacity retention, duplicate reaping,
slot reuse, multiple outstanding requests, wrong sources/epochs, malformed
envelopes/bodies, full caller identity retirement, successful stat metadata, and
both serialized completion/service-retirement orders, and terminal draining. All **134 IPC host tests**
pass, as does the production registration-adapter host test. Focused formatting
passes after correcting the initial module-order check failure. Logs:
`target/vfs-pending-table-host.log` and
`target/vfs-pending-table-registration.log`.

This is a shared ownership/routing component, not an authenticated transport
adapter. It trusts its caller to supply authenticated identities and retained
resources. Kernel credential capture, actual buffer/capability acquisition and
publication, reply-source authentication, teardown hooks, syscall suspension and
resumption remain unimplemented. The table accepts the existing wider operation
contracts; the runtime namespace dispatch profile remains stat-only. No new
feature is advertised and ordinary syscalls still use KERNEL_VFS. Host terminal
ordering tests are not concurrent native IPC/scheduler race evidence.

Final kernel/IPC/VFS-service type checks pass, with existing unrelated warnings;
log: `target/vfs-pending-table-check-final.log`. The full Phase 3.0 QEMU regression
gate passes; harness: `target/vfs-pending-table-phase30.log`; serial:
`target/vfs-pending-table-phase30-serial.log`. This establishes existing boot and
filesystem regression coverage, not runtime use of the new table. No native
table/continuation race gate or userspace-authoritative stat has run. Final code
hashes are recorded in `target/vfs-pending-table-source.sha256` and verified after
the QEMU gate. Earlier migration attempts above remain separate evidence.

Next integration work at the pending-table checkpoint: derive caller credentials
and registration identity under kernel lifecycle serialization; acquire and charge
immutable input/writable output capabilities and retained namespace objects;
admit into a boot-lived table and
publish through a kernel-only transport; authenticate completion against retained
IPC call identity; wire caller/service teardown and park/resume. Only then enable
an explicitly selected, isolated namespace for end-to-end stat without kernel
path resolution. Do not advertise dispatch or cut over the production root on
the strength of this shared-component slice.

### Kernel admission identity slice (2026-11-15)

`kernel/src/ipc/vfs_registration.rs` now captures an owned `AdmissionIdentity`
from kernel scheduler and capability-broker records. The caller PID must come
from syscall entry; this API is not exposed through IPC or a syscall. Under the
existing scheduler -> broker -> registry lock order, capture checks caller
liveness, nonzero PID/incarnation/VM generation, and the registered service's
trusted role, owner incarnation, VM generation, endpoint ownership and liveness.
It copies caller uid/gid and the complete registration, including its epoch.
The current process credential model has no supplementary groups; its snapshot
therefore contains an empty array. Future group support must extend this boundary
before dispatch is enabled. UID/GID zero supplies no namespace grant authority.

Registration and capture share one incumbent-liveness check. Capture lazily
retires a stale binding but never registers a replacement. Explicit registration
still allocates a new epoch. Revalidation compares the entire freshly captured
identity, including uid/gid and service epoch, so resource acquisition outside
the lifecycle locks cannot silently reuse a changed caller or service snapshot.
The future admission/publication path must revalidate while holding scheduler and
broker locks through admission and publication. A successful check followed by
unlocking does not pin a lifetime or authorize a later send. This API owns values
only; it retains no caller, VM, namespace, object, mapping or buffer references.

The production-adapter host test now covers missing registration, endpoint zero,
independent caller/service identities, absent/zero caller identities, credential
changes, caller exit/PID reuse/exec, service role loss/exit/incarnation/exec,
closed/reassigned endpoints, explicit replacement, and stale snapshot rejection
after epoch replacement. These checks use mock scheduler/broker records and do
not establish native concurrent lifecycle behavior. An initial host-test compile
failure from comparing different Result types was corrected; its log remains at
`target/vfs-admission-host-attempt1.log`. Final host logs:
`target/vfs-admission-host.log` and `target/vfs-admission-ipc-host.log`.
All 134 IPC host tests and the expanded production-adapter test pass. Kernel,
IPC and VFS-service type checks pass with existing warnings; log:
`target/vfs-admission-check.log`.

The complete Phase 3.0 QEMU regression gate passes; harness:
`target/vfs-admission-phase30.log`; serial:
`target/vfs-admission-phase30-serial.log`. This covers existing filesystem,
registration, injection-rejection and negotiation behavior, not runtime use of
the new capture API. Final implementation/test hashes are recorded in
`target/vfs-admission-source.sha256` and verified after the QEMU gate.

The capture/revalidation entry points deliberately remain unused by runtime file
dispatch. No request buffer is acquired, charged or published, no reply is
authenticated, and no task is parked or resumed by this slice. Runtime dispatch
features remain zero, normal syscalls use KERNEL_VFS, and isolated stat remains
NOT RUN. The next integration slice is retained namespace/object acquisition and
charged immutable-input/writable-output resources, followed by boot-lived table
admission and kernel-only publication with authenticated completion and teardown.

### Kernel resource preparation slice (2026-11-15)

`kernel/src/ipc/vfs_resources.rs` adds kernel-private directory records and
caller-bound namespace bindings. Each binding owns root and directory records
through `Arc`, scoped to the full service registration, namespace ID and
generation. Binding construction rejects mismatched scopes, zero identities and
unknown grants. Preparation rejects caller incarnation/VM or service registration
changes. Explicit binding/directory retirement rejects subsequent preparation
and namespace revalidation while existing requests retain their records and
input bytes. Namespace revalidation must be serialized with future namespace
withdrawal and final admission/publication; its atomic liveness checks alone do
not authorize a later send.

Stat preparation copies the existing dispatch encoding into a private input
allocation and allocates a separate zero-initialized metadata output buffer.
The input API exposes only an immutable slice; writable output cannot alias it.
A boot-lived ledger reserves payload bytes before buffer allocation and one
request/reference allowance before cloning the binding. Limits are 64 caller
identities, 16 requests and 64 KiB per complete caller identity, with a 1 MiB
global payload ceiling. Each request retains a binding containing two directory
references. Full caller identity includes VM generation; old-generation requests
remain globally charged rather than transferring their accounting to a new VM.
These are payload quotas, excluding allocator metadata and the separately
constructed namespace control records. No mapping/frame quota is claimed.
The non-cloneable charge rolls back on preparation failure and releases after
buffer/reference destruction. Moving resources into the existing pending table
keeps the charge through terminal state, reap and eventual resource destruction.

The production-adapter host harness now compiles this module alongside the
registration adapter. Both tests pass, covering frame decoding, credentials,
relative-path preservation, output zeroing/input isolation, malformed input,
stale identities, namespace mismatch/withdrawal, per-caller request/byte limits,
global bytes, caller-slot exhaustion, allocation-capacity failure, failed table
admission, and terminal/reap ownership. The initial test compile failure (reuse
of a moved test vector) is preserved in
`target/vfs-resources-host-attempt1.log`; final adapter evidence is
`target/vfs-resources-host.log`. All 134 IPC host tests pass in
`target/vfs-resources-ipc-host.log`. Kernel, IPC and VFS-service type checks pass
with existing warnings in `target/vfs-resources-check.log`; focused formatting
and Python syntax checks pass.

The full Phase 3.0 QEMU regression gate passes; harness:
`target/vfs-resources-phase30.log`; serial:
`target/vfs-resources-phase30-serial.log`. This verifies existing boot/filesystem
behavior, not runtime use of these preparation APIs or native namespace races.
Implementation/test hashes in `target/vfs-resources-source.sha256` were verified
after the gate. No isolated userspace-authoritative stat operation has run.

This implements kernel record retention and unmapped heap-buffer preparation.
It does not retain server-side objects, native mount objects, caller processes
or address spaces. No namespace binding is installed at runtime, and no object
number supplied by an application can acquire these references. The future
trusted namespace installation/retain-release handshake must establish remote
object authority and non-reusing namespace/object generations before using these
constructors. Restricted read-only input/writable-output mappings, capability
publication, mapping/frame charges and a charged immutable reply snapshot remain
unimplemented. Legacy bearer SHM is not used to approximate those protections.

Runtime dispatch remains disabled and normal syscalls still use KERNEL_VFS.
Next integrate resource preparation with table-generated request IDs (never
guessing or independently allocating an ID), trusted namespace installation and
remote object retention, restricted mappings and kernel-only publication. Final
admission must revalidate caller/service identity and namespace liveness under
their lifecycle serialization; authenticated completion, teardown, park/resume
and the isolated end-to-end stat gate remain outstanding.


### Transactional resource admission slice (2026-11-15)

`Table::admit_with` now allocates the request ID and validates the transfer and
reply contract before invoking a resource preparation callback. A full table or
invalid caller/service identity never invokes preparation. Preparation failure
leaves no entry and consumes the allocated ID permanently. Success moves the
resources directly into the pending entry with no subsequent fallible step.
The existing ownership-returning `admit` API delegates to this path and preserves
its failure contract. Terminal entries continue to consume capacity until reap.

`Resources::admit_stat` uses that callback to encode the exact table-generated ID
into the immutable input buffer, acquire quota and retain the namespace binding.
The arbitrary-ID stat constructor is now private to the resource module. Failed
namespace validation, quota acquisition or allocation cannot publish a partial
entry; partially prepared resources release through existing ownership cleanup.
This connects preparation to pending-table ownership without independently
allocating or predicting request IDs.

The adapter tests cover namespace withdrawal and quota failure after ID
allocation, empty-table rollback, exact decoded frame/transfer ID equality,
full-table accounting, submission failure and terminal reap ownership. Shared
component coverage adds failed preparation, invalid-contract callback exclusion,
monotonic IDs across failure/reap, and full/terminal capacity exclusion.
The two adapter tests and 135 IPC unit tests plus nine IPC integration tests pass
in `target/vfs-transaction-host.log` and
`target/vfs-transaction-ipc-host.log`. Focused Rust formatting checks pass.
Kernel, IPC and VFS-service type checks pass with existing warnings in
`target/vfs-transaction-check.log`. The full Phase 3.0 QEMU regression gate passes
in `target/vfs-transaction-phase30.log`, with serial output in
`target/vfs-transaction-phase30-serial.log`. This establishes existing
boot/filesystem regression coverage, not runtime use of transactional admission.
Implementation and adapter-harness hashes in
`target/vfs-transaction-source.sha256` were verified after the gate.

This remains a preparation/admission component: no boot-lived kernel table is
installed, no runtime namespace is installed, and no caller/service lifecycle
locks are acquired by this API. Future admission callers must hold lifecycle
serialization and revalidate captured identity plus namespace liveness across
admission and publication. Resource callbacks must not wait for a service or
reenter the table. Native concurrent lifecycle behavior and isolated
userspace-authoritative stat remain NOT RUN.

Runtime dispatch remains disabled. Next establish trusted namespace installation
and remote object retention with non-reusing generations, then restricted
mappings and their quotas, kernel-only publication, authenticated completion,
teardown and park/resume. The existing kernel VFS remains authoritative.

### Local trusted namespace policy slice (2026-11-15)

`NamespacePolicy<N>` in `kernel/src/ipc/vfs_resources.rs` adds a bounded,
kernel-private installation/lookup/withdrawal component. Installation checks
capacity and rejects implicit replacement for the complete caller identity. It
allocates a monotonically increasing namespace generation (also used as the local
namespace ID), consuming it even when record construction fails. Exhaustion fails
closed without wrapping. Directory and binding constructors are now private to
the resource module, so integration callers cannot choose their own namespace
generations through those constructors.

Each installed entry owns its binding and directory records. Lookup validates
the complete caller and service registration before returning a retained binding.
Withdrawal matches both caller and registration, invalidates the binding, then
releases the policy's reference; stale teardown cannot withdraw a replacement
registration's binding. Dropping the policy invalidates every remaining binding.
Pending requests retain buffers, records and quota through terminal reap even
after policy withdrawal. These checks require external caller/service lifecycle
serialization; the policy's mutable access only serializes policy operations.

Host adapter coverage now has three passing tests, including construction
failure and burned generations, capacity and duplicate exclusion, stale caller
VM/service registration, withdrawal with outstanding transactional admission,
replacement, policy destruction, and u64 generation exhaustion. All 135 IPC unit
tests and nine integration tests pass. Evidence is in
`target/vfs-namespace-host.log` and `target/vfs-namespace-ipc-host.log`.
Kernel, IPC and VFS-service checks pass with existing warnings in
`target/vfs-namespace-check.log`; the initial invocation used the wrong service
package name and was corrected to `sunlight-vfs-server`. Focused formatting
passes. The first Phase 3.0 regression gate passes in
`target/vfs-namespace-phase30.log`.

The final-source Phase 3.0 gate also passes in
`target/vfs-namespace-phase30-final.log`, with raw serial output in
`target/vfs-namespace-phase30-final-serial.log`. Implementation/adapter hashes in
`target/vfs-namespace-source.sha256` were verified after this gate. This covers
existing boot/filesystem behavior, not runtime use of namespace policy.

This component is not a runtime namespace installation or a remote object retain
acknowledgement. No boot-lived policy instance or native mount authority is
installed. A future integration must keep one policy for the whole boot (never
reset its generation sequence), revalidate caller/service identity under their
lifecycle locks, and supply root/directory objects only from an authenticated
remote retention protocol. The present object arguments are kernel-private
preparation inputs, not proof of server-side ownership. Remote object generations
and retain/release acknowledgements are still unimplemented. Fixed entry capacity
bounds installed bindings, not all records held by arbitrary kernel references;
request payload/reference accounting remains as documented above.

Runtime dispatch remains disabled and KERNEL_VFS remains authoritative. Next is
the authenticated service-side retain/release contract and its object generation
rules, followed by runtime policy/lifecycle wiring, restricted mappings and their
quotas, kernel-only publication, authenticated completion, teardown and
park/resume. Native installation/race tests and isolated userspace-authoritative
stat remain NOT RUN.

### Service retention ownership contract slice (2026-11-15)

`ipc/src/vfs_rpc/retention.rs` adds an allocation-free, bounded `Table<T, N>`
for service-owned root/directory reference pairs. The resolving adapter supplies
an owning `T`; successful retain inserts it before returning a private-field
receipt. Receipts bind the full service registration (including VM generation),
caller incarnation/VM generation, namespace identity/generation, request ID and
a monotonically increasing lease generation. The generation identifies the
retained pair, not an inode or reusable descriptor. Failed resolution burns its
generation; exhaustion never wraps. The adapter must keep one table throughout
the service registration lifetime and use a new authenticated epoch on restart.

Retain IDs must arrive in strictly increasing serialized transport order. Failed
resolution and capacity rejection consume their request IDs; replay cannot
recreate an old lease after release. Capacity/identity/replay/exhaustion failures
never invoke resolution. A lost retain acknowledgement leaves references owned
and capacity charged. Retrying retain uses a fresh request ID and may consume
another slot; adapters must reconcile known leases or retire the registration,
never free unknown outstanding references merely because of timeout.

Lookup matches the complete receipt and registration. Release drops the owned
references before acknowledging absence. Repeated releases are harmless, even
after slot reuse; a stale generation or changed scope cannot remove a replacement.
A release acknowledgement confirms absence only, not that a prior retain ever
succeeded. Dropping the table drops all remaining owned references; an adapter
must first prevent new dispatch and drain operations retaining those objects.

This is an ownership/transition contract, not a transport authentication
implementation or a completed remote handshake. `authenticated` arguments must
come from transport records under lifecycle serialization, never decoded frame
identities. There is no retain/release wire codec, service dispatcher, native
directory resolver, kernel acknowledgement consumer, or release queue yet.
The generic owning-reference tests do not establish real filesystem pinning.
The existing namespace preparation inputs remain unchanged and cannot acquire
remote authority through this table. Runtime dispatch remains disabled and
KERNEL_VFS remains authoritative.

Four new retention tests cover reference destruction, acknowledgement loss,
release retries and slot reuse, complete receipt/registration matching, failed
resolution and burned generations, replay/capacity callback exclusion, invalid
scope and permanent generation/request-ID exhaustion. All 139 IPC unit tests
and nine integration tests pass in `target/vfs-retention-ipc-isolated-final.log`;
three kernel adapter tests pass in `target/vfs-retention-adapter.log`.
The initial IPC test build failed on a missing test-only `std` import, which was
corrected. Kernel, IPC and VFS-service checks pass with existing warnings in
`target/vfs-retention-check.log`. Focused formatting passes. Implementation
hashes are recorded in `target/vfs-retention-source.sha256`.
The complete Phase 3.0 QEMU regression passes in
`target/vfs-retention-phase30.log`, with raw serial preserved in
`target/vfs-retention-phase30-serial.log`. The kernel was rebuilt after the final
source formatting change; implementation hashes were verified after the gate.
This verifies existing boot/filesystem behavior, not native use of retention.

Next connect the contract to an authenticated kernel-origin transport, define
bounded retain/release frames and kernel receipt validation, and provide real
service directory references and kernel release ownership through pending reap.
Only then wire runtime namespace installation/lifecycle, restricted mappings,
publication, authenticated completion and park/resume. Native installation/race
tests and isolated userspace-authoritative stat remain NOT RUN.

### Retention payload and acknowledgement validation slice (2026-11-15)

`ipc/src/vfs_rpc/retention/wire.rs` defines exactly 112 little-endian bytes
for retain, retained, release and released payloads. Every payload carries the
complete registration, caller, namespace and retain request identity. Lease
generation must be zero on retain and nonzero on all other operations. Endpoint
zero remains valid. Unknown operations/versions, zero identities, truncation and
trailing bytes fail closed. These are payloads only; no IPC labels, capability
layout, authenticated origin record or live service dispatcher is installed.
Negative acknowledgements and retry scheduling remain integration work.

`PendingRetain` snapshots the expected identity and validates a retained reply
against an independently authenticated service registration before returning its
receipt. Release acknowledgement validation requires the entire expected receipt,
including lease generation. Neither function establishes authentication itself;
integration must obtain the authenticated argument from transport records and
revalidate lifecycle state under locks before publishing authority. These values
are correlation snapshots, not linear resource owners or installed namespaces.
Timeout does not authorize reclaiming unknown service references. A duplicated
retain acknowledgement must not cause duplicate namespace publication.

Three new tests cover round trips through the service table, repeated releases,
all truncated lengths, trailing bytes, malformed identities and operation kinds,
every correlation field, changed service registration and cross-operation replies.
All 142 IPC unit tests, nine integration tests and three kernel adapter tests pass.
Logs: `target/vfs-retention-wire-ipc-final.log` and
`target/vfs-retention-wire-adapter-final.log`. The initial build failed because
Rust resolved the nested module relative to the explicitly pathed parent; an
explicit child path corrected it. Focused rustfmt checks pass.

This completes only the payload/correlation portion of the next dependency.
Runtime dispatch remains disabled and KERNEL_VFS remains authoritative. Remaining
work is authenticated kernel-origin transport and envelope enforcement, native
service directory pins, kernel release ownership through pending reap, boot-lived
namespace policy/lifecycle integration, restricted mappings/quotas, publication,
authenticated completion, teardown and park/resume. Native installation/race
coverage and isolated userspace-authoritative stat remain NOT RUN.

Kernel, IPC and VFS-service checks pass with existing warnings in
`target/vfs-retention-wire-check-final.log`. Source hashes for this slice are in
`target/vfs-retention-wire-source.sha256`.
The final-source Phase 3.0 QEMU regression passes in
`target/vfs-retention-wire-phase30.log`, with raw serial output in
`target/vfs-retention-wire-phase30-serial.log`. Source hashes were verified after
the gate. The gate confirms existing boot/filesystem behavior and that userspace
dispatch remains disabled; it does not execute the new retention payload path.

### Retention ownership bookkeeping slice (2026-11-15)

`ipc/src/vfs_rpc/retention/ownership.rs` adds bounded kernel-side bookkeeping
for the complete retain/release exchange. `Ownership<N>` reserves a slot and a
checked, non-reusing request ID before publication. Pending retains consume
capacity even without an acknowledgement; uncertain sends cannot be cancelled
or retransmitted through this API. Sends must follow allocation order under
external lifecycle serialization. Invalid scopes do not allocate an ID.

A validated retained acknowledgement moves one entry from pending to held and
returns its receipt once. Duplicate delivery cannot publish it a second time.
The caller must revalidate lifecycle state before namespace installation. Failed
publication must keep the held entry and arrange release. Once publication is
withdrawn and all local users are drained, explicit release moves the entry to
releasing. Repeated release returns the same frame. Only an exact authenticated
release acknowledgement frees capacity; malformed, stale, wrong-operation and
out-of-order acknowledgements leave ownership unchanged. ID exhaustion remains
permanent after the final entry is released.

This is a shared ownership state model, not a boot-lived kernel owner or a
linear directory reference. It cannot establish authentication or prove local
reference drain. Dropping the table cannot release remote references. Runtime
integration must preserve it throughout the registration lifetime, including
unknown pending leases, until acknowledged release or proven registration drain.
No registration-drain operation is exposed yet. Transport failure reconciliation,
negative acknowledgements, real directory pins, pending-reap release ownership
and lifecycle wiring remain open. Runtime dispatch remains disabled and KERNEL_VFS
remains authoritative.

Validation: 144 IPC unit tests, nine integration tests and three kernel adapter
tests pass. Kernel, IPC and VFS-service checks pass with existing warnings.
Focused rustfmt checks pass. Logs are `target/vfs-retention-ownership-ipc.log`,
`target/vfs-retention-ownership-adapter.log` and
`target/vfs-retention-ownership-check.log`; implementation hashes are in
`target/vfs-retention-ownership-source.sha256`. Native use of this ownership
model, installation/race coverage and userspace-authoritative stat remain NOT RUN.

The Phase 3.0 QEMU regression passes in
`target/vfs-retention-ownership-phase30.log`; raw serial is preserved in
`target/vfs-retention-ownership-phase30-serial.log`. Implementation hashes were
verified after the gate. This confirms existing boot/filesystem behavior, not
native execution of the new ownership model.

### Correlated retain refusal slice (2026-11-15)

The next bounded dependency adds terminal negative acknowledgements for retains
that acquire no lease. The 112-byte retention payload now also accepts opcode 5,
`Refused`, carrying the complete pending registration/caller/namespace/request
identity. Its final word is a closed refusal code (capacity=1, exhaustion=2,
resolution=3), rather than a lease generation. Unknown refusal codes fail closed;
the existing four operation encodings remain unchanged.

`Table::retain_reply` creates a success acknowledgement or one of these refusals
from the actual retain result under the same serialized operation. Identity and
replay failures return errors without a refusal. In particular, replay may refer
to a live lease whose success acknowledgement was lost; it cannot authorize
kernel reclamation. Resolution failures continue to burn the service generation
and request ID; capacity/exhaustion attempts continue to burn the request ID.

`Ownership::refused` frees only a pending entry after exact correlation and
independent service-registration authentication. Malformed, truncated,
wrong-operation, mismatched-identity and unknown-code replies leave ownership
charged. Refusals cannot clear held/releasing entries, deliver twice, clear a
later request, or make a request ID reusable. Timeouts and uncertain publication
still preserve pending ownership. The runtime adapter must produce refusals only
from the serialized service result and obtain authentication from transport
records; decoded payload fields are not authentication.

Validation: 147 IPC unit tests, nine integration tests and three kernel adapter
tests pass. Focused rustfmt checks pass. Logs:
`target/vfs-retention-refusal-ipc.log` and
`target/vfs-retention-refusal-adapter.log`. The first check invocation used an
incorrect package name (`vfs_server`) and did not check the sources; the corrected
kernel/IPC/service check is recorded separately in
`target/vfs-retention-refusal-check-final.log`.

Authenticated kernel-origin transport/envelope enforcement, real directory pins,
boot-lived ownership and registration drain, pending-reap release and runtime
lifecycle wiring remain open. Restricted mappings/quotas, kernel publication,
authenticated completion and park/resume remain subsequent dependencies. Runtime
dispatch remains disabled and KERNEL_VFS remains authoritative. Native execution
of this refusal protocol and native installation/race acceptance remain NOT RUN.

The corrected kernel/IPC/VFS-service check passes with existing warnings.
Implementation hashes are in `target/vfs-retention-refusal-source.sha256`.

The Phase 3.0 QEMU regression passes in
`target/vfs-retention-refusal-phase30.log`, with raw serial preserved in
`target/vfs-retention-refusal-phase30-serial.log`. Implementation hashes were
verified after the gate. This establishes existing boot/filesystem regression
coverage; it does not execute the new refusal exchange.

### Abandoned retain reconciliation slice (2026-11-15)

`Ownership::abandon` now records that a pending retain must never install a
namespace. It matches the request ID and complete caller/namespace scope under
external lifecycle serialization. Repeating abandonment is harmless; timeout or
abandonment never frees capacity or permits retain retransmission. A correlated
refusal can still reclaim the pending slot.

`Ownership::retained` now returns an explicit, must-use `Retained` disposition:
`Install(receipt)` for a live pending request, or `Release(frame)` for an
abandoned one. Late success atomically enters releasing state before returning
the frame, so a failed send cannot lose release ownership. No receipt is handed
out for installation in that case. Duplicate success cannot change the state.
If success wins serialization before abandonment, abandonment returns `State`;
the lifecycle adapter must withdraw publication and drain local users before
explicit release. This API cannot prove that drain or replace revalidation before
installation.

`Ownership::releases` provides an allocation-free, bounded retry inventory of
releasing entries only. Enumeration does not remove ownership, include held or
pending entries, or retransmit retains. An exact authenticated release
acknowledgement remains necessary to free a releasing slot. Unknown pending
leases whose retain replies never arrive still require proven registration drain.

Four new host tests exercise both serialized abandonment/success orders, complete
scope mismatches, malformed and stale replies, correlated refusal after
abandonment, request-ID non-reuse, failed release publication, lost release
acknowledgements, mixed-state retry inventory and exactly-once service reference
destruction. All 151 IPC unit tests and nine integration tests pass in
`target/vfs-retention-abandon-ipc-final.log`; three kernel adapter tests pass in
`target/vfs-retention-abandon-adapter.log`. Focused rustfmt checks pass. The first
IPC test run passed with new must-use warnings in tests; explicit disposition
assertions removed those warnings before the final run.

This completes cancellation reconciliation in the shared ownership model, not
runtime caller-exit or pending-reap wiring. Authenticated kernel-origin transport
and envelope enforcement, real service directory pins, boot-lived ownership and
registration drain remain prerequisites. Runtime dispatch remains disabled and
KERNEL_VFS remains authoritative. Native execution of abandonment/release and
native installation/race acceptance remain NOT RUN.

Kernel, IPC and VFS-service checks pass with existing warnings in
`target/vfs-retention-abandon-check.log`. Implementation hashes are recorded in
`target/vfs-retention-abandon-source.sha256`.

The final-source Phase 3.0 QEMU regression passes in
`target/vfs-retention-abandon-phase30.log`, with raw serial in
`target/vfs-retention-abandon-phase30-serial.log`. Implementation hashes were
verified after the gate. This establishes existing boot/filesystem regression
coverage; it does not execute the new abandonment/release path.

### Owned service directory pins slice (2026-11-15)

`services/vfs_server/src/directory.rs` adds an owning adapter for a deliberately
isolated tmpfs namespace. `DirectoryNamespace` takes ownership of the real backend;
`pin` opens root and relative starting directory under one backend lock and returns
a `DirectoryPair` suitable for `retention::Table`. These are existing
generation-tagged tmpfs handles, not copied metadata, pathname hashes or borrowed
references. Both handles and backend access stay private. Each pair retains the
backend independently of the namespace owner and closes both handles on Drop.
Failed second acquisition closes the first handle before returning an error.
The backend's 32-handle ceiling allows at most 16 pairs; no capacity is enlarged.

Acquisition accepts canonical filesystem-local paths from trusted policy only.
Non-directory objects, dot components, repeated/trailing separators and starting
directories outside the selected root fail closed. This checks acquisition
containment; it does not implement confined request traversal or caller search
permission checks. `metadata` reads both retained objects under one backend lock,
without pathname relookup; its local inode IDs are diagnostic identities, never
wire lease generations. Rename and replacement of the original pathname cannot
retarget the handles. FAT and the boot share page remain unsupported.

Three host tests execute the production adapter against real tmpfs: rename and
pathname replacement, live chmod/chown observations, backend-index shifts after
unlinking another inode, policy rejection, partial-acquisition rollback and
handle exhaustion, synthetic root ID zero, service replay/capacity checks,
wrong-registration release, lost release acknowledgements and final table drain.
The latter uses backend weak references to verify that the table owns the backend
after namespace withdrawal and destroys it after its final pin is dropped. Tmpfs
has no directory-removal operation; removed-directory traversal is not implemented
or claimed. Registration drain here is explicit table destruction, not proof of
runtime service teardown or caller quiescence.

Validation: three new service library tests, 101 filesystem unit tests, 151 IPC
unit tests, nine IPC integration tests and three kernel adapter tests pass.
Kernel/IPC/service checks and focused rustfmt pass with existing dependency
warnings. Logs: `target/vfs-directory-pins-service-final.log`,
`target/vfs-directory-pins-libraries.log`, `target/vfs-directory-pins-ipc-final.log`,
`target/vfs-directory-pins-adapter.log` and `target/vfs-directory-pins-check.log`.
The first service test attempt failed because the test expected an inode-quota
failure one allocation too early; the corrected test now verifies a retained
directory surviving a real backend-index shift instead. The original failure is
preserved in `target/vfs-directory-pins-service.log`.

The Phase 3.0 service self-test now includes a ring-3 fixture which stores real
directory pins in the service retention table, abandons the pending ownership,
processes late success as release, retains release retry ownership, drops the
namespace owner, observes pinned metadata, repeats release after a lost
acknowledgement and completes kernel-side ownership bookkeeping. Its registration
and caller identities are explicit fixture values. This is not an authenticated
exchange or runtime process binding. The final-source Phase 3.0 QEMU gate passes
in `target/vfs-directory-pins-phase30.log`, including the required serial marker:

```text
[VFS-RPC] isolated directory pins and abandoned release OK transport=fixture
```

The gate transcript preserves this marker; the temporary full raw serial log was
not retained for this invocation. Implementation hashes in
`target/vfs-directory-pins-source.sha256` are verified after the gate. Native
rename, exhaustion and runtime installation/race acceptance remain NOT RUN;
the rename/exhaustion checks above are host evidence.

Authenticated kernel-origin transport/envelope enforcement, scope-bound runtime
policy selection, boot-lived ownership and proven registration drain, pending-reap
and caller-exit wiring, restricted mappings/quotas, publication/completion and
park/resume remain open. Dispatch stays disabled; KERNEL_VFS remains authoritative.
This slice supplies real pins for the isolated tmpfs gate, not production mount
namespace ownership or userspace-authoritative stat.

### Scope-bound service policy slice (2026-11-15)

`services/vfs_server/src/policy.rs` now owns bounded trusted policy bindings and
the retention table together for one registration. Local `install` binds the
complete caller/namespace scope to a real directory pair, resolving trusted
filesystem-local paths exactly once. Retain requests select only an exact installed
scope; decoded requests cannot install policy or choose paths. Leases share
ownership of that pair, so rename and pathname replacement between installation
and retain cannot retarget it. Retaining an installed scope allocates no further
filesystem handles or heap objects. Policy and lease capacities remain separate;
the existing tmpfs handle ceiling remains unchanged.

Policy generations must increase across the whole registration. Withdrawal and
failed pin/allocation attempts cannot make a generation reusable; exhaustion does
not wrap. Active caller bindings reject implicit replacement. Withdrawal matches
the entire scope, removes policy ownership and blocks subsequent retains for that
scope. Existing leases continue to own the pair and accept exact release retries.
Dropping this component destroys its local table and bindings; that is not proof
of runtime service drain or kernel ownership reconciliation.

The local request adapter decodes only retain/release operations, checks the
independent registration argument, and delegates replay, capacity, correlated
refusal and idempotent release to the existing retention contract. Missing policy
produces a resolve refusal only after serialized retain admission proves no lease
was acquired. Wrong identities and replay errors produce no refusal. This is a
structural adapter: the registration argument still requires future authenticated
kernel-origin transport records. No ordinary IPC handler calls it.

Four new host tests cover every caller/namespace scope field, every registration
field, wrong directions and truncated frames, stale withdrawal, burned/maximal
generations, independent capacities, lost acknowledgements, replacement, and final
reference destruction. A real tmpfs rename/replacement test retains sixteen leases
after policy installation and verifies both original object metadata and unchanged
handle consumption. All seven service tests, 101 filesystem tests, 151 IPC unit
tests, nine IPC integration tests and three kernel adapter tests pass. Kernel,
IPC and service checks pass with existing warnings; focused rustfmt and changed-file
whitespace checks pass. Logs: `target/vfs-scope-policy-service-final.log`,
`target/vfs-scope-policy-libraries.log`, `target/vfs-scope-policy-adapter.log` and
`target/vfs-scope-policy-check.log`.

The Phase 3.0 fixture now routes its retain and release bytes through this policy
adapter, withdraws policy while an abandoned lease owns real pins, checks retained
metadata, retries a lost release acknowledgement and rejects a new retain after
withdrawal. Its identities remain synthetic. Native pathname replacement and
exhaustion, authenticated transport, runtime installation and lifecycle races are
not established by the fixture.

The final-source Phase 3.0 QEMU gate passes in
`target/vfs-scope-policy-phase30.log`; raw serial is preserved separately in
`target/vfs-scope-policy-phase30-serial.log`, including the required marker:

```text
[VFS-RPC] scope policy withdrawal and retained pins OK transport=fixture
```

Implementation and expected-marker hashes in
`target/vfs-scope-policy-source.sha256` were verified after the gate.

Remaining work: authenticated kernel-origin transport and envelope enforcement,
trusted runtime policy provisioning, boot-lived ownership and proven registration
drain, pending-reap/caller-exit wiring, restricted mappings and quotas,
publication/completion and park/resume. Dispatch stays disabled and KERNEL_VFS
remains authoritative.

### Boot-lived local admission retirement slice (2026-11-15)

`kernel/src/ipc/vfs_registration.rs` now owns the registration registry and a
bounded 64-entry kernel namespace policy together in one boot-lived `Runtime`.
Registration changes and namespace invalidation share one mutex. The namespace
generation allocator survives registration replacement; no epoch transition
reconstructs it. This is the kernel preparation policy, not the service's real
directory-pair policy. Production namespace installation remains unwired, so the
boot-lived policy stays empty in ordinary images.

The scheduler now withdraws exact caller identity at self termination, external
termination and fatal-signal termination, with idempotent backstops at group
endpoint closure and deferred resource reap. Successful live exec retires the old
VM identity at the existing image-commit boundary, after image preparation
succeeds and before replacement activation. Failed preparation does not execute
this hook; unpublished spawn does not execute it either. Authenticated endpoint
destruction retires only the matching service endpoint and complete owner
identity while the scheduler lock remains held. No lifecycle hook accepts a
decoded wire identity or performs IPC.

Caller retirement invalidates only that caller's local bindings. Exact service
retirement invalidates every local binding for that registration. Stale events
cannot remove bindings belonging to another caller incarnation/VM or service
registration. Local namespace records already owned by preparation or pending
resources remain allocated but fail admission revalidation. Lifecycle hooks do
not drop pending buffers, release their quota charges, or fabricate remote release
acknowledgements. Pending terminal transition/reap and remote lease reconciliation
still require future integration.

Host coverage executes the production registration, namespace and resource
components with mock process/broker records. Five adapter tests cover eager
retirement through the boot-lived entrypoints, every endpoint/owner identity
field, caller identity mismatches, unrelated callers, service replacement,
idempotence, and pending resource/charge retention through explicit terminal reap.
Namespace generations continue increasing across registration changes. This does
not execute the real scheduler's termination paths or live exec/endpoint syscalls.

Validation: five kernel adapter tests, seven service tests, 101 filesystem tests,
151 IPC unit tests and nine integration tests pass. Kernel/IPC/service compilation
checks, focused rustfmt and changed-file whitespace checks pass with existing
dependency/service warnings. Logs: `target/vfs-runtime-lifecycle-adapter-verified.log`,
`target/vfs-runtime-lifecycle-service.log`,
`target/vfs-runtime-lifecycle-libraries.log`,
`target/vfs-runtime-lifecycle-ipc-integration.log` and
`target/vfs-runtime-lifecycle-check.log`. An intermediate harness compile error
used a unit-result error variable in two admission-result assertions; it is
preserved in `target/vfs-runtime-lifecycle-adapter-final.log`. The corrected
assertions use the explicit admission error and the five tests pass.

The final-source Phase 3.0 QEMU regression passes in
`target/vfs-runtime-lifecycle-phase30.log`; raw serial is preserved in
`target/vfs-runtime-lifecycle-phase30-serial.log`. Implementation and host-adapter
hashes in `target/vfs-runtime-lifecycle-source.sha256` were verified after the
gate. Existing service fixtures still use synthetic identities and
`transport=fixture`. This gate proves boot/filesystem regression coverage, not
VFS-specific native retirement or pending-resource cleanup. Native exit/exec/
endpoint-replacement races with populated policy remain NOT RUN.

Remaining work: authenticated kernel-origin transport and envelope enforcement,
trusted runtime provisioning, registration-lived service policy and kernel remote
lease ownership, proven remote registration drain, pending terminal/reap and
caller-exit release reconciliation, restricted mappings and quotas,
publication/completion and park/resume. Dispatch stays disabled; KERNEL_VFS
remains authoritative.

### Boot-lived pending terminal and explicit-reap slice (2026-11-15)

The same kernel `Runtime` now owns a bounded 64-entry `Table<Resources, 64>`.
Request IDs survive caller and service replacement for the entire boot. Local
stat preparation revalidates the process/credential/registration snapshot under
scheduler and broker lifecycle serialization, selects the exact installed
namespace, and transactionally frames resources with the table-generated ID.
No production namespace provisioning or request publication is wired; ordinary
images leave this table empty. Preparation currently allocates bounded local
heap buffers under lifecycle/runtime serialization; it does not wait for IPC.
Any future mapping or remote-wait integration must revisit that preparation path.

Existing exit, successful-exec and endpoint-destruction hooks now terminalize
matching waiting entries in addition to withdrawing local namespace admission.
Caller retirement matches all three caller identity fields. Registration
retirement validates the entire registration before projecting it to the pending
table's epoch-bound service identity, including checking the service VM generation.
Repeated retirement and stale replacement events preserve the first terminal
result. Lazy service-liveness failure uses the same retirement operation.

Terminal entries retain their table slots, buffers, namespace references and
quota charges. The kernel-private `reap_next` transfers one terminal resource
owner after releasing the runtime mutex; its caller must also release scheduler
and broker guards before delivery/destruction. Reap frees a table slot, while
the returned owner retains its charge until destruction. No lifecycle hook
implicitly reaps or fabricates a remote release acknowledgement. This cleanup
contract covers only local, unpublished preparation resources; remote leases and
published mappings will require their own release/drain barriers.

Seven adapter tests pass, including two new production-component tests for
populated pending-table retirement, all caller/endpoint/registration identity
fields, replacement immunity, monotonic IDs, first-terminal-result preservation,
late completion rejection, retained references/charges, exactly-once reap, and
64 terminal entries blocking admission until reap. The harness also checks the
kernel-private preparation entrypoint rejects stale credential/process snapshots
and a valid identity without installed namespace authority. Test ledger assertions
are serialized so concurrent host tests cannot corrupt accounting observations.
These tests use mock process/broker records and local runtime instances; they
do not execute native scheduler lifecycle races or authenticate IPC.

Validation logs: `target/vfs-pending-runtime-adapter-final.log` (seven adapter
tests), `target/vfs-pending-runtime-libraries.log` (seven service, 101 filesystem
and 151 IPC unit tests), `target/vfs-pending-runtime-ipc-integration.log` (151 IPC
unit and nine integration tests), and `target/vfs-pending-runtime-check-final.log`
(kernel/IPC/service compilation with existing warnings). Focused rustfmt and
changed-file whitespace checks pass. The initial adapter compile failure used a
zero-length group scratch buffer where decoding requires `MAX_GROUPS`; it is
preserved in `target/vfs-pending-runtime-adapter.log`. The corrected final harness
passes without changing production group semantics.

The final-source Phase 3.0 QEMU regression passes in
`target/vfs-pending-runtime-phase30.log`; raw serial is retained in
`target/vfs-pending-runtime-phase30-serial.log`. Implementation, harness and
expected-marker hashes in `target/vfs-pending-runtime-source.sha256` were verified
after the gate. The existing service fixtures still use synthetic identities and
`transport=fixture`. This is boot/filesystem regression evidence; native
exit/exec/endpoint races with populated pending state remain NOT RUN.

Authenticated kernel-origin transport/envelope enforcement, trusted runtime
provisioning, registration-lived service policy and kernel remote-lease ownership,
remote drain, caller-exit remote-release reconciliation, restricted mappings and
quotas, publication/completion, park/resume and native lifecycle-race validation
remain outstanding. Dispatch stays disabled; KERNEL_VFS remains authoritative.

### Boot-lived remote retention ownership slice (2026-11-15)

The kernel `Runtime` now reserves a registration-lived `Ownership<64>` for every
trusted service registration. Four bounded registration slots include the active
service and retired epochs with unresolved leases. Capacity is admitted before
changing the registry. If all four slots remain occupied, replacement registration
fails closed; no old ownership is evicted. Idempotent registration preserves the
same owner and retention ID allocator even when its lease table is empty.
Retention IDs are registration-scoped; local RPC IDs remain boot-scoped.

Caller lifecycle retirement now abandons every pending retain matching the full
PID/incarnation/VM identity across all owned registrations and namespace scopes.
Service retirement abandons every pending retain in the exact registration and
closes new retain preparation for that epoch. The existing authenticated endpoint,
exit/successful-exec and lazy service-liveness retirement paths all use this
operation. These hooks perform bounded bookkeeping under existing serialization;
they send no IPC and infer no remote drain from endpoint destruction or VM exit.

Retired owners remain available to the private reply-correlation helpers. A late
success for an abandoned retain enters releasing state before returning its frame;
lost sends/acknowledgements leave release ownership retryable. A correlated refusal
or release acknowledgement can clear its exact obligation. Only an empty retired
owner is collected. Held and releasing leases survive caller/service retirement:
held leases still require withdrawal and independent local-user drain before
release. Local pending-request reap neither clears these obligations nor proves
that drain. Unknown pending leases can exhaust registration capacity indefinitely
until a future authenticated registration-drain mechanism is implemented.

The new runtime helpers are kernel-private preparation/correlation components,
not transport authentication. Preparation still requires future trusted scope
provisioning and lifecycle revalidation; receipt installation is not wired to
`NamespacePolicy` or pending resource cleanup. Reply identity arguments must come
from future independent transport records, never decoded payloads. Ordinary images
create empty ownership bookkeeping on trusted registration and send no retains.
No ordinary IPC route or filesystem dispatch is enabled.

Eleven host adapter tests pass. Four new tests cover mixed pending/held/releasing
states, complete caller/registration identity matching, multiple namespace scopes,
late replies across service replacement, stale/malformed/duplicate replies,
idempotent registration and non-reused per-epoch IDs, all 64 lease slots, all four
registration slots, and exact acknowledgement/refusal reclamation. Tests execute
production runtime and shared retention components with mock scheduler/broker
records and supplied reply identities. They do not authenticate IPC or execute
native lifecycle races. Logs: `target/vfs-remote-runtime-adapter-final.log`,
`target/vfs-remote-runtime-libraries.log` (seven service, 101 filesystem and 151 IPC
unit tests), `target/vfs-remote-runtime-ipc-integration.log` (151 IPC unit tests and
nine integration tests), and `target/vfs-remote-runtime-check.log` (kernel, IPC and
service compilation with existing warnings). Focused rustfmt and changed-file
whitespace checks pass. `target/vfs-remote-runtime-adapter.log` preserves the first
passing ten-test run before the additional lease-capacity test.

The final-source Phase 3.0 QEMU regression passes in
`target/vfs-remote-runtime-phase30.log`; raw serial is preserved in
`target/vfs-remote-runtime-phase30-serial.log`. Trusted VFS registration succeeds
with empty remote ownership in this image. Implementation/harness/expected-marker
hashes in `target/vfs-remote-runtime-source.sha256` were verified after the gate;
kernel/ISO hashes are in `target/vfs-remote-runtime-image.sha256`. Existing service
fixtures still report `transport=fixture`. This is boot/filesystem regression
evidence, not a native remote-retention exchange or populated lifecycle-race test.

Authenticated kernel-origin transport/envelope enforcement, trusted provisioning,
registration-lived service runtime policy, receipt-backed namespace installation,
held-lease/local-reference drain and pending-reap release reconciliation, proven
remote registration drain, restricted mappings/quotas, publication/completion,
park/resume and native lifecycle races remain outstanding. Dispatch stays disabled;
KERNEL_VFS remains authoritative.
