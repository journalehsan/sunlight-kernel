# VFS-C2I runtime root authority transition

**PARTIAL.** The VFS server can prepare its boot root without creating a
pre-readiness object in the kernel root or using native account-file
descriptions. The writable service root is exercised by the existing
isolated storage candidate. Ordinary activation remains **BLOCKED**.
Production pathname and executable routing retain their defaults.

## Cold-boot boundary

1. The kernel probes block hardware and prepares a read-only FAT share page.
   Without compressed resources, both boot environments use their compiled
   immutable inventory. With compressed resources, Limine provides a
   single validated archive. The kernel parses its entry inventory and
   maps the same physical archive into the VFS image read-only. The
   existing decoder reads bounded blocks from that archive on demand.
   The immutable blobs provide provenance independently of anyone's
   ownership of the writable namespace.
2. The kernel initializes tmpfs and mounts the legacy writable root.
   It can attach early FAT at `/boot`, `/mnt` or `/state`, subject to
   the C2B reservation and fstab deferral rules. The pre-readiness
   `/state/users` creation occurs only after the persistent FAT mount
   succeeds. Its lifecycle belongs to the persistent volume. Qualification
   features can also manipulate their own early storage or capture data.
3. The kernel prepares embedded init (PID 1), the trusted embedded VFS
   image (PID 3), and the framebuffer-backed TTY server (PID 4) before
   normal scheduler dispatch. Init supplies the name registry and drains
   service registration messages while waiting for readiness. VFS needs
   its embedded text, stack, adaptive heap, read-only boot inputs, IPC and
   init's registry. TTY is started early for the framebuffer and login
   interface and is independent of establishing VFS root authority.
   No ordinary base server is required to construct or retain the root.
4. VFS creates its endpoint and registers with init. The kernel checks
   the boot-image trust flag, endpoint owner, exact process incarnation and
   address-space generation. Registration establishes endpoint identity.
   VFS builds its mount table from the immutable fstab and seed, retains
   a shared mount owner and services the kernel-authenticated copied
   registration/retain exchange. Init waits for successful copied root
   metadata on that retained namespace before starting `storagectl fstab`
   and its base services. Sunlightd launches additional services. TTY and
   the session services can then dispatch applications. The C1A candidate
   stages ordinary child publication until the exact retained namespace
   receipt is validated. Its publication and exit transactions are reused.
5. The production compatibility service mounts its existing RamFs view.
   The C1E2 storage candidate selects the writable Tmpfs branch using the
   same seed and mounted namespace owner. Both pass through
   `BootRoot::mount`. C2I does not select the writable branch for ordinary
   boot or initialize a new filesystem authority. The existing C2E
   Downloads fence remains as before.

## Bootstrap source changes

- Removed the VFS server's pre-readiness native mkdir probe. It previously
  created `/tmp/.native-stat-mkdir-test` in the kernel root and left it
  there for the life of the boot. The equivalent native missing-path errno
  and directory type assertions run in the isolated C1E7 mutation client
  after that client can reach its service root. The two ordinary marker
  expectations for the former probe were removed.
- Changed the service's startup permission checks to read the boot records
  directly from the immutable entries. Fstab loading uses the same lookup,
  removing its temporary read-only RamFs mount. The former permission
  probe entered compatibility READ
  and opened native descriptions for public account data before publication.
  Those boot diagnostic opens are gone. Ordinary public account requests
  still use the existing runtime compatibility adapter and must be
  incorporated into coordinated routing.
- Removed the startup lookup of `resolved`. Init starts it after the root
  retention handshake. The immutable `/etc/resolv.conf` provides the
  existing default text from the first mount. Compatibility OPEN and STAT
  retain their runtime refresh. The remaining startup filesystem diagnostics
  use service-local objects, including the existing retained
  `/tmp/vfs-pins` fixture, and the read-only share page for boot diagnostics.
  No live legacy state was copied or discarded.

There are zero newly migrated ordinary consumers and zero retired ordinary
regular-file description producers. Generic OPEN/openat still reaches
`open_resolved_path` and installs kernel descriptions on legacy root and
FAT. The protected SSH staging and published reopen still use the kernel
owner. Native spawn can inherit legacy descriptions and the opt-in native C
runner still installs legacy capture streams. Embedded bootstrap ELF loads
and the immutable archive do not create user file descriptions. C2H's
incarnation retirement and failed SSH descriptor publication changes remain
in place.

## Cold-boot mount policy prerequisite

Early kernel GPT deferral and Ring-3 VFS startup now parse the same
immutable `/etc/fstab` inventory entry. The kernel previously consulted
the compiled text even when Limine provided a separate boot inventory.
A GPT source named by that inventory now skips early legacy automount.
Missing, oversized, unreadable or invalid fstab also suppresses early GPT
automount. Raw volumes without a PARTUUID retain their existing policy;
valid stock configuration keeps its existing routing. The kernel uses
heap scratch for the bounded read and creates no filesystem handle.

