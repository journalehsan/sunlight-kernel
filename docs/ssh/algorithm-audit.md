# russh 0.62.3 algorithm audit (rejected candidate)

The following is an upstream capability inventory, not an enabled SunlightOS
policy. No `russh` algorithms were compiled or enabled in this workspace.

## Upstream capability

The published crate documents `ssh-ed25519`, `rsa-sha2-256`, `rsa-sha2-512`,
ECDSA NIST keys, Curve25519 (`curve25519-sha256@libssh.org`),
`chacha20-poly1305@openssh.com`, AES-GCM, AES-CTR, and SHA-2 MACs. It also
documents legacy RSA/SHA-1, SHA-1 MAC/KEX, CBC/3DES, and optional compression.

For a future accepted implementation the intended initial policy remains:

| Area | Policy |
| --- | --- |
| host key | Ed25519 only |
| key exchange | Curve25519 only, subject to exact library spelling/interoperability test |
| ciphers | ChaCha20-Poly1305 and/or AES-GCM only |
| MAC | only SHA-2 when a non-AEAD cipher is retained |
| compression | disabled |
| disallowed | `ssh-rsa`/SHA-1, `hmac-sha1`, `hmac-md5`, group1, CBC, and 3DES without an explicit disabled legacy policy |

No OpenSSH negotiation test was run, because the candidate cannot be built on
the SunlightOS target without violating the runtime fork budget.

## Security review relevant to this decision

The current maintained line fixed several 2026 advisories: allocation-first
SSH field parsing in 0.61.0, identification parsing in 0.61.0,
post-decompression bounds in 0.61.1, and an earlier keyboard-interactive
allocation issue by 0.60.1. The audited 0.62.3 is newer than those fixed
versions. Compression would still be disabled from the start as a resource
control.

This does not overcome the runtime rejection, and it does not authorize use of
library defaults. If a future candidate reaches a buildable transport spike,
its exact negotiated KEX/host-key/cipher/MAC names must be captured from a
current OpenSSH client before acceptance.
