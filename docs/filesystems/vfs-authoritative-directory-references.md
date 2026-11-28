# VFS-C2F authoritative directory references

**PARTIAL.** The opt-in Downloads candidate now has authenticated directory
descriptions and directory-relative file and namespace operations. Ordinary
native and Linux cwd entry into Downloads remains incomplete. The host
tests exercise the existing retained cwd implementation directly.

## Directory descriptions

Read-only `O_DIRECTORY` opens use the existing copied file channel and
publish into the existing descriptor tables. The service distinguishes
directory and regular-file descriptions and pins the backend directory
handle. Descriptor duplication shares the pin. Final close drops it.
The existing caller and registration epoch checks apply before dispatch.
The service selects the directory by caller and token in the active
registration, validates the namespace receipt, and walks from the pinned
object under the filesystem lock. The backend handle encodes the mount
incarnation. Userspace syscalls supply an FD and the kernel supplies its
service token. No new descriptor table is involved.

Within Downloads the candidate supports directory-relative Linux
`openat`, `mkdirat`, `unlinkat` with zero flags, and `renameat` when
both paths are relative to the same service description. It also supports
`fstat` and `newfstatat` with an empty pathname and `AT_EMPTY_PATH`.
The retained walk rejects a descriptor-relative traversal outside
Downloads. Mixed-anchor rename, nonempty dirfd-relative stat,
directory enumeration, and directory removal via `AT_REMOVEDIR` remain
outstanding. Directory data I/O and write-mode directory opens are rejected.

The absolute Downloads splice and retired kernel subtree fence remain in
place. The candidate file channel admits ordinary Downloads path requests
through the caller's authenticated receipt independently of the executable
selection used by the earlier pilot. Service descriptor and path requests
fail closed on admission failure or service loss.

## Qualification and remaining ownership

Host tests cover opening and entering Downloads, leaving it through the
service-side retained walk, directory identity after rename, relative file
creation and lookup, mkdir, rename, unlink, stale tokens, file-as-directory
rejection, out-of-subtree rejection, descriptor duplication, retirement,
and a return to the baseline backend handle count after final close.
Ordinary cwd selection and cross-owner cwd handoff still need syscall
integration, including relative routing and exec and exit qualification.
Service-backed executable loading also remains outstanding.

The unchanged ownership inventory reports 71 sites before and after.
No-growth and final removal both **FAIL**. Ordinary boot is **BLOCKED**
at the guard. Native and Linux guest syscall qualification and guest
regression replays are **NOT_RUN**.

| Historical finding | Disposition |
| --- | --- |
| `storage::mount` | Remains for legacy FAT attachment and its C2B reservation check. |
| `linux_procfs::mountinfo_snapshot` | Remains for legacy Linux procfs topology snapshots. |
| `sys_linux_statfs` | Remains for legacy mount identity and FAT capacity. |
| `process::mmap::sys_mmap` | Remains for private mappings of legacy descriptions outside Downloads. |

The prepared C2C GPT image passed its existing SHA-256 check. The
[machine-readable results](vfs-authoritative-directory-references-results.json)
record the build and host test outcomes.
