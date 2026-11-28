# VFS-C1E12 committed FAT32 private mmap

**PASS for the opt-in native and Linux mmap objective.** Both ordinary syscall
paths populated eager private pages from the same newly created file on a
committed service-owned FAT32 mount. The unchanged ownership guard still
fails, so ordinary production cutover remains blocked.

## Original failure and repair

The archived C1E10 mmap run reached StorageCtl publication, performed
ordinary open, dup, seek and read, then panicked with `Errno::Inval`.
Booting the archived `candidate-settlement-retry.elf` on a fresh disposable
disk reproduced the panic at the original `native.rs:264:46` call site.
The embedded service disassembly around that call site contains the ordinary
file operations and the immediate invalid-argument result, with no
file-backed mmap syscall. The earlier `candidate-final.elf` also reproduced
its separate `Errno::Failed` panic at `native.rs:253:47`. The C1E10 report
identifies that attempt as the mkdir failure.

The source-level cause of the mmap failure was
`sunlight-libc/src/mman.rs::validate_mmap_request`. It required
`MAP_PRIVATE | MAP_ANONYMOUS`, descriptor `-1` and offset zero before
`mmap` could issue native syscall 50. C1E7's Linux probe issued Linux syscall 9
directly and did not exercise that native wrapper. The opt-in
`sunlight-libc/private-file-mmap` feature admits private file requests with
a nonnegative descriptor, a page-aligned offset and a representable file
range. The existing restrictions on anonymous mappings, address hints and
protections still apply. The C1E12 service fixture enables the feature.
The kernel candidate adds read-only fixture checkpoints for PTE permissions
and resource counts.

The ordinary mmap dispatcher selects the C1E7 route from descriptor
ownership or an exact caller's pending transaction. D2A verifies READ,
the retained caller receipt and registration epoch, and pins the service
description. Ordinary FSTAT separately verifies the new file's 8,201-byte
size. The mapping path uses bounded PREAD for size and EOF discovery.
PREAD validates the caller and token, then `Vfs::read` dispatches the
retained mount-local handle to `FatFs::read`. The FAT backend authenticates
the open handle and uses its first cluster and size for `Fat32::read_at`.
The shared cursor is untouched. Staging advances by actual
bytes returned, with a maximum of 4,096 bytes per request and zero padding
after EOF. Scheduler and capability locks are released for the wait. Replay
rechecks caller, epoch and placement before installing eager frames with
their final permissions. The D2A installer and rollback, D2B retirement,
and exact final Close remain the existing implementations.

The requested C1E11 standalone report
`vfs-committed-fat-mutation-creation.md` was absent from the checkout. The
C1E11 migration entry, working fixture and `target/vfs-c1e11/native-final.log`
provided the prior qualification evidence.

## Disposable guest qualification

The StorageCtl fixture used one vCPU, 2 GiB and fresh disposable FAT32
storage. The first incarnation exposed `/mnt/volume`. The caller created
`MMAP.BIN` through ordinary OPEN with creation flags and wrote 8,201 bytes
in bounded writes. Byte `i` is `(29*i + 17*(i/512)) % 251`, with integer
division.

| Route | Result | Guest checks |
| --- | --- | --- |
| Native syscall 50 | **PASS** | Three 12 KiB mappings at offset 4 KiB each copied 4,105 bytes. Ring 3 checked every byte, including the partial final file page and a page wholly past EOF. It verified writable private content, RW/NX, RX and RO/NX permissions, absent leaves after unmap, and cursor preservation through dup. A separate RO mapping at offset zero survived closure of both descriptors. |
| Linux syscall 9 | **PASS** | Two equivalent offset mappings copied 4,105 bytes each and a 12 KiB offset-zero map copied all 8,201 bytes. Linux returned EINVAL for the unaligned control. Ring 3 checked bytes and EOF padding, private writes, RW/NX and RO/NX permissions, cursor preservation, source close and unmap. |
| Ordinary StorageCtl withdrawal | **PASS** | UNMOUNT returned EBUSY while the native caller held the mapping and duplicated description. The caller could still read through the retained description. After final close, the private map remained readable. After unmap, ordinary UNMOUNT synchronized FAT, drained the transport and released the lease. The second incarnation recovered every original byte. |

