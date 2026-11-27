# VFS-C1E2: retained StorageCtl preparation

The isolated `vfs_c1e2_test` kernel and `vfs-c1e2-test` service extend
the C1E1 copied continuation. Syscall 168 checks StorageAdmin and returns an
opaque preparation cookie on success. The registered VFS image retains the
N1D reservation, FAT adapter and lease in one private transaction. The
kernel's existing storage receipt binds that cookie to the exact caller,
registration, media and lease generation; it is consumed once without
releasing the lease. No mount is installed.

N1D captures and revalidates the namespace owner, target object and topology
revision. Readiness checks the FAT root and capacity, enumerates the root,
and checks that two aliases for the fixture file resolve to the same stable
object. The service closes both temporary FAT handles before retaining the
backend. The kernel checks the live lease, readiness and zero command and
mapping charges before accepting success. Namespace inspection and device
control run separately, without keeping the namespace lock over syscalls.

The abort opcode takes only the cookie and matches the originating caller
image. It drops the backend and reservation before flushing and finishing
lease release. A wrong or repeated cookie returns ENOENT. A failed FAT stage
follows the same settlement path. Caller retirement or expiration cancels
reply authority; the service checks the cancellation and disposes of the
private preparation. Service withdrawal closes admission and the existing
retirement path waits for the exact service image before settling its lease.
An unproven settlement does not count as released; the existing lease
transport fence remains authoritative.

The allocator issue predates this work: commit `75e3d6c63` restricts the
registered allocator to native targets, while host allocator tests still
referenced it when `global-alloc` was enabled. The tests now call a test-only
`SunlightAlloc` instance directly and leave production allocator registration
and host std allocation alone. The workspace host-library rerun encounters
a separate duplicate `panic_impl` in `sunlight-ui-wayland`.

See [recorded results](vfs-storagectl-retained-preparation-results.json) for
builds, retained DMA, the closed image check and individual lifecycle gates.
The fresh disposable 2 GiB, single-vCPU boot passed three explicit abort
cycles, one FAT initialization failure, parked caller retirement with late
reply rejection, and exact service image retirement before the prepare
reply. All six cleanup paths left the lease vacant with no commands, retained
command data, mapping users or pending flush. The queue and bounce addresses
stayed fixed at 20 KiB total. The closed disk matched its inputs byte for byte,
and the independently extracted FAT partition passed fsck.

The unchanged ownership guard still reports 71 references and four unmatched
sites. The combined focused run with libc was partial because two
`secret_store` tests did not complete during observation; the allocator-only,
filesystem and registration suites passed separately.
Media replacement, transport uncertainty injection, SMP and coordinated
publication remain outside the evidence of this single-vCPU fixture.
