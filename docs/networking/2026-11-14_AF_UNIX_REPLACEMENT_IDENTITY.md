# AF_UNIX connect and readiness identity

AF_UNIX pathname `connect` uses `FdTable::replace_handle` to change the
backing representation of the same descriptor/socket from an unconnected
Unix pool entry to endpoint 0 of a pipe-backed stream pair. It preserves the
shared description, descriptor flags, epoll registration token and event data.
It is distinct from `dup2`/`dup3` replacing the logical object at a descriptor.

Local epoll interests are fd-keyed and calculate readiness from the current
entry on each collection. They do not capture the old Unix pool index. Poll
uses the same readiness calculation. Before connection an unconnected socket
has no readiness; a listener has IN only with pending accepts; a connected
stream has pipe-ring readiness. Dropping an ordinary pre-connect registration
would break this supported transition. Local registrations have no INET
external transport identity or generation to invalidate.

`sys_linux_connect` holds SCHEDULER from source lookup through enqueue,
replacement and wakeup. Close, reuse and registration collection cannot
change that source slot in the interval. The pair starts with one reference
per endpoint: the descriptor consumes endpoint 0, the listener queue consumes
endpoint 1, and accept transfers endpoint 1 into a new descriptor. Successful
replacement releases the displaced Unix reference once. Queue rejection closes
both new pair references and leaves the source untouched. The post-validation
replace failure branch cannot be reached by concurrent source close under this
lock; it is not runtime fault-injected.

Close detaches the fd key before removing the entry. Reuse requires a fresh
registration. Rejected missing-listener, full-queue and already-connected
operations preserve the existing usable descriptor and registration.

Evidence and precise validation scope:
`docs/audit/probes/native-foundation/results-2026-11-14-af-unix-replacement/REPORT.md`.
The native actor exercises existing compatibility dispatcher numbers with a
native ELF; the Linux actor exercises Linux syscall translation and packed
Linux epoll events. Native C `socket(AF_UNIX, ...)` creation is not provided by
the current IPv4 creation wrapper; existing native C socketpair is separate.
This does not extend alias/dup/fork/exec attachment semantics or Linux OFD epoll
lifetime. Connect on a listening source is specified separately: it fails with
`EINVAL` and preserves the listener, its pathname, pending queue and
registrations; see `docs/networking/2026-11-14_AF_UNIX_STATE_VALIDATION.md`.
