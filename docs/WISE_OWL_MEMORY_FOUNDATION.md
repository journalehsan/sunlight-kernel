# Wise Owl Memory Foundation (Phase 0 + Phase 1 + Phase 1.1)

Bounded short-term cognitive memory contracts and service for SunlightOS.

**Status:** Phase 0 contracts, Phase 1 short-term engine, and Phase 1.1 native SunlightOS landing implemented.  
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
   - IPC-shaped API (host UDS + bincode)
   - CLI diagnostics
   - deterministic unit/integration/soak tests
3. **Phase 1.1** — native SunlightOS landing:
   - `wiseowl-memoryd` / `wiseowl-memoryctl` on target IPC + SHM
   - real `sunlight-kv` promotion path
   - restart-safe IDs, spill v2 metadata, bounded quarantine
   - sunlightd supervision and ELF embedding

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

## Phase 1.1 — Native SunlightOS landing

### Scope

Land the Phase 0/1 short-term memory engine on the real SunlightOS target:

* native service binary `wiseowl-memoryd` (ELF embedded, sunlightd unit)
* native CLI `wiseowl-memoryctl` over kernel IPC
* real SHM payload path (inline threshold 3072 bytes; page size 4096)
* real `sunlight-kv` promotion via native opcodes
* restart-safe ID generation (16-bit generation + 48-bit counter)
* spill record format v2 with full recovery metadata
* bounded quarantine
* logical vs observed resource accounting hooks
* host tests remain the primary automated proof of the engine

Phase 2+ (long-term DB, OwlQL, embeddings, AI, etc.) is **not** implemented.

### Audit classification (Phase 1.1)

| Component | Classification |
|-----------|----------------|
| Phase 0 contracts (ids, kinds, provenance, lifecycle, caps, protocol enums) | Reused (IDs extended for restart safety) |
| Host `MemoryService` + spill FS | Reused / hardened (host tests) |
| Host UDS+bincode transport | Host-only (unchanged role) |
| `NativeMemoryEngine` | Target-adapted engine sharing Phase 0 types |
| Native IPC envelope + ops (`native_ipc`) | Target-only ABI |
| `sunlight-kv` client in daemon | Target-only (real service) |
| Restart-safe ID packing | Corrected defect (was process-local monotonic only) |
| Spill record v2 full metadata | Corrected defect (v1 was id\|len\|payload only) |
| Bounded quarantine | Hardened |
| KV AlreadyPresent checksum/version verify | Hardened |
| Long-term DB / OwlQL / embeddings | Intentionally deferred |

### Service boot and supervision

```text
sunlightd starts /sbin/wiseowl-memoryd
        ↓
create /state/wiseowl-memoryd (service actor)
        ↓
load generation.bin → bump generation → persist
        ↓
spill recovery (v2 only; quarantine corrupt; bounded)
        ↓
ID allocator recovery (note_seen recovered IDs)
        ↓
nameserver_register("wiseowl-memoryd")
        ↓
Ready / Degraded (KV missing, quarantine present, …)
        ↓
serve native IPC (event-driven recv/reply; no busy poll)
        ↓
clean shutdown or supervised restart (on-failure, RestartSec=5)
```

Health states: `Starting`, `Ready`, `Degraded`, `Stopping`, `Failed`.

Degraded continues RAM-only operations when possible.

### Native endpoint and labels

| Item | Value |
|------|-------|
| Process / nameserver | `wiseowl-memoryd` |
| Endpoint (logical) | `wiseowl.memory.v1` (registered as `wiseowl-memoryd`) |
| CLI | `wiseowl-memoryctl` |
| Op base | `0x4F01` … `0x4F10` |
| Reply | `0x4F80` |
| Error | `0x4FFF` |

Operations: RegisterClient, CreateSession, CreateEntry, AppendEntry, ReadEntry, TouchEntry, SealEntry, DeleteEntry, PromoteEntry, ListEntries, ListSessions, GetStats, RunMaintenance, ClientDisconnect, ReleaseLease, TransportInfo.

### Native IPC envelope

