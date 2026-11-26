# Compressed boot resource module prototype

This opt-in image moves the filesystem seed out of both the kernel and VFS
service ELF. Both mounts read the same immutable Limine module. The ordinary
image still uses its embedded seed. This is a prototype boot result, not a
change to the minimum supported RAM configuration.

The bootstrap executable path is unchanged: the kernel can load init, the
device and input servers, and the VFS server without decompressing an ELF.
The prototype also retains the existing embedded executable launch catalog,
including later services and applications. The module supplies the entire
filesystem seed, including early configuration such as `/etc/fstab` and the
ordinary application and user resources. It must validate before the kernel
mounts its root. There is no fallback to a disk or to a partial namespace.

## SUNRES01 format and reads

All integers are little endian. The 24-byte header holds the 8-byte magic and
four u32 values: total archive length, entry count, payload start, and end of
the inventory. Each inventory record starts with path length, UID and GID
(u32), mode (u16), directory flag and reserved byte (u8), logical size and
descriptor offset (u32), followed by the path bytes. Empty files and
directories have no descriptors. Record order is preserved.

File data is split into independently readable 4,096-byte blocks. Each
12-byte descriptor gives the absolute payload offset, stored length and
FNV-1a checksum of the decoded block. A block with stored length equal to
logical length is raw; smaller blocks are LZ4. Identical nonempty files
share descriptors and payload. The reader validates inventory and descriptor
bounds before publishing the seed and checks each touched block's size and
checksum on read. The checksum detects corruption; it is not an
authentication mechanism. The prototype trusts the build and boot media.

The decoder uses 4 KiB of stack scratch per invocation and retains no
decoded-block cache. The first preserving write to a seed inode materializes
only that inode into the existing quota-backed tmpfs, one page at a time
with a separate 4 KiB copy buffer. The kernel and VFS service each build
their own small inventory and keep their existing handles, permission
checks, mutable state and IPC contracts. The module does not unify their
live namespaces, introduce a new API, or change swap policy or the 16 MiB
heap.

## Ownership and lifetime

The kernel requires exactly one module, validates its size and checks that
its rounded extent lies in a Limine executable/module memory-map range.
The bytes stay pinned for the boot lifetime. They are outside the PMM free
domain and do not count as part of the managed kernel image. No loader pages
are reclaimed. The original VFS process receives a read-only, nonexecutable,
owner-managed mapping of those same frames using the existing
`BootSharedData` mapping kind. Process teardown owns the mapping and its
page tables; the payload frames remain pinned. There is no new VFS
registration mechanism or mechanism to attach the mapping to a replacement
service. Prototype image and VFS executable must be built as a pair.

## Evidence and reproduction

Build after preparing the ordinary service and application inputs:

```sh
bash tools/build_boot_resource_prototype.sh
out=target/boot-resource-prototype
python3 tools/check_boot_resource_inventory.py \
  docs/memory/boot-image-architecture-audit-results.json \
  target/mem-q1/qualified.elf "$out/resources.bin"
python3 tools/qualify_boot_resource_prototype.py "$out/sunlightos.iso" "$out/qualified"
python3 tools/qualify_boot_resource_prototype.py "$out/sunlightos.iso" "$out/qualified" \
  --firmware /usr/share/edk2/x64/OVMF.4m.fd --ram 512
```

The packer's round-trip read checks every entry against the current source
seed, including unaligned reads across block boundaries. An independent host
reader compared all 613 paths, types, modes, owners, sizes and 154,184,731
logical payload bytes against the SHA-256-matched ELF from the audit.
Guest gates cover kernel entry, PMM reconciliation, 16 MiB heap
initialization, VFS inventory, ordinary Phase 3, a 2 KiB VFS read over
shared memory, namespace readiness before service startup, kernel-resident
swap policy configuration and session service/bundle discovery. The guests
use QEMU 11.1.3 TCG, two vCPUs, read-only optical media and no NIC or disk.

| Firmware | Guest RAM | Boot gates | Entry footprint | Peak through pre-scheduler | Peak through session bundle discovery |
| --- | ---: | --- | ---: | ---: | ---: |
| BIOS | 2 GiB | Pass (initial control) | 325.64 MiB | 351.70 MiB | 562.07 MiB |
| BIOS | 1 GiB | Pass | 325.64 MiB | 351.70 MiB | 564.50 MiB |
| BIOS | 768 MiB | Pass | 325.64 MiB | 351.70 MiB | 559.96 MiB |
| BIOS | 512 MiB | Pass | 325.64 MiB | 351.70 MiB | 512.00 MiB |
| UEFI | 512 MiB | Pass | 331.83 MiB | 357.86 MiB | 512.00 MiB |

Footprint means configured RAM minus free PMM bytes. It includes guest
RAM unavailable to PMM, so it is not a measure of live payload alone.
The peak counter follows both single-frame and contiguous PMM allocations
and records the lowest free count through each checkpoint. At entry, free
frames match usable map bytes divided by 4,096, managed frames equal free
plus pinned kernel frames, and the combined image/module map bytes equal
the separately rounded kernel and module extents. This measures peaks
after PMM initialization; it does not trace Limine's internal transient
allocations before handoff.

| Component | Audit estimate | Prototype |
| --- | ---: | ---: |
| Page-rounded ELF staging | 115,281,920 B | 119,324,672 B |
| Page-rounded loaded kernel | 127,614,976 B | 131,657,728 B |
| Rounded module residency | 107,479,040 B | 86,392,832 B |
| Decode workspace and index budget | 4 MiB | 4 KiB decode buffer; inventory in module plus two entry tables |
| Decoded cache | 8 MiB | 0 B |

QMP confirmed the full 119,320,992-byte staged ELF at physical
`0x18d48000` after Phase 3 in a separate 512 MiB BIOS capture. The
measured BIOS handoff footprint is 12.69 MiB below the audit's modeled
loader checkpoint. The pre-scheduler peak is 109.14 MiB below the audit's
460.84 MiB proxy. The comparison differs in payload selection, compression
and lifetime: moving the shared seed out of VFS reduces startup ELF
copying, and only the seed moves into the module. The measured session
peaks also show why the audit's pre-scheduler margin was not a prediction
of post-startup headroom. Both 512 MiB guests reached **zero free frames**
by the session bundle checkpoint.

The run does not qualify desktop interaction, arbitrary application
launches, sustained load, service restart, physical machines, or ZRAM
reclaim behavior under that load. Missing and corrupt module boot cases
were not run. A successful 512 MiB boot gate does not
establish usable post-boot headroom. Keep the ordinary packaging path
until those limits are understood.

See [machine-readable results](compressed-boot-resource-module-results.json)
for artifact hashes, individual serial captures and frame counts.
