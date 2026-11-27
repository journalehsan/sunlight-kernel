# VFS-N1P: FAT32 FSInfo consistency

**PASS for ordinary native attachment (November 27, 2026).** N1O
recorded a free-cluster summary of 127004 after allocation reduced the
actual count to 127003. The existing FAT writer updated the FAT copies
and its optional in-memory capacity sample, but never changed FSInfo.
The original N1O evidence remains intact.

The writer now sets both FSInfo hints to `0xffffffff` (unknown).
The allocator still scans FAT entries, and capacity sampling still
counts them directly. Mount records the BPB's primary FSInfo sector and
the corresponding backup at backup boot LBA plus primary FSInfo LBA,
when those locations are inside the reserved region. The first FAT
write checks lead, structure and trail signatures and changes only the
two hint fields in recognized sectors. Absent and unrecognized sectors
are left alone; already-unknown fields do not incur a sector write.

File data and directories allocate through `allocate_cluster`; growing
chains and directories link through the same FAT entry writer. Truncate
and unlink publish and flush the directory change before freeing the
old chain. Replacement rename and failed directory creation also free
chains through that writer. Empty file creation changes the count only
if it grows the directory. All free/allocated transitions pass through
`write_fat_entry`.

Before any FAT copy can change, the writer completes the FSInfo writes
and confirms a device barrier. An FSInfo read, write or barrier error
returns an I/O failure before FAT mutation. A later attempt still needs
a successful barrier even if both sectors already read back as unknown.
The mount remembers successful invalidation; later FAT mutations and
ordinary sync retain existing writeback and error behavior. A partial
FAT or directory operation can still fail or leak allocation under
existing rules. Invalidation neither rolls back arbitrary I/O failures
nor provides crash recovery.

All 14 FAT host tests passed, including initial populated and unknown
hints, repeated allocation and synchronization, persisted reread,
truncate and unlink as first mutations after remount, faults at each
FSInfo write and at the barrier, and successful existing allocation
rollback after a directory entry write failure. The fault model checks
the durable hints at each attempted FAT write.

Each disposable fixture completed four isolated single-CPU QEMU cycles.
The ring-3 service initialized FAT32 and published it through N1D. It
extended a temporary file to two clusters, truncated it, wrote again,
closed and unlinked it. The service created and reopened `NEW.TXT`,
checked its size and read back its full payload. Each cycle closed
admission, read at an offset through a retained handle, closed the
handle, synchronized and confirmed transport settlement, removed the
mount and released the exclusive lease. Three fresh attachments verified
the persistent payload and every cycle verified `KEEP.TXT`. Settlement
reported zero outstanding commands and mapping users with the same
20 KiB retained DMA allocation in all eight cycles.

Independent `fsck.fat -n` exited zero on both initial and both closed
images; `mtype` returned both exact payloads without repair. The image
comparison preserved the entire 1 MiB prefix and checked every volume
byte against the expected directory entry, FAT mirrors and data. Both
fixtures changed volume LBAs 32, 1025, 2018, 2020 and 2021. LBA 2021
contains the expected temporary data in a now-free cluster. Only the
populated fixture changed FSInfo LBAs 1 and 7. The independent FAT scan
and filesystem check account for 127004 free clusters before and 127003
after. The original native runs reached their lifecycle attestations
and passed fsck; their first byte checker used an overly specific root
EOC marker. The corrected checker passed on those same captured images.
Two separate repeat attempts produced no serial output and do not
contribute to the qualification.

All 481 baseline, 59 N1H, seven N1D and three N1L names passed, along
with N1N split withdrawal and unsettled withdrawal. The production
kernel and release VFS service compiled. The unchanged ownership guard
still fails with 71 references and four unmatched sites: statfs, ProcFS
mountinfo, private mmap, and runtime FAT mounting.

The result qualifies ordinary operations within the isolated native
attachment path. Production routing, `/vfs-scratch`, application FD
installation, successor handoff, crash recovery and native fault
injection were outside this gate. Physical DMA reclamation remains
blocked and was not run. L1C-3 remains blocked, VFS-D1 deferred, and
demand-backed main stacks remain opt-in.

See [machine-readable results](vfs-fat32-fsinfo-consistency-results.json).
