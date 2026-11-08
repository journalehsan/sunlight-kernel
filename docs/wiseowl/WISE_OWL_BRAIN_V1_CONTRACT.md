# wiseowl.brain.v1 — Contract Freeze (Foundation v1)

## Endpoint

- **Name:** `wiseowl.brain.v1`
- **Protocol version:** `1` (`NATIVE_PROTOCOL_VERSION`)
- **Transport:** IPC label = `BrainOp`; body = LE wire types; payload >24 bytes via SHM + `BrainIpcHeader` (24 bytes LE).

## Operations (`BrainOp`)

| Op | Code | Body |
|----|------|------|
| Greeting | `0xB001` | `BrainRequestWire` |
| Context | `0xB004` | optional request body |
| PreferencesGet | `0xB010` | uid LE u64 |
| PreferencesSet | `0xB011` | words: uid, field, value |
| WelcomeCompleted | `0xB012` | words: uid, system_generation |
| Health | `0xB00E` | empty |
| Stats | `0xB00F` | empty |
| Reply | `0xBF80` | `BrainResponseWire` |
| Error | `0xBFFF` | error response |

## Wire types (v1)

- `BrainRequestWire` / `BrainResponseWire` / `GreetingRequestWire` / `GreetingResponseWire`
- `GroundedFact` (internal; not full wire dump to clients)
- `BrainResponseMeta` (internal provenance; provider byte on wire)
- `BrainPreferences` / `WelcomeMemoryState` (KV-encoded)
- `BrainHealthSnapshot` (diagnostics)

## Compatibility rules

1. Additive optional fields must default safely.
2. Unknown optional fields ignored where encoding allows.
3. Unknown enum values rejected or mapped deterministically (never panic).
4. Incompatible changes → protocol version 2 and/or `wiseowl.brain.v2`.
5. v1 remains supported through current Alpha unless explicitly deprecated.

## AuthZ

- Kernel badge stamps **caller PID**.
- Subject uid taken from request body (own user); root (0) valid.
- Cross-user preference/greeting claims rejected when body uids disagree.
- No forged capability bits accepted from clients.

## Golden invariants

- Greeting success does not require KV/MemoryDB/Index.
- Personalized claims require grounded facts.
- Welcome completion is owned by session/Welcome, not Brain inference.
