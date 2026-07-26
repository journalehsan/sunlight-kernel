# Wise Owl Memory Foundation (Phase 0 + Phase 1)

Bounded short-term cognitive memory contracts and service for SunlightOS.

**Status:** Phase 0 contracts and Phase 1 short-term memory service implemented.  
**Service name:** `wiseowl-memoryd`  
**CLI:** `wiseowl-memoryctl`  
**Crate:** `wiseowl-memory`

---

## Scope

This foundation provides:

1. **Phase 0** — stable cognitive-memory contracts (typed IDs, classes, kinds, provenance, lifecycle, errors, protocol).
2. **Phase 1** — a bounded short-term memory service:
   - RAM-resident working and hot memory
   - sealed LZ4-compressed cold short-term records
   - optional selective promotion into `sunlight-kv`
   - IPC-shaped API (host UDS + bincode; SunlightOS wiring reserved)
   - CLI diagnostics
   - deterministic unit/integration/soak tests

## Non-goals (explicitly not implemented)

| Area | Status |
|------|--------|
| Language models / inference | Out of scope |
| Model weight training / re-training | Out of scope |
| Embeddings / vector databases | Out of scope |
| Semantic similarity | Out of scope |
| Document directory scanning / text ingestion | Out of scope |
| Tokenizers (beyond opaque `TokenStreamRef`) | Out of scope |
| Pattern recognition / candidate/known patterns | Out of scope |
| Muscle-memory execution | Out of scope |
| Online AI providers / HTTP connectors | Out of scope |
| Long-term memory database / SQL / OwlQL | Out of scope (Phase 2+) |
| Autonomous actions / self-healing | Out of scope |
| Response alignment / NL response generation | Out of scope |
| Placeholder daemons for later phases | Not created |

---

## Service naming decision

| Candidate | Decision |
|-----------|----------|
| `sunlight-memoryd` | **Rejected** — confuses with kernel PMM/VMM, heap, ZRAM, and swap |
| **`wiseowl-memoryd`** | **Selected** — domain-specific, no collision with existing services |

Consistent naming:

- Process / nameserver: `wiseowl-memoryd`
- CLI: `wiseowl-memoryctl`
- Crate: `wiseowl-memory`
- Spill magic: `OWLS` (Owl Segment)
- KV namespace prefix: `owl.v1.shortterm`

No existing component named `sunlight-memoryd` or `wiseowl-memoryd` was found in the repository audit.

---

## Repository audit findings

Classifications of discovered foundations:

### Directly reusable

| Component | Role |
|-----------|------|
| `sunlight-kv` host protocol + client patterns (`PUT`/`GET`, SHM opcodes on SunlightOS) | Selective promotion backend |
| `sunlight-clipd` service shape (lib + daemon + CLI, nameserver register, bounded state, KV persistence) | Service structure reference |
| `lz4_flex` 0.11 (used by SIMG v2 / kernel ZRAM) | Cold segment compression |
| CRC32 IEEE (sunlight-kv style) | Spill / promotion integrity |
| `monotonic_millis` / `MonotonicMs` syscall | Active TTL lifetime (host injects ns clock for tests) |
| `ServiceCapability` bitmask pattern | Fine-grained memory rights mirror |
| SHM `shm_alloc` / `shm_map` / `shm_free` + `SHM_PAGE` | Large payload path on SunlightOS (documented; host uses inline frames) |
| sunlightd unit model (`RestartPolicy`, `capability_mask`, dependency graph) | Future supervision registration |
| Length-prefixed bincode UDS framing (sunlight-kv host) | Host IPC for daemon/CLI |
| `docs/ADDING_STATEFUL_SERVICES.md` | `/state/<service>` spill path when embedded |
| Host-testable pure engines (`sunlight-shell-appstate`, clipd lib tests) | Engine + contract testing model |

### Reusable with a small adapter

| Component | Adapter |
|-----------|---------|
| sunlight-kv SunlightOS IPC opcodes (`0x4B06` PUT_SHM, etc.) | `KvBackend` trait + `InMemoryKv` for tests; host daemon promotes via trait |
| `ServiceCapability` enum | Parallel `MemoryCapability` set until a broker mints domain rights |
| Kernel process name → FS actor mapping | Required only when embedding spill under `/state/wiseowl-memoryd` |
| Build scripts (`tools/build.sh`, ELF `include_bytes!`) | Optional future step to ship on-target daemon |

