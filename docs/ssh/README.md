# Sunlight SSH

## Current Status

Deferred after runtime compatibility audit.

## Implemented Foundations

- Phase 0.6: PTY ownership, resize, and lifecycle
- Phase 0.7: Event-driven TCP server support
- Phase 0.8: TCP memory and cleanup correctness
- Phase 0.9: Secure randomness qualification
- Phase 0.10: Atomic secret storage
- Phase 0.11: Strict service configuration

## Deferred

- Phase 0.12: russh runtime compatibility

## Resume Conditions

- stronger Sunlight libc
- broader Helios std support
- bounded async executor
- reliable AsyncRead and AsyncWrite adapters
- monotonic timers and cancellation
- required synchronization primitives
- acceptable russh dependency and fork budget

## Architectural Rule

SunlightOS will not implement a custom SSH protocol or cryptographic stack.
