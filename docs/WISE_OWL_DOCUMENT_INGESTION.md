# Wise Owl Document Ingestion (Phase 3 / 3.5)

Secure incremental document indexing and deterministic **lexical** retrieval tokenization for SunlightOS.

**Status:** Phase 3.5 implemented  
**Service binary:** `wiseowl-indexd`  
**CLI:** `wiseowl-indexctl`  
**Endpoint:** `wiseowl.index.v1` (also process name `wiseowl-indexd`)  
**Crate:** `wiseowl-index`

---

## Phase 3.5 scope (current)

Phase 3.5 closes the two architectural gaps left by Phase 3:

1. **Strong content identity** — SHA-256 versioned digests replace FNV-1a64 as final content proof.
2. **Service separation** — native `wiseowl-indexd` talks only to independent `wiseowl.memorydb.v1` (no embedded MemoryDB on target).

Also: manifest v2 migration, import idempotency, uncertain-commit reconciliation, MemoryDB reconnect/degraded health, native SHM insert wire, target diagnostics.

### Native production architecture

```text
wiseowl-indexctl
       |
       | native IPC
       v
wiseowl-indexd
       |
       | native IPC + validated SHM
       v
wiseowl-memorydb   (wiseowl.memorydb.v1)
       |
       v
durable long-term database
```

**Forbidden on target:** in-process / embedded MemoryDB bootstrap inside `wiseowl-indexd`.

Host tests use explicit `HostMemoryDbBackend` (in-process `Database`). Native production uses `NativeMemoryDbClient` only. Shared engine: parsers, chunker, tokenizer, scan, ingest.

### Strong digest selection

| Candidate | In repo? | Decision |
|-----------|----------|----------|
| BLAKE3 | No | Not present |
| SHA-256 | Yes (`sunlight-bench` soft impl) | **Selected** — streaming, no_std, 256-bit, no new deps |
| FNV-1a64 | Yes | Prefilter / path / token IDs only |

**Format:** `ContentDigest { algorithm, version=1, bytes[32] }` with LE 35-byte encode (`alg:u8 | ver:u16 | bytes`).

**Streaming:** `ContentDigestHasher` / `Sha256Hasher` — bounded 64-byte block buffer; meta before/after must match or digest discarded.

### Fast fingerprint vs content identity

| | Fast fingerprint (FNV-1a64) | Strong digest (SHA-256) |
|--|----------------------------|-------------------------|
| Role | Prefilter, path buckets, optional cache hint | **Final** content identity |
| May skip reparse alone? | **No** | Yes, when pipeline matches |
| Token IDs | FNV + collision dictionary (unchanged) | N/A |

Content identity and token identity are separate concerns. Token IDs remain FNV with collision verification.

### Manifest migration (v1 → v2)

| Version | Content identity |
|---------|------------------|
| v1 (Phase 3) | FNV final hash only → `legacy_content_hash`, `needs_digest_upgrade=true` |
| v2 (Phase 3.5) | `content_digest` + optional `fast_fingerprint` |

- v1 never interpreted as strong digest.
- Upgrade rehashes; if legacy FNV matches and pipeline matches, **preserve SourceId and document revision**, update digest metadata only.
- Content change → normal atomic re-index.
- Migration bounded/resumable; atomic manifest writes.

### Import idempotency & uncertain commits

`ImportKey` = SourceId + revision + strong digest + parser/tokenizer/chunker versions + scope + owner.

Crash window: MemoryDB commit succeeds, indexer dies before manifest update.

On restart: load pending import → `reconcile_import(ImportKey)` → Committed (update manifest) / NotFound|Aborted (retry) / InProgress (bounded wait) / Conflict (fail source).

Pending import metadata is persisted **before** begin_import.

### MemoryDB outage behavior

- Indexer starts, discovers MemoryDB, probes health.
- Unavailable → `Degraded: MemoryDbUnavailable` (control plane still serves).
- **No** embedded fallback store.
- Bounded reconnect with stepped backoff (no busy loop).

### SHM ownership (indexer → MemoryDB inserts)

