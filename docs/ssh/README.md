# Sunlight SSH

## Current decision

SSH remains deferred: there is no daemon or supported russh runtime on SunlightOS.
The [September 21, 2026 readiness audit](READINESS_AUDIT_2026_09_21.md)
rechecks the source against published russh 0.63.3 and identifies the work
needed to resume implementation.

The main blockers are a supported `std`/Tokio platform, authenticated PTY
creation and shell startup, PTY readiness and crash cleanup, a conventional
remote terminal mode, and qualified crypto/randomness and persistent host keys.
Adding russh to `pty_server` would not resolve these dependencies.

## Implementation order

1. Prove a supported platform for unmodified russh and its crypto backend.
2. Repair and share the PTY client API; add readiness and owner-death cleanup.
3. Connect authenticated identity, restricted shell spawning, and PTY authority.
4. Add a standard terminal mode to the native shell.
5. Implement a separate `services/sunlight-sshd` adapter and disabled-by-default
   sunlightd unit after the platform and session gates pass.
6. Prove OpenSSH interoperability, resource limits, cleanup, and reboot behavior.

The audit includes source evidence, concrete acceptance gates, and the exact
host-test results. Existing Phase 0 foundation code is reusable, but its presence
does not establish that these end-to-end gates pass.

## Documents

- [Current readiness audit and implementation backlog](READINESS_AUDIT_2026_09_21.md)
- [Original pre-SSH hardening plan](PRE_SSH_HARDENING_PLAN.md)
- [Strict service configuration](PHASE_0_11_STRICT_SERVICE_CONFIGURATION.md)
- Historical Phase 0.12 records for **russh 0.62.3**:
  [selection](library-selection.md), [dependencies](dependency-audit.md),
  [runtime](runtime-surface.md), [spike](compatibility-spike.md),
  [algorithms](algorithm-audit.md), [licenses](license-report.md), and
  [limitations](known-limitations.md).

## Architectural Rule

Keep SSH protocol and cryptography in a maintained library. Keep PTY brokerage
in `pty_server`, identity in UAC/kernel grants, and service lifecycle in
sunlightd. No custom SSH protocol or cryptographic stack is proposed.
