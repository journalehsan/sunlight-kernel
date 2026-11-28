# VFS-C2B: Early non-boot storage ownership reservation

**PASS** for early reservation and authenticated transport handoff in the
opt-in candidate. **PARTIAL** for the ordinary deployment workflow. See the
[machine-readable results](vfs-early-storage-ownership-reservation-results.json).

`vfs_c2b_test` keeps the legacy boot disk selection and requires an explicit
`SUNLIGHT_VFS_C2B_PCI` address within the supported legacy VirtIO PCI scan.
The candidate finds the exact PCI function, rejects the boot function or a
conflicting block I/O BAR, initializes its queue through the same helper used
for boot, and checks the disposable marker. The ordinary registry code
registers the disk capacity and discovers and registers the partition table.
This reads partition metadata before undertaking filesystem probing.

The selected queue moves directly into the existing lease owner in
`kernel/src/vfs_n1j_test.rs`. The parent registry ID fences the physical row
and every child row. The queue never enters `VIRTIO_BLK`; only the boot
device enters that slot and becomes eligible for legacy boot FAT probing.
The legacy runtime mount check also refuses every reserved row before
constructing a backend. The checks do not require a live service session.

The existing authenticated VFS registration binds the queue to the lease
owner. Acquisition revalidates the exact registered parent and source
against the installed records and uses the same opaque media incarnation.
Ownership covers all 131072 sectors; block operations are limited to
the registered FAT source at sectors [2048, 131072). Other aliases share
the exclusion but cannot acquire that source's data grant. Withdrawal and
settlement release the service lease without returning the reserved queue
to legacy storage. No second authority or mount table is involved.

Fresh one-vCPU, 2 GiB guests used a read-only boot disk at `00:0a.0` and a
disposable disk at `00:0b.0`. The serial verifier requires reservation of
parent 1 and source 2 before boot FAT detection, the protected `/boot` mount
on parent 3, both reserved rows refused by the actual legacy mount
function before service registration, and an authenticated registration.
It also verifies that the boot image is unchanged. Both FSInfo variants
passed service preparation, publication and receipt consumption, persistent
FAT mutation and reattachment, native and Linux private mmap, authoritative
mount inventory/mountinfo/statfs agreement, and fail-closed Linux mutation
and metadata calls after service loss. Independent closed-image checks
verified FAT integrity, untouched prefix, expected payloads and FSInfo.
Each guest settled seven leases with zero pending commands and zero mapping
users, using the same queue/bounce pair and 20 KiB retained DMA throughout.
The qualified caller accounting returned to 22320 frames, four VM regions
and 50 page tables after unmap, with the final service descriptor rows and
references at zero. Caller reaping reported zero swap blocks and cleared
IPC entries. Live committed-FAT descriptor exec was not exercised.

The standalone `/bin/storagectl fstab` child ran after init's namespace
readiness check and completed the default boot entries. It did not mount the
reserved partition: `sunlight-fs/etc/fstab` contains no runtime entry, and
the disposable MBR source has no GPT PARTUUID for the current fstab resolver.
Ordinary fstab qualification needs a matching GPT fixture and a seeded
runtime entry naming an empty service target, then a run proving that
init's spawned StorageAdmin child prepares and publishes that entry under
the live registration and ordinary readers inherit the published namespace.
This run proves the transaction through the scoped Ring 3 caller; it does
not establish an additional namespace readiness failure for the fstab child.

| Historical guard finding | State |
| --- | --- |
| `storage::mount` | **BLOCKED** for cutover: unrelated runtime transports still construct kernel FAT. |
| `linux_procfs::mountinfo_snapshot` | **BLOCKED** for cutover: unrouted Linux callers still require legacy topology. |
| `sys_linux_statfs` | **PARTIAL**: the reserved service view passes; legacy capacity is still read for unrouted callers. |
| `process::mmap::sys_mmap` | **PARTIAL**: committed service mappings pass; legacy descriptors still need the kernel path. |

Both unchanged guard modes fail with 71 references and the same four
unmatched sites. Guarded ordinary boot was not run. The native runs were
the isolated candidate harness. Reproduce with the release service built
with `vfs-c1e12-test`, its ELF path in `SUNLIGHT_VFS_N1J_ELF`, and a debug
kernel built with `--features vfs_c2b_test` and
`SUNLIGHT_VFS_C2B_PCI=00:0b.0`. Run
`python3 tools/test_vfs_n1h.py --c2b --kernel <elf> --fsinfo populated`
and repeat with `--fsinfo unknown`.