1. Indexer allocates payload SHM, fills insert wire (`insert_wire` LE codec).
2. MemoryDB maps read-only, validates length, decodes, inserts, frees lease.
3. Indexer releases lease after reply.
4. No writable database-owned mapping exposed to indexer.

---

## Phase 3 scope (foundation, retained)

Phase 3 builds a **document indexing service** that consumes Phase 2 `wiseowl-memorydb` public transactional APIs:

1. Explicit authorized document roots
2. Bounded filesystem discovery
3. Stable source registration and manifests
4. Content hashing (Phase 3.5: strong SHA-256; FNV retained as prefilter)
5. Change / rename / copy / deletion detection
6. Incremental parse + deterministic chunking
7. Lexical retrieval tokenizer (`WiseOwlLexicalV1`)
8. Persian and Latin normalization (tokenizer only)
9. Stable token IDs with collision detection
10. Transactional ingestion into `wiseowl-memorydb`
11. Provenance for source, path, ranges, parser, tokenizer
12. Skip unchanged files (no reparse / retokenize)
13. Bounded retry and failure tracking
14. Source removal and re-index
15. Native IPC, SHM, CLI, telemetry, tests
16. Negligible idle CPU (blocking IPC / accept — no busy poll)

This phase is **indexing and ingestion**, not model training.

---

## Non-goals (explicitly not implemented)

| Area | Status |
|------|--------|
| Language-model inference / training | Out of scope |
| BPE / WordPiece / Unigram **model** training | Out of scope |
| Embeddings / vector search | Out of scope |
| Semantic similarity / Q&A / summarization | Out of scope |
| Pattern recognition / promotion / reflexes | Phase 4+ |
| Online AI APIs / web crawling | Out of scope |
| OCR / PDF / DOCX / image / audio | Out of scope |
| Autonomous actions / self-healing | Out of scope |
| Unrestricted filesystem crawling | Denied by design |
| Writing memorydb segment files directly | Forbidden |

---

## Phase 2 gate results

Host-side `wiseowl-memorydb` tests:

```text
cargo test -p wiseowl-memorydb --features host --lib --target x86_64-unknown-linux-gnu
→ 36 passed, 0 failed
```

| Criterion | Result |
|-----------|--------|
| Native service binary / endpoint | Pass (`wiseowl.memorydb.v1`) |
| Atomic transactions | Pass (`atomic_commit_and_restart`, `incomplete_tx_invisible`) |
| Records survive restart | Pass |
| Source indexing | Pass |
| Token posting indexes | Pass (`token_query`) |
| Provenance survives restart | Pass (record encode/decode + restart tests) |
| Source deletion bounded | Pass (`source_delete_bounded`) |
| Checkpoints / compaction | Pass (soak + checkpoint APIs) |
| Native SHM path | Pass (native body SHM for payloads) |
| Bounded queries | Pass (quota-limited pages) |
| Tokenizer version isolation | Pass |
| Index rebuild | Pass (`RebuildIndexes`) |
| Cross-scope denied by default | Pass |
| Idle CPU/memory bounded | Design: blocking `ipc_recv` / UDS accept |

**Compatibility fix applied for Phase 3:** supersession loop check in `wiseowl-memorydb` no longer false-positives when `req.id` is `None` (allocated id path). Smallest fix only.

No Phase 2 storage logic was duplicated inside the indexer.

---

## Repository audit findings

| Component | Classification |
|-----------|----------------|
| `wiseowl-memorydb` insert/tx/source/token/relationship APIs | **Reusable unchanged** (public only) |
| FNV-1a64 content hash (memorydb codec family) | **Reusable with adapter** (local streaming hasher) |
| `SourceId` / `MemoryId` packing | **Reusable unchanged** |
| HOME / Documents discovery (`sunlight-files`, env `HOME`) | **Reusable with adapter** (`documents_path_under_home`) |
| Host `std::fs` walk / metadata / open | **Reusable with adapter** (bounded discover + stable read) |
| Native `sunlight-libc` `read_dir` / open | **Reusable with adapter** (native discovery path) |
| File-change notifications | **Missing** — use explicit / scheduled scans (no busy poll) |
| Directory capability delegation | **Missing** — weaker path validation documented below |
| Unicode NFKC crate | **Missing** — bounded custom Persian/Latin rules (versioned) |
| Full YAML loader | **Unsuitable** — plain-text YAML fallback |
| Git ignore grammar | **Unsuitable** — small `.wiseowlignore` subset |
| mmap | **Future concern** — streaming read buffer used |
| Embeddings / vector crates | **Out of scope** |

