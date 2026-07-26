# Adding a User-Space Binary to SunlightOS

SunlightOS embeds every user-space program into the kernel image via
`include_bytes!` and resolves it by **absolute path** at spawn time. There is no
on-disk `/bin` — the RamFS only holds tiny *stub* files whose sole job is to make
the shell's PATH probe succeed. Miss any one of the spots below and the binary
either fails to build, returns **"not found"**, or never starts.

## The wiring matrix

How many spots you touch depends on what kind of binary it is:

| Spot | File | `/bin` command (e.g. `nicectl`, `capabilityctl`, `runas`) | Daemon (e.g. `uac_service`, `niced`) |
|------|------|:--:|:--:|
| 1. Workspace member | root `Cargo.toml` | ✅ | ✅ |
| 2. Build script | `tools/build.sh` + `tools/test.sh` | ✅ | ✅ |
| 3. Embed bytes | `kernel/src/main.rs` (`*_ELF_BYTES`) | ✅ | ✅ |
| 4. Path resolver | `kernel/src/process/spawn.rs` (`embedded_bytes_for_path`) | ✅ | ✅ |
| 5. **RamFS stub** | `sunlight-fs/src/ramfs.rs` (`INITRAMFS`) | ✅ **required** | ❌ not needed |
| 6. Service unit + spawn | `services/sunlightd/src/main.rs` | ❌ | ✅ (if sunlightd-launched) |

> **The #1 gotcha:** a `/bin` command needs spot **5**. The shell checks the VFS
> for an existing file *before* it ever calls `exec`, so without the stub it
> prints "not found" even though the embed + resolver are perfectly correct.
> Daemons spawned directly by path (kernel or sunlightd) skip the stub because
> nothing PATH-probes them.

## The six spots in detail

### 1. Workspace member — root `Cargo.toml`
Add the crate under `members`. One crate can ship several `[[bin]]`s (see
`services/sunlight-uac`, which builds `uac_service`, `capabilityctl`, `runas`).

Each user-space crate also needs `.cargo/config.toml` targeting
`x86_64-unknown-none` with the linker script, **or** it must be built with
`RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static"`.
Plain `cargo build` of userland yields a `SegmentOutOfRange` panic at spawn.

### 2. Build scripts — `tools/build.sh`, `tools/test.sh`, `tools/runs.sh --build`, and image writers
Add a build line in *Step 1* (services are built before the kernel because the
kernel `include_bytes!`es their output):
```sh
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package <crate> --release
```
Building the package builds **all** its bins, so multi-bin crates need only one
line. Mirror the same line into `tools/test.sh` (redirected to `$BUILD_LOG`).
If you use `./tools/runs.sh --build` as your normal workflow, add the same line
there too so the embedded binary exists before the kernel compile starts. Any
script that rebuilds the kernel from embedded user-space artifacts should carry
the same package build line as well; today that also includes `tools/write.sh`.

### 3. Embed bytes — `kernel/src/main.rs`
Add a static near the other `*_ELF_BYTES` entries:
```rust
static NICECTL_ELF_BYTES: &[u8] =
    include_bytes!("../../target/x86_64-unknown-none/release/nicectl");
```
The path is the *bin name*, not the crate name. This errors at compile time
("couldn't read … No such file") until the binary has been built once.

### 4. Path resolver — `kernel/src/process/spawn.rs`
Add an arm to `embedded_bytes_for_path`:
```rust
"/bin/nicectl" | "/usr/bin/nicectl" => Ok(crate::NICECTL_ELF_BYTES),
```
This is what `exec` uses first (before falling back to the on-disk VFS), so the
path string here must match what the shell will hand to `exec`.

### 5. RamFS stub — `sunlight-fs/src/ramfs.rs`  *(`/bin` commands only)*
Add a stub `RamEntry` to `INITRAMFS` so the shell's PATH probe finds the file:
```rust
RamEntry::file("/bin/nicectl", 0, 0, mode::FILE_755, b"#!/sunlight/nicectl\n"),
```
The file content is a cosmetic marker — resolution is path-based via spot 4 — but
the entry **must exist** or the command is reported as not found. Add `/bin/...`;
add `/usr/bin/...` too only if you want it reachable there as well.

