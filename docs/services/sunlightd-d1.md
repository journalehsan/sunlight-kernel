# SunlightD D1: explicit on-demand activation

## Repository audit

SunlightD owns a fixed table of up to 32 compiled `.service` definitions in
`services/sunlightd/src/main.rs`. Unit names derive from the ExecStart basename
plus `.service`; definitions survive process death and explicit Stop/Restart.
Enabled state is persisted separately. Before D1, enabled services enter the boot
queue; disabled services are already dormant and can be explicitly started.
There was no activation policy or client activation protocol. `socket_act.rs`
is a stub and does not provide lazy spawning. Do not use it for D1.

`graph.rs` detects cycles among managed After/Requires edges. Boot startup gates
on nameserver registration, using special aliases for several resident services.
External dependencies are kernel/init owned. Wants is parsed but not enforced.
SunlightD has no registry mutex: one event loop owns its table and sleeps on IPC
with a 50 ms supervision deadline. No scheduler lock crosses userspace spawning.

The old supervisor distinguishes Stopped, Starting, Running, Failed, Restarting,
and Stopping, preserving unconfirmed termination and bounded explicit stops.
Type=simple is Running after spawn; Type=notify uses PID-badged NotifyReady,
without generation/endpoint validation or readiness timeout. D1 adds its own
stronger contract only for opted-in services. Existing resident behavior stays
on the original paths. Spawned services are parented to init, so SunlightD
usually cannot reap them with waitpid; existing supervision uses PID liveness.

The nameserver registers only actual runtime endpoints. The kernel validates
registration ownership and known names, derives SEND_ONLY capabilities, stamps
caller badges and checks per-process lookup capability profiles. Init lazily
removes dead registrations. Endpoint destruction/process death revokes derived
capabilities. Kernel process identity uses address-space generations; ordinary
PID liveness and the PTY-specific generation probe are insufficient for D1.
SessionD already has a separate generation-based desktop/session lifecycle. It
owns no part of this synthetic system service experiment and is unchanged.

Missing prerequisites addressed with small existing-ABI extensions:

* Kernel spawn reply now also returns the child's process generation under the
  spawn lock. Existing consumers still read word 0 (PID).
* Existing SessionGetCredentials permits the trust-chain-marked SunlightD to
  query live process incarnations.
* Existing NameserverEndpointValidate has a trusted-manager mode that checks a
  SEND token against expected PID/process generation and returns its endpoint ID.
  Init's original liveness mode is unchanged.
* Existing kill accepts an explicitly tagged generation from the trusted manager;
  validation and signaling happen under the same existing scheduler lock.
  Legacy two-argument calls do not need to initialize unused registers.
  D1 uses this for explicit stop/restart and failed-initialization cleanup.
* Kernel IPC checks ACTIVATE and legacy lifecycle-control authority even when a client is passed a manager
  SEND token. It checks the actual caller's service capability profile.
  Denied activation and nameserver lookups consume the armed next-call deadline,
  so the client can immediately make another bounded request.

## Configuration and lifecycle contract

Opt in with `[Service] Activation=on-demand`, `Type=notify`, `Restart=no`.
Absent Activation means `resident`, including existing manifests. Unknown policy
values, incompatible types/restart policies and paths that cannot fit the
existing 31-byte spawn ABI are rejected. The initial experiment adds only
synthetic definitions under SunlightD `demand-test` and kernel `demand_test`.
No Audio, Networking, VFS, Compositor, Wise Owl, or thumbnail service is converted.
Use `Capability=service-notify` for readiness notifications; it permits manager
lookup but conveys no activation, Start, Stop or Restart authority.
D1 activation is initially administrative (`service-lifecycle`); the experiment
uses a separate `demand-fixture` capability for direct synthetic operations.

The logical unit identity is discoverable through SunlightD even while inactive.
The runtime nameserver remains the authority for usable SEND capabilities.
Logical name, PID, kernel process generation, activation generation and endpoint
ID are separate. Manager resolution returns metadata only. Clients must perform
their own normal nameserver lookup and service operation authorization. D1 never
forwards a client operation or substitutes SunlightD as its caller.

The activation state is Inactive -> Starting -> Ready or Failed. Starting is
reserved before dependency startup or spawning. A checked u64 generation cannot
wrap. Concurrent requests join the same attempt. A bounded table holds at most
16 authorized deferred reply tokens across all services; excess demand returns
Busy. Tokens are tied by the kernel to the client's current IPC generation.
Client cancellation removes its waiter; a shared activation continues.

