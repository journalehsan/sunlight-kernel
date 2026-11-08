# Wise Owl Delegated Session Authority and Lifecycle IPC v1

Status: production trust interfaces installed; production GUI-triggered actions remain disabled.

## Design audit

Exactly two designs were evaluated.

| Property | Kernel-backed delegated caller capability | Authenticated sessiond → braind callback |
|---|---|---|
| Kernel changes | Three fixed-purpose authority syscalls plus bounded kernel tables | Kernel-assisted reverse-channel authentication and registration |
| IPC complexity | One braind → sessiond request and an opaque proof reply | Preparation request, independently discovered Console channel, callback, and callback correlation |
| Replay resistance | Kernel one-shot removal before validation returns | Split replay state in sessiond and braind |
| Process death | Kernel checks caller and mediator PID plus address-space generation | Callback registration must separately track death |
| Session revocation | Sessiond changes one kernel session binding, clearing all handles | Callback and pending preparation tables both require invalidation |
| Restart behavior | Caller, mediator, and authority generations invalidate handles | Channel re-registration and pending callback cleanup are required |
| Serialization risk | Only opaque IDs/tags cross IPC | Authority state is more likely to become callback payload data |
| Authority ownership | sessiond publishes session generation; kernel authenticates process facts | sessiond owns callback state but depends on a second authenticated route |
| Testability | Bounded table and fixed syscall boundary are deterministic | Multi-party timing and callback discovery increase fixture complexity |
| Minimization | Destination and operation are not caller-selectable | Adds a reverse protocol used by only one operation |

The kernel-backed capability was selected as the smallest safe design. The callback design was rejected because it adds a reverse authentication channel and duplicates replay/restart state without removing the need for kernel process-generation validation. The two designs are not combined.

## Immediate-caller loss and delegation

The IPC bus overwrites `IpcMsg.badge` with the immediate sender PID. This authenticates `Console → braind`, but a subsequent `braind → sessiond` call correctly identifies braind, so the Console identity is otherwise lost.

While servicing the original call, braind invokes the fixed `WiseOwlDelegateCaller` syscall. It accepts only a bounded lifetime. It does not accept a caller PID, destination, operation, UID, or session. The kernel obtains the Console from braind's active IPC reply target and installs an opaque record bound to:

- the exact embedded Wise Owl Console launch identity;
- Console PID and address-space generation;
- braind PID and address-space generation;
- the fixed sessiond destination and Wise Owl session-attestation operation;
- the active session ID, user, and session generation published by sessiond;
- issue/expiry time and one use.

The in-flight limit is 32 and lifetime is at most 5 seconds (the userspace request is 2 seconds). Table overflow fails closed. Process death, logout, session-generation replacement, or mediator restart invalidates validation.

## Sessiond and authority proof

`ATTEST_DELEGATED_WISEOWL_CONSOLE` carries the opaque delegation, a diagnostic client value, and protocol version. The client value is not trusted: the kernel replaces the canonical client instance with the Console process generation.

Sessiond accepts only Running or Degraded graphical sessions. Kernel validation consumes the delegation before returning. Exact embedded launch provenance supplies the canonical `org.sunlight.wiseowl` Console role; sessiond retains at most two canonical Console registration records per session and replaces stale process generations.

Successful validation creates a short-lived `SessionAuthorityProof` in the kernel. Sessiond returns only its opaque ID/tag and canonical client instance. It does not serialize session/user/PID fields and does not serialize `VerifiedGraphicalSession`.

The proof is bound to the current braind PID and address-space generation, Console process generation, session authority generation, and two-second expiry. `wiseowl-brain` consumes it through the kernel exactly once and then calls its crate-private authority constructor. No GUI constructor, payload-field constructor, daemon-local unchecked constructor, or test callback is present in production.

## Trusted launch context and lifecycle producers

`TrustedLaunchContext` is opaque and non-`Debug`, non-`Clone`, and non-`Copy`. The bounded braind registry binds execution ID, exact canonical target, graphical session, application/Control Panel instance, registry generation, launch attempt, source generation, expiry, and an opaque integrity identity. There are no argv, environment, log, title, process-name, or PID-only constructors.

The old production `SunlightLaunchAdapter` no longer turns launch traces into correlation. Both launch methods fail closed because actions are deliberately disabled in this milestone. Existing launch traces remain diagnostics only and have no conversion to lifecycle evidence.

Braind registers two separate optional endpoints:

- `wiseowl.display-lifecycle.v1`
- `wiseowl.control-lifecycle.v1`

Kernel nameserver mediation permits lookup only by the exact embedded display or Control Panel process identity. Braind authenticates the endpoint's kernel-overwritten caller badge with a braind-only syscall and records its process generation. The public GUI endpoint neither owns nor shares these capabilities.

Display event contracts distinguish application registration, first top-level surface, registry-approved readiness, and exit-before-readiness. Control Panel contracts distinguish instance registration, canonical-page activation, canonical-page readiness, and exit-before-readiness. A navigation request is explicitly insufficient: activation is accepted only after the page state machine reports that the exact canonical page is authoritative.

Accepted events use the existing trusted readiness correlation and Outcome Observer route; diagnostic traces have no observer conversion and no parallel observer endpoint exists.

## Startup, availability, and bounds

Braind registers its GUI endpoint and reaches Ready without Console, Control Panel, an active action, or connected lifecycle producers. Producers connect or reconnect later. Missing endpoints are degraded/ unavailable state, not a startup wait.

`WiseOwlLiveActionAvailability` reports policy disabled, missing session authority, unattested Console, missing display/Control Panel lifecycle, application/settings readiness, or full readiness. It is informational and is never accepted by Planner, Policy, Confirmation Authority, or Executor as authorization.

Explicit bounds:

| Structure | Bound |
|---|---:|
| delegated capabilities in flight | 32 |
| delegation lifetime | 5,000 ms kernel maximum; 2,000 ms requested |
| delegation use count | 1 |
| Console instances per session | 2 |
| authority proofs | 32, one-shot, 2,000 ms |
| display source connections | 1 active generation |
| Control Panel source connections | 1 active generation |
| launch contexts | 16 |
| lifecycle events per execution | 8 |
| source sequence records | 2 |
| expired context tombstones | 32 |
| component registration records | 2 |

All overflow paths reject without eviction of active authority. Terminal context tombstones use a bounded FIFO only after a terminal event.

## Verification

Host verification covers the Wise Owl library, IPC contracts, and sessiond. Production no_std checks cover kernel, sessiond, braind, display, and Control Panel. The QEMU gate is `wiseowl-delegated-session-lifecycle-ipc-v1`; after a normal graphical login reaches Running, its feature-gated sessiond fixture launches Console through the trusted spawn path. Console then uses the real Console → braind IPC, kernel delegation, sessiond validation, authority-proof consumption, and braind ingress paths.

The milestone does not route Console turns to `ActionCoordinator`, enable `OpenApplication` or `OpenSettingsPage`, add actions, or remove the final policy-disabled state.
