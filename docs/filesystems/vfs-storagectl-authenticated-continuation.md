# VFS-C1E1: authenticated StorageCtl continuation

The opt-in `vfs_c1e1_test` kernel and `vfs-c1e1-test` service execute syscall 168
from an independent ring-3 process with explicit StorageAdmin authority.
In this fixture, the mount request probes preparation and returns its result.
The service discards the prepared backend and the reservation; no mount is
installed or retained for later routing. Production dispatch is unchanged.

The kernel copies the target and uses the existing copied VFS file continuation
to allocate a request ID, capture the caller's PID, incarnation and address
space generation, capture the active registration, and park the caller.
The service reserves the exact target with N1D before claiming the request ID.
The kernel accepts the claim from the live registered service image at its
current epoch, checks the parked caller's authority and the registry extent,
and binds the request to a media handle and whole-device lease generation.
Sector I/O stays within the registered partition; the existing executor performs
physical I/O. The service builds and validates FAT using the same reservation.
The request ID serves as the reservation cookie across the copied channel.

N1D rechecks the target identity and topology revision before the fixture drops
the staged object. The executor flushes and settles the lease before accepting
success. The reply must match the caller, service epoch, request and lease
generation. The caller consumes the reply once, after another check of the
current registration, registry extent and settlement. Scheduler and broker
guards are released before rescheduling; the kernel VFS lock is not taken by
the continuation.

A failed reservation acquires no lease. A failed stage after acquisition drops
the backend and follows the same release barrier. Timeout or caller retirement
removes reply authority, but a live lease remains charged to the registered
service until settlement. Service withdrawal invalidates replies and sends an
active lease through the existing exact-image retirement path. A release in an
uncertain state stays fenced. Retirement and transport failure interleavings
were not exercised by the new native run.

The fresh 2 GiB, single-vCPU disposable run rejected a missing target, an
unknown source, stale epochs, a mismatched caller incarnation, incorrect
reservation and lease receipts, and duplicate replies. Two calls completed
with zero outstanding commands and mapping users, zero retained command data,
no flush pending and the authority vacant. The queue and bounce buffer retained
20,480 bytes under fixture ownership. The full closed disk image matched its
input byte for byte and the FAT check passed.

The unchanged ownership guard still rejects 71 references and the same four
unmatched sites. Coordinated routing, guarded production boot and cutover
remain blocked. See the [results](vfs-storagectl-authenticated-continuation-results.json).