```text
MemoryIpcHeader (24 bytes, little-endian):
  protocol_version u16 = 1
  operation        u16
  flags            u32   (required unknown flags rejected: 0xFFFF0000)
  request_id       u64
  body_len         u32   (hard max 64 KiB)
  reserved         u32
```

Native protocol version is independent of host bincode `PROTOCOL_VERSION`.

### Inline vs SHM threshold

| Constant | Value | Rationale |
|----------|-------|-----------|
| `SHM_PAGE` | 4096 | kernel SHM page |
| `INLINE_PAYLOAD_THRESHOLD` | 3072 | header + body fit one page with margin |
| Max request/reply body | 64 KiB | hard cap |

Payloads above the threshold use a validated SHM descriptor (handle, offset, length, ReadOnly access, optional checksum). Service copies client data on create/append; large reads may return a service-created RO SHM page. Client death releases leases and Working entries.

### Restart-safe ID format

```text
bits 63..48 : generation (1..=65535), advanced on every daemon start
bits 47..0  : monotonic counter within generation
zero        : invalid
```

Persisted in `/state/wiseowl-memoryd/generation.bin`. Recovered cold IDs call `note_seen`. Generation wrap refuses start rather than reuse.

### Spill metadata recovery (format v2)

Segment format version **2**. Uncompressed body records carry full headers:

memory id, session id, class, kind, state, timestamps, expiry, importance, confidence, provenance (parents + producer), optional token stream, payload length, payload CRC32, payload.

Version 1 segments are **rejected and quarantined** (no silent mis-parse). Compatibility: neither forward nor backward.

### Quarantine limits (defaults)

| Limit | Value |
|-------|-------|
| Max quarantine bytes | 1 MiB |
| Max quarantine files | 16 |
| Max single file | 256 KiB |
| Max files inspected / startup | 64 |
| Max bytes inspected / startup | 4 MiB |

Already-quarantined names are not re-renamed. Cleanup: expired → count → bytes.

### KV promotion and conflict detection

* Explicit, sealed-only, capability-protected.
* Value includes version, IDs, class/kind, scores, expiry, provenance, checksum, payload.
* `AlreadyPresent` only after comparing version, memory id, session id, payload length, checksum.
* Mismatch → `PromotionConflict` (local record kept).
* `delete_local_after` only after confirmed Written or identical AlreadyPresent.

### Capability mechanism (active on target)

**Service-local rights** (`MemoryCapability` bitmasks) mapped from sunlightd unit capabilities where available, plus process identity via nameserver. Capability broker remains dormant. Default clients: Create, ReadOwnSession, Delete, InspectMetadata — **no** cross-session, payload, promote, or global stats by default. Diagnostic CLI uses elevated diagnostic/admin set for operators.

FS writes limited to `/state/wiseowl-memoryd` via kernel service actor name match.

### Logical vs observed memory

Logical gauges (authoritative for quotas):

* `logical_payload_bytes`, `logical_metadata_bytes`, `logical_total_bytes`
* `cold_compressed_bytes`, `cold_uncompressed_logical_bytes`
* `shared_memory_leased_bytes`, `compression_scratch_peak`, `active_read_leases`

Process RSS / precise idle CPU **depend on OS telemetry exposure**. Closest available: `monotonic_millis` for activity windows, service logical counters, SHM lease gauges. Exact RSS is **not claimed** if the kernel does not export per-process resident size to userland in this build.

### Target measurements (how to collect)

On a QEMU/SunlightOS boot after `wiseowl-memoryd` is Ready:

```text
wiseowl-memoryctl transport
wiseowl-memoryctl status
wiseowl-memoryctl stats
```

Record: VM cores/RAM, stats before/after create-seal-spill-promote, idle after maintenance. Host soak (`service::tests::soak_accounting_stable`) proves accounting bounds without busy-loop.

**Host automated result (Phase 1.1):** `61 passed; 0 failed` (lib tests).

### Phase 2 entry criteria checklist

