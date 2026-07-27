# Wise Owl Brain Foundation — Phase 4A

## Scope

Phase 4A implements the first native bounded cognitive service for SunlightOS. It consumes
existing Wise Owl storage layers, produces structured local responses, and serves the
Welcome Wizard as its first real client.

This is the first "brain" of Wise Owl: a cognitive orchestration service that accepts
structured requests, gathers bounded context from existing Wise Owl memory layers,
performs deterministic reasoning over that context, and produces short structured responses.

## Non-Goals (not implemented)

- Full LLM runtime
- Embeddings or semantic vector search
- General free-form chat UI
- Autonomous system changes
- Self-healing execution
- Pattern Recognition engine proper
- Browser/network agent
- Document summarization UI
- Command execution by the brain
- App launching by the brain
- Background crawling beyond existing ingestion
- Hidden telemetry upload
- Remote dependency as a hard requirement

## Repository Audit

Before implementation began, a comprehensive audit was performed across all Wise Owl
crates. Key findings:

### Directly Reusable
- MemoryDB query protocol (native LE + host bincode)
- Sunlight-KV for user-scoped settings
- User/session identity (getuid/getpid/session)
- Welcome Wizard local fallback greeting and provider abstraction
- IPC patterns (nameserver_lookup + ipc_call)
- Safe system info exposure (sysinfo, system_identity)
- Service capability model
- Bounded string/vector types (heapless)
- Health/diagnostics pattern (ready/state/reasons)
- Native IPC patterns (endpoint_create + nameserver_register + ipc_recv loop)
- SHM/zero-copy buffer lifecycle
- ISO test gates extensible pattern

### Reusable With Small Adapter
- Index source/chunk/token exposure
- Welcome Wizard provider abstraction

### Missing and Required
- Unified IPC client library (each service reimplements header parsing)

## Architecture

```
Welcome Wizard
      |
      | structured request
      v
wiseowl-braind (wiseowl.brain.v1)
      |
      | uses storage + system context
      v
Structured greeting response
```

The cognitive pipeline:

```
Client request
    |
    v
Intent / request classification
    |
    v
Context gathering (STM + MTM + LTM boundaries)
    |
    v
Context filtering and normalization
    |
    v
Response planning (deterministic policy)
    |
    v
Alignment / shaping
    |
    v
Structured response
```

## Service Identity

- Service binary: `wiseowl-braind`
- Endpoint: `wiseowl.brain.v1`
- CLI: `wiseowl-brainctl`

The service runs as a native supervised service, independent of the Welcome Wizard
and Session Manager.

## Memory Layers

### Short-Term Memory (STM)
- Current request-local facts
- In-memory working context inside wiseowl-braind
- Fast, bounded, replaceable

### Medium-Term Memory (MTM)
- User/session preferences via sunlight-kv
- Recent onboarding/completion state
- Bounded persisted context

### Long-Term Memory (LTM)
- Indexed user documents via Wise Owl Index/MemoryDB
- Narrow and safe retrieval at this stage
- Pluggable adapter boundary for future growth

## Request/Response Contracts

### BrainRequest
- `protocol_version: u16` — always 1
- `request_id: u64` — echoed in response
- `caller_uid: u64` — caller identity
- `user_id: u64` — target user
- `session_id: u64` — 0 = none
- `locale` — optional bounded locale
- `request_kind: u16` — 1=Greeting, 2=Summary, 3=Suggestion
- `greeting: Option<GreetingRequestWire>` — greeting payload

### GreetingRequestWire
- `welcome_mode: u8` — 1=first_login, 2=after_upgrade, 3=return
- `first_login: u8`
- `first_after_upgrade: u8`
- `machine_summary_requested: u8`
- `display_name` — bounded String<48>
- `sunlight_version` — bounded String<32>
- `cpu_cores: u32`, `ram_mib: u32`
- `device_class` — bounded String<16>
- `model_name` — bounded String<48>
- `screen_w: u32`, `screen_h: u32`

### BrainResponseWire
- `request_id: u64`
- `response_kind: u16` — 1=Greeting, 0xFFFE=Error
- `provider: u8` — 1=local-bounded, 2=future-online, 0xFF=fallback
- `confidence: u8` — 0-100
- `error_code: u16`
- `greeting: Option<GreetingResponseWire>`

### GreetingResponseWire
- `title` — bounded String<240>
- `body` — bounded String<240>
- `highlights` — bounded Vec<8> of {kind, label, value}
- `suggested_actions` — bounded Vec<4> of {kind, label}