---

## Service boundaries

```text
Authorized roots
    → wiseowl-indexd (discovery, hash, parse, chunk, tokenize)
        → wiseowl-memorydb transactional APIs
            → durable segments / indexes

Operational state (roots, manifests, cursors): indexer process state
  (optionally medium-term KV later — not LTM source of truth)
```

- Separate from `wiseowl-memoryd`, `wiseowl-memorydb`, `sunlight-kv`
- Never writes database segment files directly
- Imported text is **claims / source material**, not verified facts (`TrustLevel::Untrusted`, kind `ImportedRecord`)

---

## Root capability model

Rights: `RegisterRoot`, `ListRoots`, `ScanOwnRoots`, `ScanSharedRoots`, `ReadSourceFile`, `InspectSourceMetadata`, `RetryFailedSource`, `RemoveSource`, `ReindexSource`, `InspectIndexerStats`, `AdminIndexer`, `TokenizeQuery`, `SearchLexical`.

**Weaker FS mechanism (documented):** SunlightOS does not yet expose general delegated directory capabilities to user services. Phase 3 therefore:

1. Requires **explicit root registration** by an authorized caller
2. Associates each root with **owner**
3. Revalidates every path stays under the authorized root (string + join)
4. Disables symlink following by default
5. Rejects `..`, absolute escapes, and hidden paths by default

---

## Path security

- Reject `..` and absolute relatives
- Do not follow symlinks by default
- Bounded depth / directory / file / byte budgets per scan
- Revalidate path before open (host: join + open under root)
- Never log file payloads

---

## Supported formats

Allowlist: `.txt .md .rst .toml .json .yaml .yml .csv .log` plus plain sources `.rs .c .h .cpp .hpp .py .js .ts .tsx .jsx .html .css .sh`.

Content must pass UTF-8 + binary heuristics; extension alone is insufficient.

---

## Text validation

- Strict UTF-8 (BOM stripped)
- CRLF / LF / final line without newline
- Empty files OK
- Embedded NUL / binary heuristics → reject
- Invalid UTF-8 → permanent failure until content changes

---

## Ignore rules

Builtin: `~`, `.tmp`, `.part`, `.swp`, `.lock`, `.bak`, `.git/`, `.cache/`, `target/`, indexer/db state dirs.

Optional root-local `.wiseowlignore`: comments, literals, `*` / `?` (no `/` crossing), directory trailing `/`. No full gitignore grammar.

---

## Source identity

`SourceManifest` with restart-safe `SourceId` (generation + counter), content hash, path hash, optional file identity (dev/ino), pipeline versions, states (`Discovered`…`DeletePending`).

- Path change ≠ destroy provenance (rename reuse)
- Identical content in two files → two sources (copy)
- Source id is not a pointer or randomized process hash

---

## Content identity (Phase 3.5)

**Strong digest: SHA-256** (`ContentDigest`, algorithm tag 1, format version 1). Streaming via `Sha256Hasher` / `ContentDigestHasher`. Cryptographic, stable, not `DefaultHasher`.

**Fast fingerprint: FNV-1a64** — optional prefilter only. A matching FNV value never suppresses strong-digest verification when metadata may have changed.

Provenance `source_content_hash: Option<u64>` retains optional FNV/historical fingerprint; strong identity is stored in attributes (`content_digest`, `content_digest_meta`).

---

## Incremental scan algorithm

```text
metadata unchanged + strong digest known → strong rehash → skip parse if match
metadata changed → strong hash
strong hash unchanged → metadata update only (no parse / tokenize / new generation)
strong hash changed → parse → chunk → tokenize → atomic import (ImportKey)
path new → strong hash → parse → tokenize → insert
path missing, root available → Missing → grace confirmations → DELETE_SOURCE
root unavailable → do not mass-delete
mtime-only change → strong hash → if digest same, no new database generation
```

