# FastLane F1.1P-R2: native exec and lifecycle repair

Status: **targeted root cause confirmed; repair implemented and targeted validation passed; complete R2 acceptance and F1.1B-0 remain blocked**. No restricted SHM, ring, Doorbell, or production FastLane implementation is included. Machine-readable results and raw logs are in `target/fastlane-f11pr2-evidence/`. This working tree already contained uncommitted F1.1P/R changes on `4c70511d09ec42a915f869c33f56c552c6def959`; R2 preserves them.

## Reproduction and classification

The previous failure was the opt-in authorized-reader branch in `tools/helios-probes/capability-probe.S`, enabled by `SUNLIGHT_CAP_EXEC_PROBE=1`, executing native syscall 31 with `/bin/true`, an empty argv vector, and no explicit environment. It ran after the auth broker had published 16 auth-session and 16 account grants. Its missing assertion was successful old-image withdrawal followed by replacement-image exit and the final lifecycle/accounting gate. Historical raw log: `target/fastlane-f11pr-evidence/20261020T155200.202280Z/authority-1cpu.serial.log`.

R2 reproduced it before changing exec, with only bounded fault diagnostics corrected:

```sh
SUNLIGHT_CAP_EXEC_PROBE=1 cargo build --locked --offline -p sunlight-kernel \
  --bin sunlight-kernel --features cap_issuance_test
cp target/x86_64-unknown-none/debug/sunlight-kernel target/fastlane-f11pr2-evidence/repro.elf
bash tools/make_hybrid_iso.sh target/fastlane-f11pr2-evidence/repro.elf \
  target/fastlane-f11pr2-evidence/repro.iso target/limine .
python3 target/fastlane-f11pr2-evidence/run.py repro-1cpu 1 repro
```

The retained `repro-1cpu.json` contains the exact QEMU argv and image hash. QEMU 11.1.1, default `pc-i440fx-11.1`, BIOS/SeaBIOS boot, Limine hybrid ISO, KASLR disabled, no extra boot arguments, TCG `thread=multi`, CPU `max`, 1 vCPU, 1024 MiB, no NIC, virtio RNG, 180-second bound. No UEFI/pflash or KVM override. Firmware hash/configuration is retained in `baseline-and-machine.json`.

```text
[EXEC] Loaded ELF entry=0x400030, stack=0x7fffffffefb0
[CAP-PR] exec authority withdrawn before VM activation records=32 remaining=0
[FAULT] #14 pid=18446744073709551615 rip=0x0 rsp=0x7fffffffe530 err=0x10
[EXEC-R2] vector=14 cr2=0x0 cr3=0x10958000 cs=0x8 flags=PageFaultErrorCode(INSTRUCTION_FETCH)
```

This is an **unexpected kernel page fault**, followed by a fault-handler hang, rather than an expected user fault, capability rejection, lifecycle assertion, or QEMU failure. Error 0x10 means non-present, read, supervisor, instruction fetch; it is a missing mapping at address zero, not a write/protection violation. CS=8 independently establishes CPL0. CR2 applies because the exception is vector 14. The previous handler printed literal zero instead of the hardware error bits; the historical zero cannot be decoded as evidence. PID `usize::MAX` is the nonblocking scheduler lookup's unavailable sentinel, not evidence of PID corruption: the fault interrupted exec while SCHEDULER was held. The fixture recipient is PID 61953; its exact original fault-time incarnation and VM generation were not captured. These values must not be invented from the sentinel or physical root. Successful R2 runs log PID, incarnation, exact old/new VM identities, kernel RSP, entry RIP and user RSP at every commit.

## Demonstrated root cause and attribution boundary

`syscall_entry` saves registers and runs Rust on the caller's user stack. `exec_into_process` prepares an independent VM, including a newly zeroed stack at the same user virtual addresses, then activates it while Rust still uses the old stack's virtual addresses. CR3 replacement substitutes the backing underneath live Rust frames and return addresses. The continuing kernel path consequently returns through zeroed replacement-stack contents and attempts to fetch at RIP zero. The last successful checkpoint is grant withdrawal immediately before activation; the exec success message is absent. The frozen ELF's `repro-exec-disassembly.log` contains the CR3 loads in `exec_into_process` at `0xffffffff80003531` and its epoch retry at `0xffffffff8000357f`. This also exposes a second necessary transition correction: returning to the old syscall's SYSRET and asking a future timer to select the new context would resume the retired image's RIP/stack and let the timer overwrite the prepared context.

