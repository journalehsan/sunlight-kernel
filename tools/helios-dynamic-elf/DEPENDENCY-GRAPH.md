# Bounded Helios dependency graph

Run from the repository root:

```sh
bash tools/helios-dynamic-elf/build.sh dependency-graph final
cargo test -p sunlight-elf --target x86_64-unknown-linux-gnu
python3 tools/helios-dynamic-elf/replay.py tcg --dependency-graph final
python3 tools/helios-dynamic-elf/replay.py tcg --dependency-graph final --replay
python3 tools/helios-dynamic-elf/replay.py kvm --dependency-graph final
```

The existing shared fixture debug gate admits the exact bundled PIE and
two named bundle resources. The application and libraries are ordinary
GCC output with a System V hash table and no libc, TLS, RELRO or ELF stamp.
The application requests the existing handoff interpreter. The app needs
`libhelios_a.so`, which needs `libhelios_b.so`. Both libraries declare a
matching SONAME. The leaf has no dependencies; it calls an exported helper
through its own PLT so the rollback suite can reject a real relocation in
the second library. The complete metadata and exact file digests are in
the build's `graph*.readelf` and `SHA256SUMS`.

The loader follows at most one DT_NEEDED edge per image. It validates each
name before looking it up in the bundle allowlist, rejects revisits, and
requires the two approved libraries in order. SHA-256 is checked against
the embedded build references. No search path or host lookup is involved.

The application and interpreter are staged first. The loader discovers
and validates both dependencies, then maps A and B in that order. Each
library has a disjoint 16 MiB window above the interpreter. Its bias is
the window start minus the lowest load page. The existing mapper checks
region and page collisions and applies segment permissions (read-only,
read/execute, or read/write; never writable and executable). Embedded
bytes are immutable for the life of the kernel; each process owns its
copied mappings and page tables until rollback or exit cleanup.

The eager symbol scope is application, A, then B. All three are checked
for a unique matching definition. Only global default-visible functions
in file-backed executable ranges qualify. Each image has one zero-addend
JUMP_SLOT; its owning image receives the write in a writable segment.
The pre-existing standalone RELATIVE control still uses its own relocation
validator. Other graph relocation formats and unsupported dynamic tags
are rejected. The scheduler sees the process only after mapping,
relocation, stack and auxiliary vector construction succeed.

Each run directory contains the build evidence and, per accelerator,
the serial log and validation result. The rollback suite injects faults
before and after mappings and relocation writes. Process construction
failure is tested at its allocation boundary, before any image is mapped.
Wayland and transport tests are outside this gate.

## Recorded run

Evidence is under `target/helios-dynamic-elf-dependency-graph/final/`.
`build.log` records exact commands; `toolchain.txt` identifies GCC
16.2.1 (20260810), target `x86_64-pc-linux-gnu`, and GNU ld 2.47.
`bundle-contents.txt` and `SHA256SUMS` record the full inventory and
digests. The graph bundle contains `Manifest.toml`, `Program/probe`,
`Program/libhelios_a.so`, and `Program/libhelios_b.so`.

| Relocation owner | Symbol | Provider | ELF offset |
| --- | --- | --- | --- |
| probe | `helios_graph_a` | A | `0x3228` |
| A | `helios_graph_b` | B | `0x31d0` |
| B | `helios_graph_multiply` | B | `0x31d8` |

All three are zero-addend `R_X86_64_JUMP_SLOT` relocations. The initial
TCG run, same-image replay, and KVM used ISO SHA-256
`0737ea14c61508ea9796cf4393de826600a1c57784e31f28756cac119bce1a14`.
All three passed the ELF controls and all 16 graph rollback cases.
The malformed dependency offset was rejected at string-table validation.
On each run, the graph printed `result=42`, exited with 0, returned
`waitpid=Ok(0)`, and reclaimed 528 user frames and 12 page tables.

| Run | Graph PID | Graph rollback baseline (free pages / page tables / owned frames) | Full gate |
| --- | ---: | --- | --- |
| TCG | 751 | 719466 / 73 / 0 | Failed one-library probe slot-reuse assertion |
| TCG replay | 752 | 719466 / 73 / 0 | Failed the same assertion |
| KVM | 755 | 719499 / 73 / 0 | Passed |

The TCG runs observed graph-to-probe slot reuse. The earlier one-library
probe's slot went to `sunlight-sessiond` on initial TCG and `timed` on
replay, so neither recorded the probe-to-probe reuse required for that
control. See each run's `result.json`, `serial.log`, and
`wait-lifecycle.log` for loader and lifecycle evidence.
