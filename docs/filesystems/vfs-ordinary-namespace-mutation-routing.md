# VFS-C1E9: Ordinary namespace mutation routing

The opt-in runtime routes native mkdir, unlink, rename and rename-no-replace,
and Linux mkdir, unlink, rename and their supported at variants through the
copied authenticated VFS file channel. Production callers keep their existing
routes. On the service route, absolute paths ignore dirfd and relative paths
use the retained cwd for AT_FDCWD. Explicit relative directory descriptors are
rejected: the existing service descriptor channel admits regular files only,
and legacy kernel descriptors do not establish service directory identity.
Nonzero unlinkat flags and renameat2 remain unsupported.

The service resolves and authorizes both rename operands under one VFS lock
using the caller's retained root and cwd. It delegates to the existing VFS
mutation methods, preserving cross-mount refusal and backend handle lifetime.
Pull checks the caller, credentials and receipt; completion additionally
matches the service registration, exact caller, operation and request ID.
Mutation frames are dispatched once. If the caller or service is lost after
dispatch, the result may be uncertain; the kernel does not replay the
operation or try the kernel namespace.

The native mkdir/stat/chdir sequence targets a directory created only in the
service view. It exercises the sequence formerly split between kernel mkdir
and service lookup. The live fixture also checks relative mkdir, file and
directory rename, collision or flag refusal, unlink, and retained cwd.

FAT publication remains **BLOCKED**. Committed FAT mutation and Busy behavior
are not guest-qualified. Publication still requires consuming the exact
preparation receipt atomically with attachment, caller and epoch validation,
lease ownership, and compensation for an unacknowledged commit.

See the [results](vfs-ordinary-namespace-mutation-routing-results.json) for
fresh checks, retained failures and the limits of qualification.
