# VFS-C2C: Ordinary fstab mount and namespace inheritance

**PARTIAL.** The opt-in GPT fixture and fstab configuration pass host checks.
The unchanged ownership guard blocks ordinary boot. The primary acceptance
criterion, a committed service mount from standalone `storagectl fstab`
accessed by ordinary child processes, remains **NOT_RUN**.

Generate a fresh image with
`python3 tools/make_vfs_c2c_fixture.py target/vfs-c2c/fixture`. The directory
must be new. The generator uses a 64 MiB regular file with an EFI System
partition at LBA 2048, 128990 sectors long, and the existing C2B disposable
marker in the protective MBR boot code. It checks both the GPT layout and
the FAT32 seed, including the exact `KEEP.TXT` payload. The recorded hashes
cover all bytes before and after the partition, including both GPT copies;
there is no post-guest image to compare in this run.

The `sunlight-fs/vfs-c2c-fixture` feature selects the shared fstab text and
the initramfs entry. It retains the boot and root rows and adds one required
FAT32 row for the fixture PARTUUID at the existing root-owned `/mnt` target.
Keep this feature confined to the disposable C2B reservation configuration.
The host test exercises the existing resolver with GPT on-disk UUID byte
order and verifies that a parent without the UUID cannot satisfy the row.
No storage or namespace authority was added. Service-side emptiness and
publication authorization for the actual child still need guest evidence.

The standalone binary and candidate service built in release mode. The
production release kernel and debug candidate both built. Host suites passed: fstab (5),
block (70), IPC storage (6), VFS service (92), and registration (112). These
checks do not establish guest behavior. The unchanged `tools/test.sh`
stopped at the guard before launching QEMU:

| Historical ownership finding | Cutover state |
| --- | --- |
| `storage::mount` | **BLOCKED**: unrelated runtime transports still construct kernel FAT. |
| `linux_procfs::mountinfo_snapshot` | **BLOCKED**: legacy callers still use kernel topology. |
| `sys_linux_statfs` | **PARTIAL**: prior service capacity coverage; legacy capacity lookup remains. |
| `process::mmap::sys_mmap` | **PARTIAL**: prior service private mapping coverage; legacy descriptions still use kernel VFS. |

The unchanged inventory reports 71 references. The no-growth guard exits
with status 1 and identifies four unmatched sites.
The [results](vfs-ordinary-fstab-namespace-inheritance-results.json) record
the unrun native gates. In particular there is no measured committed mount
count, shared incarnation, guest write, retirement, or DMA settlement for
C2C. The separate boot disk was not attached or written during host checks.
The [C2B results](vfs-early-storage-ownership-reservation-results.json)
retain the prior successful runs and the original failed attempt. Existing
workspace limitations and historical evidence bounds are unchanged.