### Unsuitable for this task

| Component | Why |
|-----------|-----|
| Kernel PMM / VMM / heap | Physical memory, not cognitive memory |
| ZRAM page compressor | Fixed 4 KiB page pool; wrong granularity |
| sun-img SIMG containers | Image-specific headers/filters |
| Capability broker (dormant) | Not running; not required for service-local rights |

### Missing and required (implemented here)

| Item | Implementation |
|------|----------------|
| Cognitive memory contracts | `wiseowl-memory` Phase 0 modules |
| Bounded short-term engine | `MemoryService` |
| Sealed LZ4 cold segments | `segments` + `compression` + `spill` |
| Diagnostic CLI | `wiseowl-memoryctl` |
| Foundation documentation | this file |

### Future concern, out of scope

| Item |
|------|
| Long-term memory DB (Phase 2) |
| Embedding index |
| Document corpus ingestion |
| Online model providers |
| Kernel nameserver registration of `wiseowl-memoryd` in default boot graph |

**Principle:** do not reimplement LZ4, CRC32, KV storage, monotonic time, or UDS framing when suitable primitives exist.

---

## Architecture

```
                    ┌─────────────────────┐
                    │  wiseowl-memoryctl  │
                    │  (diagnostics only) │
                    └──────────┬──────────┘
                               │ UDS + bincode (host)
                               ▼
┌──────────────────────────────────────────────────────────┐
│                    wiseowl-memoryd                       │
│  ┌──────────────┐  ┌─────────────┐  ┌─────────────────┐  │
│  │ Working RAM  │  │  Hot RAM    │  │ Cold segments   │  │
│  │ (mutable)    │─▶│ (sealed)    │─▶│ LZ4 + checksum  │  │
│  └──────────────┘  └─────────────┘  └────────┬────────┘  │
│         │                │                     │         │
│         │                │              spill files      │
│         │                │              (atomic rename)  │
│         └────────────────┴──────────┬─────────┘          │
│                                     │ explicit PROMOTE   │
└─────────────────────────────────────┼────────────────────┘
                                      ▼
                              ┌───────────────┐
                              │  sunlight-kv  │
                              │ owl.v1.… keys │
                              └───────────────┘

Contracts crate: wiseowl-memory (ids, kinds, provenance, lifecycle, protocol)
```

---

## Memory classes

| Class | Mutability | Storage | Compression | Notes |
|-------|------------|---------|-------------|-------|
| **Working** | Mutable | RAM only | None | Request/operation scoped; auto TTL; dropped on client disconnect |
| **Hot** | Immutable after seal | RAM | None while active | LRU + importance eviction; optional SHM views later |
| **Cold** | Immutable | Spill / compressed cache | LZ4 once after seal | Checksummed; rehydratable; still short-lived |

Promotion into `sunlight-kv` is an **output operation**, not a fourth local class.

---

## Lifecycle diagrams

### Entry lifecycle

```
        create
          │
          ▼
       [Open] ──append──▶ [Open]
          │
        seal
          │
          ▼
      [Sealed] ──promote──▶ [Promoted]  (local may remain)
          │
     spill/compress
          │
          ▼
       [Cold] ──rehydrate/read──▶ payload restored (Sealed/Hot view)
          │
     delete / expire
          │
          ▼
   [Deleted] / [Expired]   (not returned by list/read)
```

Invalid transitions return structured errors (examples):

- sealed → append
- open → promote / spill
- expired → touch / promote
- deleted → promote

### Segment lifecycle

```
Open ──seal──▶ Sealed ──compress_once──▶ Compressed
                                            │
                                         spill
                                            ▼
                                         Spilled
                                            │
                                       rehydrate
                                            ▼
                                       Rehydrated
                                            │
                                    expire / delete
```

Only one owner mutates an open segment. Compression runs at most once; repeated reads do not recompress.

---

## Quota model

Hard limits (defaults target low-memory / 512 MiB class systems):