Readiness requires HELLO from the expected spawned process, initialized endpoint
registration, and READY with the matching activation generation. SunlightD checks
the actual badge, live kernel process incarnation, nameserver name and endpoint
ownership. Spawn success is not readiness. Legacy PID-only Ready/Failed messages
cannot satisfy an on-demand activation. Cold and reused resolution paths validate
runtime identity. Clients should validate the service's generation handshake after
lookup and retry discovery after transport closure; a resolution is not a lease.

Readiness/dependency wait is bounded to five seconds, including dependency work.
Timeout completes pending calls explicitly and attempts one generation-qualified
SIGKILL for failed initialization. This is startup failure cleanup, not idle
unloading. The old runtime is retained until confirmed dead; retries cannot spawn
a duplicate if cleanup fails. A failed attempt has a one-second retry cooldown
and is retried only by a new explicit request. No autonomous D1 restart loop.
Process death invalidates Ready. Explicit Stop/Restart keeps existing termination
confirmation rules, with D1 readiness required again for a replacement.

Required managed on-demand dependencies are planned as a bounded closure before
reservation, then advanced by the event loop without recursive spawning. Managed
After edges also wait for readiness, matching the existing boot contract.
Cycles fail before reservation. Resident/external dependencies must already be
available under their existing nameserver contracts; D1 does not redesign or
restart resident dependencies. Missing dependencies fail by the deadline. Wants
remains advisory. Every generation retains a single spawn attempt.

Metrics use saturating fixed counters: requests, starts, joins, failures,
timeouts, request-to-spawn and request-to-ready milliseconds. DISCOVER provides
current state/failure; METRICS provides counts. Successful readiness logs one
bounded line. Existing maintenance deadlines drive cleanup; no extra polling
thread, per-message allocation or queued client operation is added.

## Validation

Run `tools/test_sunlightd_d1.sh` for focused host tests and native 1/2/4 CPU gates.
Artifacts and measured latencies are retained in `target/d1-evidence/`.
The fixture deliberately waits 250 ms before registration/readiness to expose
concurrent activation and cancellation even on one CPU. These are synthetic
end-to-end readiness measurements, not claims about production service latency.

Validated on 2026-10-19 with QEMU TCG, 1024 MiB, one boot per CPU count:

| CPUs | First request to process start | First request to verified Ready | Reuse activation RPC | Cold dependency request to Ready |
| --- | ---: | ---: | ---: | ---: |
| 1 | 0 ms | 250 ms | 0 ms | 560 ms |
| 2 | 10 ms | 270 ms | 0 ms | 580 ms |
| 4 | 10 ms | 270 ms | 0 ms | 570 ms |

The exported monotonic clock has 10 ms resolution. A zero measurement means
both samples fell in the same tick. The service intentionally delays readiness
250 ms. Cold-start time here is request-to-process-start, including dependency
and scheduling delay; Ready time includes verified registration. Reuse RPC time
is measured in the native caller. These individual samples establish behavior
and instrumentation; they are not a throughput or production performance claim.

All CPU counts pass the native assertions for inactive discovery, denial with a
passed manager bearer token (ACTIVATE and legacy Start), eight concurrent calls
with one cancelled waiter, exactly one process start, Ready reuse, direct IPC
identity, activation without invocation access, missing executable, pre-Ready
crash, readiness timeout, a cold dependency closure, live endpoint destruction,
explicit restart with a new process/endpoint identity, stale capability rejection,
post-Ready death, and resident timezone availability. Each initialized synthetic
instance also rejects premature Ready before endpoint registration and a stale
activation generation. Existing Phase 2.6 expected IPC foundation messages pass
on all three boots, including capability isolation, IPC deadlines, deferred
replies, memory initialization, and 1000 IPC round trips.

Host results: 18 SunlightD core tests, 9 manager/configuration/dependency tests,
42 existing IPC library tests, 35 existing kernel memory tests, and the existing
scheduler host runner (51 Rust state/accounting tests plus its production policy
and queue probes) pass. Sources and full serial evidence remain under
`target/d1-evidence/`; generated artifacts are not committed.

Commands used in addition to the D1 gate:

```sh
RUSTFLAGS='' cargo test -p sunlight-ipc --lib --target x86_64-unknown-linux-gnu
RUSTFLAGS='' cargo test -p sunlight-kernel --lib --target x86_64-unknown-linux-gnu
python3 tools/test_scheduler_host.py
SUNLIGHT_TEST_CPUS=2 bash tools/test.sh sunlightd
```

The ordinary 2 CPU resident boot gate passes with no D1 fixtures registered.
The unchanged Wise Owl identity Phase C feature also passes compilation.

The existing `sunlightd.expected` fixture asserted 12 definitions, while the
current compiled default contains 20. Its count is corrected to 20; D1 adds no
units in the default build. The experiment adds six manager definitions only
under its explicit flag (five demand services and the resident test driver).

## Wire contract and example

All operations live in `sunlight_ipc::demand`. Logical request names occupy the
four transmitted register words; `request` rejects truncation, non-ASCII and
embedded NULs. The existing `.service` suffix is accepted for logical lookup.

| Operation | Successful reply words 0–3 |
| --- | --- |
| DISCOVER | phase (0 inactive, 1 starting, 2 ready, 3 failed), activation generation, last failure, enabled |
| ACTIVATE | activation generation, actual PID, kernel process generation, endpoint ID |
| HELLO | expected activation generation (expected spawned caller only) |
| READY | ACK after verified initialization, generation, registration and endpoint ownership |
| METRICS | accepted requests, process starts, coalesced requests, failures (low 32) / timeouts (high 32) |

Errors carry label ERROR and a bounded code in word 0. ACTIVATE joins a shared
attempt using deferred completion. It returns no service SEND or owner token.
An administrative client may use:

```rust
use sunlight_ipc::{demand, ipc_call_timeout, nameserver_lookup_timeout};
let manager = nameserver_lookup_timeout("sunlightd", 500).unwrap();
let request = demand::request(demand::ACTIVATE, "d1-fixture.service").unwrap();
let resolution = ipc_call_timeout(manager, request, 8_000).unwrap();
assert_eq!(resolution.label, demand::OK);
// This lookup is checked against this client's own invocation permissions.
let service = nameserver_lookup_timeout("d1-fixture", 500).unwrap();
// The synthetic service's ping returns activation and process generations;
// validate them against resolution before using an incarnation-dependent result.
```

An on-demand service registers its endpoint under the executable basename,
obtains its expected activation generation with HELLO after initialization, then
sends READY with that generation in word 0 from its primary process. Initialization
failure may be reported by exiting; process death completes the attempt as a
failure without waiting for the full readiness timeout.

## D1 boundaries

Only explicit administrative activation is supported. Per-user activation ACLs,
custom runtime endpoint aliases, readiness notifications from worker threads,
resident dependency recovery, Wants semantics, and general socket activation
are outside this experiment. ACTIVATE plans the demand dependency closure;
legacy Start/Restart requires dependencies to be ready before spawning.
Resident boot startup does not initiate activation of dormant dependencies; the
experiment adds no such edges to existing resident definitions.
Existing resident startup contracts are reused and
are not upgraded to D1 readiness. Activation generations last for a manager
incarnation; the kernel process generation and endpoint identity must accompany
them across runtime replacement. A manager crash/recovery protocol is not added.

Readiness deadlines are observed at the existing maintenance boundary, including
bounded IPC work and scheduling delay. At most one D1 dependency/spawn operation
runs per turn. If startup cleanup cannot confirm death, the failed runtime stays
quarantined until explicit control or confirmed exit; no replacement is spawned.
A lost endpoint in a still-live Ready process requires explicit Restart. Explicit
Stop invalidates pending activation with STOPPED; discovery retains that failure
reason until the next authorized attempt. Resolution is a point-in-time identity,
not an admission lease for later operations.

## Prerequisites for D2 (not implemented)

D2 needs service-owned idle eligibility, authenticated active-client/use leases,
a race-free admission barrier during teardown, generation-qualified draining,
capability/endpoint revocation ordering, dependency use accounting, cancellation
and shutdown deadlines, resource-release evidence and restart/backoff policy.
Session and service-specific authorization must remain independent. It also needs
workload and memory measurements to justify an idle policy.

D1 contains no idle shutdown, eviction, memory pressure policy or automatic
unloading.
