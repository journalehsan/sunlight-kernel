# PY-6 ownership inventory (before production edits)

Native applications -> sunlight-libc Rust wrappers / native C adapters -> SYSCALL -> kernel KERNEL_VFS -> Vfs mount table -> FsNode::Ram (RamFs alias of page-backed Tmpfs) or FsNode::Fat (FatFs -> sunlight-fat -> block device/cache).

The separate services/vfs_server IPC namespace owns its own RamFs and boot snapshot; it is not the ordinary file syscall authority. Application launch registry and sun_open choose programs, not filesystem objects. Rust native std overlay uses native syscalls with explicitly unsupported canonicalize/link operations.

## Ownership and operations

sunlight-fs/src/vfs.rs: FileSystem owns open/close, offset-based read/write, reserve, truncate, sync_file/sync_dir, fstat_handle/stat, mkdir/chmod/chown/unlink/rename/read_dir/handle_path. Vfs owns mount routing, protected/runtime admission, no-replace rename and private publication. No backend seek cursor: kernel fd offsets own seek state. Directory reads use path-based snapshots/pagination, not pinned directory iterators. No filesystem notification contract found in these interfaces.

kernel/src/process/fd_table.rs owns per-process fd entries, rights and shared descriptions. VFS handles have mount tags, local slots and generations. Scheduler snapshots fd state before KERNEL_VFS acquisition; VFS must be released before scheduler acquisition. current_fs_actor captures uid/gid and mutation actor. open retains final permission and capability policies, private-secret and KV exceptions. Native stat and readdir have differing path/permission behavior. Cwd is shared-process textual state, not a pinned directory. Directory-relative Linux lookup reconstructs a name from a handle. That name is not an atomic authorization token.

Tmpfs dynamic records own TmpfsInode PMM pages, monotonic identity, linked state, uid/gid/mode and paths. Open slots retain entry indexes; removal repairs other indexes; unlink retains records while open. Seed objects use index+1; implicit root uses 0; materialization preserves identity. Rename changes paths, not inode identity. No timestamp lifecycle exists. Link counts are a native convention, not directory POSIX accounting.

FatFs slots own cluster, cached size/type, generation and a pathname. Reads use retained cluster; writes/truncate resolve the remembered pathname. unlink frees backend allocation without a common inode pin. Multiple opens cache size independently. Consequently stable object identity and mutation-safe descriptor pinning cannot be promised for FAT. Raw FAT stat only exports cluster/size/type. FAT case-insensitive component lookup exists; no symlinks or native POSIX permissions. Adapter ownership/mode are mount policy, not on-disk metadata.

## Application ownership

SunShell main.rs normalize_path and native_script.rs path perform lexical joining/removal; cd/pwd keep their own cwd. sunlight-files main.rs PathBuf trims surrounding whitespace and removes dot components; file_ops.rs has another absolute lexical normalizer and bounded tree transfers. Files metadata and listings are syscall-backed but navigation is lexical. Program selection uses sun_open. Native std's unsupported canonicalize is separate from host std implementations. Editors/viewers/media primarily use libc or native std for I/O; their presentation and picker paths require separate qualification.

## Source baseline

source-baseline.json records incoming revision/branch/status and regular-file SHA256s; source-identity.json records the pinned CPython archive/member comparison and matching build interpreter. Existing changes are preserved. locked-baseline.log records the retained locked native build. No old evidence will be edited.