For `sunlight-hangman`, wire both PATH-visible locations so the shell can find
it no matter which directory comes first:
```rust
RamEntry::file("/bin/hangman", 0, 0, mode::FILE_755, b"#!/sunlight/hangman\n"),
RamEntry::file("/usr/bin/hangman", 0, 0, mode::FILE_755, b"#!/sunlight/hangman\n"),
```

### 6. Service unit + spawn — `services/sunlightd/src/main.rs`  *(daemons)*
For a daemon that `sunlightd` should launch, add a unit string in `load_units()`
(with `ExecStart=/sbin/<name>`, plus `After=`/`Requires=` as needed) **and** add
the path to the spawn loop in `_start`, mirroring `timezone_service`/`niced`/`gcd`.
Daemons spawned by the kernel itself (init/vfs/tty) are wired in the kernel boot
path instead.

## Quick checklists

**New `/bin` command** → spots 1, 2, 3, 4, **5**.

For graphical desktop apps such as `eyes`, `sunlight-terminal`, `sunlight-tasks`, and `sunlight-files`, the same rule applies: they still need workspace membership, build wiring, kernel embed bytes, resolver arms, and the `/bin` stubs.

### Current native desktop reference: `silicon-echoes`

`sunlight-silicon-echoes` builds the `silicon-echoes` native graphical binary.
It is wired through all five `/bin` command locations, including
`tools/runs.sh --build`, and provides both `/bin/silicon-echoes` and
`/usr/bin/silicon-echoes` RamFS stubs. The Vortex Start Menu registers it as
**Silicon Echoes: 1993** in the Games category; desktop launch and shell aliases
(`silicon-echoes`, `silicon`) both resolve to `/bin/silicon-echoes`.

**New sunlightd-launched daemon** → spots 1, 2, 3, 4, **6**.

**New init-launched daemon** → spots 1, 2, 3, 4, plus add its absolute
`/sbin/...` path to `services/init/src/main.rs`. If the same crate also ships a
CLI, wire that CLI as a normal `/bin` command too.

Example: `sunlight-resolved` builds both `/sbin/resolved` and `/bin/resolvectl`.
That means one package build line, two `include_bytes!` statics, two
`embedded_bytes_for_path` arms, an init service entry for `/sbin/resolved`, and
RamFS stubs for `/bin/resolvectl` and `/usr/bin/resolvectl`.

Another common shape is a daemon + CLI in one package. `sunlight-clipd` builds
both `/sbin/sunlight-clipd` and `/bin/sunlight-clip`, so it needs one package
build line, two embedded binaries, two spawn resolver arms, a sunlightd unit
for the daemon, and a RamFS stub for the CLI.

### Session lock reference: `mezzo` + `mezzoctl`

`mezzo` is an **init-launched** session-lock policy service (`/sbin/mezzo` in
`services/init/src/main.rs`). Desktop login calls it to establish a lock session;
without a running mezzo, F2/GraphicalDesktop never activates after login.

`mezzoctl` is the recovery CLI (`mezzoctl lock activate|status|recover`).

Wiring checklist for this pair:

| Spot | `mezzo` (daemon) | `mezzoctl` (CLI) |
|------|:--:|:--:|
| 1. Workspace | `services/mezzo` | `mezzoctl` |
| 2. Build scripts | `tools/build.sh`, `test.sh`, `runs.sh --build`, `write.sh` with `SERVICE_RUSTFLAGS` | same |
| 3. Embed | `MEZZO_ELF_BYTES` | `MEZZOCTL_ELF_BYTES` |
| 4. Resolver | `/sbin/mezzo` | `/bin/mezzoctl`, `/usr/bin/mezzoctl` |
| 5. RamFS stub | ❌ (init path-spawns) | ✅ `/bin` + `/usr/bin` |
| 6. sunlightd unit | ❌ (init owns it) | ❌ |
| Init list | ✅ `INIT_SERVICES` | ❌ |

