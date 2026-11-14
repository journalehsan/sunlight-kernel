# Future SunApp installation contract

SFS-3 implements no package format, installer, authentication, signatures or sandbox.
For the supported same-volume bounded namespace, a future installer may:

1. Create a temporary file/directory through an authorized VFS adapter.
2. Write and verify the complete package with future SunApp/security services.
3. Fsync its data. The hosted engine commits the whole serialized volume cut.
4. Rename/replace the active entry atomically, preserving old open regular objects.
5. Fsync the containing directory (engine `fsync(root/linked directory selector)`
   currently commits the entire cut). A file fsync before rename does not cover it.
6. On crash, recover the last complete eligible checkpoint. Interrupted publication
   can select old or new state; each is a complete namespace/data closure. If the
   rename checkpoint was acknowledged under the conforming persistence model, the
   new state survives. Corruption fallback is read-only and is not a durability
   promise against arbitrary media damage.
7. Close old regular-package handles; reap eligible persistent orphans in a later
   admitted transaction. Old checkpoint closures independently retain physical
   backing until their durable slot retirement.

A nonempty installed directory cannot be replaced as a destination directory in
SFS-3. A single regular-file package is the supported atomic replacement example.
A directory package needs a future indirection/activation contract or a supported
atomic directory-exchange design; do not infer it from moving a directory subtree.
Retained directory handles/openat, authentication, signature verification, mount
rights, concurrency and native durability qualification remain future requirements.
The 16 regular objects (including old orphans), 16 nonroot directories, 16 entries
per directory, 8-level depth, 64-byte names, 64 KiB files and configured snapshot
transaction/reserve bounds make this a foundation, not an installer-ready system.
Admission failure is an explicit error; uncertain I/O freezes the writer and can
leave a durable new state without an acknowledgment. Callers must preserve that
uncertainty and must not retry installation as if failure implied rollback.
