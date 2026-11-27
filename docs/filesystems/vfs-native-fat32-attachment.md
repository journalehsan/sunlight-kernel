# VFS-N1N: Native FAT32 attachment and split-phase withdrawal

**PARTIAL (November 27, 2026).** The existing FAT32 attachment now
withdraws in separate topology and writeback phases on the host. The N1D
transaction code also builds for the native VFS service with the opt-in
`attachment-transactions` feature. The ring-3 FAT32 block binding and
integrated QEMU lifecycle remain unimplemented.

Closing admission advances the topology revision and retains the crossing.
New paths return Busy, new snapshots omit the mount, and snapshots captured
before closing cannot install. A Busy result for live handles leaves their
backend attached and available until they close. Once the mount has no open
handles, preparation moves the FAT backend into an exclusive work item and
leaves a closing mount record. The mount incarnation and shared transaction
identity correlate that work with completion; duplicate preparation cannot
take the backend again.

FAT sync and the covering release drain and barrier run with the topology
lock dropped. Finalization takes the topology lock again and checks the mount
incarnation, transaction identity, zero pins and successful sync. The lease
authority holds its synchronized release pin during atomic topology removal
and releases the lease only after removal. Sync or settlement failure keeps
the crossing closed and fences the lease. There is no automatic reopen or
retry after an uncertain outcome. No wait loop was added.

| Gate | Result | Evidence |
| --- | --- | --- |
| Host FAT readiness, publication and compensation | PASS | Existing authenticated FAT32 tests reject malformed geometry, stale topology and revoked registration, with compensation before reuse. |
| Host withdrawal ordering and accounting | PASS | The sync callback can acquire the topology lock; the test verifies closed admission, rejection of duplicate preparation and premature completion, a retained lease before sync, and vacant authority after commit. The existing unsettled-command test confirms fencing and late command retirement without release. |
| Host retained lifetimes and repeated attachment | PASS | N1D retains root and description pins; N1L tests a live FAT handle and four mount cycles with file create, write, stat, reopen, offset reads and an untouched file. |
| Native N1D transaction compilation | PASS | Release service builds with the opt-in transaction feature. |
| Native FAT adapter and publication | BLOCKED | No native staged FAT backend holds a readiness pin through N1D publication. The existing copied interface only authorizes LBAs 128–191; opcode 0x104 settles and releases together. |
| Native FAT operations and backing image | NOT_RUN | No native FAT image was mounted or modified. Direct ring-3 filesystem operations, remount persistence, and untouched-sector comparison remain open. |
| Native FAT mount-cycle DMA accounting | NOT_RUN | N1M's separate block probe historically ended with zero pending commands and mapping users and retained 20,480 bytes. No measurement was made across FAT mount cycles. Physical reclamation remains BLOCKED. |
| Named regressions | PASS | All 481 baseline, 59 N1H, seven N1D and three N1L test names passed. The broader workspace host sweep fails independently on duplicate `panic_impl` in `sunlight-ui-wayland`. |
| Production kernel and release VFS build | PASS | Both compile with the transaction feature disabled. |
| Ownership guard | FAIL | Unchanged check: 71 references, four unmatched dependencies (statfs, ProcFS mountinfo, private mmap and runtime FAT mount). |

The smallest remaining integration is to extend the isolated syscall path
with a grant for one marked disposable FAT image and a bounded readiness
pin tied to the exact live registration epoch, media generation and lease.
Bind that pin to the existing N1D reservation while initializing the FAT
backend outside the topology lock. Separate the kernel release operation
into confirmed settlement and final release so the lease stays occupied
through the service's atomic withdrawal. Then run ring-3 mount cycles and
verify transport accounting and the closed backing image directly.

Production ownership, routing and the ordinary boot disk are unchanged.
L1C-3 remains blocked, VFS-D1 deferred, and demand-backed main stacks opt-in.

See [machine-readable results](vfs-native-fat32-attachment-results.json).
