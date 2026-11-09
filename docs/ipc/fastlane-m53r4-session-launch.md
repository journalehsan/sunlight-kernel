# M5.3R4 session-launch prerequisite

**Status: BLOCKED; runtime prerequisite implemented, complete milestone not delivered.**
Production FastLane, production restricted admission and hostile-process zero-copy
remain disabled. No commit or push.

Evidence: `.fastlane-evidence/m53r4-20261109T180732Z/REPORT.md`,
`validation-summary.json`, `AUTHORITY.md`, `PUBLICATION.md`, `INHERITANCE.md`,
`QUOTA.md`, exact-source checks and the native NOT_RUN matrix.

The existing local password authority is uac_service, not the external-provider
Online Accounts broker. Its delivered login proof is consumed by the existing
SessionD/kernel Account registry. No new session registry or authentication
system was introduced.

An opt-in debug-only SPAWN_SESSION_BOUND request checks the trusted SessionD
role and exact registry owner identity. The broker issues a private move-only
LaunchAttachment for an active SessionRef; credentials come from the registry.
Commit revalidates and installs Process.account_membership before the existing
scheduler publication and runnable operations, under their existing lifecycle
lock order. Expired commit rejects and reclaims the unpublished VM. Account-bound
SHM allocations check that registry membership is still live and use the existing
atomic process/session ledger. Retirement fences new allocations without
refunding retained backing. Limits stay 512 MiB/process and 1 GiB/session.

This is not complete login-wide enforcement. Initial SessionD shell launch still
precedes session activation, so it cannot use the active-only attachment path.
An operation-specific initial-launch transition and caller conversion remain
necessary. Attached debug processes are denied child/thread/clone/IPC launch
until inheritance is implemented; allowing sessionless descendants would bypass
accounting. Exec preserves the new field but has no native session fixture proof.
Legacy missing-membership user allocations retain their previous process-only
policy. Delegated launch and shared-service charging remain unsupported.

There is no native session-launch fixture, scheduled first-execution witness or
initialization/publication failure-injection matrix. Native 1/2/4/12 CPU scenarios
are NOT_RUN. Passing host broker/quota tests and an emitted continuation ABI
check must not be interpreted as native session-launch acceptance. Continue
M5.3R4 with the initial-launch transition, trusted callers, verified inheritance,
fault injection and exact-source native matrix before changing this status.
