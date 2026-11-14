# ADR 0003: SFS-3 bounded namespace and retained object lifetime

Date: 2026-11-14. Hosted scope. Accepted before implementation.

No incompatible layout change is needed. Experimental family 0 revision 2 already
encodes directory parent identities, exact nlink, map entries for unlinked nodes,
and sorted 32-byte orphan records with unlink generation and reason 1. Preserve
all codecs, formatter, geometry and golden fixtures byte for byte. Unknown versions
remain unsupported. SFS-2's engine rejects nonempty orphans/hierarchies explicitly;
this is expanded engine support for existing structures, not reinterpretation.

Keep 16 regular objects total (including orphans), 64-byte components, 64 KiB files.
Add at most 16 nonroot directories, at most 16 entries per directory, depth 8
(root depth 0, files count toward depth), 64 input components per walk, 16 orphan
records or the lower superblock cap, 64 public descriptors and 32 open descriptions.
All persistent object counts also obey superblock caps. No NID recycling.

A node stores identity independently from its single parent/name binding. An empty
binding name denotes an orphan internally, never a valid namespace entry. Persistent
orphans stay mapped with zero links and complete backing until an ordinary admitted
checkpoint removes them. Unlink and replacement reserve orphan-table capacity before
mutation even for a closed object; table exhaustion is a safe bounded error. Last
close releases only runtime ownership, queues eligibility, and never writes or frees
physical blocks. Explicit bounded reap removes eligible objects in one full-volume
commit; cold recovery drops volatile references and exposes every orphan as eligible.
Mount remains read-only inspection. A read-only mount never reaps. No automatic
cleaner is introduced. Reaping NoSpace leaves the working objects intact; uncertain
I/O freezes the owner and retains reservations as in SFS-2.

Public descriptor tokens are opaque, mount-qualified, monotonic and never reused.
Dup creates a separate token sharing a description (offset and access mode).
Independent opens have independent offsets. Closing consumes exactly one registered
token; copied stale tokens cannot close twice or target a replacement. Descriptions
pin nodes independently of namespace and checkpoints. Runtime bytes are owned copies;
no runtime read points into reclaimed disk blocks. Every commit copies all remaining
objects, including live orphans, to private admitted backing. Physical protection
remains the union of retained closures plus quarantined reservations. No second
physical free path exists in close or reap.

Legacy FileId APIs are trusted, mount-scoped object selectors for linked objects,
not capabilities, descriptors or pins. They become stale after unlink. Only opaque
opened descriptors retain orphan access. Future VFS owns credentials, descriptor
tables, rights and operation pins; its adapter maps those pins to this single engine
lifetime policy. Hosted access modes restrict descriptor operations but do not
implement user authentication. Directory selectors do not pin; retained directory
open/relative lookup is unsupported. Nested walks do not claim openat readiness.

Rename validates both parents, names, types, empty destination directories, cycles,
entry/orphan capacity and resulting depth before changing any binding. It changes
source binding, destination orphan state and directory parent together under the
exclusive mutable engine owner. One subsequent full-volume fsync publishes all of
it. No independently committed intermediate namespace exists. Dot/dot-dot are walk
operations after directory type validation; root dot-dot clamps to root. No lexical
symlink emulation, symlinks, hard links, truncate or native integration.
