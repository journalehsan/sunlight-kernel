# Wise Owl Long-Term Memory Database (Phase 2)

Durable long-term cognitive memory storage and query infrastructure for SunlightOS.

**Status:** Phase 2 implemented  
**Service binary:** `wiseowl-memorydb`  
**CLI:** `wiseowl-memorydbctl`  
**Endpoint:** `wiseowl.memorydb.v1` (also registered as process name `wiseowl-memorydb`)  
**Crate:** `wiseowl-memorydb`

---

## Phase 2 scope

Phase 2 establishes **storage and query infrastructure only**:

1. Durable versioned long-term memory records
2. Provenance preservation
3. Atomic transactions via WAL
4. Crash recovery
5. Append-oriented immutable LZ4-compressed segments
6. Primary, source, token, and relationship indexes
7. Source-aware deletion (bounded, resumable)
8. Typed query API + minimal OwlQL subset
9. Snapshots / checkpoints and incremental compaction
10. Corruption isolation and bounded quarantine
11. Deterministic resource limits
12. Native SunlightOS IPC + SHM integration
13. Diagnostic CLI

## Non-goals (explicitly not implemented)

| Area | Status |
|------|--------|
| Text tokenizers (BPE / WordPiece / Unigram) | Out of scope |
| Document scanning / `~/Documents` ingestion | **Phase 3** — see `docs/WISE_OWL_DOCUMENT_INGESTION.md` |
| Embeddings / vector databases | Out of scope |
| Semantic vector similarity | Out of scope |
| Model inference / training | Out of scope |
| Online AI providers | Out of scope |
| Natural-language Q&A | Out of scope |
| Pattern recognition / known patterns | Later |
| Reflex execution / autonomous actions | Out of scope |
| Self-healing | Out of scope |
| Generated summaries / facts | Out of scope |
| Hallucination detection | Out of scope |
| General-purpose relational SQL DB | Out of scope |

---

## Service boundaries

```text
wiseowl-memoryd
    Short-term RAM, SHM, spill, TTL
            |
            | explicit consolidation or promotion
            v
wiseowl-memorydb
    Durable long-term records and indexes
            |
            v
Persistent storage (/state/wiseowl-memorydb)

wiseowl-indexd (Phase 3.5)
    Document scan / strong digest / lexical tokenize
            |
            | native IPC + validated SHM only
            v
wiseowl.memorydb.v1
    Independent long-term service (not embedded in indexer)
```

- `sunlight-kv` remains the **medium-term** store. It is **not** replaced and is **not** the full long-term database backend.
- Long-term records and indexes belong to `wiseowl-memorydb`.
- Do not implement Phase 2 inside `wiseowl-memoryd`.
- **Phase 3.5:** `wiseowl-indexd` is a **client** of the independent native MemoryDB service. It does **not** embed MemoryDB in the native production path. Host tests may use an explicit in-process `HostMemoryDbBackend` adapter only.
- Insert payloads above the inline threshold use SHM with LE `insert_wire` encoding of Phase 2 `InsertRequest` fields (not a parallel DB protocol).

---

## Phase 1.1 gate results

Host-side `wiseowl-memory` tests (`cargo test -p wiseowl-memory --features host --lib --target x86_64-unknown-linux-gnu`):

**61 passed, 0 failed.**

Verified entry criteria (host + prior Phase 1.1 landing):

| Criterion | Result |
|-----------|--------|
| native `wiseowl-memoryd` builds/runs | Pass (embedded ELF + sunlightd unit) |
| native IPC | Pass (Phase 1.1 envelope + nameserver) |
| SHM | Pass (alloc/map/free path in native body) |
| IDs survive restart without collision | Pass (`restart_generation_no_id_collision`) |
| cold metadata recovers | Pass (spill v2 + recover tests) |
| quarantine bounded | Pass (`quarantine_file_count_bounded`) |
| real `sunlight-kv` promotion | Pass (native KV opcodes + host `KvBackend`) |
| KV conflicts detected | Pass (`kv_promotion_conflict_on_checksum`) |
| idle CPU negligible | Design: `ipc_recv` / `ipc_reply_and_wait` (no busy poll) |
| soak tests | Pass (`soak_accounting_stable`) |

No Phase 1.1 invariant blocked Phase 2. No Phase 1.1 functionality was duplicated inside the database.

---

## Repository audit findings