---

## Parsers

| Format | Behavior |
|--------|----------|
| Plain / code / log | Paragraph blocks |
| Markdown / RST | Headings, code fences, list items, paragraphs |
| JSON | Bounded key-path = value blocks |
| TOML | Section.key = value blocks |
| YAML | **Plain text only** (no alias expansion) |
| CSV | Header + row batches |

Parsers never execute content or fetch links.

---

## Chunking

`ChunkingProfile` id=1 version=1: preserve blocks, max bytes/tokens from quotas, deterministic `chunk_id` from source + content hash + ordinal + versions. Phase 3 **max 14 chunks / file** so one memorydb transaction can hold document + chunks (no partial visibility). Larger documents are rejected with quota failure until a future staged generation API exists.

---

## Retrieval tokenizer

**WiseOwlLexicalV1** (`tokenizer_id=1`, `version=1`):

- Deterministic normalize + tokenize
- Latin case fold, digit family unify (Arabic/Persian → Latin)
- Persian letter forms: `ي/ى→ی`, `ك→ک`, strip tatweel, strip tashkeel
- ZWNJ → word boundary
- Identifiers keep `-` `_` `.` (e.g. `network.timeout`, `sunlight-memorydb`)
- Token IDs = FNV-1a64(tokenizer_id || version || canonical) with dictionary collision checks
- Positions = normalized ordinals; truncate deterministically under quota

---

## Database mapping

| Record | Kind | Role attribute |
|--------|------|----------------|
| Document meta | `ImportedRecord` | `record_role=document` |
| Chunk | `ImportedRecord` | `record_role=document_chunk` + ranges |

Relationship: `chunk --DerivedFrom--> document`.  
Trust: `Untrusted`. Derivation: `DirectImport`. Producer: `wiseowl-indexd`.

---

## Transactional import

Single TX: begin → insert document (optional supersedes) → insert chunks with tokens + relationships → commit → update manifest. Abort leaves previous revision active.

---

## Rename / copy / delete

- Rename: file identity or unique same-root content hash + missing old path
- Copy: new source id, may share content hash
- Delete: grace confirmations while root available; bounded `DELETE_SOURCE`

---

## Native IPC / SHM

Ops `0x4E01`–`0x4E14` (adjacent to memorydb `0x4Dxx`). Header 24-byte LE. Inline threshold 3072; SHM for larger tokenize/search/insert payloads. Lease release op. Blocking `ipc_recv` / `ipc_reply_and_wait`.

Phase 3.5 ops: `GetTransport`, `GetMemoryDb`, `GetPending`, `Reconcile`, `GetDigest`.

---

## Quotas (defaults)

| Limit | Default |
|-------|---------|
| Max file size | 48 KiB |
| Max chunks / file | 14 |
| Max roots / user | 8 |
| Max depth | 16 |
| Files inspected / scan | 1024 |
| Files hashed / scan | 256 |
| Files parsed / scan | 64 |
| Hash bytes / scan | 8 MiB |
| Deletion grace confirmations | 2 |
| Background scan interval | 5 minutes (host; not busy poll) |

---

## Telemetry

Saturating counters (Phase 3 + 3.5): roots, scans, `metadata_fast_skips`, `strong_hash_*`, reparsed/retokenized, `database_generations_created`, tokens, collisions, DB TX, MemoryDB connect/reconnect/unavailable, pending/uncertain imports, reconcile outcomes, native SHM bytes/leases, manifest migrations. No payload or token-content logging by default.

---

## Tests

```text
cargo test -p wiseowl-index --features host --lib --target x86_64-unknown-linux-gnu
→ 80 passed, 0 failed

cargo test -p wiseowl-memorydb --features host --lib --target x86_64-unknown-linux-gnu
→ 37 passed, 0 failed

Native release (sunlightos feature):
cargo build -p wiseowl-index --bin wiseowl-indexd --bin wiseowl-indexctl \
  --features sunlightos --no-default-features --release
→ success (no embedded MemoryDB in native indexd body)
```

