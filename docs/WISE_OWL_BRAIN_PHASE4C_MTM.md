# Wise Owl Brain Phase 4C — MTM, Status Awareness, Foundation v1

## Phase 4B audit (summary)

| Item | Classification |
|------|----------------|
| Native `wiseowl-braind` startup via sunlightd + kernel embed | directly reusable |
| RAMFS/spawn paths `/sbin/wiseowl-braind`, `/bin/wiseowl-brainctl` | directly reusable |
| Endpoint `wiseowl.brain.v1` registration | directly reusable |
| SHM request/reply + 24-byte register limit | directly reusable |
| Kernel badge = PID only (not UID) | incorrect vs design docs; handled with body subject + PID stamp |
| Root uid=0 accepted | directly reusable |
| `GroundedFact` / `BrainResponseMeta` | reusable with small adapter (extended in 4C) |
| Session + System adapters | directly reusable |
| KV / MemoryDB / Index adapters (4B) | placeholder → real status/MTM in 4C |
| sunlight-kv PUT_SHM/GET_SHM (keys ≤16 bytes) | reusable with small adapter |
| Per-user KV namespace | missing → defined as `wb1:{hex_uid}:{code}` |
| MemoryDB `GetHealth`/`GetStats` | directly reusable |
| Index `GetHealth`/`GetStats` | directly reusable |
| Welcome SHM greeting path | directly reusable |
| Host vs Native bodies | dual stack retained; pipeline shared |

## Topology

```
Welcome / brainctl
    → nameserver wiseowl.brain.v1
    → wiseowl-braind
         ├─ SessionContextSource
         ├─ SystemContextSource (sysinfo RAM)
         ├─ KvContextSource (sunlight-kv MTM)
         ├─ WiseOwlStatusContextSource (memorydb.v1 GetHealth/Stats)
         └─ IndexContextSource (index.v1 GetHealth/Stats)
              → LocalBoundedProvider / greeting planner
              → BrainResponseMeta provenance
```

## KV namespace (MTM)

Keys fit sunlight-kv register packing (≤16 bytes):

| Key | Meaning |
|-----|---------|
| `wb1:{uid_hex}:vc` | visit_count (completed Welcome visits) |
| `wb1:{uid_hex}:gen` | last_completed_generation |
| `wb1:{uid_hex}:lp` | last_successful_provider |
| `wb1:{uid_hex}:gs` | greeting_style |
| `wb1:{uid_hex}:sms` | show_machine_summary |
| `wb1:{uid_hex}:sis` | show_index_status |

### Semantics

- **visit_count**: number of *explicit* Welcome completions (`BrainOp::WelcomeCompleted`), not greeting requests. Saturates at `u32::MAX`.
- Completion ownership remains with Welcome/session; Brain only records notification.
- Malformed values → defaults; missing values → defaults; write failures never fail greetings.
- Cross-user: keys encode uid; callers supply own uid from authenticated subject; mismatched body identity fields rejected.

## Status boundaries

- MemoryDB: available, healthy (Ready), generation, active record count. **No** document payloads.
- Index: available, ready flag, sources tracked, files indexed, generation. **No** search/document text.
- “Endpoint exists” ≠ ready; ready only when service returns ready flag.

## Context budget

- max_facts=16, max_total_bytes=2048, max_source_latency_ms=50, max_total_latency_ms=200
- Order: Session → Request intent → System → KV → MemoryDB → Index
- Exhausted budget skips remaining optional sources (degraded mask).

## Greeting styles

- Concise / Friendly / Technical — deterministic wording only.
- Technical statements require grounded generation/version facts.

## Non-goals (still unimplemented)

General chat, conversation history, document retrieval, embeddings, Pattern Recognition, online AI, command execution, autonomous actions, self-healing, telemetry upload.
