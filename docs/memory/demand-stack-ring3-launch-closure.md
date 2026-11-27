# STACK-2A engineering status

**PARTIAL.** The packaged eager fixture passed startup and normal exit. The packaged demand guest did not satisfy the existing session-bundles readiness gate, so matched Ring-3 startup and all deep-growth qualification remain blocked. No sustained 512 MiB run was attempted.

| Gate | Eager | Demand |
| --- | --- | --- |
| Original HTTP delivery in fresh 1 GiB BIOS guests | Fetch exits 0; separate stat exits 1; invocation produces no foreground exit | Same |
| Packaged fixture readiness and visibility | Existing runner gates pass; stat sees a 9,304-byte root-owned regular file, mode 0755 | Runner times out waiting for `[BOOT-RES] stage=session-bundles`; fixture not submitted |
| Packaged Ring-3 baseline | PASS | BLOCKED |
| Incremental depth, zero fill, data preservation, hardware faults and continuation | NOT_RUN | BLOCKED |

The first confirmed boundary on the HTTP route is executable visibility. The userspace file service logs creation under `/tmp`, but both controls report `stat: cannot stat /tmp/stackprobe`, and neither records a fixture load or scheduler publication. For slash-qualified commands the shell probes stat before spawn; native spawn resolves bytes through embedded images and the kernel VFS. The evidence does not establish the exact stat errno or the fate of the file in the service namespace. It supports a launch delivery problem shared by both configurations. The same host ELF then ran from the compressed boot archive on eager, ruling out a fixture format defect for that case. The separate demand readiness timeout is unresolved; there is no evidence attributing it to demand stack growth.

The served artifact is a static x86-64 ELF64 System V executable, with entry `0x401000`, three load segments and no interpreter. Host SHA-256: `dcde9dcc96af8f7f572e4b44f176bff3e5758c98144457671901915164f4b761`. Both download logs report the expected length and success; the bytes stored at the guest download path could not be verified independently.

The eager serial record follows `/bin/stackprobe` through segment loading, startup stack construction, scheduler publication, and foreground PID 42. The assembly emits its serial marker only after checking CPL 3, 16-byte alignment, argv and the auxiliary random data pointer and contents. The native group recorded one terminal result (PID 42, incarnation 83, VM 93, code 0), and the shell reported exit 0 from that result. Retirement recorded 515 user frames, eight page-table frames, zero swap blocks and zero cleared IPC messages. The 515 frames are the full user-frame reclaim count: 512 eager stack pages plus three ELF pages. This establishes the observed VM reclaim and wait result; it does not independently measure recovery of every process resource or cover repeated teardown.

Historical STACK-1 matched 1 GiB document-idle measurements were 16,384 eager versus 476 demand stack frames across 32 leaders. STACK-2's separate demand workload recorded 476 at idle and 478 after document recovery. Its 616 KiB reference is the historical workload span; STACK-2A measured no fixture growth or attributable hardware faults.

The existing worktree implements an opt-in 2 MiB native main-stack reservation with guards, seeded startup pages, authorized page backing and retirement accounting. Eager remains the default. STACK-2A changed fixture packaging and diagnostic inputs under `target/stack-2a` and updated documentation; it made no production code changes. Existing adaptive heaps, PMM accounting, ZRAM and VFS migration prerequisites were left in place. Earlier fixture assembly/link and one host ledger test passed. The recorded eager and demand release builds passed; a fresh production-default build for this report was not run.

Next milestone: resolve the packaged demand session-bundles readiness failure without changing the gate. Confirm the same baseline witness on demand; only then run the matched growth cases and correlate fixture instructions, hardware faults, physical commitment and retirement. Guard and execute rejection, allocation failure, same-page concurrency, and repeated teardown remain unqualified.

Evidence: `target/stack-2a/{eager,demand}/` for the original delivery runs; `target/stack-2a/{eager,demand}-stat/serial.log` for the stat diagnostics; `target/stack-2a/{eager,demand}-packaged/serial.log` and sibling runner logs for the packaged attempts; `target/stack-1/final/build-{eager,demand}.log` for the earlier release builds.