| Limit | Default |
|-------|---------|
| Total service RAM (working + hot) | 2 MiB |
| Total cold spill | 4 MiB |
| Max entry size | 64 KiB |
| Max segment size (uncompressed) | 256 KiB |
| Max entries | 512 |
| Max sessions | 32 |
| Per-session RAM | 256 KiB |
| Per-session cold | 512 KiB |
| Max provenance parents | 8 |
| Max list results | 64 |
| Max decompress allocation | 256 KiB |

Rules:

- Client-supplied sizes never drive unchecked allocations.
- Payload length is validated against the actual buffer and `max_entry_size`.
- Decompress path checks `uncompressed_size ≤ max_decompress` **before** allocation.
- All size/offset math uses checked arithmetic.

---

## Eviction policy

Deterministic order:

1. Expired working entries  
2. Expired hot entries  
3. Expired cold entries  
4. Low-importance sealed hot entries  
5. Least-recently-used sealed hot entries  
6. Low-importance cold segments  
7. Reject allocation if no safe candidate remains  

**Never evict:**

- entries with `pin_count > 0`
- open segments being written
- entries mid-promotion (local retained on KV failure)
- service metadata required for recovery

**Tie-break** (for tests): lower `importance`, then older `last_access_ns`, then lower `MemoryId`.

---

## Compression format

Cold spill blob:

```
Offset  Size  Field
0       4     magic "OWLS"
4       2     format version (1)
6       1     compression (1 = LZ4 block via lz4_flex)
7       1     reserved
8       8     segment_id (u64 LE, non-zero)
16      8     session_id
24      4     uncompressed_size
28      4     compressed_size
32      4     record_count
36      4     CRC32 (IEEE) of uncompressed plain
40      8     created_at_ns (monotonic domain)
48      8     expires_at_ns
56      …     compressed payload
```

Plain layout (before compression):

```
repeat: memory_id(u64 LE) | payload_len(u32 LE) | payload bytes
```

Validation before exposing data: magic, version, sizes, CRC32, record_count vs parsed IDs.

---

## KV promotion flow

```
caller PROMOTE_ENTRY
   │
   ├─ require PromoteToKv capability
   ├─ entry must be sealed/cold/promoted (not open/expired/deleted)
   ├─ build key: {namespace}.{session_id}.{memory_id}
   │     default namespace: owl.v1.shortterm
   ├─ encode versioned blob (provenance + checksum + payload)
   ├─ kv.put_if_absent(key, value)
   │     ├─ Unavailable → error; local record unchanged
   │     ├─ Written → success (new)
   │     └─ AlreadyPresent → success (idempotent)
   └─ delete_local_after only if write confirmed and requested
```

Idempotent retries never duplicate values. Failed promotion does not corrupt or remove the local record.

---

## Capability model

| Capability | Purpose |
|------------|---------|
| `Create` | Create/append entries, create sessions |
| `ReadOwnSession` | Read metadata for owned sessions |
| `ReadSharedSession` | Cross-session metadata |
| `Delete` | Delete entries |
| `InspectMetadata` | List / inspect headers |
| `InspectGlobalStats` | Service-wide counters |
| `PromoteToKv` | Explicit promotion |
| `RunMaintenance` | Bounded maintenance |
| `AdminQuota` | Administrative (reserved) |
| `ReadPayload` | Payload bytes (not granted by default diagnostics) |

Default clients cannot inspect other sessions. `inspect` prints metadata only unless `ReadPayload` is granted.

---

## IPC protocol versioning

- `PROTOCOL_VERSION = 1`
- Every request carries `protocol_version`; mismatch → `UnsupportedProtocolVersion`
- Host transport: `u32 LE length || bincode(Request/Response)` on Unix socket
- Env: `WISEOWL_MEMORY_SOCKET` (default `/tmp/sunlight/wiseowl-memory.sock`)
- Env: `WISEOWL_MEMORY_SPILL` (default `/tmp/sunlight/wiseowl-memory-spill`)

### Operations

| Operation | Purpose |
|-----------|---------|
| `CREATE_ENTRY` | Create working/hot entry |
| `APPEND_ENTRY` | Bounded mutation of open working entries |
| `READ_ENTRY` | Metadata (± payload with capability) |
| `TOUCH_ENTRY` | Refresh last_access |
| `SEAL_ENTRY` | Seal; optional working→hot |
| `DELETE_ENTRY` | Delete |
| `PROMOTE_ENTRY` | Explicit KV promotion |
| `LIST_ENTRIES` | Hard-capped metadata list |
| `GET_STATS` | Observability counters |
| `RUN_MAINTENANCE` | Incremental work-budget maintenance |
| `CREATE_SESSION` / `LIST_SESSIONS` | Session management |
| `REGISTER_CLIENT` / `CLIENT_DISCONNECT` | Ownership / cleanup |