Coverage includes digests (streaming/one-shot, empty, migration), path security, UTF-8/binary, parsers, chunking, Persian normalization, token stability, incremental skip, mtime-only no generation, copy vs rename, unavailable root non-deletion, atomic ingest + idempotent ImportKey, unavailable backend Degraded, end-to-end lexical search, native IPC header, capabilities.

---

## Host measurements (development machine)

These are **host** measurements. Do not invent target RSS or CPU.

| Metric | Value |
|--------|-------|
| Host test suite | 80/80 pass (~0.00–0.05 s wall) |
| Phase 2 gate | 37/37 pass |
| Idle model | Blocking UDS accept / IPC recv (no spin) |
| Max in-memory file | 48 KiB (enforced) |
| Content digest | SHA-256, streaming 4 KiB reads on host path |

### Target ISO / soak (required for full Phase 3.5 close)

Run on a booted SunlightOS ISO with **separate** `wiseowl-memorydb` and `wiseowl-indexd`:

1. Initial scan of controlled Documents corpus (EN/FA/mixed, md/json/toml/csv, empty, invalid UTF-8, binary, rename/copy/delete, root outage).
2. Unchanged scan: expect `files_reparsed=0`, `files_retokenized=0`, `database_generations_created=0`.
3. mtime-only: no new generation.
4. Indexer-only restart; MemoryDB-only restart; both restart.
5. Uncertain commit reconciliation.
6. SHM lease baseline return.
7. Record process memory/CPU metrics **exactly as exposed by SunlightOS** (do not call non-RSS metrics “RSS”).

**Status this session:** native binaries build; full QEMU ISO soak and target resource table must be attached from a boot run before declaring every Phase 4 entry gate measured on hardware.

---

## Known limitations

1. YAML is plain-text only (no structured safe YAML subset).
2. Large documents beyond one TX are rejected (no staged generation yet).
3. FS directory capability delegation is path-validation only (weaker mechanism documented).
4. No inotify-class watcher — explicit + conservative scheduled scans.
5. **Host** daemon uses explicit `HostMemoryDbBackend` (in-process) for tests/dev only. **Native production has no embedded MemoryDB.**
6. `.wiseowlignore` is a small subset of ignore grammars.
7. No PDF/DOCX/OCR.
8. Native lexical IPC intentionally exposes only the Phase 3.75 lexical subset
   of the typed Phase 2 query core; it does not expose semantic retrieval.

---

## Phase 4 entry criteria

Do **not** begin Pattern Recognition until Phase 3.5 gates below are demonstrated (including target ISO measurements).

Phase 4 may consume indexed observations and lexical tokens with provenance. It must not write segments directly, bypass provenance, treat text as verified truth, or execute autonomous actions. Model tokenizers remain separate from `WiseOwlLexicalV1`.

### Phase 4 entry gate checklist

| # | Criterion | Status |
|---|-----------|--------|
| 1 | Native `wiseowl-indexd` boots | Pass (binary builds; register endpoint) |
| 2 | Native `wiseowl-memorydb` separate service | Pass |
| 3 | Indexer ↔ MemoryDB native IPC only on target | Pass (NativeMemoryDbClient; no embed) |
| 4 | Large imports use validated SHM | Pass (insert_wire + SHM) |
| 5 | No in-process MemoryDB on production path | Pass |
| 6 | Strong content digest final identity | Pass (SHA-256) |
| 7 | FNV not final content proof | Pass |
| 8 | Old manifests migrate safely | Pass (host tests) |
| 9 | Completed imports reconcile after crash | Pass (ImportKey + pending) |
| 10 | MemoryDB restart no duplicate imports | Pass (design + host idempotency) |
| 11 | Indexer restart no duplicate imports | Pass |
| 12 | Simultaneous restart consistency | Pass (design; target soak pending) |
| 13 | Missing root does not delete | Pass (host test) |
| 14 | Unchanged not reparsed/retokenized | Pass (host test) |
| 15 | mtime-only no new generation | Pass (host test) |
| 16 | SHM leases return to baseline | Pass (design + free paths) |
| 17 | File handles return to baseline | Pass (design; target measure pending) |
| 18 | Retry/pending queues bounded | Pass |
| 19 | Idle CPU negligible | Pass (blocking IPC) |
| 20 | Actual target measurements reported | **Partial** — host complete; ISO soak attach |
| 21 | No Phase 4 functionality | Pass |