The narrow kernel-stack repair permits the same `/bin/true` transition, exit, grant cleanup, and exact frame baseline to complete. It does not change segment permissions, exception policy, TLB checks, generation allocation or timeout limits.

The committed pre-overlay source also contains the user-stack syscall entry and immediate activation sequence. Copies are under `committed-baseline-source/`. **Pre-existing mechanism is inferred from source; a matched pre-R baseline boot is unavailable/unproven.** F1.1P and F1.1P-R were uncommitted overlays, and no exact pre-R source snapshot was recovered. HEAD is the earlier F1 commit, not a complete pre-R tree with the identical fixture and prerequisites. No destructive reset or fabricated attribution was used. Reproduction before/after R2 uses separately frozen ELFs/ISOs; hashes identify the actual images, rather than assuming a current target binary is the one booted.

## Exec timeline and identity contract

1. Dispatch performs existing ABI translation and admission. The exec trampoline selects the calling task's kernel stack, reserving its top 256 bytes for the 160-byte initial context. The trampoline copies the 120-byte syscall register input onto the kernel stack as well. Image preparation and commit run below the reservation with interrupts disabled; no input frame is read through the retired VM.
2. `sys_exec` copies bounded pathname/argv/envp input before replacement. It locates the existing embedded/VFS image, validates ELF framing and acquires SCHEDULER then PMM. Exiting tasks, borrowed-VM tasks and VM owners with outstanding borrowers cannot replace the VM.
3. `exec_into_process` allocates a new root and checked, nonzero VM generation. Under SCHEDULER it temporarily installs the candidate VM in the process object for the existing loader; the hardware still uses the old root. The loader preplans bounded ELF regions, checks collisions and W^X, maps segment frames with declared permissions, reserves/maps an NX user stack, copies argv/envp/auxv through HHDM and installs the new iret context. The existing loader/stack code remains responsible for rollback. The existing ELF planner rejects an entry outside an executable PT_LOAD; no ELF permission relaxation is introduced.
4. Preparation failure restores the previous address-space object/personality/trusted-role fields and reclaims the candidate VM. Authority withdrawal is not reached. Native missing-image failures and existing loader/stack host tests cover portions of this contract; native allocation-failure-at-every-stage injection remains unverified.
5. Successful preparation reaches the semantic exec commit under SCHEDULER: CAP_BROKER withdraws old-image auth/account grants and both Wise Owl participant relationships, and old pending/reply identities are cleared. There are no lifecycle reservations. No intervening issuer can observe candidate state because it needs the same lifecycle lock.
6. The replacement's FS base is reset; its runtime establishes TLS. CR3 activation updates the exact root/generation active-CPU record through the unchanged epoch protocol. Old SHM views are cleaned using the old VM object; transferable peers retain their mappings. The retired VM's active mask must be zero before its private pages, page tables and ledger are reclaimed.
7. Successful `sys_exec` finishes CLOEXEC handling, trusted-role installation, and drops PMM/SCHEDULER guards and owned temporary vectors. Its kernel-stack trampoline enters the prepared iret context directly. It never returns through the retired syscall stack. Failure returns to the original syscall frame.
8. Entry checks externally requested termination and pending default fatal signals, restores initial extended-register state and FS base, then uses existing `iretq_to_context` to install GPRs, user RIP, CS=0x2b, RFLAGS=0x202, RSP and SS=0x23. The native probe checks entry, stack alignment, canonical stack/register argv/envp agreement, copied strings and unchanged PID before any later exec. Eventual exit uses the existing scheduler/reaper safety protocol.

Exec preserves process PID and foreground incarnation; a new VM root/generation identifies the execution image. Physical root reuse does not reuse the generation. Spawn has an unpublished process and uses `activate_on_success=false`; it does not acquire a second broker guard or use the live-exec trampoline. Independent fork remains explicitly unsupported. Borrowers share a VM protection domain and are not separate memory isolation boundaries.

## Authority ordering and memory safety

