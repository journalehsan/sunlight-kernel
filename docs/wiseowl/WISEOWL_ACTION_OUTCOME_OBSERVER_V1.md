# Wise Owl Action Outcome Observer v1

Action Outcome Observer v1 is the bounded post-dispatch stage for the existing
`OpenApplication` and `OpenSettingsPage` operations. It adds no executable
operation and has no launch, retry, shell, filesystem, generic process-query,
process-termination, or learned-memory capability.

## Lifecycle boundaries

The action path is:

`Policy authorization -> Confirmation -> ReadyForExecution -> Dispatch
acceptance -> Process creation -> Application/window registration or exact
settings-page activation -> Application readiness`

These stages are deliberately not interchangeable:

- Policy authorization says that policy permits the typed intent. It does not
  perform the action.
- Confirmation proves the bound user response when policy requires one.
- `ReadyForExecution` is the final validated executor envelope.
- Dispatch acceptance says only that the authoritative launcher accepted the
  typed request.
- Process creation identifies a correlated process instance, not readiness.
- Window registration is trusted display-server evidence, not title matching.
- Settings-page activation must name the exact canonical page ID.
- Application readiness is the registry-declared terminal condition.
- Application lifetime after readiness is outside v1. A later exit does not
  rewrite a completed launch observation.

Dispatch acceptance cannot be reported as application readiness because the
process may fail to start, exit early, never register, open the wrong settings
page, or miss its deadline.

## Correlation and trusted evidence

The trusted executor mints an opaque correlation token over the execution ID,
intent ID, canonical target, session, requester, launch attempt, and registry
generation. The token is carried by the typed launch request and copied into
trusted lifecycle evidence.

A PID alone is insufficient because PIDs are reused and do not bind an intent,
target, requester, session, attempt, or registry generation. Application names
and window titles are also insufficient. Titles are user-visible mutable text,
may collide across applications, and can contain private application data.

Evidence is accepted only from a bounded source kind appropriate to the typed
event: executor, process lifecycle, session manager, display server,
application registry, or Control Panel. Exact execution, token, session,
target, timestamp, and relevant generation must correlate. Unrelated,
duplicate, wrong-session, wrong-target, and late terminal evidence is rejected
or ignored and audited.

## Readiness and terminal behavior

The authoritative application/settings registry supplies one typed readiness
contract. Background applications may complete at `ApplicationRegistered`;
windowed applications may require `FirstWindowRegistered`; settings requests
require activation or readiness of the exact canonical page.

The forward-only state machine terminates at `Ready`, `Failed`,
`ExitedEarly`, `TimedOut`, `SessionInvalidated`, `RegistryInvalidated`, or
`Cancelled`. Deadlines are bounded and target-specific. Timeout never retries,
and late evidence cannot resurrect an observation. A materially changed
registry contract fails closed.

Cancelling observation means “stop waiting.” It does not close the application
or terminate its process. Those are separate executable operations and are
outside this milestone. The observer cannot reopen a consumed confirmation or
initiate another dispatch.

Repeated queries return the same immutable result. Duplicate observer creation
for an execution, replayed evidence, duplicate readiness results, and late
terminal transitions are suppressed.

## Presentation and memory

With outcome observation enabled, the coordinator enters `AwaitingOutcome` and
first presents “Opening Calculator…” / “در حال باز کردن ماشین حساب…”. It says
“Calculator is ready.” / “ماشین حساب آماده است.” only after a typed `Ready`
outcome. Timeout uses a separate typed public reason and does not expose
process, IPC, registry, or window internals.

The bounded audit ring contains IDs, operation, readiness contract, state,
event kind, and timestamp only. It never contains window titles, application
text, paths, environment, policy internals, conversation history, or secrets.

Learned Memory must not infer or reinterpret launch success independently. It
may consume only the presentation-safe terminal result supplied by the
coordinator; it cannot turn dispatch acceptance, a PID, or a remembered prior
launch into readiness.

## Verification

Host tests cover dispatch-only state, process/window/application contracts,
exact settings-page activation, correlation rejection, PID reuse, early exit,
timeout and late events, replay, cancellation, session invalidation, bounded
redacted audit, idempotency, and English/Persian coordinator wording.

The `wiseowl-outcome-observer-v1` QEMU gate is feature-gated with
`outcome-observer-v1-test` and injects only typed test lifecycle evidence. It
does not add a production test endpoint or backdoor.
