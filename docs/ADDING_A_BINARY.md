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