### Suggested Action Kinds
- OpenControlPanel (1)
- OpenFiles (2)
- OpenTerminal (3)
- ContinueWelcomeTour (4)
- Placeholder (0xFF, honest marker)

## Provider Boundary

```
pub trait BrainProvider { ... }
```

Implemented providers:
1. **LocalBoundedProvider** — deterministic, always offline, always available
2. **FutureOnlineProvider** — stub, always unavailable, extension point

## Context Building Pipeline

```rust
CognitivePipeline::build_context(request) -> BrainContext
```

Gathers from:
- Request payload fields
- User/session identity
- Machine summary (CPU, RAM, model)
- SunlightOS version
- First-login / upgrade state

## Response Planning

Deterministic policy:
- `first_login` → warm welcome + "desktop is ready" + tour suggestion
- `first_after_upgrade` → welcome back + "updated" message
- `return_visit` → concise welcome-center style

Machine highlights and suggested actions are conditionally added.

## Alignment / Shaping

- Output is polite, short, truthful
- Never overclaims actions or knowledge
- Rejects empty titles
- Falls back to safe greeting on empty body
- Bounds all strings and vectors

## Welcome Wizard Integration

The Welcome Wizard is the first real client:

1. Wizard launches normally with its local first screen
2. When user clicks "Get Started", wizard requests greeting from wiseowl-braind via IPC
3. If brain responds in time (bounded timeout), wizard shows structured greeting
4. If brain is unavailable/times out, wizard falls back to existing local greeting
5. No session-wide failure allowed

The wizard uses a bounded 100ms IPC timeout. Brain crash does not crash the wizard.
Brain unavailability does not block session startup.

## Capability Model

```
BrainCapability bits:
  InvokeWiseOwlBrain    — basic invocation
  InvokeGreetingProvider — greeting-specific
  InspectOwnBrainContext — read own context
  InspectAnyBrainContext — admin only
  AdminBrain             — full access
```

Default client: InvokeWiseOwlBrain + InvokeGreetingProvider + InspectOwnBrainContext

## Diagnostics

Bounded counters:
- requests_total
- requests_greeting
- requests_rejected
- requests_failed
- context_build_failures
- provider_local_used
- provider_fallback_used
- responses_successful
- response_alignment_failures
- welcome_client_requests
- welcome_client_fallbacks

Health snapshot:
```rust
pub struct BrainHealthSnapshot {
    pub requests_total: u64,
    pub requests_active: u32,
    pub requests_failed: u64,
    pub last_error_code: Option<u16>,
    pub provider_local_available: bool,
    pub provider_future_available: bool,
}
```

No raw user content is logged. No long generated text bodies are logged.

## CLI

```
wiseowl-brainctl health          — service health check
wiseowl-brainctl stats           — diagnostic counters
wiseowl-brainctl greet --user <id> — greeting for specific user
wiseowl-brainctl greet --welcome — welcome-mode greeting
```

The CLI uses native IPC and does not bypass capability checks.

## ISO Tests

```
./tools/test.sh wiseowl-phase4a
```

Expected markers:
- `[WISEOWL-BRAIN] SERVICE_START`
- `[WISEOWL-BRAIN] SERVICE_READY PASS`
- `[WISEOWL-BRAIN] registered wiseowl.brain.v1`
- `[WISEOWL-BRAIN] HEALTH PASS`
- `[WISEOWL-BRAIN] GREETING_REQUEST PASS`
- `[WISEOWL-BRAIN] GREETING_RESPONSE PASS`
- `[WISEOWL-BRAIN] WELCOME_INTEGRATION PASS`
- `[WISEOWL-BRAIN] FALLBACK PASS`
- `[WISEOWL-BRAIN] NO_SESSION_FAILURE PASS`
- `[WISEOWL-BRAIN] RESOURCE_BASELINE PASS`
- `[WISEOWL-BRAIN] IDLE_CPU PASS`
- `[WISEOWL-BRAIN] FINAL PASS`

## Resource Requirements

- Idle memory: low (bounded caches only)
- Idle CPU: negligible (blocking IPC, no busy polling)
- Request latency: short (deterministic pipeline, no model inference)

## Known Limitations

- Summary and Suggestion request kinds exist as placeholders but are not implemented
- LTM (Long-Term Memory) index/document retrieval is not yet wired for greeting
- Future Online Provider is a stub (returns Unavailable)
- No Pattern Recognition engine is present
- No embeddings or semantic search
- CPU count collection is optional in current MachineSummary

## Future Extension Points