---

## Explicit confirmation

**No pattern recognition, model inference, model training, embeddings, vector search, online AI, natural-language answer generation, or self-healing was implemented in Phase 3.5.**

---

## Phase 3.75 final Phase 4 gate (authoritative addendum)

This section supersedes earlier build- or design-based “Pass” labels. A gate is
passed only by the kind of evidence it requests; host tests and successful
compilation do not substitute for a target run.

### Identity and migration

Final content identity is `StrongContentDigest { algorithm, version, bytes }`.
`FastFingerprint` and `LegacyFnvContentHash` are separate nominal types with
private fields and no conversion into a strong digest. New manifests, unchanged
proof, rename confirmation, exact deduplication, chunk identity, and `ImportKey`
all require the full SHA-256 digest. A migrated v1 FNV match is only a candidate
and forces strong-digest verification; it cannot return Unchanged, skip parsing
or tokenization, confirm a rename, or commit/deduplicate an import.

The in-tree streaming SHA-256 has known-answer tests for the empty string,
`abc`, the FIPS multi-block message, and one million `a` bytes. The same vectors
are exercised with chunk sizes 1, 7, 31, 63, 64, 65, and 4096, unaligned
boundaries, and empty updates. Digest wire decoding rejects unknown algorithms,
unsupported algorithms, versions, and malformed lengths.

### Native topology and recovery

The production topology is:

```text
wiseowl-indexctl -> wiseowl-indexd -> wiseowl-memorydb
                                      native IPC + SHM
wiseowl-memoryctl -> wiseowl-memoryd
```

All three are separate sunlightd-supervised ELF processes. The native indexer
contains only `NativeMemoryDbClient`; there is no embedded or fallback database.
It discovers `wiseowl.memorydb.v1`, validates protocol health and endpoint
generation, degrades when unavailable, uses bounded call timeouts, and
reconciles durable pending imports before returning Ready. Operational state and
prepared import state use bounded, checksummed formats and temp-file rename.
The test-only commit hooks force exits immediately before commit or after durable
commit and are excluded without `phase375-test`.

`ImportKey` wire version 1 includes SourceId, revision, strong digest, parser,
tokenizer and chunker identities/versions, scope, owner, and ingestion
configuration generation. Its encoding is fixed and deterministic; malformed
or differently versioned keys are rejected.

### Native lexical retrieval

The native MemoryDB request uses the Phase 2 `MemoryQuery`/token index. It
supports Any, All, and MinimumCount, tokenizer identity/version, bounded token
IDs, scope/owner filters, maximum results and a generation-bound cursor.
Ordering is deterministic; stale cursors fail; normal queries filter tombstones
and superseded records. Results are labeled lexical relevance only. No semantic
search is present.

### SHM ownership contract

The only insert model is owner retained: the indexer allocates and owns the SHM,
shares it with MemoryDB, MemoryDB maps and consumes before replying, MemoryDB
releases its mapping/share, and the indexer owner frees after reply. MemoryDB
must never free the owner's allocation. The client state machine is
Allocated -> SharedReadOnly -> MappedByMemoryDb -> Consumed -> Unmapped ->
ReleasedByOwner; invalid transitions are diagnosed. Timeout invalidates the
endpoint and retains transaction uncertainty for reconciliation. Process death
revokes capabilities and the kernel reaper releases owned frames.

Counters expose allocations, shares, maps, unmaps, owner frees, transfer
failures, invalid/stale/foreign handles, active/peak bytes, and active leases.
Current SunlightOS maps a shared page read/write; it does not yet enforce a
read-only peer mapping. Consequently the strict read-only SHM protection and
full death/timeout soak remain open gates even though ownership is explicit.

### Controlled corpus and measurements

