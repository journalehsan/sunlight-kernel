# Phase 0.9: Secure Randomness Qualification

## Security contract

Cryptographic bytes are issued only through this path:

```text
approved platform source
  -> kernel entropy collector + ChaCha20 conditioner
  -> SecureEntropyReady syscall
  -> rand_service ChaCha20 DRBG
  -> sunlight_libc::getrandom(flags = 0)
  -> cryptographic caller
```

The approved sources are:

| Platform | Approved source | Qualification |
| --- | --- | --- |
| QEMU | Legacy `virtio-rng-pci` backed by QEMU host entropy | Required by `tools/build.sh`, `tools/runs.sh`, and `tools/test.sh` |
| VMware | `RDSEED`, or `RDRAND` only when `RDSEED` is unavailable | CPUID-gated and retried up to ten times per word |

The collector needs a full 40-byte seed before it becomes ready. It conditions
that seed with a kernel-owned ChaCha20 stream. TSC, RTC, timing jitter, counters,
and user-controlled state are not entropy sources and cannot make the secure
state ready.

SunlightOS supports Ivy Bridge-era hardware, so `RDSEED` is never assumed. The
kernel detects `RDSEED`/`RDRAND` with CPUID, checks the carry/success flag on
every instruction, uses a bounded retry (10), and treats repeated failure as
source failure. Raw hardware samples are never returned to callers; only the
conditioned ChaCha20 stream is.

This document does **not** claim NIST, FIPS, or SP 800-90 compliance. The
construction follows established ChaCha20 keystream practice but has not
undergone a formal entropy assessment or validation program.

## Fail-closed behavior

- If no approved source yields the boot seed, `SecureEntropyReady` returns `0`.
- `rand_service` refuses to register when entropy is unready or initial seeding fails.
- `sunlight_libc::getrandom` returns `-1` for both cryptographic and
  `GRND_NONCRYPTO` requests while entropy is unready (except the empty-buffer
  case, which returns `0` without IPC); it never installs a predictable fallback
  seed and never downgrades crypto requests to xoroshiro.
- Linux-compatible `getrandom(2)` returns `EAGAIN` until entropy is ready.
- Exec stack `AT_RANDOM` generation fails rather than writing zero bytes.
- TLS uses its custom `getrandom` handler for handshake randomness and rejects
  a session secret if the secure request fails.
- Kernel-minted UAC authentication-session grants use a fresh conditioned
  entropy word and are unavailable while entropy is unready.

This makes unavailable entropy visible at the first dependency instead of
silently creating weak SSH host keys, ephemeral exchanges, authentication salts,
session secrets, tokens, or other cryptographic material.

## Service and caller policy

`rand_service` is a privileged seed consumer, not an entropy source. It receives
only conditioned kernel bytes, seeds a 256-bit ChaCha20 DRBG, and reseeds after
8192 output blocks (~512 KiB). Reseed builds a candidate state, rejects
all-zero keys, wipes temporary seed buffers with volatile stores, and only then
publishes the new state. A failed reseed leaves the previous state intact and
returns `ERROR` to the caller for that request.

`sunlight_libc::getrandom` routes default requests through this service with
bounded IPC timeouts (lookup 2s, per-chunk call 5s) so a dead service cannot
hang TLS forever. Callers must treat a negative result as fatal for
cryptographic work. On failure the destination buffer is wiped so partial fills
are not left readable as success.

Non-crypto (`GRND_NONCRYPTO`) remains a separate local xoroshiro64** path and is
never used as a crypto fallback.

### IPC contract (`RandMsg`) — preserved

| Opcode | Value | Role |
| --- | --- | --- |
| `GET` | `0x7201` | `words[0]` = requested length, clamped to 32 |
| `STATS` | `0x7202` | Additive non-sensitive telemetry (optional) |
| `REPLY` | `0x72FF` | Success |
| `ERROR` | `0x72FE` | Failure |
| `MAX_CHUNK` | 32 | Register-IPC byte budget |

Service name remains `"rand"`. Existing GET/REPLY encoding is unchanged.

`STATS` reply words (never seeds/keys/output): ready, total_requests,
total_bytes, packed failure counters, last_reseed_reason, policy constants.

### Reseed policy

| Event | Reason enum |
| --- | --- |
| Initial service start / restart | `ServiceRestart` (or `Initial` in tests) |
| After 8192 produced blocks | `ByteThreshold` |
| Entropy recovery (future) | `EntropyRecovery` |

Justification: ChaCha20 remains secure far beyond 512 KiB per key; the bound is
defense-in-depth for a long-lived ring-3 DRBG that can reseed from the kernel
conditioned stream. Wall-clock, PID, TSC, boot count, and process identity are
never used as entropy (domain separation only would be acceptable if needed).

### Snapshot / resume risk (remaining)

SunlightOS does not currently expose a resume/VM-snapshot notification to
user-space. If a VM is snapshotted and restored, `rand_service` may continue the
same keystream until the next reseed threshold or service restart. Documented as
a remaining risk; no kernel change in this hardening pass.

## Qualification tests

Run the deterministic QEMU gate:

```bash
./tools/test.sh phase0.9
```

The test attaches `virtio-rng-pci` and requires:

```text
[ENTROPY] secure source=virtio-rng conditioner=ChaCha20 readiness=ready
```

Engine unit tests (host):

```bash
cargo test -p rand_service --lib --target x86_64-unknown-linux-gnu
```

For VMware qualification, boot the production `.vmx` on the intended VMware
version and retain the serial log showing either `source=RDSEED` or
`source=RDRAND`. A VMware boot that logs `source=none` is a failed security
qualification and must not be used for cryptographic services.

## Deferred improvements

- `/dev/random` / `/dev/urandom` pseudo-devices
- VirtIO RNG in user-space (kernel already uses legacy virtio-rng at boot)
- TPM RNG
- New kernel entropy syscalls
- Resume/snapshot reseed notification
- Formal FIPS / SP 800-90 validation
- getrandom crate upstream integration beyond the TLS custom handler