The serialization order remains SCHEDULER → PMM (exec/SHM) → CAP_BROKER → delegated registry as needed. Issuance uses SCHEDULER → CAP_BROKER → entropy. No extra scheduler lock, reservation subsystem, polling, long-lived pin or new TLB mechanism is added.

* Grant insertion first: publication still owns SCHEDULER through CAP_BROKER insertion. Exec then acquires lifecycle serialization, sees the committed record and withdraws it before activating the new image.
* Exec first: the recipient's previous pending call is cleared and VM generation replaced under the same lock. Old issuer reply state cannot match a cleared recipient request; issuance rejects. A fresh request explicitly reauthorizes issuance in the new VM; account binding uses that new generation.
* Failed preparation: withdrawal never executes, so existing authority and original VM survive. Exit under SCHEDULER invalidates admission; physical cleanup can remain deferred without permitting new publication. An external kill after exec commit does not restore the retired image.

Auth grants retain their existing recipient-PID ABI plus lifecycle withdrawal. Account records bind recipient PID and VM generation. Wise Owl records bind both participant generations and retire when either participant execs/exits. Ordinary endpoint, VFS and legacy SHM bearer capabilities intentionally retain their distinct process/transfer contracts. R2 does not blanket-revoke process capabilities on exec. CLOEXEC still follows existing descriptor flags. Host tests verify an ordinary endpoint survives image-grant withdrawal and disappears on recipient exit.

The pre-existing TLB mechanism tracks exact root/generation per active CPU, retries activation over a mutation epoch, and uses synchronous remote acknowledgement before mapping/backing reuse. It remains unchanged. Live exec excludes shared-VM borrowers; scheduler selection is serialized while the current task pivots and replaces its VM. The retained native diagnostics assert zero active CPUs at each retired-root reclaim. Reaper protections against live owner/core/stack references and delayed migration remain intact. F1 resident tests separately exercise remote SHM release and acknowledged retirement. These checks do not constitute a newly completed deterministic migration/exit/exec race matrix.

## Validation and remaining blockers

Run the reusable gate:

```sh
python3 tools/fastlane_f11pr2_validate.py --native --regressions
```

It forces the opt-in exec probe, freezes separate images, retains raw results, and requires exactly eight replacement-image user assertions, nine old-VM reclamation checkpoints, the 32-record preactivation withdrawal and the final lifecycle/resource markers per CPU. The final `/bin/true` execution is reached after the repeated probe. The probe also executes a missing-image failure in every replacement and fails if successful exec ever returns. Host barriers exercise the production broker/issuance functions, with an explicitly modeled process-table exec transition; they are not hardware scheduler or CR3 tests.

Final result tables, image hashes, commands and resource measurements are retained in `validation-summary.json`. Existing unrelated repository WAV-fixture failure is recorded separately. Workspace-wide `cargo fmt --all -- --check` reports pre-existing formatting differences; R2 additions were formatted without rewriting unrelated source. `git diff --check` passes. Complete native coverage is still missing for deterministic issuance preparation versus exec, recipient/issuer exit during preparation, concurrent exec/cleanup, native PID reuse, shared-VM borrower rejection, exec with live old-image SHM views, forced migration/delayed remote VM access, nonzero-FS/TLS and Linux-image exec handoff, Wise Owl interleavings, and every allocation/rollback stage. The authorized reader unmaps its legacy view before exec, so independent F1 SHM retirement passes must not be relabeled as that missing exec-with-live-view test. No claim of measured scheduler hold-time tails, heap-capacity release or universal historical bearer nonrecurrence is made.

F1.1B-0 remains **blocked** even when the focused exec and compatibility gates pass. Secure-token fail-closed/collision/reservation evidence must be evaluated independently; native validation does not turn the 62-bit probabilistic bearer namespace into deterministic historical exclusion. The full recipient/VM reuse and lifecycle matrix and the numeric VFS broker-PID authorization concern remain open. Restricted recipient grants, RO mapping/revocation faults, active view withdrawal, SPSC, Doorbell and production integration remain deferred.

## Recorded final-source exec results