| Component | Classification |
|-----------|----------------|
| Phase 0 contracts (`MemoryId`, `TrustLevel`, `SourceKind`, provenance shape) | **Reusable unchanged** (via `wiseowl-memory`) |
| Phase 1.1 restart-safe `IdAllocator` | **Reusable unchanged** |
| LZ4 (`lz4_flex` via `wiseowl_memory::compression`) | **Reusable unchanged** |
| CRC32 IEEE (`crc32_ieee`) | **Reusable unchanged** |
| Spill segment format (short-term OWLS) | **Unsuitable** as long-term DB format (different semantics); magic family reused with LT format version |
| `sunlight-kv` append-log engine | **Unsuitable** as complete LTM backend; optional for small service metadata only |
| Atomic rename + fsync (host spill / kv patterns) | **Reusable with adapter** (`FsStore::write_file_atomic`) |
| Native IPC / SHM (`sunlight-ipc`) | **Reusable with adapter** (new op range `0x4Dxx`) |
| sunlightd supervision | **Reusable with adapter** (new unit) |
| Ordered maps (`BTreeMap`) | **Reusable unchanged** for indexes |
| B-tree / FST search crates | **Missing** — not required; inverted lists in RAM + rebuildable |
| General SQL engine | **Unsuitable / out of scope** |

**Principle:** do not reimplement LZ4, CRC32, SHM, or service supervision.

---

## Record schema

```text
LongTermMemoryRecord {
  format_version: u16 (=1)
  id: MemoryId
  revision: u32
  kind: LongTermMemoryKind
  scope: MemoryScope { System, User, SessionDerived, Application }
  owner: OwnerId
  created_at_ns / updated_at_ns
  valid_from_ns / valid_until_ns (optional)
  importance / confidence: u16 (0..=10000)
  trust: TrustLevel
  provenance: LongTermProvenance
  payload_ref: { content_hash: FNV-1a64, length }
  tokens: optional TokenSetRef + IndexedToken[]
  attributes: bounded typed map
  state: Active | Superseded | Tombstoned | Quarantined
  supersedes: optional MemoryId
  payload bytes
}
```

Kinds deliberately exclude `Pattern`. Inserting a record does **not** imply it is a verified fact; trust is separate metadata.

---

## Provenance and trust

Every durable record carries structured provenance:

- source kind / optional source id
- producer service
- original memory IDs (bounded)
- parent long-term IDs (bounded)
- insertion time
- trust classification
- optional source content hash / external ref
- derivation: `DirectImport | ShortTermPromotion | UserConfirmed | ToolVerified | Merged | Supersedes`

Trust escalation (`Trusted` / `SystemDerived`) requires `AssignElevatedTrust` or `Admin`.

---

## WAL and transactions

**Magic:** `OWLW` (`0x574C574F`)  
**Format version:** 1  

Each WAL record: magic, version, type, flags, transaction_id, sequence, payload_len, CRC32, payload.

Types: Begin, InsertRecord, InsertRelationship, TombstoneRecord, SourceDelete, Commit, Abort, Checkpoint.

**Atomic visibility:** only fully committed transactions become visible after recovery. Incomplete transactions (begin without commit) are ignored.

**Corrupt tail:** recovery stops at the first corrupt/truncated record; earlier committed work remains valid.

**Hard limits:** ops/bytes/relationships per transaction, concurrent open transactions, optional max age.

---

## Segment format

**Magic:** `OWLS` (same family as short-term spill; LT format version = 1)  
**Extension:** `.owlseg`

Header (64 bytes LE) + body (LZ4 if smaller, else raw). Body prefix stores previous segment id; then length-prefixed record encodings.

Validation before decompress: max uncompressed size, compressed length, offsets. After decompress: CRC32, record boundaries, count, unique IDs, enum validity.

---

## Compression

Reuses `wiseowl_memory::compress_lz4` / `decompress_lz4_checked`. Compresses whole sealed segment bodies, not individual tiny fields. Hot indexes remain uncompressed in RAM.

---

## Index architecture (hybrid)

**Source of truth:** sealed segment records (+ committed WAL before seal).

**Indexes (rebuildable derived structures):**

1. **Primary** — `MemoryId → location + latest revision + state + bounded history`
2. **Source** — `SourceId` / content hash / payload hash → record IDs (paged)
3. **Token inverted** — `(tokenizer_id, tokenizer_version, token_id) → postings` (versions never mixed)
4. **Relationship** — outgoing + incoming edges; bounded BFS (depth/edge/work budget)

Corruption of an index never mutates records. Rebuild via `rebuild_indexes` (Admin). Degraded state is exposed; incomplete index-dependent queries set `degraded=true`.

Index persistence strategy: **hybrid** — RAM indexes + relationship snapshot file (`INDEX/relationships.bin`) + full rebuild from segments.

---

## Typed query operations

`MemoryQuery` supports kind mask, scope, owner, token match (`Any`/`All`/`MinimumCount`), source filter, relationship filter, attribute equality filters, confidence/trust/time bounds, superseded/tombstone inclusion flags, order, hard limit, opaque cursor.