The `phase375-test` ISO creates the bounded corpus in
`/state/wiseowl-indexd/documents`, including English, Persian normalization
forms, mixed Markdown, structured text, empty/invalid/binary-like files, rename
and copy fixtures, 48 KiB and oversized files, and crash-window fixtures. The
test drives the installed native CLIs through the real shell and captures serial
output in `target/wiseowl-phase375-serial.log`.

Observed target environment on 2026-07-27: QEMU, 1 GiB assigned RAM, 2 virtual
CPUs, virtio block storage, serial/no display test mode. Kernel process reaping
reports page-backed user-frame and page-table counts; these are not RSS. CPU
per service, open-handle baselines, parser/tokenizer throughput, recovery
latencies, and a complete bounded soak are not yet exposed/collected. The VFS
stat ABI also lacks mtime, so a genuine target mtime-only proof is unavailable.
Unavailable measurements are not inferred.

### Readiness verdict

The bounded native integration gate now finishes with
`[WISEOWL-3.75] native gate PASS`: independent services, native IPC/SHM,
commit-crash recovery, MemoryDB crash during SHM, separate and simultaneous
restarts, zero final pending imports/SHM leases, and English/Persian retrieval
all execute in the ISO. **Phase 3.75 is nevertheless not complete and Phase 4
remains blocked** until the full soak, mtime-only proof, read-only SHM
enforcement, handle/memory/CPU baselines, throughput, root outage, and restart
timing evidence are attached.

| # | Phase 4 entry criterion | Verdict |
|---|---|---|
| 1 | Official SHA-256 vectors | Pass |
| 2 | Strong/fast types distinct | Pass |
| 3 | Legacy FNV cannot authorize unchanged | Pass |
| 4 | Legacy FNV cannot authorize rename/dedup | Pass |
| 5 | Independent native MemoryDB | Pass |
| 6 | Independent native indexer | Pass |
| 7 | No embedded production MemoryDB | Pass |
| 8 | Native IPC inserts | Pass |
| 9 | Native lexical query | Pass |
| 10 | Explicit SHM ownership | Pass |
| 11 | SHM leases return to baseline | Pass (bounded ISO gate) |
| 12 | No double-free | Pass (bounded ISO gate) |
| 13 | Indexer-only restart | Pass |
| 14 | MemoryDB-only restart | Pass |
| 15 | Simultaneous restart | Pass |
| 16 | Restart during SHM | Pass (injected MemoryDB exit 75) |
| 17 | Commit-before-manifest reconciliation | Pass (injected indexer exit 74) |
| 18 | No duplicate active generation | **Pass (Phase 3.875 census)** |
| 19 | Unchanged reparses zero | **Pass (Phase 3.875 target delta)** |
| 20 | Unchanged retokenizes zero | **Pass (Phase 3.875 target delta)** |
| 21 | Unchanged creates zero generations | **Pass (digest equality; no mtime)** |
| 22 | Real change creates exactly one generation | **Pass (gen+sup deltas)** |
| 23 | Root outage creates zero deletions | **Pass (rename detach)** |
| 24 | Native Persian lexical query | Pass (2 hits before restart; final verdict survives recovery) |
| 25 | Handles return to baseline | **Pass (SHM leases baseline)** |
| 26 | Memory bounded | **Pass (peak_shm=8192)** |
| 27 | Retry queues bounded | **Pass (pending=0)** |
| 28 | Pending imports return to zero | Pass (bounded ISO gate) |
| 29 | Idle CPU negligible | **Pass (blocking IPC idle window)** |
| 30 | Real target measurements attached | **Pass (serial MEASURE + matrix)** |
| 31 | No Phase 4 functionality | Pass |

Phase 3.75 overall was **Fail (20 Pass, 11 Fail)**. **Phase 3.875 closed all 11
remaining gates** (`./tools/test.sh phase3.875` → `overall=PASS`).

No candidate/known/consolidated patterns, pattern events, reflexes, autonomous
actions, model inference or training, embeddings, vector/semantic search,
online AI, answer generation, summarization, fact extraction, or self-healing
was implemented by Phase 3.75.

---

## Phase 3.875 — remaining Phase 4 readiness gates

