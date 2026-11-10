# Temporary files

`/tmp` is already provisioned as a sticky, world-writable directory (1777).

Run `mktemp` (also `sunlight-utils mktemp` or `/sunlight-utils/mktemp`) to
create an empty private file and print its pathname. With no arguments, the
utility chooses the first nonempty value of `TMPDIR`, `TEMP`, or `/tmp`, and
appends `tmp.XXXXXX`. An explicit template is used verbatim:

```
mktemp
mktemp /tmp/editor.XXXXXX
```

Templates must end in six X characters. The utility supports `--help`; other
options, including unsafe name-only mode, are unsupported. Exit status is 0
on success, 1 on creation/output failure, and 2 for invalid arguments.
The file remains until the caller removes it. A failed output write can leave
the created file behind.

`sunlight_libc::temp::mkstemp(&mut template)` accepts path bytes without a NUL
and returns an open read/write descriptor. C applications can include
`sunlight-libc/include/stdlib.h` and call `mkstemp(char *)` with a writable,
NUL-terminated template. Both replace the six X characters in place and use
atomic `O_CREAT | O_EXCL` creation with mode 0600. The caller owns closing and
unlinking. The API uses the supplied path directly; environment selection is
utility policy. Randomness comes from the secure rand service and fails closed.

SunlightOS accepts `TEMP` as a compatibility alias in this utility. `%TEMP%`
is Windows shell notation; POSIX shells use `$TEMP` or `$TMPDIR`. Environment
variables need not be set for the default `/tmp` behavior.

The native syscall ABI currently collapses AlreadyExists into generic failure.
The implementation retries generic creation failures and directory collisions
at most 128 times; other failures return immediately. There is no separate
existence check and existing files are never truncated. Exhaustion maps to EIO.

`tmpfile()` exclusively creates a mode-0600 read/write file using the template
`/tmp/.sunlight-stdio.XXXXXX`, then unlinks it before returning the canonical
unbuffered FILE stream. It deliberately uses the boot tmpfs at `/tmp`, rather
than environment directory selection. No name-secrecy guarantee is made.
Entropy failure returns NULL/EAGAIN. Creation and unlink failures return NULL
with the native error mapped to errno; cleanup preserves the failing operation's
error. The current unlink syscall collapses backend errors to generic EIO. An
unlink failure can leave a named file, but never returns a successful stream;
the descriptor and FILE allocation are released.

Tmpfs removes the directory name immediately, retaining the inode, pages and
quota charge until its last open handle closes. Existing shared descriptor
references (dup and inherited descriptors) defer that backend close; the kernel
reaper already drains descriptors on process exit. This lifetime is verified
by host execution of the actual tmpfs/VFS and kernel descriptor code. Native C
compile/link and ELF inspection pass, but guest execution, native entropy and
process-exit reclamation remain UNKNOWN. See the Phase 3 evidence in
[`audit/std-progress.md`](audit/std-progress.md).

Validation: host library tests cover template validation, collision retries,
secure entropy failure, and directory/template selection. Native command
compilation is checked separately; host tests do not execute Sunlight syscalls.

Earlier mktemp validation results: all four focused temporary-file tests pass;
`cargo check -p sunlight-utils --bin mktemp --bin sunlight-utils` and
`cargo check -p sunlight-fs --lib` pass on the configured native target.
The broader host library test run reports failures in allocator and file_ops
tests outside this change; it is not a passing regression gate. QEMU behavior
has not been validated for this change.