Comparison reads and independent image extraction confirmed that private
writes never reached the file. The existing eager EOF behavior zero-fills
the rounded span, including a page entirely past EOF.

## Integrated metadata control

| Route | Result | Observation |
| --- | --- | --- |
| StorageCtl LIST_MOUNTS and INVENTORY_MOUNTS | **PASS** | Both returned exactly one service mount row for the committed incarnation and agreed on mount ID, device source and path. The next publication received a fresh mount ID. |
| Linux ProcFS mountinfo | **PASS** | Mount ID `0x8000000100000002` and registered source 2. Exactly one visible writable `vfat` row for `/mnt/volume` had the same mount ID and registered `helios-block` source as StorageCtl. The row disappeared on withdrawal. |
| Linux statfs | **PASS** | Ordinary statfs on the committed file returned FAT magic 0x4d44, 512-byte block and fragment sizes, 127,006 blocks and 126,984 free and available blocks. It returned ENOENT when the file became hidden on withdrawal. |

## Accounting and regressions

The mapping checkpoints compare each caller's owned user frames, VM
regions and lower-half page tables before mapping and after each unmap.
Both callers returned to baseline. The Linux description returned to the
native caller's one-row, two-reference baseline. The final native sample
had zero service rows and references. The named callers and service fully
reaped with zero swap blocks and queued IPC entries.

All seven lease settlements verified zero outstanding commands, mapping
users and retained command data, with no pending flush. The same queue and
bounce allocations held 20 KiB of retained DMA throughout. Independent
`fsck.fat -n` and mtools passed on the closed images. The partition prefix
was byte-identical, both FAT copies had the expected new cluster chain, and
the changed sectors matched the directory and data writes and applicable
FSInfo updates. mtools recovered `KEEP.TXT`, the C1E11 payload and directory,
and the exact original mmap payload.

The guest reran C1E11 creation, sparse extension, pinned Busy, rename,
truncate, unlink and same-boot reattachment checks, plus C1E6
service-loss and C1E7 ordinary Linux mapping controls. The focused host
checks and builds are listed in the results file. The fresh diskless D1B,
D2A and D2B controls passed on one, two and four vCPUs. The C1E6B retirement
marker appeared without stale-alias consumption. The kernel stack remains
32 KiB. Standalone production
boot and D3 reclaim and real fault restoration were **NOT_RUN**. Concurrent
exec by a caller parked in mmap remains **BLOCKED** by the single-executor
contract.

## Ownership guard

Both unchanged guard modes **FAIL**. The final check inventories 71
references. The no-growth check reports these four unmatched sites.

| Finding | Candidate result | Remaining dependency |
| --- | --- | --- |
| `process::mmap::sys_mmap` | **PASS** for native and Linux committed FAT service descriptors | Kernel-owned file mappings still use kernel FSTAT and positional reads. |
| `linux_procfs::mountinfo_snapshot` | **PASS** for the committed FAT caller view | The legacy route still snapshots kernel topology. |
| `sys_linux_statfs` | **PASS** for the committed FAT caller view | The legacy route still reads kernel capacity. |
| `storage::mount` | **PASS** for the disposable service-owned mount | Kernel FAT construction remains for other runtime devices. |

## Reproduction

Build the release VFS service with `vfs-c1e12-test` and the repository's
userspace linker flags. Set `SUNLIGHT_VFS_N1J_ELF` to that ELF when building
the debug kernel with `vfs_c1e12_test`. Run
`python3 tools/test_vfs_n1h.py --c1e12 --fsinfo populated --kernel <elf>`,
then repeat with `--fsinfo unknown`. The harness creates a fresh disk for
each run and retains the serial log and closed FAT partition. The
[machine-readable results](vfs-committed-fat-private-mmap-results.json)
record the logs and gate outcomes.