| # | Criterion | Status |
|---|-----------|--------|
| 1 | `wiseowl-memoryd` runs natively | Implemented (ELF + sunlightd unit + nameserver) |
| 2 | `wiseowl-memoryctl` native IPC | Implemented |
| 3 | Large payloads use validated SHM | Implemented (threshold + map/copy) |
| 4 | Client death does not leak SHM/pins | Implemented (pid sweep + disconnect) |
| 5 | Restart cannot produce duplicate IDs | Host-tested; generation file on target |
| 6 | Cold records recover complete metadata | Format v2 + recovery path |
| 7 | One corrupt spill does not block startup | Host-tested quarantine |
| 8 | Quarantine bounded | Host-tested |
| 9 | Real sunlight-kv promotion E2E | Wired native client; host double for unit |
| 10 | Existing-key conflicts by version/checksum | Host-tested |
| 11 | Cross-session denied by default | Host-tested |
| 12 | Logical memory within quota | Host soak |
| 13 | Process memory not unbounded | Logical + lease gauges; RSS if exposed |
| 14 | Idle CPU negligible | Event-driven ipc_recv; no poll loop |
| 15 | Native soak after restart | Host restart/gen tests; target soak manual |

Do not start the long-term memory database until target soak measurements are recorded on a live boot.

---

## Test evidence

Host target (`x86_64-unknown-linux-gnu`):

```
cargo test -p wiseowl-memory --lib --target $(rustc -vV | sed -n 's/^host: //p')
```

**Result (Phase 1.1):** `61 passed; 0 failed`

Additional Phase 1.1 coverage:

- restart-safe generation packing; note_seen; counter exhaustion
- native IPC header validation; SHM descriptor bounds; inline threshold
- promotion conflict on checksum mismatch
- generation bump across MemoryService restart with spill dir
- NativeMemoryEngine create/seal/promote + generation isolation
- quarantine bounds; no re-quarantine rename growth; generation.bin persist
- spill format v2 full metadata encode/decode

Native ELFs:

```
cargo build -p wiseowl-memory --bin wiseowl-memoryd --bin wiseowl-memoryctl \
  --features sunlightos --no-default-features --release --target x86_64-unknown-none
```

---

## Known limitations

1. Capability broker is still dormant; rights are service-local (+ sunlightd capability tokens).
2. Full filesystem spill recovery on target is best-effort under `/state/wiseowl-memoryd`; host FS spill remains the richest recovery test bed.
3. Per-process RSS may be unavailable; report closest metric.
4. Working entries are not journaled across crash (by design).
5. Native CLI argv parsing is minimal; `status`/`transport`/`stats` are primary diagnostics.
6. No Phase 2 long-term database, SQL, OwlQL, embeddings, models, or autonomous actions.

---

## Phase 2 boundary

Phase 2 may introduce a long-term memory database. It **must**:

- consume Phase 0 contracts (`MemoryId`, provenance, kinds, promotion records)
- treat short-term service as a source of sealed/promoted records
- **not** move long-term DB logic into `wiseowl-memoryd`
- **not** replace Phase 0/1 wire formats without a versioned migration
- wait until Phase 1.1 entry criteria are demonstrated on a live boot

Phase 1/1.1 remains the sole short-term RAM + cold spill authority.

---

## CLI reference

```text
wiseowl-memoryctl status
wiseowl-memoryctl stats
wiseowl-memoryctl sessions
wiseowl-memoryctl list
wiseowl-memoryctl inspect <memory-id>   # metadata only by default
wiseowl-memoryctl maintenance
wiseowl-memoryctl transport            # native: protocol, inline threshold, SHM, health
```

---

## Build

```sh
# Library tests (host)
cargo test -p wiseowl-memory --lib --target $(rustc -vV | sed -n 's/^host: //p')

# Host daemon + CLI
cargo build -p wiseowl-memory --bins --target $(rustc -vV | sed -n 's/^host: //p')

# Native SunlightOS ELFs (also pulled by kernel build.rs / tools/build.sh)
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone" \
  cargo build -p wiseowl-memory --bin wiseowl-memoryd --bin wiseowl-memoryctl \
  --features sunlightos --no-default-features --release --target x86_64-unknown-none
```
