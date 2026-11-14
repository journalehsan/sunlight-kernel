# PY-5 pre-implementation design

Source inspection precedes production edits. Existing metadata ABI stays unchanged.

| POSIX API | Existing native mechanism | Missing contract | Minimal safe action |
|---|---|---|---|
| stat | syscall61 -> Vfs::stat -> backend FileStat | device/inode, timestamps, component search permissions, symlink target | BLOCKED: retain native subset; do not declare POSIX success |
| lstat | same backend only File/Directory | symlink storage and final-component no-follow | BLOCKED: no synthetic link adapter |
| fstat | syscall48 -> generation-tagged VFS/backend handle | full metadata and descriptor pin across concurrent close | BLOCKED: retain existing subset, evaluate handle lifetime |
| realpath | lexical app normalizers; Vfs::handle_path | permission-checked existing-component walk, symlinks, authoritative spelling | BLOCKED: no lexical substitute |

A direct VFS reuse supplies native subset only. B metadata/resolution extension requires backend identity/time policy, symlink representation, all-backend operations and ancestor search authority; that is broader architecture and deferred. C shared client cannot manufacture absent authority. D libc translation cannot fill missing fields honestly. No new resolver or POSIX declarations are proposed.

One narrow production repair is justified independently: Vfs::handle_path currently treats every absolute backend-local path as a global path and loses non-root mount prefixes. Reproduce with mounted tmpfs handles, then fix mount/local joining at VFS owner. Test root, mount-root, nested paths, insufficient buffer and stale handle. This API reports a handle-associated pathname, not realpath or authorization; do not use it to implement realpath. Update its inaccurate captured-at-open wording: tmpfs reconstructs current entry path; FAT retains open spelling.

Retain bounded local buffer, generation checks and mount pins. No ABI, metadata, syscall, permission, namespace or capability changes. Native applications continue their current public paths. POSIX contracts require a separately scoped successor proposal.
