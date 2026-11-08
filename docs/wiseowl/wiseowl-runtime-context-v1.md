# Wise Owl Runtime Context Provider Framework v1

Runtime Context is the live, read-only operating-system snapshot consumed by
Wise Owl. Context Providers exist so the Brain does not contain subsystem
clients or need to know how services expose their state. Each provider is a
small adapter over one existing owner of state.

This framework adds no daemon and no IPC protocol. It uses the existing
`wiseowl-braind` process, existing service operations, existing files, and
existing syscalls.

## Layer Boundaries

Foundation Memory is immutable knowledge generated at build time. It contains
permanent identity, product, and policy facts and is unchanged by this work.

Runtime Snapshot is a transient view of the current boot, machine, services,
and active session. It is rebuilt in memory, never persisted, never learned,
and discarded when Wise Owl exits.

Conversation is request/session input. Learned Memory would contain durable
knowledge derived from use, but it is not implemented by this milestone.

The thinking order is therefore:

1. Foundation Memory
2. Runtime Snapshot
3. Conversation and request context
4. Bounded response planning

Thinking receives one `RuntimeContextSnapshot`; it never queries runtime
services directly.

## Provider Contract

`ContextProvider` has four responsibilities:

- identify itself with `name`
- declare a `RefreshClass`
- clear only the fields it owns
- collect a bounded view from its existing subsystem

Providers do not cache state independently, manage services, duplicate service
ownership, persist results, or start background work. Adding a provider means
implementing the trait and registering it with `RuntimeContextCache`; the
thinking pipeline does not change.

The default registry contains system, uptime, session, timezone, network,
display, and user-visible service-status providers. Power, thermal, storage,
and supervisor providers implement the same contract but are not registered by
default: the current capability model exposes control-plane lookup rather than
a read-only status grant. The framework leaves those values unknown instead of
granting Wise Owl management authority. Storage capacity also has no read-only
capacity query, so the provider does not infer values or repurpose the
write-oriented storage manager.

## Provider Lifecycle

Providers are registered when `CognitivePipeline` constructs its runtime cache.
They live for the lifetime of that pipeline and are invoked synchronously when
their refresh class is due. They are adapters only; registration does not create
a worker, timer thread, daemon, endpoint, or protocol.

Every provider is optional. An empty registry is valid and produces an
unavailable snapshot whose fields are all unknown.

## Snapshot Lifecycle

Refresh starts from the last complete snapshot. Each due provider runs against
a private candidate. On success its complete contribution is retained. On
failure its owned fields stay cleared, representing unknown. After all due
providers have run, availability metadata is recomputed and the candidate
replaces the published snapshot with one assignment.

Readers can therefore observe only an old complete snapshot or a new complete
snapshot. They cannot observe partial provider updates. Callers receive a shared
reference or clone, so collection cannot mutate a snapshot already being used
by thinking.

## Refresh Strategy

- `Static`: collected once during normal lifecycle; architecture, build,
  version, locale, and hostname.
- `Slow`: collected every 30 seconds; timezone, network, display, storage, and
  service status.
- `Fast`: collected every 5 seconds; uptime, session, battery/power, and thermal
  state.

An explicit `refresh` can rebuild all registered contributions. Normal request
handling calls `refresh_if_due`, which reuses non-due data and avoids querying
every service for every request.

## Failure Semantics

Provider errors never stop startup or request handling. A failed or unavailable
provider clears only its own fields to `None`; no defaults are invented. Other
providers continue, and the completed degraded snapshot is still published.
Provider counts and current failures are recorded as snapshot metadata for
diagnostics without changing reasoning semantics.

## Existing Sources

- system syscall: uptime
- `/etc/hostname`, `/etc/locale.conf`, release-generation file: static system
- `sunlight.session.v1`: active session
- `tz`: timezone
- `networkd`: connectivity
- `display_server`: display metrics
- `powerd`: power profile, AC source, and battery percentage
- `thermald`: thermal classification, temperature, and fan RPM
- `sunlightd` and the name service: service availability

Future installer, recovery, desktop, server, OEM, and cloud providers use the
same trait and registry. They do not require service knowledge in Brain or a
redesign of the thinking pipeline.
