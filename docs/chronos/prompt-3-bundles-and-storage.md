# Chronos Prompt 3: Bundles and DOS Storage

## Bundle contract

An unpacked external bundle ends in `.sunapp` and contains `Manifest.toml`, a
`Program/` root, optional `Dependencies/`, and bundle-local resources. Version
one accepts only `runtime.type = "chronos"` and `bundle.format = 1`.

`sun-exec /absolute/path/App.sunapp` reads at most 8 KiB of `Manifest.toml`,
validates the stable app id, display name, icon relative path, and `C:\` entry
path, and rejects host-absolute paths, `..`, unknown runtime types, missing
entries, and unsupported formats. The entry is always resolved below
`Program/`; bundle resource references remain bundle-relative.

The launch descriptor is represented by `ApplicationLaunchRequest` and
`RuntimeKind` in `sunlight-libc::sun_exec`. Native app-id resolution continues
to use the existing `sun-exec` path.

## Direct and bundled launches

A bundle launches the native `sunlight-chronos` adapter with bounded metadata:
bundle root, resolved program entry, app id, display name, and (when granted)
the current user's Documents root. A direct absolute `.COM` argument also
dispatches to Chronos, uses its containing directory as a read-only C: base,
uses a temporary in-memory overlay, and keeps the existing `Chronos - Sunlight
DOS Terminal` user-facing mode.

The current window API only accepts static titles, so the Chronos client renders
the bundle display name in its own header. Dynamic dock/task icon propagation
requires a display/task metadata ABI extension and is intentionally not faked.

## Drive model

Each `Runtime` owns a `DriveTable` and a DOS handle table. Guest code sees only
C:, D:, and T: DOS paths; no host path or file descriptor reaches guest memory.

- **C:** Read-only Program/ plus Dependencies/ base maps and a private overlay.
  Overlay entries take precedence. Writes copy base entries up, deletion creates
  tombstones, and rename copies to the overlay then hides the base source.
- **D:** The manifest's document permission grants the user Documents root only
  for `read-write`. `none` and `read-only` are mounted read-only in this v1
  adapter. Host paths outside the supplied root are not imported.
- **T:** Exists as a per-runtime writable in-memory mount. It is discarded when
  the runtime is destroyed.

For bundled C: overlays, the native adapter persists imported overlay files to
`$HOME/.config/sunlight/chronos/<app-id>/overlay`. The bundle stays unchanged.
Tombstone persistence and full host-backed directory capability checks are the
next hardening step; they are not claimed as complete here.

## DOS compatibility surface

The `chronos-core` DOS layer implements current-drive selection, per-drive
current directories, DTA set/get, mkdir/rmdir/chdir/getcwd, create/open/close,
read/write/seek/delete/rename, basic attributes, and Find First/Find Next.
Errors set CF and return documented DOS error values in AX. Handles 0, 1, and 2
map to Chronos console behavior; regular files start at 5. PSP command tails
are bounded to 126 bytes and end in CR.

DOS paths use case-insensitive uppercase names, backslash parsing, drive
relative versus drive absolute behavior, and root-escape rejection. Directory
searches are bounded and DTA output contains attributes, DOS-clamped timestamp
placeholders, size, and an opaque host-side result index. `*`, `?`, `*.TXT`, and
`*.*` match guest-visible names deterministically.

## Current limitations

This vertical slice uses deterministic uppercase guest names rather than a
persistent long-name 8.3 alias index. The bundled example and imported files
must therefore already be DOS-safe names. Read-only document permission is
parsed but lacks a host-read-only capability distinction in this initial
adapter. Timestamps are deterministic DOS placeholders because the current
userland stat ABI has no timestamps. There is no installer, archive support,
child process support, MZ support, or dynamic shell/task identity transport.