This changes when a configured GPT source can first be mounted. It does
not change which owner StorageCtl assigns it, so it retires zero ordinary
producers. A focused host regression covers matched and unmatched GPT
sources and invalid seed entries.

## Activation dependencies

The next source dependency is to reserve the eligible persistent extent
before its first mutable mount and publish its committed service mount
through the authenticated VFS before ordinary dispatch. Early
`/state`, `/mnt` and `/boot` FAT and unreserved runtime StorageCtl devices
still have legacy owners. The read-only boot share is a boot resource,
independent of a full FAT mount. A service-root switch without the
corresponding mount crossings would hide supported paths or expose a
seeded shadow of a persistent path. Select an exclusive owner before
the initial mount and publish the persistent crossings before admitting
ordinary callers. Retain C2B reservations. Reuse committed C1E10–C1E12
service attachments for covered devices, with settlement and exclusive
ownership for each remaining transport. Keep the current devices and
routing until that is possible.

At the same activation boundary, publish authoritative mount and retained
root/cwd state before ordinary dispatch and prevent generic pathname paths
from selecting the legacy root. Shared-cwd workers still hit the
`shared_process_index(idx) != idx` admission rejection for retained pathname
operations. Admit them with the shared owner's retained anchor while
authenticating the actual calling incarnation and credentials. Qualify
native and Linux OPEN/openat, directory and cwd operations, Linux append
and ordinary positional I/O, the service descriptor/stdio transfer
boundary, private mmap, and exec/exit reference accounting together.
Protected SSH create, publish, cleanup and reopen need a single service
owner with the existing HostKeyAdmin, parent, ownership and mode checks.
Trusted KV and synthesized account policy also need to hold under the
authoritative route. Service loss must reject requests without recreating
or accessing a legacy root.

The executable snapshot cap remains 1,048,576 bytes. The measured embedded
ordinary artifacts that would exceed it if published byte-for-byte as
service sources are shown below. The existing embedded source selection
still serves these applications. In particular, `sunlightd` configures
`/sbin/sunlight-tls` as an ordinary managed service, so the bound matters
before all ordinary boot services can switch to service-supplied ELF bytes.
These sizes are whole local ELF artifacts, which is the relevant measure
for the current snapshot protocol.

| ELF | Bytes |
| --- | ---: |
| sunlight-tls | 1,217,328 |
| dict | 14,058,032 |
| yazi / yazi-baseline (key-injection fixture) | 26,942,728 / 24,379,624 |
| sunlight-chronos | 1,053,304 |
| silicon-echoes | 1,221,240 |
| sunlight-files | 2,342,016 |
| voice-recorder | 1,073,784 |
| melody-mina | 1,241,720 |
| rappid-rabbit | 5,502,192 |
| sunlight-dialogd | 1,889,096 |
| sunlight-vortex-shell | 5,813,072 |
| control-panel | 2,397,008 |
| sunlight-docserv (optional rendering service) | 5,043,424 |
| sunlight-docview, two source variants | 1,454,232 / 1,127,312 |

## Ownership guard

Inventory **PASS**, 71 before and after. No ownership sites were added or
removed. Two existing lines in kernel startup shifted due to the policy edit.
No-growth **FAIL** on the same four historical findings. Final removal
**FAIL** on the same 71 references. Guarded ordinary boot is **BLOCKED**
before QEMU.

| Historical finding | Reachable owner |
| --- | --- |
| `storage::mount` | Yes. Unreserved StorageCtl devices still construct legacy FAT and fstab can request that route. |
| `linux_procfs::mountinfo_snapshot` | Yes. The legacy caller graph is still sourced from `KERNEL_VFS`. |
| `sys_linux_statfs` | Yes. Unrouted Linux callers still obtain mount capacity from the kernel owner. |
| `process::mmap::sys_mmap` | Yes. Readable regular descriptions from generic legacy root and FAT opens still reach kernel stat and positional page population. |

## Qualification

The new host regression starts from the boot inventory, reads the seed
without a resolver or a kernel account view, mounts the writable service
root and demonstrates that the service policy rejects publication before
binding. It then checks retained-cwd relative create through the copied
file decoder, caller credentials, service-owned token I/O and positional
read. Registration withdrawal denies further file I/O and closes file
descriptions. Release of the delivered namespace receipt closes its two
directory pins. The ordinary native and Linux transition cases, ordinary
process admission under the new root, executable acquisition across that
root, protected secrets, and guest retirement under ordinary activation
are **NOT_RUN**. The host registration and isolated storage fixture are
separate evidence. See the [qualification results](vfs-runtime-root-authority-transition-results.json)
for test status and logs, including the unchanged C2C GPT checksum.
