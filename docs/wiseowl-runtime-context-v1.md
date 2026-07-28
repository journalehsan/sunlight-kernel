# Wise Owl Runtime Context v1

Runtime Context is the live, read-only system snapshot consumed by Wise Owl at startup.

It is not Foundation Memory.
It is not learned memory.
It is not conversation history.

## Layer Boundaries

Foundation Memory:
- Immutable.
- Generated at build time.
- Tokenized once during the OS build.
- Contains permanent identity and policy facts.

Runtime Context:
- Reconstructed whenever Wise Owl starts.
- Reflects only the current machine and session state.
- Never persisted as learned memory.
- Refreshed lightly on a bounded timer after startup.

Learned Memory:
- Not part of this milestone.
- Must remain separate from both Foundation and Runtime Context.

Examples:
- Foundation: assistant name, SunlightOS identity, safety principles.
- Runtime Context: timezone, hostname, active session mode, network status, display metrics.
- Learned Memory: user preferences inferred from use over time. Not implemented here.

## Runtime Context Sources

Runtime Context v1 reuses existing SunlightOS services and files only:

- `tz` for the active timezone
- `networkd` for interface and connection state
- `display_server` for live display metrics
- `sunlight.session.v1` for active session identity and state
- `sunlightd` for supervised service state where available
- existing VFS-backed files such as `/etc/hostname`, `/etc/locale.conf`, and `/etc/sunlight/release-generation`
- existing syscalls for uptime

No new daemon, background service, or IPC protocol is introduced.

## Snapshot Model

Wise Owl now maintains a bounded `RuntimeContextSnapshot` with four read-only groups:

- `system`
- `network`
- `display`
- `services`

Unknown values remain unknown. Missing services do not block startup.

## Startup And Refresh

At Wise Owl startup:

1. Foundation Memory loads first.
2. Runtime Context fixed fields are loaded once.
3. Dynamic fields are collected once into a bounded snapshot.

After startup:

- Wise Owl reuses the cached snapshot.
- Dynamic fields refresh on a lightweight timer.
- Fixed fields are not re-read unless Wise Owl restarts.

This keeps runtime tokenization at zero and avoids per-request service fan-out.

## Pipeline Order

The effective Wise Owl context pipeline is now:

1. Foundation Memory
2. Runtime Context
3. Session and request conversation data
4. Thinking / response planning

Foundation remains immutable.
Runtime Context always reflects "now" as best-effort live state.

## Boot Behavior

- Runtime Context is best-effort only.
- Validation failure is not applicable because Runtime Context is not persisted.
- Query failures degrade to unknown values.
- Desktop boot, login, and installer behavior remain independent of Wise Owl availability.

## Future Work

Runtime Context v1 intentionally does not implement:

- learned memory
- user memory persistence
- installer-specific context
- per-request service polling
- new service ownership

Future milestones can extend the same snapshot interface for installer and other live contexts without changing Foundation Memory semantics.
