# Phase 0.10: Atomic Private Secret Storage

## Scope and contract

`sunlight_libc::secret_store::SecretStore` is the reusable storage primitive
for future service-private material. It is intentionally format-agnostic:
callers supply bounded bytes plus a validator and choose `CreateIfAbsent` or
`ReplaceExisting`. It is not an SSH writer and does not generate an SSH key.

The current privileged policy is deliberately narrow:

- destinations must be direct children of `/etc/sunlight/`;
- private files must be root-owned `0600` regular files with one link;
- `/etc/sunlight` must be root-owned and not group- or other-writable;
- the caller must have the existing `HostKeyAdmin` service capability;
- the owner in `SecretFileOptions` must match the caller's current UID/GID.

Thus an ordinary root shell does not gain authority merely from UID 0. The
disabled `secret_store_test.service` is the sole in-tree consumer and receives
only `host-key-admin`, `secure-random`, and `logging`.

## Filesystem capability inventory before this phase

| Capability | Previous state | Phase 0.10 state |
| --- | --- | --- |
| file creation | `open(O_CREAT)`; existing object was opened | explicit `O_EXCL` and VFS exclusive creation |
| creation mode | `open` ignored the ABI mode and created `0644` | `open_with_flags_mode` supplies the mode at creation |
| truncate | `O_TRUNC` was exported but not implemented by kernel open | still not relied upon by secret storage |
| no-follow | no symlinks and no follow control | flag is exported; secret kernel operations accept only regular VFS objects |
| close-on-exec | flag absent from public libc | secret staging descriptors are `CLOEXEC`; `exec` closes marked descriptors |
| create-only publish | absent | atomic VFS-locked no-replace private publication |
| replace publish | path rename replaced files | atomic VFS-locked private replacement after metadata validation |
| chmod/chown | path operations existed | not used to make a private staging file safe |
| stat/fstat | both existed | secret helper validates open descriptors with `fstat` |
| unlink/rename | path operations existed | private temp cleanup and publish have narrow kernel operations |
| lstat | absent | no symlinks exist today; future symlink support must add no-follow descriptor open |
| umask | absent | does not affect creation |
| ownership selection | ordinary create used process UID/GID | secret create uses current service credentials and validates them before writing |
| fsync/fdatasync/directory sync | absent | `RequireDurability` fails; no durability claim is made |

`O_TRUNC`, `O_NOFOLLOW`, and `O_CLOEXEC` names now exist in libc. Only
`O_EXCL`, mode-at-create, and `O_CLOEXEC` are part of the completed private
secret path. `O_TRUNC` and `O_NOFOLLOW` must not be used to claim semantics
that the general VFS does not yet implement.

## Path resolution and race analysis

The general VFS has absolute checked paths: it rejects empty paths, relative
paths, trailing slashes, embedded NUL, `.` and `..`. It has no symlinks, hard
links, special objects, or mount aliases in the current RAMFS model. Its file
types are regular file and directory only; RAMFS reports one link for files.
FAT is mounted read-only, so the private write path is RAMFS today.

Ordinary `stat(path)` followed by `open(path)` can race. The secret API does
not use a pre-open stat to authorize the staged file. `SecretCreate` creates
the exact temporary object exclusively with `0600`; the helper immediately
uses `fstat` to verify type, UID, GID, mode, and link count before secret
bytes are written. `SecretPublish` holds the kernel VFS lock while validating
source and destination metadata and performing same-directory rename. This
prevents an interposed path swap within this VFS instance.

Temporary files are direct siblings of their destination:

```text
/etc/sunlight/.<basename>.tmp.<32 secure-random hexadecimal bytes>
```

The random token comes from the Phase 0.9 secure `getrandom` service. Creation
fails closed if random generation fails. Each collision gets a fresh token,
with eight bounded attempts; an existing object is never truncated.

## Publication sequence

1. Validate options, destination grammar, caller ownership, size bound, and
   requested durability.
2. For create-if-absent, validate an existing destination if one is present;
   a valid existing file is retained. A suspicious existing object fails.
3. For replace, load and validate the current destination before creating the
   replacement.
4. Generate a secure same-directory temp name.
5. Exclusively create the temporary regular file with `0600`.
6. Validate its opened descriptor and metadata before writing.
7. Complete a partial-write-safe loop; zero progress fails.
8. `fstat` final size, validate secret bytes, and close. Close failure prevents
   publication.
