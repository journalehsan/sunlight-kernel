# Helios wait identity and slot reuse

```sh
bash tools/helios-dynamic-elf/build.sh slot-reuse traced-baseline
python3 tools/helios-dynamic-elf/replay.py tcg --slot-reuse traced-baseline
python3 tools/helios-dynamic-elf/replay.py tcg --slot-reuse traced-baseline --replay
python3 tools/helios-dynamic-elf/replay.py kvm --slot-reuse traced-baseline
cargo test -p sunlight-elf --target x86_64-unknown-linux-gnu
```

The first failure was background-service interference with the test's
allocation-order assumption. With the original assertion and allocation
tracing enabled, the real probe `750:129` (parent `719:98`) vacated slot
25. The next occupant was `timed` `751:130` (parent `727:106`), while
the launcher collected graph child `752:131` from slot 28. The old
assertion required a later probe to occupy slot 25. It did not hardcode
a PID or slot, but it assumed no unrelated allocation would take that
slot first.

The validator now joins spawn and wait events by PID and incarnation
under the matching parent. It verifies the complete set of stale
waits, and proves actual reuse by joining each slot release to the next
allocation and checking that the matching parent's kernel lookup failed
after the overwrite. It records the occupant and replacement separately,
including when they are two different probes. A missing reuse witness is
reported as missing coverage. The fixture still waits on its returned
child PID and requires status zero and `Err(Failed)` for stale PIDs.

The tracer adds scheduler publication diagnostics under the existing
debug gate. The Linux ELF loader, wait lookup, timeouts, allocation
order, and exit and cleanup behavior are unchanged. Live slot reuse
changes both PID and incarnation; the PID allocator does not recycle
the numeric PID in these runs.

Evidence is under `target/helios-dynamic-elf-slot-reuse/traced-baseline/`.
All three boots use ISO SHA-256
`e6de4c773b739a925f542a1793e27299817d7d769d6901469af99f7a44c3f5b4`.
The build log and SHA256SUMS record the commands and artifacts. Each
accelerator folder has full serial and filtered wait lifecycle logs
and a JSON validation result. Prior dependency graph evidence is
unchanged.

Initial TCG, same-image TCG replay and KVM passed the full gate with
two vCPUs. The real probe reclaimed 524 frames and 11 page tables;
the graph reclaimed 528 frames and 12 page tables. Both returned
42 and status zero. All 16 graph rollback cases and all 14 host ELF
tests passed. The runs include same-slot and different-slot replacements,
unrelated occupants, and child cleanup before collection. Dedicated
parent-exits-before-collection and numeric PID recycling scenarios
were not run. Wayland and transport were not rerun.
