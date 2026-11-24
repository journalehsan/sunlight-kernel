# Helios waitpid and slot reuse

The first failure is **slot reuse (zombie/reap ordering)**. In the original
two-vCPU TCG run, child 750 (parent 719) printed the relocated-pointer marker,
exited 0, and reclaimed 521 user frames and 10 page tables. The allocator
published PID 751 in slot 28 before parent 719 reported `Err(Failed)`.
A fresh boot of the same unmodified ISO reproduced the sequence on the
handoff-only ET_EXEC control: child 746 exited 0, slot 26 was overwritten by
PID 747, and the parent reported `Err(Failed)`. That boot stopped at the
existing assertion before the static and relocation controls. The historical
static control passed; the historical TCG replay and KVM boots also passed.
The harness has no switch to disable slot reuse.

The native syscall waits for a specific PID. Linux wait4 is rejected and no
wait-any operation is implemented here. Spawn registers the parent PID and
incarnation under the scheduler lock. Exit publishes status before waking a
matching parent under that lock. The blocking syscall parks and reports
`EAGAIN`; libc issues its existing subsequent lookup after wake. Before the
fix, cleanup retained the exit code in the process row, but the allocator
could overwrite that row before this lookup (or the parent's first lookup).
The identity did not alias: the requested PID disappeared. Address-space
teardown did not clear the status.

Syscall-spawned children now retain their wait record until the matching
parent incarnation consumes it or that parent exits. Resource reclamation
still runs independently and may finish before the wait. General process
allocation and thread admission both respect the outstanding wait record.
Successful wait consumes it once; physical cleanup and the existing CPU and
stack checks still gate reuse. Ownership is set at publication, so failed
creation leaves no wait record. Kernel-internal launches keep their existing
supervision behavior. No ELF loader logic or wait timeout changed.

Evidence is in `target/helios-dynamic-elf-waitpid/`. The baseline directory
contains the fresh handoff-only failure and a copy of the original TCG
evidence. The original relocation evidence remains in place. The verified
directory contains the traced build, per-boot command JSON, serial trace,
artifact hashes, and validation summaries. To run the gate:

```sh
bash -x tools/helios-dynamic-elf/build.sh waitpid verified
python3 tools/helios-dynamic-elf/replay.py tcg --waitpid verified
python3 tools/helios-dynamic-elf/replay.py tcg --waitpid verified --replay
python3 tools/helios-dynamic-elf/replay.py kvm --waitpid verified
cargo test -p sunlight-elf --target x86_64-unknown-linux-gnu
```

The guest gate checks the original child status, exact slot and incarnation
across reuse, stale PID rejection, and the replacement child's own status.
It also checks that cleanup preserves an unconsumed status whenever cleanup
wins the race, and that replacement follows both cleanup and the parent wait.

Initial TCG, same-image TCG replay, and KVM all passed on two vCPUs. Each
returned `Ok(0)` for the relocated child after it reclaimed 521 user frames
and 10 page tables. Both handoff controls, the unchanged static control, and
all 11 rollback cases passed on each boot. All 14 host ELF tests and both
host PID allocator tests passed. The two-vCPU KVM Wayland gate passed with
two acknowledged frame captures. Standalone transport, libwayland, and
Canvas suites were not rerun.