9. Atomically publish with VFS-locked same-directory rename:
   `CreateIfAbsent` uses no-replace; `ReplaceExisting` validates and replaces
   a regular private destination.
10. On every pre-publish failure, close and best-effort remove only the exact
    temporary path. Secret buffers are volatile-zeroed best effort.

Concurrent creators produce one winning destination. The loser receives
`CreateResult::Existing`, deletes its candidate, and should load the retained
secret. A create race does not rotate or overwrite the winner.

## Existing writers and confirmed unsafe patterns

The audit found no current production SSH private-key or TLS private-key file
writer. TLS trust anchors are stored through `sunlight-kv`; the existing
`sunlight-tls` service does not persist a private server key.

The following legacy paths remain outside this phase and must not be confused
with `SecretStore`:

- `services/sunlightd/src/main.rs` persists service enablement through the
  predictable `enabled-services.tmp`, creates with default permissions and
  chmods later, then uses replace rename.
- `services/sunlight-uac/src/bin/uac_service.rs` migrates `/etc/shadow` by
  writing its final destination directly through VFS IPC. This is
  secret-adjacent and non-atomic; it needs a separate migration to a policy
  permitting the secret storage primitive outside `/etc/sunlight`.
- `services/sunlight-thumbd/src/main.rs` uses predictable thumbnail temp paths
  and replace rename, but thumbnails are not private secrets.
- editors, wallpaper configuration, storage-manager writes, and other
  configuration writers use general create/truncate/rename APIs and are
  outside private-secret scope.

Before Phase 0.10, `open(O_CREAT)` created through the VFS as `0644`, ignored
the supplied mode, and had no exclusive semantics. Existing `rename` replaces
an existing regular destination in RAMFS; VFS rejects cross-mount rename, so
it has no copy/delete fallback. Rename changes the namespace atomically while
the live RAMFS is running. Open handle identity is not POSIX-stable in the
old RAMFS replacement implementation and must not be relied upon by secret
consumers; the secret helper closes staging descriptors before publish.

## Crash, buffers, and stale files

Atomic visibility is supported. During a running system, post-publication
readers see a complete old or complete new secret, never a partial destination.
There is no `fsync`, `fdatasync`, or directory sync. A process crash before
rename leaves the old destination and can leave a temp file. A crash after
rename normally leaves one complete version during that boot. Host crash, VM
termination, power loss, or storage loss can retain either complete version,
lose recent contents, or lose rename metadata. No durable atomic replacement
is claimed.

Future durable ordering is:

```text
write temp -> fsync temp -> close -> rename -> fsync parent directory
```

`Durability::RequireDurability` fails now. `DurableWhenSupported` currently
has atomic-visibility behaviour and must be reported as limited by callers.

RAMFS resets on boot, so stale files do not survive a reboot but can survive a
service crash during a running boot. `cleanup_stale_temps` only considers
matching private temp grammar, verifies type/owner/mode through metadata, and
never publishes stale contents or deletes unknown siblings.

Secret bytes exist transiently in the caller buffer, a bounded kernel write
buffer, RAMFS data, and possible compiler/VFS copies. The helper clears its
caller-owned temporary buffers with volatile writes where practical. It does
not claim complete zeroization of compiler temporaries, allocator reuse,
shared-memory copies, or filesystem cache.

## Validation and manual test status

Focused host unit coverage includes exclusive creation, atomic no-replace
publication, replacement rejection that preserves old bytes, path policy, and
CLOEXEC descriptor-table behavior. The normal bare-metal target cannot run a
Rust `std` test harness, so tests run with
`CARGO_BUILD_TARGET=x86_64-unknown-linux-gnu`.

The disabled `secret_store_test.service` exercises secure random opaque test
bytes, create-if-absent, validated load, explicit replacement, stale-temp
cleanup, and reload. It prints only safe outcome strings and never prints
bytes, hashes, fingerprints, or random tokens.

The Phase 0.9 QEMU boot gate passed on July 20, 2026. VMware and the manual
secret-service acceptance scenarios remain required to finish platform
validation. The secret test service is deliberately disabled until a test gate
can start it under its narrowly scoped capability.
