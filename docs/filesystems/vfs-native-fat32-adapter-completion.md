# VFS-N1O: Native FAT32 adapter completion

**PARTIAL (November 27, 2026).** All three prerequisites and four integrated
mount cycles passed in the isolated QEMU configuration. Closed-image validation
passed with optional FSInfo hints marked unknown. A separate run with a
populated free-cluster count exposed a FAT backend limitation.

| Prerequisite | Result | Evidence |
| --- | --- | --- |
| Bounded FAT sector grant | PASS | The isolated kernel checks capacity, sector size, writable/flush support and sector-zero marker. The existing authority grants one exclusive whole-device lease for device-wide barriers; syscall 180 confines copied 512-byte data requests to LBAs 2048–131071. The service checked both boundaries and rejection of the previous lease generation. |
| Native FAT readiness through N1D publication | PASS | The registered ring-3 VFS initializes the existing FAT32 backend, validates BPB, extent and root chain, and reads the root before publication. The private stage holds the exact N1D reservation, backend and shared lease. Under the topology lock it revalidates owner, revision and target identity and makes a non-I/O check of the live registration, epoch, media and lease. In this single-CPU, single-owner probe the lease stays occupied and the coordinator cannot release it between validation and insertion. This does not qualify a concurrent production publication pin. |
| Separate settlement and release | PASS | Opcode `0x106` closes lease admission, confirms drain and a covering flush, and retains the synchronized lease. `0x107` releases it only after the service finishes topology withdrawal. Premature release, acquisition before release and data access after settlement were rejected. `0x108` fences failures; `0x109` validates readiness without storage I/O. |

The probe uses the existing N1D namespace and N1N withdrawal work item. It
borrows the namespace exclusively for file operations, without holding the
topology mutex. Withdrawal closes admission, returns Busy for a retained
handle, and lets that handle finish its positional read and close. FAT sync
and transport settlement run after the preparation lock is dropped. The
service then finalizes the same mount incarnation before releasing the
lease. A reported sync or settlement failure fences the lease; native fault
injection was **NOT_RUN**. The host split-phase and unsettled withdrawal
regressions passed.

The guest registered endpoint 1, PID 3, incarnation 43, address-space
generation 18, epoch 1. It created and wrote `NEW.TXT`, reopened it, checked
size, read the full payload and read five bytes at offset two. Three fresh
attachments reopened the same file and verified its contents and the
unchanged `KEEP.TXT`. All four cycles logged readiness, publication,
confirmed settlement, topology removal and release in order. The transport
reported zero pending commands and mapping users on each settlement and the
same four queue pages and one bounce page (20,480 retained bytes). Physical
DMA reclamation remains **BLOCKED**; it was **NOT_RUN**.

The harness closed QEMU before reading storage. The passing image returned
both exact payloads through independent host FAT tools; read-only
`fsck.fat -n` exited zero. The complete 1 MiB prefix was unchanged, and
only four FAT-relative sectors changed: 32 and 1025 (FAT copies), 2018
(root directory), and 2020 (new file data). The initial fixture in the
earlier run had a populated FSInfo free-cluster summary. Its payloads and
four native cycles passed, but the check **FAILED** because the existing
writer left the summary at 127004 when the real count was 127003. The
earlier image remains as captured. The passing run prepared a separate
fixture with both optional hints unknown before boot. The smallest
remaining prerequisite for qualifying populated FSInfo is for the FAT
writer to maintain or invalidate that summary durably when allocation changes.

All 481 baseline, 59 N1H, seven N1D and three N1L tests passed by name.
Production kernel and release VFS service builds passed. The unchanged
ownership guard still **FAILS**: 71 references, four unmatched sites
(statfs, ProcFS mountinfo, private mmap, runtime FAT mount). Native
application FD installation, ordinary routing and successor handoff remain
**BLOCKED**. This work used disposable storage only; ordinary boot storage,
runtime FAT ownership and `/vfs-scratch` were unchanged. L1C-3 stays blocked,
VFS-D1 deferred and demand-backed main stacks opt-in.

See [machine-readable results](vfs-native-fat32-adapter-completion-results.json).
