# Risk register

All entries are open design/integration risks unless a future milestone supplies
execution evidence. Priority follows correctness before availability/performance.

| ID / priority | Risk and consequence | Control / owner / stop condition |
|---|---|---|
| R1 critical | Unsupported or dishonest flush loses acknowledged writes | Block adapter qualification SFS-1/5; read-only/freeze on unsupported/error |
| R2 critical | VirtIO timeout followed by buffer reuse permits late DMA/request confusion | Quiescence/fencing SFS-5; no writable integration without it |
| R3 critical | Retire CP without preserving closure corrupts fallback | Union liveness and R barrier SFS-2; any early reuse stops |
| R4 critical | Self-accounting tables/summaries overflow reservation or recurse | Precomputed bounded layout fixed point SFS-1/2; stop before I/O on nonconvergence |
| R5 high | Corrupt CP indistinguishable from interrupted publication reduces availability | Explicit read-only degradation SFS-2; no guessed writable recovery |
| R6 high | Full-map rewrites and two fallback generations exhaust space | Volume/node/transaction bounds + computed reserve SFS-1/4; reject infeasible geometry |
| R7 high | Node/mount/handle reuse revives stale authority | Nonwrapping generations, no nid recycling SFS-3/5; exhaust safely |
| R8 high | Retained directory/openat TOCTOU or mount-boundary bypass | VFS object pins and atomic relative operations SFS-5/6; no path-based pin substitute |
| R9 high | Filesystem identity mistaken for grant or forged ownership | VFS credentials/device grants remain authoritative SFS-5; reject unauthorized traversal |
| R10 high | Invalid pointers/overflow/cycles exhaust memory or expose bytes | Checked codecs, iterative budgets, visited sets SFS-1/2; reject before use |
| R11 high | Blocking disk under VFS/scheduler spinlocks causes deadlock/starvation | Pin/drop/revalidate and lock-order proof SFS-5; native forced schedules required |
| R12 high | Cleaner reserve exhausted by metadata/fallback pins | Worst-case reserve, bounded victim/cut, ENOSPC SFS-4; no forced reclaim |
| R13 high | Orphan count/open ref overflow or last-close loses backing | Checked admission, persistent orphan table, operation pins SFS-3; no unowned deletion |
| R14 high | Symlink loops/absolute targets escape namespace | VFS40-link/expanded-byte budget and mount grant checks SFS-6; unsupported until ready |
| R15 medium | Metadata ABI invents times/ino/link counts | Provenance, full identity and time-valid mask SFS-6; no fake struct stat |
| R16 medium | Checker shares allocator bug and accepts bad state | Share codec only, independent graph reconstruction SFS-1/2 |
| R17 medium | CRC misses collision or hostile media forgery | Explicit nonadversarial model; no authentication claim; stronger integrity needs ADR |
| R18 medium | UTF-8/255-byte names exceed current64-byte listing ABI | Versioned lossless enumeration SFS-5/6; reject integration truncation, never alias names |
| R19 medium | Cloned UUID or parallel service/raw writes create split authority | Exclusive volume/device owner SFS-5; reject duplicate writable access |
| R20 medium | Benchmark/model confused with durability/runtime evidence | Distinct static/model/host/native/UI/hardware labels; any unsupported PASS corrected |
| R21 high | Old acknowledged identity reused after corrupt fallback | Fallback read-only; future explicit recovery burns high-water IDs or new volume identity |

No risk is closed merely because it appears in this table. SFS-0 completion means
contracts/mitigations/dependencies are reviewable; implementation gates remain future.
