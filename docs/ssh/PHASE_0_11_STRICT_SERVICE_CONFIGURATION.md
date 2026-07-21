# Phase 0.11: Strict Service Configuration

## Scope

`sunlight_libc::ssh_config` is the fail-closed configuration foundation for
the future `sunlight-sshd` daemon. This phase adds no SSH transport,
authentication, host-key generation, listener, `sunlightd` unit, live reload,
or filesystem watcher. Consequently `sunlight-ssh` remains absent and disabled.

The system image supplies `/etc/sunlight/ssh.toml`:

```toml
listen_address = "0.0.0.0"
port = 22
host_key_file = "/etc/sunlight/ssh_host_ed25519_key"
password_authentication = true
max_auth_attempts = 3
max_connections = 8
max_sessions_per_connection = 1
login_timeout_seconds = 30
```

All eight fields are mandatory and must appear once. Fields are case-sensitive.
Unknown fields—including `enabled`, `autostart`, `start_on_boot`, and
`service_enabled`—are errors. Service enablement belongs solely to
`sunlightd`; this file cannot alter it.

## Parser and Types

No maintained TOML crate currently exists in the no-std SunlightOS workspace.
The loader therefore implements a deliberately narrow strict subset, not
arbitrary TOML: UTF-8, comments, whitespace, basic single-line quoted strings
with `\"`, `\\`, `\n`, `\r`, and `\t`, decimal integers, exact `true`/`false`,
and top-level `key = value` assignments. Tables, arrays, inline tables,
dotted keys, multiline strings, hexadecimal integers, and interpolation are
rejected.

`RawSshConfig` contains decoded source values. It is immediately converted to
`ValidatedSshConfig`, which uses four IPv4 octets, `NonZeroU16` limits, a
checked monotonic `Duration`, and `ValidatedAbsolutePath`. Future host-key,
listener, PTY, and authentication code must only accept the validated snapshot.

`ValidatedSshStartup` enforces this startup order:

```text
read and validate configuration
-> load or create host key
-> create and bind listener
-> publish ready
```

Invalid configuration calls no dependency operation, so it cannot reach
host-key access, socket allocation, bind, listen, or readiness notification.

## File Policy

The complete file is read into one bounded 16 KiB buffer before parsing. Empty
files, oversize files, short reads, growth during read, invalid UTF-8, and
close failures are rejected. The loader checks the parent before opening, then
checks the opened descriptor with `fstat`.

The file must be a root-owned single-link regular file with no group or other
write bit. Thus `0600`, `0640`, and `0644` are accepted; `0664`, `0666`, and
other group/other-writable modes are rejected. The parent must be a
root-owned directory without group or other write access.

The current VFS represents only files and directories, with no symlinks, hard
links, or special files. Although libc exposes `O_NOFOLLOW`, general VFS
no-follow semantics are not implemented; this loader does not claim them.
Future symlink support must add descriptor-based no-follow open before this
policy is extended.

When the future daemon is explicitly started, a missing configuration produces
`FileMissing` and prevents all startup side effects. The daemon must not create
the file or use compiled defaults. While the service is disabled or absent,
this path has no boot effect.

## Validation

- `listen_address`: numeric IPv4 only; wildcard and loopback are accepted,
  multicast and `255.255.255.255` are rejected. DNS, IPv6, CIDR, interface
  names, and address-plus-port syntax are unsupported.
- `port`: integer `1..=65535`. Network bind authority remains a separate
  capability check for the future listener.
- `host_key_file`: non-empty lexical absolute path directly under
  `/etc/sunlight/`, without traversal, trailing slash, NUL, or equality with
  the configuration path. Phase 0.10 validates the runtime private-key object.
- `max_auth_attempts`: `1..=10`.
- `max_connections`: `1..=8`.
- `max_sessions_per_connection`: exactly `1`.
- `login_timeout_seconds`: `1..=300`, represented as a monotonic duration.

The network stack supports 128 TCP slots and a 32-entry wait set. The current
PTY server has 16 sessions; Phase 0.11 reserves eight for Terminal and system
use, leaving eight SSH sessions. The loader also checks supplied TCP reserve,
wait-set, and PTY budgets with checked multiplication. Impossible combinations
are rejected; values are never clamped.

## Diagnostics and Restarts

`ConfigError` carries an error kind, optional line/column, typed field,
duplicate first-definition line, bounded unknown-field name, and a
missing-field bitset. It never retains the raw buffer or values.

The future daemon must load one immutable snapshot at process startup. Editing
the file cannot alter active connections; a restart is required. Once a unit
exists, commands use the existing service-manager syntax:

```text
sunlightctl restart sunlight-ssh
sunlightctl status sunlight-ssh
```

Restart success requires the replacement process to validate the complete
configuration, finish local startup dependencies, bind, and publish readiness.
Malformed replacement configuration leaves the new process stopped and creates
no partial listener.

## Current Limits

The current syscall ABI reports generic file errors only, so production `stat`
failure on this fixed configuration path is surfaced as `FileMissing`; future
errno-rich VFS support should distinguish access and I/O errors. There is no
SSH daemon or listener adapter yet, so local-address availability, privileged
port authority, QEMU/VMware service-start validation, and timing or peak-memory
measurements remain prerequisites for the future consumer.
