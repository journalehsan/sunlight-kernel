# STACK-2: Native Ring-3 demand stack qualification

The [STACK-2A follow-up](demand-stack-ring3-launch-closure.md) confirmed baseline Ring-3 execution with the packaged fixture on the eager image. The attempt results below are retained as historical evidence.

**PARTIAL.** The host ledger test and short guest boot checks passed. The dedicated fixture did not reach a confirmed launch, so STACK-2 does not qualify production demand stacks or sustained 512 MiB operation.

The [fixture](../../tools/tests/main_stack_ring3.S) builds as a static native ELF. Its positive cases check startup alignment, arguments, the auxiliary vector's random-data pointer, and Ring-3 entry. They then either move RSP straight to the usable floor and touch a single page, or descend a page at a time, check each page for zeros, write distinct values, and verify them on the way back. The deep case reaches the floor of the 2 MiB window and crosses 616 KiB. No direct kernel backing calls are used by the fixture.

| Check | Result | Evidence |
| --- | --- | --- |
| Fixture build and host guard ledger | PASS | Assembly and link succeeded; 1 host test passed. |
| Dedicated startup, deep and sparse growth, zero fill, and data preservation | BLOCKED | Guest fetched the ELF, but the baseline invocation produced neither a fixture marker nor a process exit before timeout. |
| Fixture hardware page faults and continuation | BLOCKED | QEMU interrupt logs exist; there is no attributable fixture launch or continuation. |
| Guard, out-of-range, execute permission and terminal fault attribution | NOT_RUN | Intentional fault cases were omitted from the safer run. |
| Allocation failure and rollback | NOT_RUN | STACK-1 injected failure in the kernel fixture; no STACK-2 Ring-3 allocation failure was induced. |
| Exact fixture commitment, reclamation and repeated teardown | BLOCKED | No fixture process or teardown record. |
| Same-page concurrent execution | NOT_RUN | Reachable in principle by source review; no native interleaving observed. |
| Fresh native workload regression | PARTIAL | 1 GiB BIOS, 2 vCPUs: compressed boot, sunlightd, shared VFS boot read, document workload, interactive shell and CA setup passed. Concurrent TLS did not run in this attempt. |

The first 1 GiB BIOS attempt fetched the ELF, then stopped at a failed chmod command. The safer follow-up omitted that command and stopped on the first invocation. These are harness launch failures; neither exercised the stack fixture's assertions. The logs are under `target/stack-2/demand-bios-1024` and `target/stack-2/safe-demand-bios-1024`, with separate runner logs alongside them.

The safer guest reported 476 stack frames across 32 leaders at document idle, and 478 after document recovery. The corresponding free PMM counts were 166,234 and 166,232; the boot-to-recovery minimum was 153,418. The recovery checkpoint reported 350 cumulative successful stack handler samples totaling 7,081,695 TSC ticks, with a maximum of 1,118,026. These are workload-wide TCG measurements; they do not measure fixture latency or fixture resource recovery.

The main stack reservation belongs to the VM owner. [Native thread creation](../../kernel/src/arch/x86_64/syscall.rs#L3015) shares the address-space handle; [owner lookup](../../kernel/src/sched/mod.rs#L2300) resolves the owner by VM identity, and the [fault handler](../../kernel/src/arch/x86_64/interrupts.rs#L667) does not require the current task to be the leader. Separate native tasks can therefore access the same reserved page while using separate execution stacks. The scheduler and PMM locks serialize handler work; this source property and the earlier duplicate commit check do not establish that two vCPUs actually entered with faults on the same absent page.

The [STACK-1 results](demand-backed-main-user-stacks-results.json) contain the prior matched eager/demand passes for 1 GiB, 768 MiB and 512 MiB BIOS and 512 MiB UEFI at two vCPUs, including verified TLS overlap. STACK-2 did not repeat that full matrix or run the 1/4 vCPU variants. To resume positive qualification, first deliver the fixture through an existing executable packaging path, confirm its baseline launch, then correlate Ring-3 page-fault records with the fixture instructions, exit status and VM retirement accounting. No production kernel changes were made for STACK-2.