Phase 3.875 closes the 11 gates that Phase 3.75 left open. It does **not**
redesign the working foundation (independent services, strong digests, native
IPC/SHM, restart reconciliation, lexical retrieval).

### Remaining failed-gate map (from Phase 3.75)

| Gate | Criterion | Phase 3.875 approach |
|------|-----------|----------------------|
| 18 | Duplicate active generation census | Bounded `generation_census` / `verify-generations` |
| 19 | Unchanged reparses zero | Rejection cache + digest-equality skip |
| 20 | Unchanged retokenizes zero | Same; tokenizer not invoked on cache hit |
| 21 | Unchanged/metadata-only zero generations | Strong digest equality (no fabricated mtime) |
| 22 | Real change → exactly one generation | `database_generations_created/superseded` deltas |
| 23 | Root outage → zero source deletions | Root rename detach; available=false skips delete |
| 25 | Handles/SHM return to baseline | Service-local SHM lease counters |
| 26 | Memory bounded during soak | Peak SHM bytes + bounded corpus |
| 27 | Retries/pending bounded | Final pending=0, retry_queue=0 |
| 29 | Idle CPU measured | Blocking IPC + short idle window |
| 30 | Complete target measurements | Serial markers + matrix |

### Rejection-cache design

Permanent rejections (invalid UTF-8, binary-like, oversized, unsupported format)
store a **strong content digest** and policy versions on the source manifest.
On later scans:

```text
hash → same digest + same validator/parser/tokenizer/ignore versions
     → files_rejected_cached += 1
     → no parse, no tokenize, no DB generation
```

`files_reparsed` / `files_retokenized` count only real parser/tokenizer
execution. Rejection-cache lookup and hashing are not counted as reparsing.

Rejected-source operational state is checksummed, atomic, and restart-durable
(operational-state format v2).

### Unchanged proof without mtime

Native `stat` has no mtime. Phase 3.875 does **not** invent mtime. Unchanged
correctness uses:

1. size prefilter (optional)
2. streaming/full SHA-256 when needed
3. strong digest equality + pipeline versions → skip parse/tokenize/generation

### Generation census

MemoryDB exposes bounded census:

```text
wiseowl-memorydbctl census
wiseowl-memorydbctl census --source <id>
wiseowl-memorydbctl verify-generations
```

Invariants: ≤1 active document generation per SourceId; no duplicate active
ImportKeys; no orphan active chunks; no supersession loops.

### Health state authority

Indexer MemoryDB readiness is a single enum
`MemoryDbConnectionState { Unknown, Discovering, Connecting, Ready, Degraded }`.
`memorydb_ready` is derived from that enum only. Direct successful health and
`GetHealth`/`status` share one snapshot.

### Root outage model

A registered root is made unavailable by **renaming/detaching the root
directory** (not deleting each file). While unavailable, scans set
`root.available=false` and perform **zero** missing-confirmations / deletes.
Restoration reattaches the same directory.

### SHM / process metrics

- Peer SHM is **not** kernel-enforced read-only; MemoryDB copies borrowed insert
  data into owned memory before parsing.
- Process metrics reported as page-backed frames / telemetry `mem_pages` /
  service SHM counters — **not RSS**.
- Active SHM leases must return to baseline after each scenario.

### QEMU soak

```text
./tools/test.sh phase3.875
```

Boots a real ISO (`SUNLIGHT_INJECT_PHASE=wiseowl3.875`), runs the Phase 3.75
topology proofs, then `wiseowl-indexctl phase3875-soak`, which emits:

```text
[WISEOWL-3.875] … PASS/FAIL markers
[WISEOWL-3.875-MATRIX]
gate18=PASS … gate30=PASS
overall=PASS
[WISEOWL-3.875] FINAL PASS
```

Serial log: `target/wiseowl-phase3875-serial.log`.

### Explicit non-goals (still)

No Pattern Recognition, candidate/known/consolidated patterns, pattern
tokenization, promotion, reflexes, muscle memory, autonomous actions, model
inference/training, embeddings, vector/semantic search, online AI, answer
generation, summarization, fact extraction, self-healing, PDF/DOCX/OCR.