**Linker gotcha:** never build these with bare `cargo build -p mezzo` from the
workspace root. The workspace default `rustflags` use the *kernel* linker script
and produce `SegmentOutOfRange` at spawn. Always:

```sh
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package mezzo --release
# SERVICE_RUSTFLAGS includes -Tservices/user-space.ld
```

`kernel/build.rs` also refuses to embed a kernel-linked userspace ELF (entry
outside `0x400000..0x0000_8000_0000_0000`) and rebuilds it with the correct
flags.

## Chronos DOS bundles

Chronos guest applications are not native ELF binaries. A `.sunapp` bundle
contains a DOS `.COM` or real-mode MZ `.EXE` under `Program/`, and the native
`sunlight-chronos` adapter loads it as interpreted guest code. Do **not** add a
guest executable to `kernel/src/process/spawn.rs`.

The default DOS terminal is
`ChronosDosShell.sunapp/Program/SUNSH.EXE`. Its Pascal source is
`guest/sunshell/sunshell.pas`; it is a 16-bit i8086 MS-DOS MZ executable.

To make a bundled Chronos application available in a normal SunlightOS boot:

1. Add the bundle directories and every required file to
   `sunlight-fs/src/ramfs.rs` using `include_bytes!`. The bundle root must be
   mounted at `/Applications/<Name>.sunapp`.
2. Ensure `Manifest.toml` has `runtime.type = "chronos"` and a `C:\` entry
   pointing at a file below `Program/`.
3. Route the launcher command or start-menu entry to the bundle path, for
   example `/Applications/ChronosDosShell.sunapp`. `sun-exec` validates the
   manifest and launches `/bin/sunlight-chronos` with the scoped bundle roots.
4. Build the native `sunlight-chronos` adapter before the kernel; it remains a
   normal embedded ELF binary.

The Chronos DOS Terminal is launched from the desktop/start menu through the
`sunlight-dos-terminal` alias. It displays the guest-side `CMD C:\>` prompt,
not a host-native command interpreter.

### Rebuilding `SUNSH.EXE`

The checked-in MZ executable lets ordinary Rust builds run without Free Pascal.
When the i8086 cross-compiler and its matching MS-DOS RTL are installed, rebuild
the guest asset with:

```sh
PPC8086=/path/to/ppc8086 \
FPC_I8086_RTL=/path/to/rtl/units/msdos \
./guest/sunshell/build.sh
```

The deterministic compiler command is:

```sh
ppc8086 -Tmsdos -Pi8086 -Mtp -WmLarge -Wh -Xs -O1 \
  -Fu/path/to/rtl/units/msdos -FD/usr/bin -XP \
  -oChronosDosShell.sunapp/Program/SUNSH.EXE \
  guest/sunshell/sunshell.pas
