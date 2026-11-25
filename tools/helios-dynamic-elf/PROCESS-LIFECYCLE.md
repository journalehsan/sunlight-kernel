# Helios process lifecycle completion

Evidence: `target/helios-process-lifecycle-complete/orphan-v2/`. The initial
TCG, same-image TCG replay, and KVM boots used two vCPUs and ISO SHA-256
`de77ce67088a2ec81047962c58b7aef1a166de08f73db17781a91d09604ad6d2`.
Each run has its QEMU command, full serial log, filtered lifecycle trace,
and JSON results. The earlier `orphan-v1` capture predates the residual
resource diagnostics. Its initial validator confused VM generation with
process incarnation; revalidation of that capture corrected the identity join.

| Control | Initial TCG | TCG replay | KVM | Orphan frames / page tables reclaimed (initial TCG) |
| --- | --- | --- | --- | --- |
| Static non-PIE | Pass | Pass | Pass | 517 / 8 |
| Handoff-only PIE | Pass | Pass | Pass | 520 / 10 |
| R_X86_64_RELATIVE | Pass | Pass | Pass | 521 / 10 |
| One library | Pass | Pass | Pass | 524 / 11 |
| Two-library graph | Pass | Pass | Pass | 528 / 12 |

The table covers parent exit before collection. A userspace parent launches
each control, observes it stop through the process liveness API, and passes
the PID returned by spawn to the supervisor. It exits without calling
`waitpid` on any control. The validator joins PID to process incarnation
and slot using allocation traces. The children published status zero while
their parent was live. None woke the parent or had its status collected.
Once the parent exited, the old relationship remained in the reaped row,
but the kernel no longer retained that row for collection. The supervisor,
which is the parent's parent and not the children's parent, got
`Err(Failed)` for each orphan both after parent exit and after slot
overwrite. The next occupants had fresh identities and no inherited wait
status. All 15 fixture replacements returned `Ok(0)` to their own parent.

The parent and all five orphans reached `Reaped` with zero mapping
regions and zero PID-owned frames, reclaimed page-table roots, no kernel
stack, and no pending cleanup. Each orphan slot was overwritten, dropping
the old status tombstone. This is process-scoped accounting.

Same numeric PID recycling is **blocked by the production allocator** for
every control and accelerator. Allocation takes the checked maximum PID
across all rows, including reaped rows; insertion or overwrite raises that
maximum. Exhaustion fails closed. The two host allocator tests passed.
The guest runs prove slot reuse with a different PID and incarnation;
they do not exercise two incarnations with the same PID. The current
`waitpid` interface takes only a PID and checks the caller's parent
relationship in the kernel. A future recyclable PID design would need an
identity-aware wait contract before claiming stale-handle protection.

The existing dynamic ELF gate passed on all three boots, including 16/16
graph rollback cases per boot; host ELF tests passed 14/14. The loader,
relocation code, wait assertions, wait timeouts, and process semantics
were unchanged by this work. Wayland and transport were not rerun.

The strict milestone remains open if it requires a live same-PID reuse
witness. Under the allocator's existing no-recycling contract, the
reachable lifecycle gates support starting a bounded TLS milestone.

```sh
bash tools/helios-dynamic-elf/build.sh lifecycle RUN
python3 tools/helios-dynamic-elf/replay.py tcg --lifecycle RUN
python3 tools/helios-dynamic-elf/replay.py tcg --lifecycle RUN --replay
python3 tools/helios-dynamic-elf/replay.py kvm --lifecycle RUN
```
