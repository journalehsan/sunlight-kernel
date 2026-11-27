# VFS-C1E4: StorageCtl runtime namespace ownership

**PASS: shared ownership in the opt-in runtime. BLOCKED: publication commit.**

C1E3's `run_retained` constructed a private `DirectoryNamespace<Device>`
before the service could initialize its ordinary `State`. That loop owned
the N1D reservations and prepared FAT adapters; the ordinary state owned a
separate `Vfs<NullDevice>` for request dispatch and mounted root policies.

With `vfs-c1e2-test`, ordinary `State` now constructs the C1B block-capable
mounted owner using the fixture device type. The seeded tmpfs root provides
the disposable mount targets. Ordinary path dispatch uses the owner's
shared VFS reference. The typed retention runtime uses the same owner for
retained directory roots and the file descriptions derived from them.
The copied file drain dispatches storage frames to the retained transaction
in that state and passes ordinary frames to the existing runtime dispatcher.
The reservation references the same VFS and checks its revision and target
object. The FAT adapter and lease remain private in that transaction until
commit. There is no second mount table or FAT adapter. The C1E1 standalone
fixture and the default build keep their existing paths.

The caller ran three real StorageCtl prepare/abort cycles. While each
preparation was live, ordinary IPC STAT reached the service dispatcher:
the target directory was visible and its FAT file returned ENOENT. Six
authorized publication attempts revalidated the exact runtime reservation
and returned EOPNOTSUPP. The service also tried the exact live cookie
without StorageAdmin on each attempt; the syscall returned EPERM.
Wrong and aborted cookies returned ENOENT. Stable FAT alias identity was
checked during each completed stage. The guest covered failed FAT
initialization, parked caller retirement with late reply rejected, and
exact service image retirement before reply. The release assertions
checked zero outstanding commands, mapping users and retained command
data, and no pending flush. Queue and bounce addresses remained fixed at
20 KiB total across five explicit settlements and service retirement.
The independently extracted FAT partition passed read-only fsck and
recovered the untouched nine-byte payload; the whole disposable image
matched its input. The initial unknown FSInfo count was preserved.

Commit remains closed after reservation revalidation. The immediate
ordinary consumer is StorageCtl inventory/unmount in `kernel/src/storage.rs`:
it still uses the kernel mount table and could neither identify nor
withdraw a service-only mount. `linux_procfs::mountinfo_snapshot` and
`sys_linux_statfs` still observe kernel topology and capacity; ordinary
private mmap still lacks the general service descriptor admission needed
to use the service mount. The commit also needs registered source and
protection metadata plus one receipt consumption and backend/lease
transfer ordered against cancellation, retirement and withdrawal.
Committed FAT access, ordinary retained FAT descriptor/mmap lifetime,
write persistence and guarded ordinary boot were not run.

The [results](vfs-storagectl-runtime-namespace-integration-results.json)
record the native run, builds and focused suites. The unchanged guard
fails at the same four unmatched sites (71 references). The workspace
library run fails at the preexisting `sunlight-ui-wayland` duplicate
`panic_impl`; the separate `secret_store` run stalled in
`default_options_are_private_and_bounded` and timed out.