| CPUs | User entry assertions / retired VM reclaims | Old-image grants | Free physical frames before → after | Shared-data / page-table frame delta |
| --- | --- | --- | --- | --- |
| 1 | 8 / 9 | 32 → 0 | 68224 → 68224 | 0 / 0 |
| 2 | 8 / 9 | 32 → 0 | 67966 → 67966 | 0 / 0 |
| 4 | 8 / 9 | 32 → 0 | 67454 → 67454 | 0 / 0 |
| 12 | 8 / 9 | 32 → 0 | 65406 → 65406 | 0 / 0 |

All rows reached the terminal lifecycle assertion after `/bin/true` exited. Broker tokens, auth/account records, shared objects and task SHM views were empty after final reaping. Each of the nine retired VM reclaims asserted an exact zero active-CPU mask. Global active-VM inventory and heap metadata capacity were not separately counted. These are per-run baselines; absolute free counts across different ELF builds are not a drift comparison.

Final-source host tests: **77 kernel**, **115 IPC/service**, **11 ELF planner** tests passed, plus the scheduler host gate. The two added barrier tests are `auth_publication_vs_exec` and `account_publication_vs_exec`. Workspace formatting failures and the missing WAV fixture are retained as unrelated limitations rather than silently omitted. The complete initial compatibility run also passed scheduler 1/2/4/12, combined D1/D2/D3/RecoveryBlocked 1/2/4, resident/F1 SHM 1/2/4/12 and local TCP/TLS 12. Final compatibility rerun results are recorded separately in the JSON assessment.

The authority-policy blocker is specific: `trusted_auth_broker`, lock/TTY trust-chain roles, deferred IPC replies and inherited process-level credentials need an explicit cross-exec inheritance review. Current exec clears the recipient grant tables and immediate pending/reply relationship, while some process roles intentionally have a different lifetime. R2 preserves existing behavior and does not claim that every service-role or deferred-message authority is VM-bound or safe to revoke wholesale. That audit, live-view exec coverage and the missing deterministic native interleavings prevent complete security acceptance.

## Final regression decision

Final run: `target/fastlane-f11pr2-evidence/20261020T172801.239481Z/`. All captured source hashes matched after validation; each frozen ELF/ISO hash was verified again when producing the final assessment.

| Gate | CPU configurations | Outcome |
| --- | --- | --- |
| Native exec plus lifecycle issuance | 1, 2, 4, 12 | PASS, eight entry assertions and nine retirements per run |
| Resident boot and F1 SHM retirement | 1, 2, 4, 12 | PASS, remote acknowledgement counts checked |
| Scheduler lifecycle | 1, 2, 4, 12 | PASS |
| Combined D1/D2/D3/RecoveryBlocked | 1, 2, 4 | PASS, all four completion markers reached |
| Exact-VM TLB replacement/accounting | 12 | PASS |
| Anonymous unmap/shootdown | 4, existing supported gate | PASS |
| Permission/mprotect shootdown | 4, existing supported gate | PASS |
| Local TCP/TLS/SOCKS fixtures | 12 | PASS on serialized final retry |
| Secure entropy unavailable | 1, CPU entropy instructions disabled and no RNG device | PASS, no user authority published |
| Kernel / IPC-service / ELF host | Host | 77 / 115 / 11 passed |
| Default target check, Python syntax, diff whitespace | Applicable host/target | PASS |
| Workspace formatting | Workspace | Existing formatting differences; raw log retained |
| Media host | Host | Existing missing WAV fixture; raw failure retained |

The first final-image TCP/TLS attempt timed out waiting for the final hostname-check command. Serial showed an injected command missing characters (`wrong.sunlig38151...`) and the guest rejecting malformed `--resolve` arguments, rather than executing the intended hostname assertion. Its log, serial and classification are retained under `network-final-12cpu-first-attempt*`; it is **failed harness/incomplete assertion**, not a pass or native exec fault. Concurrent TCG load may have contributed to keyboard delivery loss, but that attribution is unproven. The unchanged fixture, timeout, image and CPU configuration passed all eleven checks after other QEMU gates completed. No check was removed or timeout increased.

The final JSON status is `targeted_repair_validated_full_lifecycle_acceptance_blocked`; `f11pr2_complete=false` and `f11b_0_unblocked=false`. Remaining native interleavings and inheritance-policy questions above are blockers. Restricted SHM and later FastLane phases remain deferred.
