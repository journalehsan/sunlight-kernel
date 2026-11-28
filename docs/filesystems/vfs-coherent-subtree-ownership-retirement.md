# VFS-C2E coherent subtree ownership retirement

**PARTIAL.** The opt-in `vfs_c2e_candidate` fences the existing
`/root/Downloads` subtree in the kernel before installing the legacy root
and mounts it as protected tmpfs in the authenticated VFS service. The
ordinary boot gate is **BLOCKED** by the unchanged ownership guard. Runtime
retirement and coverage across the full supported caller population remain
unqualified.

## Boundary and retirement

`/root/Downloads` is an existing root-owned, mode 0700 data directory in
the initramfs. It has no seeded files or embedded executable and was not
found in early boot configuration or the legacy native C guest fixture. The
candidate uses a cold-start boundary. At kernel VFS construction, it
validates the exact seed entry and rejects any seeded descendant before the
first backend can be installed. Legacy traversal and creation then reject
the mountpoint and descendants, while legacy unlink, rename and mount
installation reject paths that can remove, relocate or shadow it. The boot
seed still supplies the empty mountpoint entry. A directory listing of its
parent can still expose that seed name.

The service verifies that the mountpoint has the expected metadata, is
empty and has no open backend handles before installing the protected
mount. It completes setup before publishing directory leases. These
conditions avoid a live handoff. There can be no old open description, cwd
pin, outstanding operation or private file-backed mapping acquired through
this kernel VFS inside the boundary. The legacy executable-source reader is
also fenced by VFS lookup. Existing buffers or executable images outside
this boundary keep their existing lifetime contracts. A nonempty seed,
live service-side handle or existing service-side content prevents the
cutover. The candidate copies no mutable data.

## Consumers

| Consumer | Candidate behavior |
| --- | --- |
| Native Open and Linux Open/OpenAt | The generic open adapter selects the mount by path and submits a copied request using the caller's existing C1A receipt. The resulting authenticated service description carries I/O and lifetime authority. |
| Native StatPath, Linux metadata and native/Linux mkdir, unlink and rename | Supported pathname adapters submit through the same service. Generic rename rejects a boundary crossing. |
| Ordinary mkdir, stat, cp and cat | Calls within the subtree use the generic pathname and descriptor routes regardless of the executable name. Either endpoint of a copy inside the boundary uses service authority. Actual utility execution is NOT_RUN. |
| Dup, close, I/O, exec and exit, private mmap | Service descriptions use the existing descriptor, cleanup and D2A/D2B private population contracts. Native resource settlement is NOT_RUN. |
| Retained directories and cwd | The service supports retained directory objects, exercised by host coverage. Generic legacy chdir into the mount explicitly fails. Directory FD open and service-dirfd-relative Linux operations remain unsupported. Existing fully service-routed profiles continue to use the retained directory transition. |
| Executable sources | Embedded executable loading stays outside this subtree. Kernel-backed source reads for non-embedded Spawn and Exec remain in source and fail against the retired boundary. Service-backed executable loading is outstanding. |

Generic routing requires a live receipt and rejects rebased root profiles
on the path splice. Processes without an applicable receipt fail closed.
Other legacy pathname adapters that lack a service implementation, such as
legacy directory enumeration inside the mount, cannot traverse it. Append
and directory opens retain explicit rejection in the existing service open
adapter. Service withdrawal has no legacy fallback. A successive service
owner cannot recover the lost in-memory contents of an exited tmpfs owner.
Service-loss and replacement behavior were not exercised in a guest.

The change eliminates the legacy open/create producer paths **for this
subtree** when the candidate is enabled. No generic legacy producer source
site was removed. Unrelated native and Linux opens, inherited legacy
descriptions, SSH private secret creation, and native C guest handles remain.
Legacy cwd and executable-source machinery remain reachable elsewhere.

## Guard and validation

The initial working tree already contained a partial Downloads candidate.
The unchanged guard found 73 references and six unmatched sites on entry.
After consolidating initialization into the existing kernel VFS construction
and applying the reservation before its first mount, the guard reports 71
references and four unmatched sites. No-growth and final removal both
**FAIL**. Each historical finding remains reachable.

| Historical finding | Disposition |
| --- | --- |
| `storage::mount` | Remains for legacy FAT construction through StorageCtl and the C2B reservation check. |
| `linux_procfs::mountinfo_snapshot` | Remains for legacy Linux procfs topology snapshots. |
| `sys_linux_statfs` | Remains for legacy mount identity and FAT capacity lookups. |
| `process::mmap::sys_mmap` | Remains for private mappings of surviving legacy descriptions outside Downloads. |

Focused host tests cover boot reservation, rejected nonempty seed, legacy
ancestor moves and mount overlays, service rejection with pins or content,
retained-path mkdir/stat/open, stable reads after rename and return to the
baseline handle count. They also read an unaffected boot-seeded sibling
file. This measures host VFS handles and bytes. Native frames, retained
leases, descriptor cleanup, boot-resource integrity and C1E10–C1E12/C2B
replays are **NOT_RUN**. The ordinary boot script stopped at its original
ownership check before building or launching a guest. The prepared C2C
GPT fixture was untouched and its image checksum was rechecked.

See the [machine-readable results](vfs-coherent-subtree-ownership-retirement-results.json).