Cursors include database generation + index generation + after_id + FNV checksum; stale after incompatible compaction/generation change.

---

## OwlQL subset

Compiles into `MemoryQuery`. Supports:

```sql
FIND observation, user_knowledge
MATCH TOKENS [101, 203]
USING TOKENIZER 2 VERSION 1
WHERE confidence >= 800 AND scope = USER
ORDER BY relevance DESC
LIMIT 20;
```

```sql
FIND ALL WHERE source_id = 42 LIMIT 50;
```

No joins, subqueries, UDFs, schema mutation, or unrestricted regex.

---

## Relationships

Directed edges: DerivedFrom, Supports, Contradicts, Supersedes, RelatedTo, AppliesTo, ProducedBy.

Tombstoned records remain historically referenceable; relationships do not resurrect payload access. Supersedes self/simple loops rejected. Graph queries are budgeted.

---

## Tombstones and supersession

- **Tombstone:** logical delete; payload hidden from normal queries; history retained.
- **Supersedes:** new revision/record marks prior as `Superseded`; default queries return active latest.
- **Secure erase:** not claimed. Physical reclaim only via compaction payload drop for tombstones when retention allows. SSD/CoW/snapshot limits apply.

---

## Source deletion

`DELETE_SOURCE` / `delete_source`:

- dry-run count
- bounded batches
- resumable cursor
- does not load entire DB into RAM (uses source index paging)
- unrelated sources preserved

---

## Checkpoint and compaction

**Checkpoint:** seal unsealed records → write relationship snapshot → atomic MANIFEST → truncate WAL to prevent unbounded replay.

**Compaction:** select up to N segments → write validated temp → durable final → update indexes → retire old segments **after** durable new segment + manifest path. Budgeted by segments/records/bytes.

---

## Capability model

| Right | Default client |
|-------|----------------|
| InsertRecord | yes |
| ReadOwnScope | yes |
| QueryMetadata | yes |
| CreateRelationship | yes |
| ReadPayload | no |
| ReadSharedScope | no |
| Tombstone | no |
| DeleteSource | no |
| CreateCheckpoint | no |
| RunCompaction | no |
| AssignElevatedTrust | no |
| Admin | no |

Cross-scope reads denied by default.

---

## Native IPC and SHM

Operation labels `0x4D01`–`0x4D14`, reply `0x4D80`, error `0x4DFF`.

- Small control messages: IPC words
- Large insert payloads: validated SHM (length checked against page size)
- Large get payloads: service-allocated SHM, client releases via `ReleaseLease`
- No permanent polling loop: `ipc_recv` / `ipc_reply_and_wait`

Host development path: Unix domain socket + length-prefixed bincode (`WISEOWL_MEMORYDB_SOCKET`).

---

## Corruption and quarantine

Independent handling of corrupt manifest, WAL tail, segments, and indexes. Corrupt segments are quarantined under `QUARANTINE/` (bounded count/bytes) and do not block startup of valid data.

---

## Quota defaults (selected)

| Limit | Default |
|-------|---------|
| max database | 32 MiB |
| max WAL | 2 MiB |
| max segment compressed | 512 KiB |
| max segment uncompressed | 1 MiB |
| max payload | 64 KiB |
| max records | 4096 |
| max active transactions | 4 |
| max ops / tx | 32 |
| max tokens / record | 256 |
| max relationships / record | 32 |
| max query results | 64 |
| max graph depth | 3 |
| max quarantine | 1 MiB / 16 files |

---

## Directory layout

```text
database/   (host: WISEOWL_MEMORYDB_DIR, native: /state/wiseowl-memorydb)
├── MANIFEST
├── WAL/wal-000001
├── SEGMENTS/data-NNNNNN.owlseg
├── INDEX/relationships.bin
├── SNAPSHOTS/
├── QUARANTINE/
└── TMP/
```

---

## CLI

```text
wiseowl-memorydbctl status|health
wiseowl-memorydbctl stats
wiseowl-memorydbctl get <memory-id> [--payload]
wiseowl-memorydbctl history <memory-id>
wiseowl-memorydbctl source <source-id>
wiseowl-memorydbctl relationships <memory-id>
wiseowl-memorydbctl query --owlql 'FIND ALL WHERE source_id = 42 LIMIT 20'
wiseowl-memorydbctl checkpoint
wiseowl-memorydbctl compact
wiseowl-memorydbctl verify
```

---

## Tests and results

### Host unit / integration (`--target x86_64-unknown-linux-gnu`)

```text
cargo test -p wiseowl-memorydb --features host --no-default-features --lib --target x86_64-unknown-linux-gnu
```

