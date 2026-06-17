# SunlightOS Filesystem Write Policy

## Immutable Root

Root and OS directories are immutable by default. Read/list operations are separate from write policy, so readable immutable directories can still be listed.

## User Writable Paths

Normal users may create and write files only under:

- `/tmp`
- `/home/<user>`

## Runtime Paths

Some runtime paths such as `/run` may require UAC approval. The current filesystem adapter is deny-by-default unless the caller supplies an approved UAC decision to the shared policy function.

## Protected Paths

The following paths are not writable by normal users, even with UAC:

- `/boot`
- `/kernel`
- `/bin`
- `/sbin`
- `/services`
- `/etc`
- `/proc`
- `/sys`

## Service State

Trusted services write only to their own state directory:

- `/state/<service>`
- `/var/lib/<service>`

The initialized service state roots are:

- `/state/sunlight-kv`
- `/state/sunlight-tls`
- `/state/sunlight-uac`
- `/state/capability-broker`

## UAC

UAC approves limited user-visible privileged actions. UAC does not override protected immutable OS paths.

## Capability Broker

The broker is an internal OS service. It mints scoped capabilities only for trusted OS services after policy approval. It does not ask the user directly, and it is not a general bypass for filesystem limits.

Filesystem capabilities are path-scoped, subject-scoped, and rights-scoped. They cannot grant global `/` write and cannot override protected immutable paths.

## Error Semantics

- `EROFS`: immutable/read-only region
- `EACCES`: actor lacks access to a writable region
- `EPERM`: operation requires elevated permission or a valid capability

## Current Integration Note

Kernel file syscalls pass the current process uid/name through the common policy function. IPC `vfs_server` write messages do not yet carry caller identity, so raw IPC writes are denied by default until that protocol grows authenticated actor metadata.
