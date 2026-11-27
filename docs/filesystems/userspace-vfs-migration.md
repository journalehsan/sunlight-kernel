# Userspace VFS migration

## Namespace consistency staging (2026-11-27)

The [namespace audit](vfs-namespace-consistency-audit.md) found five unmatched
references. [VFS-N1 ownership closure](vfs-ownership-closure.md) reconciles
the existing bootstrap constructor; the four historical dependencies still
stop ordinary validation before QEMU. Historical component and candidate
results do not clear this gate.

[N1A: authoritative ProcFS mount snapshots](vfs-authoritative-mount-snapshots.md)
leaves the ProcFS finding open. Before switching its reader, make mount
publication, source changes and withdrawal advance one caller-visible topology
revision at the live owner. Bind that revision to a retained caller view and
validate it through FD installation. Registration alone cannot publish the
kernel's FAT mounts. [Qualification results](vfs-authoritative-mount-snapshots-results.json)
separate existing host contracts from the unimplemented snapshot request.

[N1B: retained positional file reads](vfs-positional-file-reads.md) adds a
bounded `PREAD` on the service-owned description, qualified on the host with
the shared cursor, rights, rename/unlink identity and retirement cleanup
preserved ([results](vfs-positional-file-reads-results.json)). D2A consumes
that primitive in the isolated native fixture. The historical private mmap
still reads through [the kernel VFS](../../kernel/src/process/mmap.rs#L277),
so the ownership finding stays open. Production routing still requires
ordinary descriptor admission and coordinated cutover.

[N1C mount authority and coordinated storage handoff](vfs-mount-authority-storage-handoff.md)
defines one service-owned attachment graph and a kernel-enforced device lease.
Runtime FAT initializes the complete existing mount record at insertion.
The four historical findings remain open.

1. **Qualify service attachment and exclusive extent ownership separately.**
   [N1D](vfs-service-attachment-transaction.md) qualifies volatile attachment,
   withdrawal, retained views and the snapshot installation fence at the
   existing service owner ([results](vfs-service-attachment-transaction-results.json)).
   [N1E](vfs-exclusive-device-lease.md) qualifies whole-device acquisition,
   drain, flush, transfer, release and fail-closed retirement using a host fake
   transport ([results](vfs-exclusive-device-lease-results.json)).
   [N1F](vfs-authenticated-device-admission.md) qualifies host admission
   through live registration bindings, bounded byte extents, operation
   rights and media generations while retaining whole-device exclusivity
   ([results](vfs-authenticated-device-admission-results.json)).
   [N1G](vfs-transport-dma-settlement.md) qualifies separate caller credits,
   retained transport buffers, and confirmed barriers in the host fixture
   ([results](vfs-transport-dma-settlement-results.json)).
   [N1H](vfs-virtio-settlement-qualification.md) qualifies ordinary typed
   settlement and buffer reuse on one isolated QEMU VirtIO device.
   [N1I](vfs-live-virtio-lease-integration.md) binds ordinary I/O and
   handoff on that device to the existing lease authority with fixture
   registrations ([results](vfs-live-virtio-lease-integration-results.json)).
   [N1J](vfs-live-endpoint-lease-binding.md) qualifies one live authenticated
   endpoint, repeated lease generations and exact image exit with transport
   settlement on the disposable queue
   ([results](vfs-live-endpoint-lease-binding-results.json)).
   [N1K](vfs-lease-attachment-integration.md) locates the next integration
   boundary: give the existing N1D transaction an authenticated device
   backend with readiness pinned to the exact reservation through
   publication, then couple mount retirement to drain, confirmed flush
   and lease release ([results](vfs-lease-attachment-integration-results.json)).
   [N1L](vfs-device-backed-attachment.md) connects N1D to FAT32 over the
   existing authenticated lease authority on a disposable, RAM-resident host
   sector image ([results](vfs-device-backed-attachment-results.json)).
   Actual FAT operations and host withdrawal pass.
   [N1M](vfs-native-authenticated-block-io.md) qualifies copied block commands
   from the registered ring-3 service on the isolated VirtIO device
   ([results](vfs-native-authenticated-block-io-results.json)).
   [N1N](vfs-native-fat32-attachment.md) separates FAT sync and transport
   settlement from the topology lock on the host and makes N1D transaction
   validation available to the native service behind an opt-in feature.
   [N1O](vfs-native-fat32-adapter-completion.md) adds the bounded disposable
   FAT grant, native readiness through N1D publication, and separate copied
   settlement and release. The ring-3 service passes four isolated QEMU mount
   cycles with persistent contents and a fixed retained DMA allocation
   ([results](vfs-native-fat32-adapter-completion-results.json)). The closed
   image passes with optional FSInfo hints unknown.
   [N1P](vfs-fat32-fsinfo-consistency.md) closes ordinary native FAT32
   qualification on disposable storage. The writer durably invalidates
   primary and backup FSInfo hints before FAT mutation. Populated and
   unknown fixtures each pass four native cycles, read-only filesystem
   checks and full image comparison
   ([results](vfs-fat32-fsinfo-consistency-results.json)). Production
   ownership cutover and crash recovery remain separate gates.
   Live successor handoff is a
   coordinated cutover gate; physical DMA reclamation is a transport
   destruction gate. Neither is needed for the isolated cycle while the
   live transport retains its fixed allocation and uncertain ownership
   stays fenced. Production still needs authenticated submission, alias
   and media fencing, fault settlement, FAT retained objects and capacity
   pinning, and the kernel FD installation fence. Production routes remain
   disabled.
2. **Stage the cutover prerequisites before ordinary native acceptance.**
   The unchanged guard blocks ordinary boot on live FAT, statfs, ProcFS
   and mmap dependencies. [VFS-D1A](vfs-service-descriptor-lifetime.md)
   host-qualifies the epoch and compensating Close components.
   [VFS-D1B](vfs-native-descriptor-lifecycle.md) qualifies authenticated
   pre-dispatch binding, native FD installation, duplicated cursor and
   positional reads, exact full-table compensation, and exit and exec
   cleanup in the isolated caller. Kernel and service resources returned
   to baseline after each cycle
   ([results](vfs-native-descriptor-lifecycle-results.json)).
   [VFS-D2A](vfs-service-private-mmap.md) qualifies eager private mmap
   in the isolated native caller on one, two, and four vCPUs using the
   retained description and bounded PREAD. Live retirement during a pending
   native mmap and the historical kernel ownership finding remain open.
   Authenticated pre-dispatch binding for both ordinary launch adapters
   and cross-caller descriptor handoff remain open.
   Keep broad L1C-3 blocked and production VFS-D1 deferred at this boundary. In a
   future coordinated candidate, remove the FAT/ProcFS/statfs dependencies
   together and remove the mmap dependency through retained positional I/O.
   Run the unchanged guard before ordinary boot. Reproduce the matched native
   matrix only after the guard permits it, distinguishing pre-publication
   errors from authenticated ENOENT.
3. **Qualify /vfs-scratch, a new volatile data-only mount.** Build a complete service operation
   contract off the production launch path. Switch all exposed operations for
   the mount together, with one live mount generation and no kernel fallback.
   Require ordinary native and Helios agreement after mutation, including
   across processes and IPC; require consistent execution rejection for a
   data-only fixture or authoritative loader and mapping support for an
   executable mount. Verify retirement fails closed.
4. **Expand by mount.** Move root only when its policy, directory, loader and
   mapping consumers are covered. Move FAT only with one live storage owner
   and capacity, topology, sync and retirement contracts. Keep immutable
   bootstrap and virtual endpoints explicit. Remove obsolete engines only
   after acceptance on the primary tree.

Preserve the compressed boot module, adaptive document/TTY/TLS heaps,
eager default main stacks, 16 MiB kernel heap, PMM accounting and ZRAM policy.

## Active cutover plan (2026-11-16)

This plan supersedes the sequencing below where it makes later hardening a
prerequisite for cutover. Completion means one runtime VFS in ring3 and no
duplicate namespace or filesystem engine executing in ring0. Existing behavior,
authorization and resource cleanup must survive the move. Advanced recovery,
separate filesystem processes and optimization are follow-up work.

1. **Inventory and deletion gate (started).** Run
   `python3 tools/audit_vfs_cutover.py` to enumerate kernel namespace owners,
   direct users and filesystem constructors. Initial inventory: 68 references,
   including boot FAT parsing, storage helpers and native test fixtures.
   `--check` must pass at completion. This conservative source audit includes
   test-only code and needs manual review; it does not prove runtime routing.
2. **Make the existing service namespace usable by syscalls.** Connect trusted
   caller root/cwd policy to the service's actual mounted namespace, rather than
   creating another isolated filesystem. Implement one authenticated stat round
   trip with caller park/resume and output copying. Verify it natively without
   kernel traversal. Keep the ordinary service loop working during this stage.
3. **Move file and descriptor operations.** Route open/create, read/write,
   seek/stat, truncate, close and duplication through service-owned open objects.
   Preserve offsets, rights, unlink lifetime and exit/exec cleanup. Route special
   secret/KV policy through the same namespace without weakening access checks.
4. **Move remaining namespace and storage ownership.** Transfer directory,
   metadata, mount and sync operations; put FAT parsing/cache ownership in ring3
   over the device interface. Replace the kernel-generated boot file view.
   Preserve existing root seed, fstab and persistent-volume behavior.
5. **Remove kernel consumers and engines.** Migrate exec loading, scheduler
   cleanup, native guest fixtures and storage tests. Order startup so the VFS
   service boots from immutable image bytes before filesystem-dependent clients.
   Delete KERNEL_VFS, its initialization, kernel filesystem constructors and
   obsolete dispatch paths. Do not mask unsupported operations with a kernel
   fallback. Keep device transport and descriptor bookkeeping in ring0.
6. **Accept the cutover.** Run the source removal gate, kernel/service builds,
   native filesystem and exec regressions, persistent-storage checks and existing
   legacy IPC client checks. Demonstrate shared state between syscall and IPC
   clients in the single service namespace. Record failures and remaining
   hardening separately; no zero-feature negotiation or empty-channel test
   counts as a completed migration.

Required failure behavior for cutover: a dead/unavailable service fails or
terminates affected requests without reviving a kernel filesystem, leaking their
resources or leaving callers parked forever. Automatic service restart,
cross-restart reconciliation, remote drain optimization and expanded adversarial
race matrices are subsequent hardening work.

The boot-loaded init image is now explicitly selected for a metadata-only root
and cwd policy in the shared service mount table. The populated authenticated
handoff passes natively. Native `StatPath` (61) now routes through the service
for this selected caller, with kernel-side park/resume and the existing native
metadata/error ABI. Routing stays selected after admission withdrawal or exec,
so unavailable or revoked service authority cannot restore kernel traversal.
Selected Linux stat/lstat/newfstatat now share this route for absolute paths and
AT_FDCWD-relative paths. Unselected callers and other ordinary file syscalls
still use KERNEL_VFS.
Selected callers now provision their initial cwd through an authenticated
snapshot and service-owned directory pins. Copied authenticated chdir now
replaces pins transactionally for selected callers,
with kernel continuation and synchronized shared-process cwd commit. Selected
UID-zero callers can narrow their service metadata root through native
`VfsNarrowRoot` (182), resetting shared cwd to `/` through the same authenticated
commit. Selected native and embedded/session spawn callers and successful exec
now derive independent metadata bindings from an exact retained source lease.
Service-local regular-file descriptions now own handles and shared offsets.
Distinct authenticated copied exit/exec barriers now tear down file owners while
metadata withdrawal preserves established descriptions. File syscall dispatch
remains outside the authenticated copied profile.

Status: authenticated registration and copied retention, trusted selected-root
provisioning, retained-object spawn/exec inheritance, service-mounted metadata
lookup, caller-owned publication and completion, guarded copyout, and
exit/exec/cwd/credential withdrawal are implemented. Native StatPath uses this
path for selected callers. Thread-specific provisioning, broader syscall
routing, copied open-object dispatch and syscall descriptor integration, boot/storage
migration and kernel filesystem deletion remain open.

## Roadmap phase tracking

The eight migration phases are ordered acceptance gates. Existing FS-0..FS-5
milestone names below group related work; they do not imply completion of a phase.

| Phase | Work | Current state |
| --- | --- | --- |
| 1 | Namespace-service contract | Stat context, codecs and real service directory pins implemented |
| 2 | Authenticated kernel dispatch | Selected-root provisioning, copied stat publication/completion and lifecycle delivery implemented |
| 3 | Syscall continuations | Copied pilot and native StatPath park/resume, guarded copyout and retirement implemented |
| 4 | One isolated pathname syscall | Selected native StatPath and Linux pathname stat route through the service; other callers remain on kernel paths |
| 5 | Open-file lifecycle | Service-local regular-file descriptions and copied owner teardown implemented; copied file operations and syscall descriptors remain open |
| 6 | Production root/mount namespace | Selected initial cwd, copied live chdir, metadata-root narrowing and spawn/exec inheritance use service directory pins; thread-specific provisioning and full cutover remain open |
| 7 | Remaining namespace operations | Codecs only; execution unsupported |
| 8 | Failure handling and kernel retirement | Stat timeout/service-loss and exit/exec/cwd/credential withdrawal pass; full kernel ownership retirement remains open |

Service-side retained-object caller derivation and root narrowing are implemented
as trusted lifecycle primitives. Root narrowing uses an authenticated copied
kernel continuation for selected UID-zero callers. Kernel spawn/exec provisioning
now authenticates a retained-lease snapshot that survives source withdrawal.
Service-local open objects and distinct copied owner teardown now provide the
ownership core. The next integration work is copied file operation dispatch and
syscall descriptor ownership, followed by remaining
namespace/storage operations and removal of kernel filesystem engines. Copied live chdir is integrated for the selected
root profile; this does not complete the overall cutover.
A registration or negotiation pass cannot satisfy runtime routing acceptance.
Never fall back from a selected userspace namespace to the bootstrap resolver
after an operation fails. The dated entries below preserve the earlier staging
and acceptance history.

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
| Namespace ID/generation and root/directory objects | Kernel allocates non-reusing scope for trusted provisioning; server allocates retained pair generation | Server checks scope membership and service epoch; kernel installs correlated receipt and retains references for the request |
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

### Registration-lived service runtime policy slice (2026-11-15)

The service now keeps a `Runtime<64, 64, 4>` in its process-lived `State`.
Registration policies are allocated lazily and fallibly; ordinary startup leaves
the owner unbound because nameserver registration supplies no independently
authenticated registration snapshot. No decoded frame can bind or provision this
owner, and no ordinary IPC handler routes requests into it. Future provisioning
must validate caller/service liveness and authority under lifecycle serialization;
the component's identity arguments alone do not authenticate those facts.

Trusted binding is idempotent for the exact active registration, preserving pins,
request replay state, namespace generation and lease generation allocators.
Replacement requires explicit retirement and a strictly newer epoch. Capacity
and allocation are admitted before publishing the replacement or advancing the
epoch watermark. The four registration slots retain retired policies even when
their lease tables become empty: delayed retains must remain unable to resurrect
policy, and lost release acknowledgements must remain retryable. Exhaustion fails
closed. There is deliberately no slot reclamation or remote-drain acknowledgement.
The existing service bump allocator is unchanged; allocation exhaustion can close
admission before the logical registration capacity is reached.

Trusted caller retirement matches PID, incarnation and VM generation across all
owned epochs and withdraws matching policy bindings. Registration retirement
matches the complete registration, permanently closes installation for that
policy and withdraws every binding. Both preserve existing leases and their real
directory pins. Retired epochs can process correlated releases and duplicate
release retries; new retains cannot resolve withdrawn policy and receive the
existing terminal refusal contract, with replay watermarks still enforced. A
repeated old retirement cannot withdraw a replacement. These are serialized
component operations; authenticated lifecycle event delivery is not wired yet.
Service-process death does not preserve this userspace owner across a restart
and provides no drain acknowledgement to the boot-lived kernel owner. Kernel
obligations remain governed by the preceding remote-ownership slice.

Five new host tests cover unbound rejection, invalid and idempotent binding,
strict epoch progression, exact caller/registration matching, permanent policy
retirement, delayed retain replies across replacement, abandoned-release
reconciliation using the shared kernel ownership component, lost release
acknowledgements, replacement immunity, retained metadata and fail-closed capacity
including empty retired epochs and a zero-capacity owner. All 12 service tests,
11 kernel adapter tests, 101 filesystem tests, 151 IPC tests and nine IPC
integration tests pass. Kernel/IPC/service compilation, focused rustfmt and
changed-file whitespace checks pass with existing dependency/service warnings.
Logs: `target/vfs-service-runtime-tests.log`,
`target/vfs-service-runtime-libraries.log`,
`target/vfs-service-runtime-adapter.log`,
`target/vfs-service-runtime-ipc-integration.log` and
`target/vfs-service-runtime-check.log`. Allocation-failure injection is NOT RUN;
capacity admission failure is exercised separately.

The Phase 3.0 ring-3 fixture now uses the same runtime component for idempotent
binding, caller/registration retirement, replacement and old-epoch release retry.
It reports `registration policy lifecycle OK transport=fixture`. This fixture
uses synthetic identities; it does not populate the ordinary service owner.
The final-source Phase 3.0 QEMU gate passes in
`target/vfs-service-runtime-phase30.log`; raw serial is preserved in
`target/vfs-service-runtime-phase30-serial.log`. Implementation, host-test and
expected-marker hashes in `target/vfs-service-runtime-source.sha256` were verified
after the gate; kernel/ISO hashes are in
`target/vfs-service-runtime-image.sha256`. This proves boot/filesystem regression
and component execution in ring 3, not an authenticated retention exchange or
native scheduler lifecycle race.

Authenticated kernel-origin transport/envelope enforcement and lifecycle delivery,
trusted provisioning, receipt-backed namespace installation, held-lease/local-user
drain and pending-reap release reconciliation, proven remote registration drain,
restricted mappings/quotas, publication/completion, park/resume and native
lifecycle races remain outstanding. Dispatch stays disabled; KERNEL_VFS remains
authoritative.

### Receipt-backed kernel namespace and local drain slice (2026-11-15)

The boot-lived kernel runtime now records bounded namespace preparation alongside
each remote retain obligation. Preparation reserves a non-reusing boot-scoped
namespace ID/generation, stores the full caller/credential/registration snapshot
and attenuated grants, and returns the existing single-send retain frame. Failed
or abandoned preparation never reuses a scope. Trusted service policy provisioning
must precede publication; these component helpers neither provision policy nor
authenticate identity arguments or messages.

`install_retained` first correlates the success against the independently supplied
registration and the pending retain. Malformed, stale, cross-scope and duplicate
replies cannot install or consume preparation. A success installs only when the
supplied revalidated live snapshot still matches every captured identity and
credential field and the registration remains active. Caller/service retirement
instead routes late success directly to release. Duplicate local bindings and
local namespace capacity failure also record compensating release ownership
before returning the installation error. Exact refusals reclaim both preparation
and the corresponding remote obligation; timeout supplies no such proof.

Installed directory records use the receipt's service-assigned pair generation,
not backend inode numbers or the kernel scope generation. Both context object
fields carry that pair identity: the root field selects its root member and the
directory field its cwd member. Future service dispatch must validate the entire
scope/registration and select the corresponding member of the retained pair.
The new adapter tests use distinct real tmpfs directories and deliberately
different scope and pair generations. No pathname is resolved in the kernel.

A bounded runtime tracker retains each installed binding. Withdrawal invalidates
admission; lifecycle hooks reconcile already drained bindings without sending IPC.
Pending entries, terminal entries, reaped cleanup owners and retained preparation
references continue pinning the same binding. Under runtime serialization, only
an invalidated binding with the tracker as its sole strong owner may transition
its lease to releasing. No weak references escape production code. Future cleanup
workers must invoke the private reconciliation helper after dropping reaped
resources and lifecycle guards. Reaping alone never proves drain. Reconciliation
records retryable release state before dropping the tracker; lost sends and lost
acknowledgements preserve it across replacement. A live binding remains held even
after its pending requests drain. This is local reference drain, not proof of
remote service-registration drain or future mapped-request transport drain.

All 19 host adapter tests pass, including eight new tests combining the production
kernel runtime, service runtime and real tmpfs directory pins with supplied
identities. They cover pending/reaped ownership, explicit withdrawal, malformed and
wrong-identity replies, changed credentials, compensation, exact refusals, both
bounded capacities, late replies and held leases across replacement, and release
retry after a lost acknowledgement. The existing namespace-policy test also checks
scope allocation exhaustion. The first combined run failed because the fixture
attempted a second service policy for an already provisioned caller; it is preserved
in `target/vfs-receipt-adapter.log`. The corrected test explicitly withdraws the
old service policy while retaining its old kernel binding, then verifies rejection
and compensation at kernel installation. Final adapter results are in
`target/vfs-receipt-adapter-final.log`.

The 12 service, 101 filesystem and 151 IPC unit tests pass in
`target/vfs-receipt-libraries.log`; 151 IPC unit tests and nine integration tests
pass in `target/vfs-receipt-ipc-integration.log`. Kernel/IPC/service compilation
passes in `target/vfs-receipt-check.log` with existing dependency/service warnings.
Focused rustfmt and changed-file whitespace checks pass. Existing `Arc` allocation
behavior is unchanged; allocation-failure injection is NOT RUN.

The Phase 3.0 QEMU regression passes in `target/vfs-receipt-phase30.log`; raw serial
is preserved in `target/vfs-receipt-phase30-serial.log`. Source and image hashes
are recorded in `target/vfs-receipt-source.sha256` and
`target/vfs-receipt-image.sha256`. The receipt installation/drain flow is exercised
by host component tests only. Native authenticated retention, populated kernel
receipt-backed bindings and scheduler lifecycle races are NOT RUN.

Next implement authenticated kernel-origin retention transport/envelope enforcement,
trusted scope provisioning and lifecycle delivery. Restricted mappings/quotas,
publication/completion, park/resume, proven remote registration drain and native
lifecycle races remain open. Dispatch stays disabled; KERNEL_VFS remains authoritative.

### Authenticated copied retention control slice (2026-11-15)

Native syscall 180 adds a service-only, fixed-size pull channel. SNAPSHOT returns
exactly 40 little-endian bytes containing the active registration. NEXT copies
one kernel-selected 112-byte retention frame; REPLY copies exactly 112 bytes into
kernel memory before validation. Epoch zero is permitted only for SNAPSHOT;
unknown operations, wrong lengths and nonzero reserved arguments fail closed.
No SHM attachments, user-selected service endpoint, claimed sender identity or
ordinary IPC badge establish authority on this channel. Negotiation still
advertises zero executable features, and KERNEL_VFS remains authoritative.

Scheduler -> broker -> runtime serialization authenticates the syscall caller's
boot-stamped VFS role, live PID/incarnation/VM generation and continued endpoint
ownership. The supplied epoch selects a retained registration; it cannot replace
its owner identity. Old-epoch replies/releases may complete while that exact
owner still lives and owns the endpoint, including after registration replacement.
Service exit, exec, endpoint withdrawal and a replacement owner cannot inherit
that channel. User copies occur outside these locks.

Each registration has a bounded unsent-retain inventory admitted before namespace
preparation. NEXT consumes retains once, in allocation order despite slot reuse.
Only one retain may be delivered and awaiting a correlated reply per registration;
this prevents concurrent service readers from processing higher IDs first after
kernel unlock. A failed output copy or lost retain response preserves unknown
ownership and blocks subsequent retains for that epoch. It does not authorize
retransmission, cancellation, namespace installation or remote reclamation.
Pending ownership remains until a correlated response or future proven drain.
Releases remain available for retry, including while a retain awaits its reply.

REPLY rejects retain replies that were never delivered through this channel and
correlates the full frame against the independently authenticated registration.
Before installation it rechecks the captured caller incarnation, VM generation,
UID/GID and non-exiting state under the same lifecycle locks. Withdrawal or changed
credentials turns late success into an owned release. Invalid replies preserve
preparation; successful installation and compensation stop accepting retain
replies. Release acknowledgements settle only the original owned receipt.

The ordinary VFS process obtains an authenticated registration snapshot after its
existing self-tests and lazily allocates its registration policy through the
existing fallible owner. Binding itself grants no namespace policy. Its service
loop polls at most 64 control exchanges per existing IPC turn. Ordinary IPC
handlers do not route control frames. No production producer calls the private
namespace staging helper: trusted scope provisioning and service wakeups must be
implemented before enabling live retain publication. Caller/registration lifecycle
delivery to userspace also remains unwired; local kernel withdrawal still applies.
The service loop currently polls its active epoch only; retired-epoch servicing
and restart reconciliation need integration before live provisioning is enabled.

Five new kernel adapter tests exercise the actual control component using supplied
scheduler/broker records, the service runtime and real tmpfs directory pins. They
cover owner/role/endpoint/epoch rejection, malformed and undelivered replies,
credential-change compensation, single installation, local-reference drain,
retirement/replacement, lost release acknowledgements, retained obligations after
a discarded fetch, allocation ordering after slot reuse, one in-flight retain and
bounded admission. These are host component observations, not native user-copy
fault injection or scheduler races. Two new IPC tests cover exact syscall envelopes
and registration snapshot encoding, including valid endpoint zero.

All 24 adapter, 153 IPC, 101 filesystem and 12 service unit tests pass, as do
nine IPC integration tests. Logs: `target/vfs-copy-adapter-final.log`,
`target/vfs-copy-libraries-final.log`, and
`target/vfs-copy-ipc-integration.log`. The initial adapter compilation failure
used an incorrect fallible tmpfs constructor in the new test fixture; it is
preserved in `target/vfs-copy-adapter-initial.log`. The corrected fixture uses the
existing infallible constructor. Kernel/IPC/service compilation passes in
`target/vfs-copy-check-final.log` with existing dependency/service warnings.
Focused rustfmt and changed-file whitespace checks pass.

The Phase 3.0 QEMU regression passes in `target/vfs-copy-phase30.log`; raw serial is
preserved in `target/vfs-copy-phase30-serial.log`. The real VFS process emits:

```text
[VFS-RPC] authenticated kernel registration snapshot OK transport=copy
```

That check performs repeated real snapshot syscalls, binds the service runtime,
checks empty-channel polling and rejects an invalid zero epoch. Implementation,
fixture and expected-marker hashes in `target/vfs-copy-source.sha256` were verified
after the gate; artifact hashes are in `target/vfs-copy-image.sha256`. This proves
the native snapshot/empty-copy boundary and boot regression, not a populated native
retention exchange, untrusted native caller rejection, copy-fault injection or
scheduler lifecycle races. Those native cases are NOT RUN.

Next implement trusted scope provisioning, ordered userspace lifecycle delivery,
retired-epoch servicing and service wakeups before enabling retain publication.
Restricted mappings/quotas, file publication/completion, park/resume, proven remote
registration drain and native lifecycle races remain open. Dispatch stays disabled;
KERNEL_VFS remains authoritative.

### Retained-epoch service polling slice (2026-11-16)

The copied channel now supports STATUS with an exact 48-byte output: the full
40-byte registration plus a canonical little-endian retirement bit. Epoch zero,
wrong lengths, reserved arguments and noncanonical status encodings fail closed.
The kernel resolves the retained epoch independently and authenticates the same
live boot-stamped service role, PID/incarnation/VM identity and endpoint owner as
NEXT/REPLY. Retired status remains queryable without an active registration while
that exact owner and retained row survive. STATUS is read-only and retryable;
a failed lookup or copy proves neither retirement nor drain.

The ordinary service now uses a shared bounded polling component. Each existing
IPC turn refreshes the authenticated active snapshot, retires the previous policy
on a strictly newer snapshot, and lazily binds the replacement. Invalid snapshots
cannot mutate policy. Replacement capacity/allocation failure leaves the old
policy retired with its leases and replay state intact; later turns can retry
binding. An authenticated retired status also retires a known policy when no
replacement exists. Every retained policy gets up to 16 control exchanges per
turn, at most 64 with the production four-policy inventory. Old release retries
therefore cannot consume the replacement's polling budget. The service correlates
status against the complete stored registration before requesting any frames.

Service policies remain retained after the kernel collects a settled old row.
Missing status causes that epoch to be skipped; it never authorizes service-row
reclamation, replay-state reset or implicit reactivation. This is bounded
same-process reconciliation of known registrations, not restart recovery or proof
of remote registration drain. Endpoint withdrawal, exec and process replacement
still close the old owner's channel. Caller retirement events and ordering between
those events and retained requests remain unwired. A status read is not an ordered
lifecycle delivery barrier.

Host component tests exercise the production polling loop against the actual
kernel control component and service policies with real tmpfs pins. They cover a
populated retain/install, registration retirement and replacement, repeated old
release acknowledgements lost before kernel acceptance, replacement refusal during
those retries, final old release settlement, retained replay rows after kernel
collection, retired status without an active registration, and full owner/role/
endpoint rejection. Service tests also cover missing/mismatched status, monotonic
snapshot observation, invalid snapshots, and fail-closed replacement capacity.

All 26 adapter, 154 IPC, 101 filesystem and 14 service unit tests pass, as do nine
IPC integration tests. Evidence: `target/vfs-epochs-adapter.log`,
`target/vfs-epochs-libraries-final.log` and
`target/vfs-epochs-ipc-integration.log`. Kernel/IPC/service compilation passes in
`target/vfs-epochs-check.log` with existing dependency/service warnings. Focused
rustfmt and changed-file whitespace checks pass. Allocation-failure injection is
NOT RUN; bounded replacement capacity failure is covered separately.

The final-source Phase 3.0 QEMU gate passes in `target/vfs-epochs-phase30.log`;
raw serial is preserved in `target/vfs-epochs-phase30-serial.log`. The real service
checks active status, rejects epoch zero, executes the shared empty polling loop,
and emits:

```text
[VFS-RPC] authenticated registration status OK transport=copy
```

Implementation/fixture/expected-marker hashes in `target/vfs-epochs-source.sha256`
were verified after the gate (`target/vfs-epochs-source-verified.log`); kernel/ISO
hashes are in `target/vfs-epochs-image.sha256`. This proves native active-status
and empty-loop execution plus boot/filesystem regression. Populated replacement,
retired status/release exchanges, adversarial native caller rejection, copy-fault
injection and scheduler lifecycle races are NOT RUN natively; populated exchanges
remain host component evidence.

Next implement trusted scope provisioning, ordered caller lifecycle delivery and
service wakeups before enabling live retain publication. Restart reconciliation,
receipt-backed remote registration drain, restricted mappings/quotas,
publication/completion, park/resume and native lifecycle races remain open.
Dispatch stays disabled; KERNEL_VFS remains authoritative.

### Ordered caller retirement control slice (2026-11-16)

The copied retention channel now carries canonical 112-byte CallerRetired and
CallerRetiredAck frames (operations 6 and 7 inside NEXT/REPLY). They contain the
full registration, exact caller PID/incarnation/VM generation and a nonzero
registration-local event ID. Namespace and lease-generation words are reserved
and must be zero. Existing syscall lengths, caller authentication and endpoint
ownership checks apply; ordinary IPC and the ordinary lease request router
cannot deliver lifecycle changes.

Namespace preparation reserves one of 64 caller-delivery rows per registration
before allocating a namespace or remote lease. Repeated scopes for one live
caller share its row. Rows survive lease settlement and refusal because those
outcomes do not prove that service policy pins are gone. Capacity or event-ID
exhaustion closes new-caller preparation; no lifecycle event allocates memory or
overwrites another caller. Future trusted provisioning must use this admission
path and revalidate lifecycle identity before publication. The private generic
retention fixtures are not a provisioning interface.

Existing exit and successful-exec retirement hooks mark matching admitted
callers in every retained epoch, alongside local withdrawal and lease abandonment.
NEXT retries pending retirement events before any later retain or release.
Only an exact independently authenticated acknowledgement of a delivered event
opens that barrier and releases its caller slot. Discarded output copies and
lost acknowledgements preserve the same event. Slot reuse consumes a fresh event
ID; duplicate, undelivered, mismatched and stale acknowledgements cannot clear a
replacement event. IDs correlate reservations; they do not establish a global
chronological order among independent callers or across registration epochs.

The shared service polling loop applies each authenticated retirement to exact
caller policy bindings across its retained epochs before acknowledging it.
Repeated delivery is idempotent and cannot withdraw another incarnation or VM
generation. Existing lease pins and replay watermarks survive. A retain fetched
before retirement may still complete, but kernel abandonment owns compensation
and prevents namespace installation. A later queued retain encounters the
withdrawn service policy after the barrier acknowledgement. Pending barriers
and live caller-delivery rows also prevent local kernel epoch collection, even
with no outstanding lease. Thus an exit after old-lease settlement still has a
delivery destination. Retired epochs remain charged until their tracked callers
retire and acknowledge, or a future authenticated drain closes those obligations;
long-lived callers can exhaust replacement capacity. Service rows still require
a separate proven remote drain before reclamation.
Unacknowledged barriers deliberately defer release delivery too, preserving
ownership rather than claiming progress or reclamation after a failed copy.

Three new host adapter tests exercise the real kernel control component, shared
service polling loop and tmpfs pins. They cover discarded event fetches, lost
acknowledgements, full owner/caller/event correlation, undelivered and duplicate
acks, already-fetched success compensation, later-retain refusal, bounded
capacity after lease settlement, all 64 callers retiring without allocation,
pending-event epoch retention, old/replacement polling fairness, PID reuse and
fresh event IDs after slot reuse. IPC tests verify canonical lifecycle encoding;
service tests verify origin separation, full registration matching, idempotent
withdrawal and retained lease/new-incarnation preservation.

All 29 adapter, 155 IPC, 101 filesystem and 15 service unit tests pass, as do nine
IPC integration tests. Evidence: `target/vfs-lifecycle-adapter-final.log`,
`target/vfs-lifecycle-libraries.log` and
`target/vfs-lifecycle-ipc-integration.log`. Final kernel/IPC/service compilation
passes in `target/vfs-lifecycle-check-final.log` with existing dependency/service
warnings. Focused rustfmt and changed-file whitespace checks pass. The initial
old empty-poll assertion failure is preserved in
`target/vfs-lifecycle-adapter-initial.log`; old collection expectations after
adding live caller obligations are preserved in
`target/vfs-lifecycle-adapter-retention.log`. An intermediate test edit applied
the new collection assertion to the wrong fixture; its failure is preserved in
`target/vfs-lifecycle-adapter-fixture-correction.log`. The final fixtures retain
the old local-reference drain and lease-settlement assertions, then separately
require caller retirement delivery before old-epoch collection.

The final-source Phase 3.0 QEMU regression passes in
`target/vfs-lifecycle-phase30.log`, with raw serial in
`target/vfs-lifecycle-phase30-serial.log`. Implementation/fixture/expected-marker
hashes in `target/vfs-lifecycle-source.sha256` were verified after the gate in
`target/vfs-lifecycle-source-verified.log`; kernel/ISO hashes are in
`target/vfs-lifecycle-image.sha256`. The ordinary service still proves native
authenticated active snapshot/status and empty polling. Populated lifecycle
events, native event-copy faults, lost acknowledgements and exit/exec scheduler
races are NOT RUN natively; those populated exchanges remain host component
evidence. No native lifecycle marker or production namespace producer was added.

Next implement trusted scope provisioning and service wakeups before enabling
live retain publication. Restart reconciliation, receipt-backed remote
registration drain, restricted mappings/quotas, publication/completion,
park/resume and native lifecycle races remain open. Dispatch stays disabled;
KERNEL_VFS remains authoritative.

### Copied control service wakeup slice (2026-11-16)

Pending caller-retirement barriers, deliverable unsent retains and retryable
releases now provide level-triggered service wakeups. Blocking receive,
nonblocking receive and reply-and-wait check for work after ordinary queued
messages; blocking paths perform their last check under the same scheduler
serialization as parking. The BSP scheduler tick also checks blocked receivers,
using the existing IPC Ready-queue/reschedule path. Work arriving after a receiver
parks therefore remains discoverable without another ordinary IPC request.
The tick makes work eligible for a wake; this is not a measured dispatch-latency
bound or proof that the task executed.

Hints match the boot-trusted service role, non-exiting PID/incarnation/VM identity
and a registered endpoint belonging to that exact owner. A receiver blocked on
an outgoing call or holding an ordinary reply target is excluded. Work in an old
epoch can wake a different replacement endpoint of the same owner, because the
service polls its entire retained inventory. The receive syscall continues to
authenticate endpoint receive ownership; the hint supplies no channel authority.
Each copied operation independently authenticates its original epoch and endpoint,
so a wake cannot restore a withdrawn endpoint or transfer an old owner's access.

The payload-free `CONTROL_WAKEUP` hint uses no endpoint queue slots, allocations
or caller reply target. The service recognizes its kernel-stamped zero badge and
empty word/capability counts, runs its existing bounded authenticated polling loop,
then receives again without replying to the hint. An ordinary caller's stamped
nonzero PID badge prevents its lookalike label from bypassing ordinary handling.
Neither the label nor its badge delivers or acknowledges a lifecycle event.

Hint delivery coalesces retry eligibility across the owner's retained epochs for
the current clock tick. A discarded hint or unacknowledged event/release remains
eligible next tick; repeating receive within that tick cannot spin on the same
obligation. Waking a parked task does not consume this budget before its receive
retry. An unknown single-send retain is not retransmitted and alone causes no
retry wake; live caller-delivery rows without pending retirement also cause none.
There is no periodic userspace poll when the inventory has no deliverable work.
Permanently unserviceable obligations can still cause bounded retry wakes until
future drain/reconciliation resolves them; a failed authenticated poll proves
neither progress nor reclamation.

Four new host adapter tests cover both modeled sides of receive/park, repeated
tick checks, hint coalescing, discarded hints/events, single-send retain blocking,
release retries, full owner/endpoint rejection, outgoing-call/reply-target
exclusion, old/replacement epochs and a changed replacement endpoint. They execute
the production readiness and receiver-selection component with host process
records. The harness substitutes a Ready-state setter for the native scheduler
wakeup function: these tests do not exercise native Ready queues, IPIs, the actual
IPC receive/reply-wait continuations or concurrent scheduler interleavings.
A new IPC test covers hint label, badge and payload separation.

All 33 adapter, 156 IPC, 101 filesystem and 15 service unit tests pass, along with
nine IPC display integration tests. Logs: `target/vfs-wakeup-adapter-final.log`,
`target/vfs-wakeup-libraries.log` and
`target/vfs-wakeup-ipc-display-integration.log`. Kernel/IPC/service compilation
passes in `target/vfs-wakeup-check-final.log` with existing warnings. Focused
rustfmt, changed IPC-function formatting and changed-file whitespace checks pass.
The initial adapter compilation failure (missing host message re-export and
private sibling helpers) is preserved in `target/vfs-wakeup-adapter-initial.log`.
An attempted nonexistent IPC test target is preserved in
`target/vfs-wakeup-ipc-integration.log`; the actual display integration target
passed separately as recorded above.

The final-source Phase 3.0 QEMU regression passes in
`target/vfs-wakeup-phase30.log`; raw serial is preserved in
`target/vfs-wakeup-phase30-serial.log`. The existing native authenticated
snapshot/status and empty-loop markers pass. Implementation/fixture hashes in
`target/vfs-wakeup-source.sha256` were verified after the gate in
`target/vfs-wakeup-source-verified.log`; kernel/ISO hashes are in
`target/vfs-wakeup-image.sha256`. This proves the empty-channel boot/filesystem
regression, not populated native wakeups. Queue-saturation progress, retry timing,
populated receive/reply-wait continuations and exit/exec races are NOT RUN
natively. Existing populated retention/lifecycle exchanges remain host component
evidence. No production namespace producer or native lifecycle/wakeup-success
marker was added.

Next implement trusted scope provisioning with lifecycle revalidation before
enabling live retain publication. Native populated wakeup/lifecycle verification,
restart reconciliation, receipt-backed remote registration drain, restricted
mappings/quotas, publication/completion and park/resume remain open. Dispatch
stays disabled; KERNEL_VFS remains authoritative.

### Live mounted-root traversal prerequisite (2026-11-16)

The service's `namespace::stat_root` adapter now walks the existing mounted VFS
component by component. It accepts repeated separators and `.`; `..` stops at the
service root. Each parent is checked for directory type and caller search
permission before processing the next component, including `.` and `..`.
Consequently missing, non-directory or denied components cannot disappear through
lexical normalization. Trailing separators require a directory. Mount selection
continues through the live service VFS. The adapter explicitly rejects captured
paths longer than 255 bytes, relative paths and embedded NUL, without truncation.
It uses bounded stack storage for the normalized path.

Host tests cover denied/missing components before `..`, non-directory traversal,
root confinement, repeated separators, trailing separators, UTF-8 parent removal,
mount crossing and oversized input. All 21 service library tests pass in
`target/vfs-root-traversal-tests.log`; the service target check passes in
`target/vfs-root-traversal-check.log`. The ownership inventory was rerun in
`target/vfs-root-traversal-audit.log`. This is component evidence only; no native
boot gate was run for this slice.

This adapter still accepts only the live service root and the existing two-field
`Credential`; it does not implement retained alternate roots/cwd, supplementary
groups, authenticated syscall dispatch or RPC metadata provenance. Trusted scope
provisioning and the authenticated stat park/resume round trip remain the next
cutover work. KERNEL_VFS remains authoritative for file syscalls.

### Shared mounted-service directory ownership and migration ceiling (2026-11-16)

The production service now moves its existing mounted VFS into shared ownership.
Legacy IPC operations and the `DirectoryNamespace` used by trusted provisioning
share that exact table. Mounted directory pairs own real VFS handles and keep
the table alive; close returns the existing per-mount open-handle accounting.
`Vfs::handle_object_id` delegates to the retained backend handle and does not
re-resolve its pathname. These IDs remain backend-local diagnostics, not RPC
metadata provenance or authority. The separate boot-share adapter is unchanged.
Isolated component fixtures still own small tmpfs backends; they are not used to
construct production policy. No new kernel namespace or traversal path was added.

Host tests cover shared metadata updates, rename/replacement without retargeting,
backend lifetime and handle-count cleanup across a non-root mount. Existing
protected boot mounts remain protected after pins close. All 22 service, 101
filesystem, 156 IPC and 33 kernel-adapter tests pass. Evidence:
`target/vfs-shared-namespace-tests-final.log`,
`target/vfs-shared-namespace-libraries.log`, and
`target/vfs-shared-namespace-adapter-final.log`. Kernel and service target checks
pass in `target/vfs-shared-namespace-kernel-check-final.log` with existing warnings.

The Phase 3.0 native gate passes in
`target/vfs-shared-namespace-phase30-retry.log`; serial is preserved in
`target/vfs-shared-namespace-phase30-serial.log`. A new ring-3 fixture installs a
policy against the actual live mount table, changes ownership through the existing
legacy handler, observes it through the retained pair, then releases its handles:

```text
[VFS-RPC] live mounted namespace pins shared with legacy handlers OK transport=fixture
```

This is live service ownership evidence with fixture identities, not authenticated
kernel provisioning, a cross-process stat request, or syscall cutover. The first
native run failed before retention fixture markers; its log is preserved in
`target/vfs-shared-namespace-phase30.log`. The initial implementation allocated a
full mount table for each isolated fixture against the service's 64 KiB bump heap.
Keeping isolated fixtures on their small tmpfs backend resolves that regression.
An initial host assertion incorrectly expected a protected boot mount to unmount;
it is corrected to preserve that protection and verify handle accounting.

`audit_vfs_cutover.py --no-growth` now compares exact path/category/code sites and
multiplicity against `userspace-vfs-kernel-baseline.json`, ignoring line numbers.
Deleting an owner cannot buy permission to add a different owner. Both the native
boot test script and workspace checker enforce it before builds. Baseline entries
should be removed during migration; do not add owners to make the guard pass.
Two host guard tests pass in `target/vfs-shared-namespace-audit-tests.log`.
The guard passes with zero additions in
`target/vfs-shared-namespace-no-growth.log`. This remains a conservative source
scan, not a Rust semantic analysis. Final `--check` deliberately still fails:
68 ring-0 ownership references remain, recorded in
`target/vfs-shared-namespace-removal-gate.log`.

Next: authenticated, lifecycle-revalidated provisioning into this shared mount
table, followed by stat publication/completion and atomic caller park/resume.
Open-object operations, storage/exec consumers and removal of KERNEL_VFS remain
required for cutover. File syscall dispatch remains unchanged in this slice.


### Authenticated root-policy handoff (2026-11-16)

The copied retention channel now has `AUTHORIZE_ROOT`, an exact 112-byte input
operation. It validates the full pending retain against a separately staged
kernel root-policy bit. Successful validation authorizes only the service's
mounted `/` as both root and cwd; retain payloads cannot choose paths or grant
that bit. The private trusted producer `queue_root_namespace` stages this
profile; ordinary `queue_namespace` staging does not authorize it. No public
caller-selection/provisioning syscall was added.

Authorization requires the independently authenticated service/endpoint/epoch,
an active non-retired registration, the exact delivered pending request and
scope, no outstanding caller-retirement barrier, and a live caller with the
original PID/incarnation/address-space/UID/GID. Neither unsent requests nor
bound or settled receipts pass. User copies occur outside lifecycle locks.

The production service loop now polls with `poll_root_control` against its shared
mounted `DirectoryNamespace`. For a retain it independently authorizes the root
profile, pins `/` twice, revalidates, and only then acquires the remote lease.
A revalidation race withdraws the provisional pins. Final reply installation
still revalidates kernel caller identity, with compensating release on a late
race. Pin/allocation/profile failures use retention-table refusal accounting
without consulting an older policy binding; replay failures never become false
negative acknowledgements for potentially retained leases.

Host adapter evidence now covers a populated control exchange with no service
fixture policy installation: kernel staging, independent authorization, shared
mounted-directory pins, retained reply, correlated kernel binding, caller
retirement and final handle drain. It also covers retirement between pinning
and revalidation, forged correlation fields, changed caller credentials and
lifetimes, endpoint loss, unauthorized ordinary callers, unsent requests,
ungranted profiles, continued progress after a refusal, and release of provisional
pins when lease capacity is exhausted. Mounted host fixture
construction lives in `tools/test_vfs_registration_host.py`, outside kernel boot
sources; the no-growth ownership baseline remains unchanged at 68 references.

Validation: 37 host registration/ownership adapter tests, 156 IPC tests and 23
service tests pass. Kernel/service `cargo check` passes with existing warnings.
`bash tools/test.sh phase3.0` passes, including authenticated copied registration
status, shared mounted-directory fixtures and the legacy VFS IPC checks. Native
log: `target/vfs-root-policy-phase30-final.log`. This is regression evidence,
not a populated native root-policy or stat round-trip proof.
Logs: `target/vfs-root-policy-adapter-final.log`,
`target/vfs-root-policy-libraries-final.log`,
`target/vfs-root-policy-check-final.log`, `target/vfs-root-policy-no-growth.log`.

This completes the authenticated root-policy handoff consumer, not syscall
cutover. The private root-profile producer is exercised by the host adapter;
boot/process lifecycle code does not yet select or stage production callers.
The native regression gate exercises the ordinary service loop without a
populated root-profile exchange. Next wire a deliberately selected caller and
its explicit root policy, then implement the authenticated stat publication,
park/resume and result-copy path. Arbitrary cwd/root profiles remain unsupported;
ordinary file syscalls still use the kernel VFS.


### Boot caller selection and populated native root handoff (2026-11-16)

The boot-loaded init image now carries the private `vfs_root_metadata` marker.
This explicitly selects the service-mounted `/` for both root and cwd with only
`LOOKUP_METADATA`; PID, image name, UID zero and ordinary IPC cannot select it.
Both process constructors default the marker to false, so spawn and fork do not
inherit it. Successful exec withdraws it with the other image-scoped roles.
This is a provisioning pilot, not a file-syscall routing switch. It does not claim
that init's kernel cwd or namespace has migrated.

An authenticated active `SNAPSHOT` stages the profile under scheduler, broker and
runtime serialization. It captures caller incarnation, VM generation and current
credentials before queueing through the existing root-policy producer. Existing
caller delivery records make the staging decision idempotent across pending,
bound and refused work. A refusal is terminal for that caller in that service
epoch; polling does not repeatedly allocate scopes or pin directories. Capacity
failure before publication can retry on a subsequent snapshot. A replacement
registration can provision a fresh non-reusing scope; retired epochs retain their
existing cleanup obligations.

The service startup probe now drains populated control work through the ordinary
root-policy handler before checking for an empty channel. It must not consume a
single-delivery retain with an empty-channel assertion. Kernel serial evidence
is emitted only after a correlated retained reply installs a live binding for a
selected process:

```text
[VFS-RPC] boot root policy installed caller=1 epoch=1 transport=copy
```

The Phase 3.0 gate now requires this marker. `bash tools/test.sh phase3.0` passes,
including existing legacy IPC and shared namespace checks. Logs:
`target/vfs-production-selection-phase30.log` and
`target/vfs-production-selection-serial.log`. This is populated native
provisioning evidence, not a native stat RPC or syscall cutover proof.

Validation: 41 host adapter tests, 156 IPC tests and 23 service tests pass (220
total). New adapter cases cover unauthenticated selection attempts, exact captured
credentials, duplicate snapshots, credential races, full shared-root provisioning
without fixture staging, pending-operation pin lifetime through retirement/reap,
refusal without restaging, fresh scopes after replacement, and invalid/exiting
caller identities. Kernel/service target checks pass with existing warnings.
Logs: `target/vfs-production-selection-host-final.log`,
`target/vfs-production-selection-libraries.log`,
`target/vfs-production-selection-check.log`.

The ownership audit still reports 68 references and zero additions in
`target/vfs-production-selection-audit.log`. File syscalls still use KERNEL_VFS.
Next implement authenticated stat publication/completion, atomic caller
park/resume and validated result copying, including service-loss termination.
Open descriptions, remaining namespace/storage/exec consumers and kernel VFS
removal remain required for the migration acceptance gate.

### Stat publication and continuation component (2026-11-16)

Added a kernel-private, bounded stat publication component alongside the
boot-lived pending table. Admission reserves a publication slot, checks the exact
caller/VM and credentials and rejects an existing IPC or stat continuation.
Fallible resource preparation finishes before publication and caller parking
under scheduler/broker/runtime serialization. Prepared input remains owned by
the quota-charged pending resources. The copied pull component checks the full
registration, delivers IDs in allocation order and consumes delivery only after
a successful bounded copy. Short destination buffers do not consume work.

Service pull/completion wrappers authenticate the syscall-origin PID against the
retained registration and endpoint owner. Completion requires a delivered request
from the bound epoch; malformed bound replies become terminal, while wrong IDs,
epochs and duplicates cannot replace a result. Validated metadata bytes are copied
into retained output backing. Terminal wakeups match the caller incarnation and
VM generation and run once. The existing BSP wake pass now discovers stat work
for service hints and terminal caller continuations, including service loss.
Exit/exec discards the matching continuation while leaving terminal resources
for generic cleanup. Live continuations reserve their result until an exact
caller-owned take; unrelated abandoned operations remain independently reapable.

This is a host-validated continuation component, not an enabled native stat
transport. No syscall invokes publication/pull/completion yet. The copied service
syscall ABI, service dispatch loop, syscall continuation yield and validated user
result copy remain required before a native stat round-trip test or any routing
switch. In particular, the park operation must be followed by a scheduler yield
when its future syscall entry is integrated. File syscalls still use KERNEL_VFS.

Validation: 45 host adapter tests, 156 IPC tests and 23 service tests pass (224
total). New cases cover atomic park/publication, preparation rollback, short-copy
retry, exact input identity, single delivery, reply-before-delivery rejection,
duplicate completion, malformed replies, service loss, replacement VM isolation,
validated successful metadata retention and exact caller result ownership.
Kernel/service target checks pass with existing warnings. Logs:
`target/vfs-stat-publication-host.log`,
`target/vfs-stat-publication-libraries-isolated.log`,
`target/vfs-stat-publication-check-isolated.log`.
The ownership audit remains at 68 references with zero additions in
`target/vfs-stat-publication-audit.log`. QEMU was not rerun for this inactive
component; the earlier populated root-policy boot evidence is unchanged.

### Native authenticated root-stat pilot (2026-11-16)

Syscall 181 now exposes a deliberately narrow root-metadata operation to the
explicitly selected boot caller, plus service-only copied NEXT/COMPLETE operations.
The caller supplies a result destination, never a PID, path, scope, credentials,
endpoint or policy. The kernel publishes `Stat { path: "/" }` using the retained
root profile. This is a separate pilot ABI; ordinary stat(61) and the other file
syscalls continue using KERNEL_VFS. General pathname traversal is not enabled.

Exact operation/epoch/size/reserved-argument checks precede dispatch. Service
pull/completion authenticate the current process, endpoint and full registration
independently of message bytes. Successful copied pulls establish kernel origin;
ordinary IPC never enters the stat handler. Fixed bounded request packets carry
an explicit initialized input length and zero padding. Replies carry four words
and at most 56 initialized metadata bytes, with canonical unused padding.
Malformed replies preserve the raw correlation ID so the bound delivered request
can fail terminally without disturbing another operation. A failed pull copy
retires the epoch instead of retransmitting uncertain work.

The service drains stat work after retention control on normal IPC turns and
kernel receive hints. It resolves the authenticated dispatch scope and opaque
pair generation through live policy and retained directory leases, then reads
the pinned root object. It accepts only root stat, metadata-only grants and the
currently supported empty supplementary group set. Metadata provenance identifies
the retained scope and root pair for this pilot; it is not an exported backend
inode/handle identity. Filesystem mode type bits are removed because RPC metadata
carries kind separately. Retired policies, withdrawn scopes and altered pair
identities cannot supply successful metadata.

Caller publication and parking remain atomic under scheduler/broker/runtime
serialization. The syscall accounts the block, removes queued runnable copies
and requests rescheduling. The userspace wrapper retries the same owned logical
call only after wakeup, following the existing native IPC convention. Terminal
results are checked before active-service lookup, so service loss remains
observable. Nested IPC continuations keep their own blocked state. Completion
resources survive lifecycle unlock; caller incarnation and VM generation are
checked under the same scheduler guard as the final user-memory validation and
copy. Caller exit/exec requests bounded cleanup on syscall return, with resource
destruction outside lifecycle locks. Live callers retain their result ownership.

Init exercises the pilot after its base-service spawn pass, allowing nameserver
traffic to continue if root provisioning is not ready. The Phase 3.0 gate waits
for the native result marker and requires correlated park, validated reply and
successful kernel result-copy evidence:

```text
[VFS-RPC] root stat parked caller=1 request=1 epoch=1
[VFS-RPC] root stat reply validated=true errno=0 bytes=56
[VFS-RPC] root stat result caller=1 status=0x0
[VFS-RPC] native root stat caller resume/result copy OK transport=copy
```

Validation: 47 host adapter tests, 158 IPC tests and 23 service tests pass (228
total), including the copied service round trip, altered retained-object refusal,
post-retirement refusal, resource pin lifetime and nested IPC continuation
isolation. Kernel/init/service target checks pass with existing warnings.
`bash tools/test.sh phase3.0` passes, including the previous boot-root handoff and
legacy VFS checks. Evidence: `target/vfs-stat-native-host-final.log`,
`target/vfs-stat-native-libraries.log`, `target/vfs-stat-native-check-final.log`,
`target/vfs-stat-native-phase30-final.log`, `target/vfs-stat-native-serial.log`.
The ownership audit remains at 68 references with zero additions in
`target/vfs-stat-native-audit.log`.

Next extend authenticated stat to general paths with retained-root/cwd traversal,
search permissions and stable object provenance, and prove native caller/service
loss and VM-race behavior before enabling ordinary syscall routing. Open
descriptions, remaining namespace/storage/exec consumers and kernel VFS removal
remain required for migration acceptance. This pilot does not complete migration.

### Authenticated pathname stat in retained profiles (2026-11-16)

The copied stat pilot now accepts absolute and relative paths within the selected
service root/cwd profile. Syscall 181's PATH_METADATA operation copies an exact
UTF-8 pathname of 1–4095 bytes into kernel storage before publication. It accepts
no caller, namespace, credential, endpoint or policy fields. Output remains a
separate fixed 56-byte metadata record. Exact lengths and reserved arguments are
validated; empty, NUL-containing, malformed UTF-8 and oversized inputs fail before
admission. Retries must match the immutable admitted pathname, so a different
request cannot consume an earlier call's result. The root-only operation remains
available. Both use the same authenticated service pull/completion and caller
continuation machinery.

`Vfs::stat_beneath` authenticates retained directory handles against their live
mount incarnation, current linked pathname and tmpfs inode identity under one
exclusive mounted-VFS borrow. It does not authorize from a saved policy path or
an unauthenticated `handle_path` observation. Renamed directories remain usable;
a cwd moved outside the retained root is refused for relative lookup. Absolute
lookup starts from the retained root independently of cwd. Overmounting an
anchor cannot silently redirect its pin. Mount mutation and traversal share the
service's namespace mutex.

The component walker checks directory type and search permission using captured
UID/GID before each component, including `.` and `..`. Parent traversal clamps
at the retained root. Missing or non-directory intermediates cannot be erased by
lexical cleanup; trailing slashes require a directory. The final file need not
be readable merely to observe metadata. All temporary traversal paths use bounded
stack buffers, avoiding cumulative allocation in the service's bump allocator.
No supplementary-group support or symlink semantics are inferred.

Successful metadata now carries mounted tmpfs provenance rather than the prior
pilot's retained-pair placeholder: mount slot plus non-reusing mount incarnation,
non-reusing inode ID plus one (including implicit root), and object generation
one because inode IDs never reuse within a mount. These identities are local to
the mounted namespace and must be interpreted with the authenticated service
epoch; they do not claim global uniqueness across service restarts. FAT and other
backends without this stable identity contract return EOPNOTSUPP. Policy and
retention scope checks still precede traversal, and withdrawal closes admission.
The service returns correlated ENOENT, ENOTDIR, EACCES, EINVAL, EBADF, ENOMEM and
EOPNOTSUPP where applicable.

The independent input/output contract is now explicit in `Pending::new_copied`
and dispatch validation. A long immutable path does not require inflating the
56-byte writable metadata region. The original combined-buffer pending
constructor retains its previous validation contract. Boundary tests cover
maximum-length copied frames and real kernel-adapter resource preparation.

The native probe compares absolute and relative file metadata, verifies a
service-only directory with UID/GID changed through legacy service handlers,
checks root confinement and POSIX path errors, and places the final filename at
the end of a 4095-byte path to detect truncation. The Phase 3.0 gate now waits for
`[VFS-RPC] native path stat traversal/provenance/errors OK transport=copy` in
addition to the earlier root handoff and caller park/reply/result-copy evidence.

Ordinary file syscalls, including stat(61), remain on KERNEL_VFS. The selected
profile is the explicitly provisioned service root/cwd, not a migration of the
process's kernel cwd. Native caller/service-loss and VM-race tests, profile
lifecycle integration and ordinary syscall routing remain subsequent work.
Open descriptions, remaining namespace/storage/exec consumers and final kernel
VFS removal are still required for migration acceptance.

Validation: 49 host adapter tests, 161 IPC tests, 24 service tests and 107 filesystem
tests pass (341 total). The filesystem cases cover search permissions including
captured GID, dot/dotdot ordering, renamed root/cwd anchors, cwd moved outside root,
mount crossings, overmount refusal, inode identity across write/rename/recreation,
simulated mount-incarnation replacement and FAT refusal. Copied adapter tests
exercise kernel-captured credentials, maximum input, successful lookup and
correlated EACCES/ENOENT/ENOTDIR completion. Kernel/init/service native builds and
`bash tools/test.sh phase3.0` pass, including the maximum-length native path.
Evidence: `target/vfs-path-host-final.log`, `target/vfs-path-libraries.log`,
`target/vfs-path-fs.log`, `target/vfs-path-phase30-final.log`,
`target/vfs-path-serial.log`. The ownership audit remains at 68 references and
zero additions in `target/vfs-path-audit.log`.

### Bounded stat waiting and native failure gate (2026-11-16)

Authenticated pilot stats now have a five-second budget starting at publication.
The BSP wake pass expires both undelivered and delivered requests through the
existing single terminal transition. Service pulls, completions and caller
retries also check elapsed ticks under lifecycle serialization; neither delivery
nor retry renews the budget. Expiration returns ETIMEDOUT (110), wakes the exact
caller once and keeps its result resources until consumption or lifecycle reap.
A completed reply or caller/service retirement that already won remains terminal.
Elapsed-tick arithmetic supports counter wraparound. Late replies cannot complete
an expired request or replace a newer request's result. Timeout does not replay
work or retire an otherwise live service; this pilot admits only metadata reads.

`bash tools/test.sh vfs-stat-failure` builds init and VFS with an explicit
`stat-failure-test` feature. Its injection handlers and invalid-copy wrapper are
absent from normal images. After the ordinary native pathname probe, the test
withholds a real authenticated reply until the caller times out, submits that
late reply while a fresh request is waiting and verifies successful fresh
metadata. It then consumes a real service pull into an invalid user destination,
exercising the existing failed-copy epoch retirement path. The parked caller must
resume with EIO and the following call must report unavailable provisioning,
without falling back to kernel traversal. This proves native epoch-loss handling;
it does not simulate killing or restarting the service process.

Host cases cover deadline boundaries before/after delivery, exact caller-owned
timeout consumption, fresh IDs and late replies, counter wraparound and terminal
event ordering. Caller teardown is checked before delivery, after delivery and
after completion against exit, VM replacement and process-incarnation replacement.
Native caller exit/exec and VM replacement races remain a separate acceptance
gate, followed by profile lifecycle integration and ordinary syscall routing.
Ordinary file syscalls still use KERNEL_VFS; FAT lookup and open descriptions
remain unsupported by the authenticated pilot.

Validation: 52 host adapter tests, 161 IPC tests, 24 service tests and 107
filesystem tests pass (344 total). The dedicated native failure gate passes,
including a legacy IPC acknowledgement after the consumed-copy fault so its final
marker cannot hide a stalled service loop. Evidence:
`target/vfs-stat-failure-host-final.log`,
`target/vfs-stat-failure-libraries.log`, `target/vfs-stat-failure-fs.log`,
`target/vfs-stat-failure-native-final.log`,
`target/vfs-stat-failure-serial.log`.
The no-growth audit still reports 68 references and zero additions in
`target/vfs-stat-failure-audit.log`.
The ordinary `bash tools/test.sh phase3.0` gate also passes after rebuilding init
and VFS without fault injection (`target/vfs-stat-failure-phase30.log`). Both
native gates build the kernel and service images; existing build warnings remain.

### Native stat output mapping mutation (2026-11-16)

The dedicated `vfs-stat-failure` image now mutates a real cross-page user output
mapping after the stat publication returns PENDING and before result collection.
Unmapping the second page must reject the whole copy with the native failure
sentinel and preserve the first-page canary. A second case replaces that page
with a writable anonymous mapping at the same VA; collection must populate the
current mapping with the expected metadata. Both cases release their mappings
and verify a fresh stat succeeds, checking that failed copyout consumes the old
completion without poisoning the service or caller continuation.

This exercises mutation between publication and collection, including a reply
that may already have completed. It does not force concurrent mutation inside
copyout: scheduler serialization protects validation and copying. Same-VM page
replacement is supported; it is distinct from exec replacing the address-space
identity. Native caller exit/exec and concurrent VM replacement remain required
acceptance work. Ordinary syscalls continue using KERNEL_VFS.

Validation: 52 host adapter tests pass; the feature-enabled init builds. The
ownership audit remains at 68 references and zero additions. Evidence:
`target/vfs-vm-host.log`, `target/vfs-vm-build.log`, `target/vfs-vm-audit.log`.
The dedicated QEMU failure gate passes with the new required marker
`[VFS-RPC] native stat output unmap/remap atomic copy/recovery OK` plus all
previous timeout and epoch-loss markers. Native evidence:
`target/vfs-vm-native.log`, `target/vfs-stat-failure-serial.log`.

### Native stat caller exit and exec retirement (2026-11-16)

The dedicated `vfs-stat-failure` image now launches disposable embedded stat
callers from boot init. A separate kernel feature, `vfs_stat_failure_test`, grants
only init's direct `/bin/vfs-stat-caller` spawn the pilot metadata role. The
fixture binary and this grant are absent from ordinary images. Its namespace is
provisioned through the existing authenticated snapshot/root-policy path; no
fixture supplies caller identity or namespace authority in a stat request.

The service holds a real delivered reply until an explicit regression control
releases it. A failed exec of a missing image must preserve the pending request,
pilot permission and output canary; releasing that reply must produce the exact
original metadata, followed by a successful fresh call. A caller that exits with
a delivered request must leave the held reply uncompletable. Successful exec
runs a replacement image at the same PID; that live replacement asks the service
to reject the old reply before exiting and verifies its stat call returns
NOT_READY because exec withdrew the pilot role. Init waits for each fixture's
successful exit and checks fresh service metadata after every case. These use
the production exit/exec retirement hooks and normal copied stat transport.

This closes native caller exit/exec validation for delivered, outstanding stats.
Undelivered and already-completed retirement orderings remain covered by host
adapter tests. Concurrent VM replacement inside copyout remains acceptance work;
this gate does not claim that race. Profile lifecycle integration and ordinary
syscall routing remain subsequent work. Ordinary file syscalls still use
KERNEL_VFS.

Validation: 52 host adapter tests pass. The dedicated QEMU failure gate includes
required failed-exec preservation, live exec-replacement denial and caller
exit/exec recovery markers alongside all earlier failure checks. Evidence:
`target/vfs-caller-host.log`, `target/vfs-caller-native-final.log`,
`target/vfs-caller-serial.log`. The no-growth audit remains at 68 references and
zero additions (`target/vfs-caller-audit.log`).
The ordinary `bash tools/test.sh phase3.0` gate also passes after rebuilding init,
VFS and the kernel without the stat fixture features
(`target/vfs-caller-phase30.log`). Existing build warnings remain.

### Native concurrent stat output mapping replacement (2026-11-16)

The dedicated failure image now runs 32 rounds of stat result collection while
an actual sibling native thread repeatedly unmaps and MAP_FIXED remaps the
second page of a cross-page output buffer in the shared VM. Atomic handshakes
require worker mapping progress before each collection and quiesce the worker
before inspecting output. A successful copy must leave the exact metadata
prefix in the stable first page; a failed copy must leave its entire canary
unchanged. The second page may be replaced after successful copyout and is not
used as evidence of a failed copy. Every round checks a fresh metadata request.
The worker exits and is waited for before its stack, TLS and output are unmapped.

This exercises production shared-VM mutation and scheduler-serialized copyout
under native concurrency, extending the earlier sequential unmap/remap fixture.
It is a bounded race stress gate, not an instruction-level forced interleaving
inside the guarded page walk. Both deterministic mapping outcomes remain
covered by the earlier fixture. The new module is compiled only with
`stat-failure-test`; ordinary syscalls continue using KERNEL_VFS. Profile
lifecycle integration and ordinary syscall routing remain subsequent work.

Validation: 52 host adapter tests and the dedicated two-CPU QEMU failure gate
pass, including the required marker
`[VFS-RPC] native stat concurrent VM replacement atomic copy/recovery OK`.
Evidence: `target/vfs-concurrent-host.log`, `target/vfs-concurrent-native.log`,
`target/vfs-stat-failure-serial.log`. The no-growth audit remains at 68 references
with zero additions (`target/vfs-concurrent-audit.log`).
The ordinary Phase 3.0 gate also passes after rebuilding without fixture
features (`target/vfs-concurrent-phase30.log`). Existing build warnings remain.


### Root-profile withdrawal on cwd commit (2026-11-16)

Successful native `chdir` now withdraws the selected caller's root-only pilot
profile under the scheduler guard that commits cwd. It uses the scheduler's
existing shared-resource ownership rule to cover all callers sharing that cwd,
including a leader while a native std sibling performs `chdir`. Native threads
with private cwd snapshots do not withdraw their parent's profile. Failed path
lookup or directory validation preserves authority and outstanding results.
Every successful `chdir`, including a return to `/`, conservatively withdraws;
there is no automatic regrant or general cwd provisioning in this slice.

Withdrawal retires local admission, pending requests and remote retain ownership,
and queues the existing copied caller-retirement barrier. A parked stat owner is
woken before its publication is discarded; nested IPC continuations retain their
own wake ownership. Abandoned results are destroyed by syscall-return cleanup
after lifecycle unlock. The service remains registered and unrelated callers
continue making fresh requests. Root authorization now rechecks the caller's
live pilot grant, including after a retirement barrier has been acknowledged:
an old queued retain must not install policy for a still-live but withdrawn
caller. Retained replies also recheck that grant and compensate late receipts.
Final metadata copyout checks the grant again under the guarded user copy, so a
completion taken just before sibling withdrawal cannot write stale output.

The dedicated failure image adds disposable callers for failed `chdir`, private
sibling cwd mutation, direct successful `chdir`, and shared sibling cwd mutation.
They hold delivered replies across the transition, check exact preserved results
or untouched output canaries, reject stale replies while the original caller is
alive, and confirm that a service snapshot and return to `/` cannot regrant the
profile. Init verifies fresh metadata after each fixture. Host coverage includes
withdrawal before delivery, after delivery and after completion; nested IPC wake
ownership; queued provisioning and late retained-receipt compensation; remote pin
drain and prevention of snapshot regrant.

This starts profile lifecycle integration with fail-closed cwd withdrawal.
General root/cwd profiles, credential-change integration and ordinary syscall
routing remain subsequent work. Ordinary file syscalls still use KERNEL_VFS.
The existing concurrent VM gate remains bounded stress without forced
instruction-level interleaving.

Validation: 55 host adapter tests, the dedicated two-CPU QEMU failure gate and
the ordinary Phase 3.0 gate pass. Evidence: `target/vfs-cwd-host.log`,
`target/vfs-cwd-native.log`, `target/vfs-cwd-serial.log`,
`target/vfs-cwd-phase30.log`. The no-growth ownership audit remains at 68
references and zero additions (`target/vfs-cwd-audit.log`). Existing build
warnings remain.


### Root-profile withdrawal on credential commit (2026-11-16)

Native `setuid` and `setgid` now withdraw the caller's root-only pilot profile
when the authorized operation changes its UID or GID. Credential commit and
withdrawal hold the same scheduler guard, so queued provisioning and guarded
metadata copyout cannot retain authority across the transition. The existing
withdrawal path retires pending requests, wakes pilot continuations and queues
remote retirement without destroying resources under lifecycle locks.

Successful no-op credential calls and denied calls preserve the profile. Current
credentials are per-process, including native shared-cwd siblings; credential
withdrawal therefore targets only the caller. Restoring old credentials cannot
regrant the withdrawn profile. Exec and cwd withdrawal retain their existing
behavior. General cwd provisioning and ordinary syscall routing remain pending.

The host publication-phase matrix now also commits UID and GID changes before
withdrawal. Native disposable callers cover unchanged credentials with a held
result, UID and GID changes with a stale reply and untouched output canary,
denied changes after dropping UID, and GID restoration without regrant. Init
checks fresh metadata from its unaffected profile after each disposable caller.

Validation: 55 host adapter tests (with the expanded publication-phase matrix),
the two-CPU native failure gate and ordinary Phase 3.0 gate pass. Evidence:
`target/vfs-credentials-host.log`, `target/vfs-credentials-native-retry.log`,
`target/vfs-credentials-serial.log`, `target/vfs-credentials-phase30.log`.
The first native attempt stalled in the pre-existing failed-exec fixture after
rejecting its held reply, before reaching the credential cases; the complete
retry passed. Its serial evidence is retained in
`target/vfs-credentials-first-serial.log`. The no-growth ownership audit remains
at 68 references and zero additions (`target/vfs-credentials-audit.log`).
Focused rustfmt checks and `git diff --check` pass. Existing build warnings remain.


### Selected native StatPath routing (2026-11-16)

Native `StatPath` (61) now publishes metadata requests through the authenticated
service path for boot init and the dedicated disposable regression callers.
The syscall parks with the existing caller-owned publication and rewinds the
native SYSCALL instruction on PENDING. On resume it consumes that publication,
converts the validated service metadata to the existing 24-byte native record
(including type bits in mode), and performs one guarded output copy. POSIX
service errors are translated to native libc sentinels; timeout and terminal
service failures produce native I/O failure. The copied pilot API remains
available with its existing framing and PENDING contract.

A separate sticky `vfs_stat_routed` choice survives cwd/credential withdrawal
and successful exec. The withdrawable metadata grant still controls admission
and guarded copyout. A withdrawn or unavailable selected caller receives native
EAGAIN without entering KERNEL_VFS, including after restoring credentials or
returning cwd to `/`. New children and sibling threads start unselected; only
boot provenance or the feature-gated fixture launch selects this route. This
slice does not provision general cwd profiles or migrate other callers, Linux
stat, descriptors, or other file operations. Kernel filesystem ownership remains.

Native acceptance compares ordinary libc stat against copied service metadata
for absolute/relative files, a directory with fixture-specific UID/GID, and root
clamping. It checks native empty/missing/not-directory errors, failed output
copy and fresh-request recovery, timeout/resume, and service loss without
fallback.
Disposable cwd, credential and exec callers also issue ordinary StatPath while
their profile is withdrawn and require EAGAIN with an untouched output canary.
Host tests check the native record layout, type bits, metadata validation and
POSIX-to-native error translation.

Validation: 57 host adapter tests and the dedicated two-CPU QEMU failure gate
pass. Evidence: `target/vfs-route-host.log`, `target/vfs-route-native.log`,
`target/vfs-route-serial.log`. The no-growth ownership audit remains at 68
references and zero additions (`target/vfs-route-audit.log`).
The ordinary Phase 3.0 gate also passes after rebuilding without fixture
features (`target/vfs-route-phase30.log`). Focused rustfmt checks and
`git diff --check` pass. Existing build warnings remain.


### Selected Linux pathname stat routing (2026-11-16)

Linux `newfstatat`, including translated `stat` and `lstat`, now shares the
selected native StatPath continuation and authenticated service lookup. Absolute
paths ignore dirfd; relative AT_FDCWD paths use the retained service cwd directly,
without kernel textual resolution. Relative directory-FD requests return
EOPNOTSUPP because service-owned descriptors do not yet exist. Unsupported flags
return EINVAL; AT_SYMLINK_NOFOLLOW is accepted for the current symlink-free
backends. Empty paths return ENOENT; AT_EMPTY_PATH remains unsupported.

The adapter emits Linux x86_64's 144-byte stat record, with service mount/object
IDs instead of a pathname hash, captured ownership, type/mode, link count and
size. These IDs retain the existing service-epoch scope and do not promise
cross-restart identity. Signed size overflow returns EOVERFLOW. As in the prior
compatibility implementation, timestamps are zero, block size is 4096 and block
count is logical size rounded to 512 bytes. The service profile supplies no
physical allocation or timestamp metadata. Full Linux filesystem semantics remain
outside this metadata slice.

Linux completion preserves POSIX errno values, including ETIMEDOUT; invalid
copyout returns EFAULT. Output is copied under the same guarded caller/VM/grant
check as native StatPath. Selected callers retain fail-closed routing after
withdrawal, exec or service loss. Replayed stat/lstat calls restore the original
argument registers before Linux syscall normalization runs again.

Host tests cover record fields, reserved bytes, types and signed-size overflow.
Native init exercises the normalized Linux entry for shared absolute/relative
identity, metadata, pathname errors, unsupported FD/flag cases, failed copyout
and recovery. The dedicated failure image additionally checks timeout/recovery,
withdrawn cwd/credential/exec profiles and unavailable-service rejection with
untouched output canaries. These fixtures invoke normalized entry 1026 from a
native personality; they do not independently qualify Linux ELF entry translation.
General root/cwd provisioning, service-owned open objects, other syscall routing
and kernel filesystem retirement remain pending.

Validation: 58 host adapter tests, Phase 3.0 and the dedicated two-CPU QEMU
failure gate pass. Evidence: `target/vfs-linux-stat-host.log`,
`target/vfs-linux-stat-phase30-final.log`, `target/vfs-linux-stat-native.log` and
`target/vfs-linux-stat-serial.log`. The no-growth audit remains at 68 references
and zero additions (`target/vfs-linux-stat-audit.log`). `git diff --check` passes.
Existing build warnings remain.


### Selected initial cwd provisioning (2026-11-16)

The selected metadata profile now captures the caller's initial shared cwd under
scheduler serialization, together with its identity and credentials. The copied
service-only `INITIAL_CWD` control operation authenticates the exact delivered
retain and returns a bounded, length-prefixed cwd record. No application-supplied
path or ordinary IPC payload can select policy. Root remains `/`; arbitrary root
profiles are not enabled. The existing root-only authorization operation still
refuses nonroot cwd profiles.

The service resolves this trusted provisioning path once in its shared mount
table, pins the actual root and cwd objects, and rechecks the exact snapshot
before retaining them. The kernel also checks cwd equality when accepting a late
receipt, compensating mismatches instead of installing policy. Relative stat
requests traverse from the retained object without a textual cwd prefix. Rename
and recreation at the original spelling cannot redirect the retained cwd.
Provisioning allocation or path lookup failure leaves no admitted namespace.

This enables nonroot initial cwd for explicitly selected callers. It does not
select ordinary descendants, preserve a profile across successful chdir, or
regrant a withdrawn profile. Successful cwd/credential changes and exec retain
the existing fail-closed withdrawal. Live service-owned cwd transitions,
arbitrary roots, descriptor-relative lookup and open-file lifecycle remain open.

Host coverage adds a complete nonroot handoff across a mounted filesystem,
rename/recreation and relative lookup, plus staged-cwd mismatch and late-receipt
compensation. Codec tests cover bounds, exact envelopes and malformed records.
The dedicated native fixture starts a selected disposable caller in `/etc`,
compares copied, native and Linux relative stat with absolute stat, then checks
withdrawal on chdir. This still uses normalized Linux entry 1026, with the same
Linux ELF qualification limit as the previous slice.


Validation: 61 host adapter tests, 162 IPC library tests and 24 service library
tests pass. The dedicated two-CPU QEMU failure gate and ordinary Phase 3.0 gate
pass. Evidence: `target/vfs-initial-cwd-host.log`,
`target/vfs-initial-cwd-libs.log`, `target/vfs-initial-cwd-native-final.log`,
`target/vfs-initial-cwd-serial.log` and `target/vfs-initial-cwd-phase30.log`.
The ownership audit remains at 68 references and zero additions
(`target/vfs-initial-cwd-audit.log`). Focused rustfmt checks and
`git diff --check` pass. Existing build warnings remain.


### Service-side cwd transition primitive (2026-11-17)

`Runtime::change_directory` now provides the trusted local lifecycle operation
needed before routing live chdir. It requires the active authenticated
registration, exact previous scope, same caller and namespace, a fresh
registration-wide namespace generation, independently captured credentials and a
serialized lifecycle revalidation callback. Ordinary IPC and copied stat requests
cannot invoke it. This API is a service-side prerequisite, not a completed syscall
cutover or a new source of authentication.

The mounted VFS resolves relative paths from the retained cwd and absolute paths
from the retained root, preserving rename identity, root containment, component
search checks and mount traversal. Final directory type and search permission are
checked before opening an independently owned replacement pair under the same
VFS borrow. Partial opens and failed allocation drop provisional handles.

Lookup failure preserves the old binding while consuming the attempted generation.
Failed lifecycle revalidation withdraws old admission and drops the provisional
pair. Successful publication replaces the binding without requiring another policy
slot. Old leases preserve their exact objects for completion accounting and release,
but stat dispatch against the old scope fails after replacement. Stale withdrawal
cannot remove the replacement.

The kernel still withdraws the selected profile on successful chdir. Wiring this
primitive into an authenticated copied chdir request, park/resume, shared-process
cwd synchronization and commit acknowledgement remains the next integration step.
Service-owned descriptors and directory-FD-relative stat remain unimplemented;
the latter continues to return EOPNOTSUPP.

Validation: 61 host adapter tests and 298 library tests (108 filesystem, 162 IPC,
28 service) pass, including five new transition/rollback tests. The dedicated
two-CPU QEMU failure gate and ordinary Phase 3.0 gate pass as regression checks;
they do not claim native coverage of the new local transition API. Evidence:
`target/vfs-cwd-transition-host.log`, `target/vfs-cwd-transition-libs.log`,
`target/vfs-cwd-transition-native.log`, `target/vfs-cwd-transition-serial.log`
and `target/vfs-cwd-transition-phase30.log`. The ownership audit remains at 68
references and zero additions (`target/vfs-cwd-transition-audit.log`). Focused
rustfmt checks and `git diff --check` pass. Existing build warnings remain.


### Copied authenticated chdir and cwd commit (2026-11-17)

Selected native chdir and its existing Linux translation enter a kernel-owned,
bounded continuation before pathname resolution. The kernel copies the path once,
parks the actual calling thread, and queues a fresh retain scope in the same
namespace. The service obtains the old scope, credentials and captured path from
an independently authenticated copied control query, rather than ordinary IPC.
It resolves from retained directory objects, pins the replacement, and returns a
correlated retained receipt and cwd spelling through a dedicated finish syscall.

The finish syscall validates service ownership, exact retain delivery, caller/VM,
credentials and the prior shared cwd under scheduler/broker/runtime locks. It
replaces the local namespace binding and commits the shared cwd before returning
its acknowledgement. The service keeps exclusive runtime access through this
acknowledgement; rejected commits release provisional pins. Old leases drain via
the existing release protocol. Lookup and retention-capacity failures preserve
the old cwd and binding. A failure obtaining a representable cwd spelling also
rolls back the provisional policy.

Shared workers use the selected shared owner's profile while their own identity
owns parking and result delivery. Other independently provisioned sibling profiles
are withdrawn atomically at commit; broader provisioning of those profiles remains
open. A stat continuation in the same shared process makes new chdir admission
return EAGAIN, preserving the existing owned stat result. After admission, stat
and other cwd transitions serialize against that transaction. Continuation origin
correlation prevents a nested syscall from stealing the result, and resumption
consumes the captured operation without rereading mutable pathname bytes.

Timeout or service loss fails waiting calls without kernel traversal or mutation
replay. An uncertain timeout retires the service registration; recovery is a later
step. Caller/VM/credential withdrawal cancels admission, and authenticated late
retention replies still own compensation and release. Terminal delivery wakes a
caller once and respects an outstanding nested IPC continuation. The ordinary
service IPC loop continues to use the same mounted namespace. The service now
uses the existing reclaiming libc allocator instead of its non-freeing 64-KiB
bump heap. Grant-test heap inventory reports live requested bytes. The native
cwd stress gate compares live service heap usage before and after 64 pairs of
transitions, after draining old leases.

Validation: 66 host adapter tests and 299 library tests (108 filesystem, 163 IPC,
28 service) pass. The two-CPU native failure/stress gate and ordinary Phase 3.0
boot gate pass. The stress test verifies 64 pairs of transitions without live
service heap growth. Evidence: `target/vfs-chdir-host.log`,
`target/vfs-chdir-libs.log`, `target/vfs-chdir-native.log`,
`target/vfs-chdir-serial.log`, `target/vfs-chdir-phase30.log`, and
`target/vfs-chdir-audit.log`. Focused rustfmt and `git diff --check` pass.
The ownership audit remains at 68 references with zero additions; this milestone
does not remove kernel engines or implement service-owned open files.


### Retained caller derivation and root-policy narrowing (2026-11-17)

`Runtime::derive_caller` now provides the trusted service-side prerequisite for
broader caller provisioning. It requires the active authenticated registration,
an exact live parent scope, a distinct child identity in the same namespace,
a fresh registration-wide generation, policy capacity and serialized lifecycle
revalidation of both callers. The child owns a separately withdrawable binding
to the exact parent directory pair. It does not resolve the parent's cwd spelling
again, inherit credentials, or select kernel syscall routing. Failed revalidation
publishes no child policy and consumes the attempted generation. Capacity refusal
preserves existing callers and remains retryable. Parent transitions and withdrawal
do not change a previously derived child's objects or admission.

`Runtime::change_root` narrows an existing caller's root beneath its retained
root/cwd using independently captured search credentials and trusted root-policy
authorization. This is a local lifecycle operation, not a POSIX chroot syscall.
It resets cwd to the new root, avoiding a relative anchor outside confinement.
Lookup, final directory/search checks and both replacement opens run under one
VFS borrow. Scope/generation validation, rollback, lifecycle withdrawal and old
lease drainage use the same policy machinery as cwd transitions. Mount crossing
and rename identity are preserved; stale scopes cannot withdraw the replacement.

Mounted cwd reporting now authenticates both retained anchors, checks containment
and returns a root-relative spelling. Renamed roots and mounted roots therefore
do not expose the service-global path; an escaped or overmounted cwd fails closed.

Coverage includes child identity/scope/epoch rejection, generation replay,
revalidation and policy capacity; independent parent/child cleanup; per-request
credentials; rename/recreation; root clamping and mounted roots; failed lookup,
search denial, non-directory targets, partial handle exhaustion and stale teardown.
The copied kernel provisioning protocol, spawn/exec caller selection and root
commit continuation remain the next integration work. Ordinary IPC cannot invoke
these primitives. Service-owned files and kernel engine retirement remain open.

Validation: 66 host adapter tests and 306 library tests (110 filesystem, 163 IPC,
33 service) pass. The dedicated two-CPU QEMU failure/stress gate passes as a
regression check, including repeated chdir heap/pin reclamation. The new local
APIs are covered by host tests; this does not claim native copied provisioning
or root-transition coverage. Evidence: `target/vfs-root-provision-host.log`,
`target/vfs-root-provision-libs.log`, `target/vfs-root-provision-native.log`,
`target/vfs-root-provision-serial.log` and `target/vfs-root-provision-audit.log`.
The ordinary Phase 3.0 boot gate also passes after rebuilding without fixture
features (`target/vfs-root-provision-phase30.log`). The ownership audit remains
at 68 references with zero additions. Focused rustfmt and `git diff --check` pass.
Existing build warnings remain.


### Copied metadata-root narrowing and cwd reset (2026-11-17)

Native `VfsNarrowRoot` (182) accepts a captured NUL-terminated UTF-8 pathname
in rdi and returns the native chdir status ABI. Only an already provisioned
selected UID-zero shared owner and a UID-zero calling thread may begin the transaction;
root-policy authority is checked independently from service filesystem search.
The syscall never enters the kernel resolver. This narrows the service metadata
profile; it is not POSIX chroot or complete process confinement. Open and other
unmigrated filesystem operations still use kernel paths. No Linux chroot entry
is enabled by this slice.

The existing bounded cwd continuation now captures a directory/root operation
kind. Copied context uses a 72-byte header: the prior 64 bytes followed by a
u64 kind (0 directory, 1 root), then the bounded pathname and zero padding.
Unknown kinds and non-exact syscall envelopes fail. The kernel derives kind
from the syscall, not caller memory or ordinary IPC. Context queries authenticate
the exact delivered retain, active service registration, prior scope, caller/VM,
credentials and shared cwd. Resume correlates kind as well as syscall origin,
so a nested directory syscall cannot consume a root result or the reverse.

The service invokes retained-object `change_root`, retaining the narrowed root
and resetting cwd to that root under exclusive runtime access. Finish revalidates
lifecycle and requires the reported cwd to be exactly `/` before installing the
replacement receipt and committing shared cwd. Failed lookup and lease exhaustion
preserve the prior scope and cwd. Rejected or late commits compensate provisional
leases; stale replies cannot bypass finish through ordinary retention control.
Timeout/service loss, sibling withdrawal and old-lease drainage share the existing
cwd transaction machinery. Retained roots survive rename/recreation and mounted
roots report paths relative to their boundary.

Broader kernel caller derivation, spawn/exec selection and service-owned open
objects remain subsequent work. Existing selected callers keep fail-closed
metadata routing after withdrawal; the migration still retains kernel engines.


Validation: 68 host adapter tests and 306 library tests (110 filesystem, 163 IPC,
33 service) pass. The copied-channel host failure matrix now runs both directory
and root operations through authentication, timeout, rejected commit, lease
exhaustion and service loss. Additional coverage verifies UID-zero authorization,
operation-kind correlation, mounted-root rename/recreation, root clamping,
mandatory `/` commit spelling, stale context and duplicate finish rejection.
The two-CPU native failure/stress gate passes with root narrowing, rollback,
root-relative native/Linux metadata, cwd reset and 64 repeated root transitions
without live service heap growth. The ordinary Phase 3.0 boot gate passes after
rebuilding without fixture features. Evidence: `target/vfs-root-wire-host.log`,
`target/vfs-root-wire-libs.log`, `target/vfs-root-wire-native.log`,
`target/vfs-root-wire-serial.log`, `target/vfs-root-wire-phase30.log` and
`target/vfs-root-wire-audit.log`. The ownership audit remains at 68 references
with zero additions. Focused rustfmt and `git diff --check` pass. Existing build
warnings remain.


### Copied retained-object spawn/exec caller provisioning (2026-11-17)

Native spawn and embedded/session launch inherit selected metadata routing from
the authenticated launching task's shared owner. A live admitted source yields
a snapshot of its exact retained root/cwd lease and grant ceiling. The child gets
a fresh scope in the same namespace, independently captured credentials, and a
separately withdrawable policy. Successful exec captures that snapshot before
old-image retirement and provisions the new VM identity after image commit.
Failed exec leaves the old profile intact. Image provenance roles still withdraw.
Old continuations cannot deliver into the replacement VM.

Service-only retention control operation `DERIVE_SOURCE` (8) returns the exact
source receipt for an independently staged, delivered retain. It validates the
service registration and destination identity, VM, credentials, admission and
shared cwd. The service derives only from the corresponding live retained lease,
rechecks the copied context before publishing, and the kernel revalidates again
before accepting the child receipt. Ordinary IPC and an application-supplied
receipt cannot invoke derivation. Initial-root authorization rejects derived
preparations, including after a derivation fails. A separate `vfs_boot_root`
provenance bit gates initial-root staging. Derived profiles and exec replacements
never acquire it, and successful cwd/root transitions clear it. Replacing a
service epoch therefore cannot silently widen those profiles back to `/`.

The source binding has a strong local reference until preparation settles. Parent
exit, exec, root/cwd transitions or credential withdrawal may retire source policy
without releasing that lease prematurely. The service can derive from its pinned
pair after the source retirement barrier; no cwd pathname is resolved again.
Rename/recreation cannot redirect inheritance. Settling the preparation drops
the snapshot and reconciles drained source leases. The destination owns its own
lease, lifecycle retirement and compensation obligations. Refused or stale
receipts never install admission; late successes own compensating release.
Snapshots retain only the binding reference and retrieve its receipt from the
existing bound row. A compile-time registration-size ceiling leaves headroom on
the 32-KiB kernel stack used during registration construction.

A selected source with no live admission (including pending provisioning,
withdrawal, capacity refusal or service loss) passes on sticky fail-closed routing
without new authority. The child receives EAGAIN on metadata operations and is
not silently reprovisioned at `/`. Provisioning remains asynchronous; a runnable
child/replacement may receive EAGAIN until its service turn completes. This does
not make spawn/exec loading use the service: executable bytes and other unmigrated
file operations still use their existing bootstrap/kernel paths. Shared workers
continue to use their shared owner's profile; independent private-thread and
Linux thread provisioning are outside this slice.

Host coverage includes source retirement before delivery, exec VM replacement,
rename/recreation, destination credential/cwd/VM changes, capacity refusal,
authenticated delivery, stale epoch/owner rejection, late-receipt compensation,
service-replacement rejection of initial-root regrant, independent teardown and final pin reclamation. Service tests verify exact-lease
lifetime, revalidation, generation replay, namespace boundaries and retirement.
Native acceptance spawns a child beneath a narrowed root, then execs that child,
checks continued root confinement and independent parent admission, rejects the
old held exec completion, and checks children of withdrawn profiles stay selected.

Validation: 74 host adapter tests and 308 library tests (110 filesystem, 163 IPC,
35 service) pass. The two-CPU native failure/stress gate passes with narrowed-root
spawn/exec inheritance, stale exec completion rejection, withdrawn-parent child
routing, and existing root/cwd heap/pin stress. The ordinary Phase 3.0 boot gate
passes after rebuilding without fixture features. Evidence:
`target/vfs-inherit-host.log`, `target/vfs-inherit-libs.log`,
`target/vfs-inherit-native.log`, `target/vfs-inherit-serial.log`,
`target/vfs-inherit-phase30.log`, and `target/vfs-inherit-audit.log`.
The ownership audit remains at 68 references with zero additions. Focused rustfmt
and `git diff --check` pass. Existing build warnings remain. Service-owned open
objects and kernel filesystem retirement remain subsequent work.


### Service-local regular-file descriptions (2026-11-17)

The service runtime now owns a bounded regular-file description table in each
registration entry. Trusted local preparation opens an existing regular file
relative to the exact live retained root/cwd receipt, checks directory search and
final requested read/write permissions, and revalidates lifecycle before token
publication. Traversal, access checks and backend acquisition occur under the
shared mounted-VFS lock. Rename/recreation cannot redirect the retained anchors;
escaped or hidden anchors, directories and unsupported backends fail closed.
Failed preparation and allocation drop their provisional handle.

Tokens are monotonic within their exact registration, never backend handle
numbers. Every descriptor operation also checks the full authenticated caller
PID, incarnation and VM generation. Each epoch has L descriptor rows separate
from its L retained-lease rows. Duplicate rows share one reference-counted
description, its access mode and serialized offset. Separate opens have separate
offsets. Read/write enforce the established access mode and check offset
arithmetic; seek rejects negative/overflowed positions without changing the old
offset. Fstat and truncate-to-zero use the retained handle, without resolving a
pathname. Truncate preserves the offset. Unlink and pathname replacement leave
an open inode usable until its final description closes. Capacity refusal,
stale tokens and duplicate close do not release another description.

Metadata admission withdrawal preserves established open descriptions. The
existing copied `CallerRetired` frame represents metadata-profile withdrawal as
well as exit/exec; using it for file teardown would incorrectly close files on
credential withdrawal. A separate trusted local `retire_file_owner` operation
closes every description for the exact old owner identity, while registration
retirement closes all descriptions in that epoch. Runtime destruction also
reclaims every remaining backend handle. A future copied file channel must
provide distinct authenticated file-owner lifetime delivery before publication
is enabled for syscall clients.

This slice supplies service-local ownership primitives, not authenticated file
syscall routing. The copied dispatch profile remains metadata-only and does not
grant file-open rights through a metadata receipt. Ordinary IPC does not call
these primitives. Create/append/open flags, directory descriptors, secret/KV
policy routing, descriptor inheritance and close-on-exec behavior, copied file
publication/compensation, syscall descriptor bookkeeping and file-owner lifecycle
transport remain subsequent integration work. Existing executable loading and
unmigrated file syscalls retain their current paths; kernel engines remain.

Host tests cover independent/shared offsets, access rights, exact caller and
epoch checks, seek overflow, capacity refusal, rejected publication, released
receipts, token nonreuse, retained-anchor rename/overmount behavior, unlink and
replacement, metadata withdrawal, explicit owner teardown, registration
replacement and final runtime destruction. The ring3 mounted-namespace fixture
checks duplicate offsets, unlinked-inode lifetime and independent file-owner
teardown using the real service namespace and allocator. Its identities are
fixture values and do not claim authenticated copied file dispatch.

Validation: 74 host adapter tests and 317 library tests (112 filesystem, 163 IPC,
42 service) pass. The native VFS failure/stress gate and ordinary Phase 3.0 boot
gate pass, both requiring the new ring3 fixture marker. The native file check is
service-local and does not qualify a copied file syscall route. Evidence:
`target/vfs-open-host.log`, `target/vfs-open-libs.log`,
`target/vfs-open-native.log`, `target/vfs-open-serial.log`,
`target/vfs-open-phase30.log` and `target/vfs-open-audit.log`.
The no-growth ownership audit passes at 68 references. Focused rustfmt and
`git diff --check` pass. Existing dependency warnings remain.


### Copied file-owner lifetime delivery (2026-11-17)

This next ownership-integration slice adds `FileOwnerRetired` and
`FileOwnerRetiredAck` to the existing authenticated copied retention/control
channel (wire opcodes 8 and 9). Their exact registration, caller PID,
incarnation, address-space generation and nonzero event ID are correlated
independently from the metadata-withdrawal opcodes. Reserved namespace and
lease-generation words must be zero. Ordinary lease requests and ordinary IPC
cannot invoke file-owner teardown.

Kernel exit/termination and successful exec's old-image retirement mark a
separate file-owner obligation in every retained registration. The existing
preallocated caller row now survives metadata acknowledgement until its exact
file-owner acknowledgement. Thus exit after credential/profile withdrawal still
closes descriptions without requiring an allocation or a live metadata binding.
The bounded 64-row inventory charges live owners even after their namespace
leases settle; exhaustion refuses new namespace admission before publication.
IDs remain monotonic when a fully acknowledged row is reused. Retired epochs
remain charged until both lifecycle obligations and all remote leases settle.

The service first handles the existing metadata barrier, preserving files, and
then closes all descriptions for the exact retired file owner on the distinct
barrier. Duplicate delivery is idempotent. Lost event copies and lost reply
copies leave the same barrier retryable ahead of later retains. Undelivered,
wrong-kind, stale-event, foreign-owner and foreign-registration acknowledgements
do not discharge it. Wakeups remain active for pending file teardown but become
quiet after metadata-only acknowledgement when no other work is ready. A new
incarnation or VM generation retains its independently opened files.

This delivers the file-owner lifetime prerequisite identified in the preceding
slice. It does not enable copied file operations or change syscall descriptor
routing. Open/duplicate publication compensation, immutable copied I/O,
continuations, descriptor inheritance/close-on-exec, and special secret/KV
routing remain next. Failed exec still preserves the old owner; successful exec
currently tears down all descriptions belonging to the old service owner. Future
descriptor migration must explicitly transfer surviving non-CLOEXEC entries
before that old-owner barrier.

Host validation: 76 adapter tests and 318 library tests (112 filesystem, 163 IPC,
43 service) pass. Coverage includes metadata acknowledgement followed by later
exit, release-independent owner lifetime, exact old-VM teardown, unchanged files
of another lifetime, lost copies/acks, undelivered/wrong-kind/stale acknowledgements,
full registration/caller correlation, duplicate teardown, preallocated capacity,
retired-epoch drain and wakeup quiet/retry behavior. The source no-growth audit
passes at 68 references; focused rustfmt and `git diff --check` pass. Evidence:
`target/vfs-file-owner-host.log`, `target/vfs-file-owner-libs.log` and
`target/vfs-file-owner-audit.log`. Copied file syscall routing remains unqualified.

The native VFS failure/stress gate passes on an unchanged rerun, including the
existing exit/exec, credential-withdrawal, root/inheritance, repeated-chdir and
service-local file fixtures. The first run stopped at a held-reply/failed-exec
check with a rejected completion; its serial evidence is retained at
`target/vfs-file-owner-native-first-serial.log`. Passing gate evidence:
`target/vfs-file-owner-native.log` and `target/vfs-file-owner-serial.log`.
These native checks qualify regression behavior; file operations themselves
remain service-local and do not qualify copied file syscall dispatch.
The ordinary Phase 3.0 boot gate also passes; evidence:
`target/vfs-file-owner-phase30.log`. Existing dependency warnings remain.


### Copied file operations and syscall descriptor integration (2026-11-17)

This slice adds the copied file-operation channel and wires the descriptor
syscalls onto the service-owned descriptions, but leaves routing disabled.

The channel is a fixed-size copied frame: a 22-word (176-byte) header plus a
4096-byte payload, 4272 bytes total. Operations are Open, Read, Write, Seek,
Fstat, Truncate-to-zero, Close and Duplicate, with separate channel
pull/reply opcodes reusing the existing copied retention syscall. Every frame
carries the full registration and caller PID/incarnation/VM-generation identity,
the request id, the opaque token, the operation arguments and the payload
length. Only Open carries namespace/receipt correlation and caller uid/gid; the
descriptor operations must leave those words zero, because once a description is
open the exact authenticated caller and token are the only authority. Decode is
total: version, operation, identity, reserved words, payload length and fully
zeroed padding are all validated, and the reply is rejected unless it names the
exact authenticated request, caller and operation.

The service decodes one independently authenticated frame and dispatches it
through the existing service-local description API; ordinary IPC never reaches
that path regardless of frame contents. The pull/reply control operations are
driven from the poll loop with a bounded per-poll budget. Open still runs its
independent revalidation and returns a monotonic per-registration token; a
failed revalidation drops the provisional backend handle before publication.

Kernel descriptors name a kernel-side service row that holds only the opaque
token and epoch. Duplicate and fork share that row and a reference count; the
token is never a descriptor number and is never copied to userspace. Only the
final kernel reference may issue a copied Close, and an explicit close(2) of the
last reference does so. Exit, exec and close-on-exec drop the kernel reference
but intentionally emit no kernel-side close: the existing `FileOwnerRetired`
barrier owns that teardown, so withdrawal and teardown stay distinct.

The native and Linux `open`, `read`, `write`, `lseek`, `fstat` and `close`
syscalls route a service-backed descriptor onto the copied channel when, and
only when, the caller has been admitted. Admission parks the caller in
`BlockedOnIpc`, leaves the scheduler before any user copy, and resumes on the
exact completion; a failed user copy never retransmits an executed operation.
Write copies `min(count, 4096)` bytes from user memory into a kernel buffer
before admission. Fstat promotes the native 24-byte record to the Linux x86_64
144-byte layout for compat callers; no worked pointer, length or credential
field is trusted from userspace. Publication now performs the same atomic
park-and-publish transition as the stat and chdir channels, and the shared
expiry/wake path bounds each request and delivers each terminal result once.

Routing is gated on `Process::vfs_file_routed`, which remains false: `init` is
not marked routed and no caller reaches the copied file channel, so `KERNEL_VFS`
stays authoritative for every live open. Enabling it requires descriptor
inheritance and close-on-exec transfer, because surviving non-CLOEXEC rows must
be transferred before the old-owner barrier runs. Create, append, exclusive,
truncate and directory opens fail closed (errno 95) on the routed path; `pread64`
and `pwrite64` stay on the kernel path; directory descriptors, secret/KV routing
and descriptor transfer remain subsequent work. A `dup2` that displaces a
service descriptor still defers that close to the owner barrier rather than
closing immediately; this is intentionally left to the descriptor-inheritance
slice.

Validation: 80 host adapter tests (including four kernel-side file-channel tests
covering park-and-publish, idempotent pull, exact-caller/exact-op reply
rejection, single result take, timeout terminality and non-retransmission, retire
and the Open receipt/credential rules) and 325 library tests (112 filesystem, 167
IPC, 46 service) pass. The service-side tests route a real copied request through
the poll-loop control and prove shared-offset duplicate/close, access-mode
refusal, foreign-registration rejection and owner/registration teardown.
Evidence: `target/vfs-file-op-host.log`, `target/vfs-file-op-libs.log` and
`target/vfs-file-op-audit.log`. The no-growth ownership audit passes at 68
references; focused rustfmt and `git diff --check` pass. Existing dependency
warnings remain. The ordinary Phase 3.0 boot gate and the native VFS
failure/stress gate pass unchanged with the existing
`service open objects shared offset/unlink/teardown OK transport=fixture`
marker; evidence: `target/vfs-file-op-phase30.log` and
`target/vfs-file-op-native.log`. These checks compile and cover the routed
branches but do not exercise them at runtime: with routing disabled there is no
native copied file syscall route, so copied file syscall dispatch remains
unqualified.

### Descriptor inheritance and close-on-exec transfer (2026-11-17)

This slice adds the descriptor-lifetime prerequisite for enabling copied file
routing. It does not enable routing: `Process::vfs_file_routed` remains false and
`KERNEL_VFS` stays authoritative for every live open.

The copied file channel gains a kernel-origin `TRANSFER` operation (8). Its
22-word header names the exact old caller and token; its 24-byte payload names
the replacement caller as PID, incarnation and VM generation. Decode requires a
nonzero token, zero reserved/namespace/receipt/credential words and a fully
nonzero replacement identity. The service rekeys only when the old and new
identities are either the same process lifetime (same PID and incarnation, new
VM generation after exec) or the same address space (thread/fork sharing). It
never moves a description to an unrelated process. The token, shared
description, access mode, offset and backend pin are preserved; the old caller
immediately stops naming the description.

Successful exec captures the old caller before image commit. The kernel sets a
file-transfer obligation before old-image retirement, then publishes one
`TRANSFER` per distinct surviving service row after the new VM is installed.
Only non-CLOEXEC rows survive `take_cloexec_handles`; CLOEXEC rows drop their
kernel reference and remain owned by the existing `FileOwnerRetired` barrier.
The barrier is gated on both the publication gap and every pending transfer
reply. A transfer timeout is terminal: the slot is released, no retransmission
occurs, and the barrier closes anything left untransferred (fail closed). This
ordering prevents the service from observing an empty transfer set or closing a
row that is about to be rekeyed. The replacement caller's ordinary namespace
lease is still provisioned by the existing exec inheritance path.

`dup2`/`dup3` now return the exact displaced service token when the replacement
drops the final kernel reference. The syscall installs the new descriptor first,
then sends a copied `Close`; earlier references still defer to the owner
barrier. This removes the previous unconditional deferral for displaced service
descriptors.

Validation: 83 host adapter tests (including three new kernel-side tests for
barrier gating, transfer timeout and unrelated-owner rejection) and 168 IPC plus
47 service library tests pass. The service test rekeys a real copied description,
proves old-owner `BadHandle`, continuation of the shared offset by the new owner
and survival of old-owner teardown. The kernel checks clean. Evidence:
`target/vfs-descriptor-transfer-host.log`,
`target/vfs-descriptor-transfer-ipc.log`,
`target/vfs-descriptor-transfer-service.log` and
`target/vfs-descriptor-transfer-check.log`. The ordinary Phase 3.0 boot gate and
the native VFS failure/stress gate pass with the existing
`service open objects shared offset/unlink/teardown OK transport=fixture`
marker; evidence: `target/vfs-descriptor-transfer-phase30.log` and
`target/vfs-descriptor-transfer-native.log`.

Remaining boundaries: thread/fork transfer publication is not wired even though
the service relation accepts same-address-space moves; fork remains rejected.
Create/append/exclusive/truncate/directory opens still fail closed on the routed
path, `pread64`/`pwrite64` remain on the kernel path, and directory descriptors,
secret/KV routing and actual routing enablement remain subsequent work. Because
routing is still disabled, no native copied file syscall route is claimed.

The [Helios authority audit](helios-vfs-authority-audit.md) records the current
Qalculate stop boundary: production descriptor routing, a retained positional
read contract for mmap, and authoritative FAT topology and capacity publication.

[VFS-L1](vfs-l1.md) records the early spawn rollback repair and the unresolved
retirement obligation when a namespace owner disappears before replying.
Both ordinary spawn adapters still lack authenticated publication.

[VFS-L1C](vfs-l1c.md) records exact namespace staging request correlation and
full-capacity local PID retention coverage. Authenticated private-child
publication and VFS-L1Q readiness remain open. VFS-D1 remains blocked.

### VFS-D2A: Service-owned private mmap

The [isolated D2A route](vfs-service-private-mmap.md) uses authenticated
D1B admission, stages positional reads outside PMM and kernel VFS locks,
and commits eager private mappings after exact image, epoch, and address
revalidation. Native qualification on one, two, and four vCPUs covers
permissions, private writes, source close, shared cursor preservation,
repeated unmap, rollback and process exit. Live retirement during a
pending native mapping remains open. The historical Linux-compatible eager path still depends on the
kernel backend. Keep that ownership finding open and production routing
blocked pending coordinated cutover.

### VFS-D3: ZRAM independence and private mappings

Keep ZRAM swap-out, compression, decompression, page-table updates and
fault-driven swap-in in the kernel. They must work without disk access,
VFS dispatch or a userspace acknowledgement. Preserve the authenticated boot
policy submission by `sunlight-swapd`; reclaim must not depend on that process
staying alive or on a filesystem service making progress.

KLC-Q1 keeps the existing ZRAM implementation and leaves private file-backed
mmap deferred. Retain the ownership guard and ordinary boot gate. Track native
anonymous swap-out, fault-driven swap-in, restored bytes and permissions,
exit and exec with live swapped pages, image retirement accounting, and
scheduler progress during pressure independently. A kernel fixture that calls
the restore routine directly does not exercise the fault handler. Host and
compile checks do not count as native swap activity.

VFS-D3 compatibility is **BLOCKED** pending native reclaim and fault
restoration of service-populated private pages. D2A provides the isolated population prerequisite. The required
native gate is:

1. Open a real file through the authoritative userspace VFS and populate a
   private mapping with bounded positional reads on a retained description.
   Verify initial bytes and permissions, then write to a writable private page.
2. Drive production reclaim and verify that this exact page has compressed
   backing and a non-present PTE. Access it to trigger a real fault; verify
   the exact modified bytes and permissions. Check that restoration generated
   no VFS request, userspace acknowledgement or disk I/O, and that the file
   contents did not change.
3. Repeat with the original descriptor closed. Where the fixture can safely
   isolate service loss, repeat with VFS unavailable after population. Unmap
   and verify that mapping, frame and slot ownership return to baseline.
   Run under both UP and SMP and record each check as PASS, FAIL, BLOCKED or
   NOT_RUN.

Drop scheduler, PMM and incompatible address-space locks before a blocking
VFS request. Retain the exact descriptor and caller/image identity across the
request, revalidate before publication, and preserve rollback, page ownership
and TLB synchronization when committing or cancelling the mapping.

Track three separate limitations: restored pages are not re-enrolled in
reclaim, tmpfs backing frames are outside the anonymous candidate filter, and
ordinary production descriptors still lack the service-backed population route. Moving tmpfs
to ring 3 does not make its backing pages swappable.
