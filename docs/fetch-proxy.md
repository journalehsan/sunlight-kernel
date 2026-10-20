# Fetch through a SOCKS proxy

Fetch supports an explicit SOCKS5 proxy for HTTP and HTTPS. The proxy resolves
destination hostnames, including redirected destinations. HTTPS verifies the
destination's certificate and hostname using the normal trust store.

For QEMU user networking, reach a SOCKS listener on the host using `10.0.2.2`:

```sh
fetch --socks5-hostname 10.0.2.2:9050 -o /tmp/archlinux.html https://archlinux.org
```

The proxy endpoint must be a numeric IPv4 address and nonzero port. The current
implementation supports the SOCKS5 no-authentication method. Each invocation
chooses its route explicitly; fetch uses direct networking when the option is
absent.

If a direct fetch reports `code=14` with `SynSent->Closed`, TCP failed before
the TLS handshake. Installing a CA or disabling certificate verification cannot
repair that connection. Check direct access from the host, then check the proxy:

```sh
curl --noproxy '*' --connect-timeout 10 -I https://archlinux.org
curl --proxy socks5h://127.0.0.1:9050 --connect-timeout 10 -I https://archlinux.org
```

If only the second command succeeds, use the guest command above. Port 9050
must have a running SOCKS listener on the host; the fetch option does not start
one. Using `--socks5-hostname` also lets the proxy resolve the destination,
avoiding a failing local DNS route.

Rebuild both fetch and sunlight-tls when updating the image. Their extended
connection request sends the full TLS hostname in shared memory: the native
register IPC transport carries four words, which left only sixteen hostname
bytes in the original connection request. The daemon still accepts that original
request from older clients.

The deterministic gate uses temporary local HTTP, HTTPS, and SOCKS servers. It
checks certificate rejection, trust installation, repeated HTTPS downloads,
proxy rejection, HTTP/HTTPS tunnelling, and a verified hostname longer than
sixteen bytes without a local DNS lookup:

```sh
python3 tools/network_tls_gate.py --cpus 12 --uefi
```

With the host's SOCKS listener running, append `--arch-proxy 10.0.2.2:9050`
to also verify the real Arch HTTPS download from the guest.

Serial evidence and a packet capture are retained in `target/network-tls/`.

## Direct HTTPS with a DNS override

When DNS returns an unreachable or incorrect address, fetch can connect directly
using an explicit hostname/port override:

```sh
fetch --resolve archlinux.org:443:209.126.35.79 -o /tmp/archlinux.html https://archlinux.org
```

This address was verified on 2026-10-20 and can change. Obtain a current address
from a trusted source when needed. The override preserves the URL hostname for
TLS certificate verification, SNI, and the HTTP Host header. It applies only to
the specified hostname and port, including redirects to that same destination.
Repeat `--resolve` for additional destinations. It cannot be combined with the
SOCKS option. This is an explicit workaround for DNS problems, not a permanent
system DNS repair.

Validate direct guest access with:

```sh
python3 tools/network_tls_gate.py --cpus 12 --uefi --arch-resolve 209.126.35.79
```