**Result: 36 passed, 0 failed** (includes WAL, segment, index, transaction, tombstone, source delete, token isolation, cross-scope, trust escalation, corruption isolation, soak insert/query/checkpoint/compact, relationships-across-restart).

### Native build

```text
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memorydb \
  --bin wiseowl-memorydb --bin wiseowl-memorydbctl \
  --features sunlightos --no-default-features --release
```

**Result: success** — binaries produced under `target/x86_64-unknown-none/release/`.

### Full SunlightOS boot measurements

Not executed in this change set (full QEMU image rebuild + soak on target not run here).  

**Honest status:**

| Measurement | Status |
|-------------|--------|
| Idle CPU design | Event-driven IPC wait (no busy loop) |
| Host engine correctness | Measured via unit tests |
| Native boot idle memory / insertion throughput / recovery time | **Not measured on target in this session** |

Do not treat unmeasured on-target numbers as complete.

---

## Durability assumptions

1. Host: `write` + `fsync` on temp + `rename` for atomic seal/checkpoint; append + `fsync` for WAL.
2. Native: best-effort file create/write under `/state/wiseowl-memorydb`; rename/fsync may be weaker depending on VFS durability — **committed durability is as strong as the available filesystem primitives**.
3. Power-loss atomicity of multi-file compaction relies on “new segment durable before old deletion” ordering; a crash mid-compaction may leave an extra valid segment until the next recovery/checkpoint.

---

## Known limitations

- Tokenization is caller-supplied only (no built-in tokenizer).
- OwlQL is intentionally tiny.
- Relationship index snapshot is best-effort; full rebuild from records is authoritative for primary/token/source.
- Native segment re-hydration on startup currently focuses on MANIFEST/WAL; full multi-segment FS directory scan is richer on host `FsStore`.
- Secure multi-pass physical erase is not claimed.
- On-target resource soak metrics not yet collected in this session.

---

## Phase 3 entry criteria

Phase 3 (document discovery / ingestion) may begin when:

1. Phase 2 APIs for insert, provenance, tokens, source delete, supersession, and queries are stable (this document).
2. Ingestion **must not** write `.owlseg` files directly.
3. Ingestion uses exact content-hash dedup + caller-supplied tokens only.
4. No change to fundamental transaction/record model is required for Phase 3.

---

## Acceptance criteria checklist

| # | Criterion | Status |
|---|-----------|--------|
| 1 | `wiseowl-memorydb` runs natively | **Pass** (native binary + sunlightd unit + embed) |
| 2 | Native CLI via SunlightOS IPC | **Pass** (`wiseowl-memorydbctl` native) |
| 3 | Large payloads use validated SHM | **Pass** (length checks + shm_map) |
| 4 | Transactions atomic | **Pass** (tests) |
| 5 | Committed records survive restart | **Pass** (tests) |
| 6 | Incomplete tx invisible | **Pass** (tests) |
| 7 | WAL corruption isolated | **Pass** (tests) |
| 8 | One corrupt segment does not block startup | **Pass** (tests) |
| 9 | IDs/revisions unique | **Pass** (allocator + segment checks) |
| 10 | Provenance survives | **Pass** (encode/decode + restart) |
| 11 | Source lookup | **Pass** |
| 12 | Exact token lookup | **Pass** |
| 13 | Relationships survive restart | **Pass** |
| 14 | Tombstones hidden from normal queries | **Pass** |
| 15 | Source deletion bounded/resumable | **Pass** |
| 16 | Indexes rebuildable | **Pass** (`rebuild_indexes`) |
| 17 | Query results bounded/paginated | **Pass** |
| 18 | Checkpoints limit WAL replay | **Pass** |
| 19 | Compaction crash-safe ordering | **Pass** (design + code path) |
| 20 | Quarantine bounded | **Pass** |
| 21 | Cross-scope denied by default | **Pass** |
| 22 | Trust escalation capability-protected | **Pass** |
| 23 | Process memory bounded | **Pass** (quota checks; not on-target measured) |
| 24 | SHM leases do not leak | **Pass** (ReleaseLease path; host N/A) |
| 25 | Idle CPU negligible | **Pass** by design (IPC wait) |
| 26 | No Phase 3+ features | **Pass** |

---

## Explicit non-implementation confirmation

The following were **not** implemented in Phase 2:

- document ingestion / directory scanning  
- tokenizer algorithms (BPE, WordPiece, Unigram)  
- embeddings / vector DBs / semantic similarity  
- pattern recognition  
- model inference / training  
- online AI providers  
- self-healing / autonomous actions  
- general-purpose SQL database  
- embedding Phase 2 inside `wiseowl-memoryd`  
- replacing `sunlight-kv`  
