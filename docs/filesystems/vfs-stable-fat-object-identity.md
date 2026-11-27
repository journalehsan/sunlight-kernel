# VFS-C1D: stable FAT object identity

**PARTIAL.** Prepared service attachments now support anchored FAT lookup and
retained file and directory objects. Ordinary StorageCtl and production
ownership transfer remain **BLOCKED**.

## Identity and lifetime

| Backend value | Meaning |
| --- | --- |
| Path and display spelling | Namespace address. FAT compares components without ASCII case; a rename changes the address, including the address of descendants. |
| Short directory entry (sector, offset) | Locator of the current link. LFN slots describe its name. Growth and truncation edit the short entry in place; rename can publish a new short entry, and unlink frees the old slot for reuse. |
| First cluster and size | Current data extent. Empty files can all have cluster zero. Extension, truncation and deletion can allocate or free clusters; directory clusters can also be reused after retirement. |
| Object identity | Mount incarnation plus a monotonic mount-local object number. The implicit root has a reserved number and no directory entry. Published object metadata uses the existing mount ID and generation fields. |

Only prepared service device attachments enable the identity index. Under the
same VFS borrow as lookup and mutation, the index interns short-entry
locators. Separate opens of one linked entry share its object number and own
independent backend handles; duplicate description tokens share one handle
and cursor through the existing service table. Closing the last handle
releases its retained path and locator. Linked entries keep their identity
index row so a later lookup returns the same number; unlink or replacement
discards the retired row, and mount withdrawal drops the whole index. The
index grows with distinct observed live entries, not with repeated opens.
There is no persistent inode database.

The adapter authenticates a handle against its current link and reads current
size and cluster information. Rename changes the source locator only after
backend success, retains its object number, and rewrites canonical paths for
pinned descendants of a same-parent directory move. Anchored traversal
authenticates root and cwd in the visible mount, uses the on-disk component
spelling, and clamps parent traversal at the retained root. A renamed
directory remains the root even if another directory takes its old name.
Cross-parent directory moves and directory replacement return Unsupported;
the existing FAT writer does not update directory parent links or implement
safe directory replacement.

Unpinned regular files can be unlinked or replaced. Unlink or replacement of
a pinned entry returns Busy before backend writes; detached FAT chains are
not implemented. The legacy kernel FAT adapter keeps its previous revoked
handle semantics. An uncertain mutation poisons the affected adapter:
no new identity is published and subsequent lookup and retained I/O fail
closed, while exact closes still drain. Recovery of the on-disk state is
separate. FAT retains destination-first rename and the existing FAT/FSInfo
dirtying and barriers. A failed directory read is not a clean missing name.

Withdrawal closes new crossings before waiting for retained handles and
keeps the device available to admitted references until drain and sync.
Remount assigns a fresh VFS tag; old handles do not resolve in the new
incarnation.

## Qualification

| Gate | Status | Evidence |
| --- | --- | --- |
| Host FAT, FSInfo, VFS, block and service libraries | **PASS** | 296 tests. Includes case aliases, retained-directory rename and old-name reuse, shared cursor and positional reads, pinned unlink refusal, uncertain rename, handle drain and remount isolation. |
| Registration host harness | **PASS** | 112 tests. |
| Production kernel, release VFS and isolated candidate builds | **PASS** | Isolated kernel embeds the rebuilt Ring-3 fixture service. |
| Native FAT object subset, 1 vCPU / 2 GiB | **PASS** | Four live registry lease cycles on each of two fresh disposable images (populated and unknown FSInfo). Ring-3 anchored lookup, distinct empty files, retained file rename, extension and truncation observed by a second handle, offset read, pinned unlink refusal, slot reuse, and zero open handles at the end of the identity exercise. Data survives reattachment. |
| Native directory anchors and shared descriptor lifecycle on FAT | **NOT_RUN** | Covered on host. Native fixture uses backend handles and does not exercise ordinary FD dispatch or service-token Seek/PREAD. |
| Native 2 and 4 vCPU / 2 GiB | **BLOCKED** | Existing native block fixture expressly requires one CPU. No SMP acceptance is inferred from archived ordinary namespace runs. |
| Unchanged ownership guard | **FAIL** | Both modes reject; 71 ring-0 references, four historical sites outside the baseline. |

Both native images passed independent read-only `fsck.fat -n`, persistent
payload checks, full-image comparison, and predecessor verification of the
lease/publication/withdrawal ordering. Each lease covered the whole 131072
sector medium while data access was confined to [2048, 131072). Free clusters
changed from 127004 to 127003. All eight releases settled with zero pending
commands, zero mapping users, and stable queue and bounce addresses within
each guest, retaining 20480 bytes. Native identity index memory and service
descriptor rows were not independently measured in this fixture.

The first C1D checks rejected the images because the original N1P byte
expectation did not include the deleted MOVED.TXT entry. The rejection logs
remain intact. The explicit C1D verifier checks that entry's exact location,
name bytes, former cluster and size in the same full-image comparison; the
original N1P/C1C expectation and settlement checks remain intact. The C1C
closed-image artifacts pass their original profile. Earlier C1B and D2A/D2B
rejections remain separate in the [C1C record](vfs-live-storage-registry-integration-results.json).
The unchanged C1A, D1B, D2A and D2B verifiers also passed on the corresponding
archived one-, two- and four-CPU accepted runs. Those are artifact checks,
not fresh execution on the C1D candidate.

## Remaining routing prerequisites

Qualify the native FAT descriptor and retained root paths with authenticated
caller dispatch and extend the block fixture to SMP. Bind StorageCtl caller
admission, target reservation, registered source, exclusive lease and mount
incarnation in one publication transaction. Move mount and unmount,
inventory, ProcFS mountinfo, Linux statfs, path and descriptor operations,
exec and private mmap to that graph. Boot-selected FAT and production USB
transfer have separate withdrawal and handoff requirements. Historical
kernel FAT, ProcFS topology, Linux statfs and private mmap dependencies
remain live. Production defaults, stack opt-in, memory policies and the
ownership guard were preserved.

See the [machine-readable results](vfs-stable-fat-object-identity-results.json).