---

## Restart and corruption behavior

**Shutdown:** stop mutations; complete/abort promotions deterministically; seal eligible segments when safe; flush approved cold metadata; leave no partial spill files (temp + rename).

**Restart:**

- reject incomplete `*.tmp` spill files (delete)
- validate headers + CRC; restore only valid non-expired cold segments
- do **not** resurrect RAM-only working entries
- one corrupt segment is quarantined (`quarantine-*`); service continues
- counters expose `quarantined_spill_records`

**TTL / clocks:**

- Active lifetime uses a **monotonic** clock (injected ns in host tests; `monotonic_millis` on target)
- Wall-clock is not used to extend live entries
- Restart recovery of cold records uses stored expiry fields; RAM working state is not restored

---

## Security considerations

- No raw process pointers in IDs or IPC
- No payload content in error messages or default diagnostics
- Session isolation by default
- Untrusted remote provenance marked `RemoteUnverified` / `Untrusted`
- Parent provenance chain preserved on derive (bounded)
- Attacker-controlled uncompressed sizes rejected before allocation
- SHM validation failures counted (path reserved for SunlightOS large payloads)

---

## Test evidence

Host target (`x86_64-unknown-linux-gnu`):

```
cargo test -p wiseowl-memory --lib --target $(rustc -vV | sed -n 's/^host: //p')
```

**Result (this delivery):** `44 passed; 0 failed`

Coverage includes:

- identifier parsing; importance/confidence bounds
- lifecycle transitions; segment layout checked math
- quota accounting; TTL; eviction ordering; provenance parent bounds
- LZ4 round-trip; corrupt compressed data; checksum mismatch; oversized decompress header
- malformed/unsupported protocol versions
- create/read/seal/delete; working→hot; hot→cold spill + rehydrate
- session isolation; pinned protection; KV unavailable + retry; idempotent promote
- restart with corrupt segment; maintenance work budget; soak accounting stability

**Not measured in this delivery:** idle CPU % or RSS on a live SunlightOS boot (host tests only). Soak test verifies accounting stays within quotas and maintenance returns without busy-loop.

---

## Known limitations

1. SunlightOS nameserver registration and kernel ELF embedding are **not** wired into the default boot graph yet; host UDS daemon proves the engine and protocol.
2. SHM large-payload path is specified for target IPC; host uses inline frame payloads with a 1 MiB frame cap.
3. Fine-grained `MemoryCapability` is service-local (no running capability broker).
4. Cold rehydrate after spill keeps compressed bytes in memory when spill is optional; production may drop RAM compressed copies after disk write under tighter budgets.
5. Working entries are not journaled across process crash (by design).

---

## Phase 2 boundary

Phase 2 may introduce a long-term memory database. It **must**:

- consume Phase 0 contracts (`MemoryId`, provenance, kinds, promotion records)
- treat short-term service as a source of sealed/promoted records
- **not** move long-term DB logic into `wiseowl-memoryd`
- **not** replace Phase 0/1 wire formats without a versioned migration

Phase 1 remains the sole short-term RAM + cold spill authority.

---

## CLI reference

```text
wiseowl-memoryctl status
wiseowl-memoryctl stats
wiseowl-memoryctl sessions
wiseowl-memoryctl list --session <id>
wiseowl-memoryctl inspect <memory-id>   # metadata only by default
wiseowl-memoryctl maintenance
```

---

## Build

```sh
# Library tests (host)
cargo test -p wiseowl-memory --lib --target $(rustc -vV | sed -n 's/^host: //p')

# Host daemon + CLI
cargo build -p wiseowl-memory --bins --target $(rustc -vV | sed -n 's/^host: //p')
```

Workspace default target is bare-metal (`x86_64-unknown-none`); always pass the host triple for this crate until a `sunlightos` feature binary is embedded like `sunlight-kv`.