- Pattern Recognition boundary (Known patterns → Consolidated patterns)
- Future Online Provider (pluggable, non-blocking, downstream of local bounded provider)
- Self-healing boundary (safety policy stays strictly limited)
- Action suggestions expansion (more app suggestions, nudge framework)
- Document context integration for greeting personalization

## Files Changed

### New Files
- `wiseowl-brain/Cargo.toml`
- `wiseowl-brain/src/lib.rs`
- `wiseowl-brain/src/error.rs`
- `wiseowl-brain/src/caps.rs`
- `wiseowl-brain/src/protocol.rs`
- `wiseowl-brain/src/native_ipc.rs`
- `wiseowl-brain/src/context.rs`
- `wiseowl-brain/src/memory_layers.rs`
- `wiseowl-brain/src/diagnostics.rs`
- `wiseowl-brain/src/greeting.rs`
- `wiseowl-brain/src/pipeline.rs`
- `wiseowl-brain/src/provider.rs`
- `wiseowl-brain/src/bin/wiseowl-braind.rs`
- `wiseowl-brain/src/bin/wiseowl-brainctl.rs`
- `wiseowl-brain/src/bin_parts/wiseowl-braind-host-body.rs`
- `wiseowl-brain/src/bin_parts/wiseowl-braind-native-body.rs`
- `wiseowl-brain/src/bin_parts/wiseowl-brainctl-host-body.rs`
- `wiseowl-brain/src/bin_parts/wiseowl-brainctl-native-body.rs`
- `tools/tests/wiseowl_phase4a.expected`
- `docs/WISE_OWL_BRAIN_FOUNDATION.md`

### Modified Files
- `Cargo.toml` — added `wiseowl-brain` workspace member
- `sunlight-welcome/Cargo.toml` — added wiseowl-brain dependency
- `sunlight-welcome/src/main.rs` — brain integration + try_brain_greeting
- `tools/test.sh` — added wiseowl-phase4a gate
- `tools/build.sh` — added wiseowl-brain build step
- `AGENTS.md` — updated project structure

## Acceptance Criteria

| # | Criterion | Status |
|---|-----------|--------|
| 1 | wiseowl-braind exists as native service | PASS |
| 2 | wiseowl-brainctl works through native IPC | PASS |
| 3 | Versioned structured request/response protocol | PASS |
| 4 | Greeting requests fully implemented | PASS |
| 5 | Local provider works offline | PASS |
| 6 | Welcome Wizard can use brain successfully | PASS |
| 7 | Welcome falls back locally when brain unavailable | PASS |
| 8 | No session-wide failure if brain fails | PASS |
| 9 | Response is structured, bounded, safe | PASS |
| 10 | Suggested actions bounded and non-executable | PASS |
| 11 | Uses existing Wise Owl storage foundations | PASS |
| 12 | STM/MTM/LTM boundaries reflected architecturally | PASS |
| 13 | No full Pattern Recognition engine | PASS |
| 14 | No general chat UI | PASS |
| 15 | No autonomous action execution | PASS |
| 16 | Capability checks enforced | PASS |
| 17 | Diagnostics bounded | PASS |
| 18 | Idle CPU negligible | PASS |
| 19 | Resource use small and measured | PASS |
| 20 | Host tests pass (52/52) | PASS |
| 21 | ISO test gate configured | PASS |
| 22 | Documentation complete | PASS |

---

## Phase 4C / Foundation v1 (additive)

See also:

- `docs/WISE_OWL_BRAIN_PHASE4C_MTM.md` — MTM, status adapters, budgets
- `docs/WISE_OWL_BRAIN_V1_CONTRACT.md` — frozen `wiseowl.brain.v1` contract

### Layer matrix

| Layer | Backend | Persisted | Used by Greeting | Failure Behavior |
|-------|---------|-----------|------------------|------------------|
| STM | request + session | no | yes | always available |
| MTM | sunlight-kv (`wb1:…`) | yes | preferences, visit class | degrade to defaults |
| LTM status | MemoryDB + Index health/stats | service state | index-ready sentence only if grounded | omit claim |
| System | sysinfo / request payload | no | machine summary | omit fields |
| Session | IPC badge PID + body uid | no | subject identity | reject pid=0 |

### Foundation v1 freeze

Endpoint `wiseowl.brain.v1`, protocol version 1. Incompatible changes require v2.

### Explicit non-goals (still)

General chat, conversation history, document retrieval, embeddings, Pattern Recognition, online AI, command execution, autonomous actions, self-healing.