```

`./tools/runs.sh --build` checks `PPC8086` and `FPC_I8086_RTL`: when both are
set it rebuilds `SUNSH.EXE` before compiling the initramfs; otherwise it uses
the checked-in executable and continues normally. After boot, open **Sunlight
DOS Terminal** from the Compatibility section or run:

```text
sun-exec /Applications/ChronosDosShell.sunapp
```

## Worked example: the three "not found" binaries

`nicectl`, `capabilityctl`, and `runas` all returned "not found" because they
were missing spot 5. `nicectl` was missing 3, 4, and 5 (it built but was never
embedded at all). The fix:

- `sunlight-fs/src/ramfs.rs` — stubs for `/bin/nicectl`, `/bin/capabilityctl`, `/bin/runas`.
- `kernel/src/main.rs` — `NICECTL_ELF_BYTES`.
- `kernel/src/process/spawn.rs` — `/bin/nicectl` resolver arm.

(`capabilityctl` and `runas` already had spots 3 and 4 from the `sunlight-uac`
crate extraction; they only needed the stub.)

## Phase 2B.5 utility example

The standalone `tr`, `paste`, `join`, and `printf` utilities are already built,
embedded, and mapped by the kernel resolver. They still need `/bin` RamFS
stubs so the shell's PATH probe can find them:

| Binary | Release ELF size |
|--------|------------------:|
| `tr` | 24,944 bytes |
| `paste` | 20,856 bytes |
| `join` | 24,952 bytes |
| `printf` | 20,840 bytes |

Add one executable stub per utility to `sunlight-fs/src/ramfs.rs`:

```rust
RamEntry::file("/bin/tr", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
RamEntry::file("/bin/paste", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
RamEntry::file("/bin/join", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
RamEntry::file("/bin/printf", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
```

Without these entries, the shell reports the command as **not found** even
though the ELF is present in `target/x86_64-unknown-none/release/` and the
kernel has a matching resolver arm.

## Phase 2B.7A utility batch

The POSIX-oriented `tee`, `nl`, `od`, and `split` commands are standalone
bins in the `sunlight-utils` package. Adding one of these commands requires
the same five command spots:

1. Add a `[[bin]]` entry and source module under `sunlight-utils/src/`.
2. Keep the package build in `tools/build.sh`, `tools/test.sh`, `tools/runs.sh
   --build`, and `tools/write.sh`; one package build produces all four ELFs.
3. Add its `include_bytes!` entry to `kernel/src/main.rs` and its explicit
   recovery build entry to `kernel/build.rs`.
4. Map `/bin/<name>`, `/usr/bin/<name>`, and `/sunlight-utils/<name>` in
   `kernel/src/process/spawn.rs`.
5. Add the executable `/bin/<name>` RamFS stub in `sunlight-fs/src/ramfs.rs`.

Each utility keeps its behavior in a testable library module and uses the
shared bounded native-argv startup path. The current supported baseline is:

| Binary | Baseline behavior |
|--------|-------------------|
| `tee` | stdin to stdout and files; `-a` append |
| `nl` | numbered input; `-ba`, `-n`, `-s`, `-v`, `-i`, `-w` |
| `od` | byte/word display; `-b`, `-c`, `-d`, `-o`, `-x`, `-t`, `-A`, `-j`, `-N` |
| `split` | line/byte chunks; `-l`, `-b`, `-a`, input and prefix operands |

The Phase 2B.5 expected-output file remains the deterministic boot gate for
this batch; add command exit markers there whenever the injected sequence is
extended.

## Phase 2B.7B pipeline batch: find → xargs → grep → sort → uniq

These five utilities form the common search/filter/order pipeline. `grep`,
`sort`, and `uniq` were already standalone from Phase 2B.4. `find` and `xargs`
are new standalone bins in `sunlight-utils` and need the same five command spots:

1. `[[bin]]` + library module under `sunlight-utils/src/`
2. Package build line (one package build emits all ELFs)
3. `include_bytes!` in `kernel/src/main.rs` + recovery entry in `kernel/build.rs`
4. Resolver arms for `/bin/<name>`, `/usr/bin/<name>`, `/sunlight-utils/<name>`
5. Executable `/bin/<name>` RamFS stub in `sunlight-fs/src/ramfs.rs`

| Binary | Baseline behavior |
|--------|-------------------|
| `find` | path walk; `-name`, `-type f\|d`, `-print`, `-maxdepth` |
| `xargs` | stdin tokens to command; `-n`, `-0`/`-d`, `-r`; default `/bin/echo` |
| `grep` | pattern search (Phase 2B.4) |
| `sort` | line ordering (Phase 2B.4) |
| `uniq` | adjacent dedup (Phase 2B.4) |

`find` must **not** remain on the multicall `SUNLIGHT_UTILS_ELF_BYTES` arm once
the dedicated binary exists — the shell PATH probe still uses the RamFS stub,
but `exec` resolves through the dedicated `FIND_ELF_BYTES` mapping.
