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

## Fail-closed behavior

- If no approved source yields the boot seed, `SecureEntropyReady` returns `0`.
- `rand_service` refuses to register when entropy is unready.
- `sunlight_libc::getrandom` returns `-1` for both cryptographic and
  `GRND_NONCRYPTO` requests while entropy is unready; it never installs a
  predictable fallback seed.
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
8192 output blocks. `sunlight_libc::getrandom` routes default requests through
this service; callers must treat a negative result as fatal for cryptographic
work.

The current in-tree cryptographic caller is `sunlight-tls`, whose rustls
provider obtains randomness through this path. No SSH server or host-key
generator is currently present in the workspace. Any future SSH, password,
token, session, or persistent-key component must use
`sunlight_libc::getrandom(buf, 0)` and must not use syscall 87 directly.

## Qualification tests

Run the deterministic QEMU gate:

```bash
./tools/test.sh phase0.9
```

The test attaches `virtio-rng-pci` and requires:

```text
[ENTROPY] secure source=virtio-rng conditioner=ChaCha20 readiness=ready
```

For VMware qualification, boot the production `.vmx` on the intended VMware
version and retain the serial log showing either `source=RDSEED` or
`source=RDRAND`. A VMware boot that logs `source=none` is a failed security
qualification and must not be used for cryptographic services.
