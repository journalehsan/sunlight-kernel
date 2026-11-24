# Qalculate! Qt Widgets on Helios

The upstream Qt Widgets UI builds dynamically on the host. The first guest
blocker is ELF execution: Helios requires `ET_EXEC` and also rejects
`PT_INTERP`. The Qt 6 build is a glibc `ET_DYN` PIE and requests
`/lib64/ld-linux-x86-64.so.2`. Relinking with `-no-pie` would only move the
rejection to the interpreter check. See `sunlight-elf/src/lib.rs` and
`kernel/src/process/elf_loader.rs`. The guest probe uses the real
`sun_exec::launch` path with a GUI/Helios manifest and the original Wayland
fixture. It embeds just the manifest and Helios-stamped executable in RAMFS; adding
shared libraries to the guest cannot affect this first failure.

## Host build and packaging

- UI: upstream `Qalculate/qalculate-qt` commit
  `04a765857db5ac9a3589340a53e25c92d693ab0a` (project version 5.13.0).
  No application source changes. Existing installed libqalculate 5.13.1
  (`libqalculate.so.23`) supplied the headers, pkg-config data and library.
- Toolchain: GCC 16.2.1, `x86_64-pc-linux-gnu`, glibc 2.44, Qt 6.12.0.
  Qt 5.15.19 development files are also installed. No repository Qt sysroot
  or build configuration was found. The host ELF uses the x86-64 Linux ABI;
  its own version requirements reach GLIBC_2.38 and GLIBCXX_3.4.32. The
  transitive libraries have their own requirements. The host C++ ABI
  resolves at load inspection; guest C++ execution was not reached. The
  separate native C++ runtime (`tools/build_native_cxx.sh`) does not
  provide the GNU/Linux dynamic ABI required by this executable.
- Qt 6 plugins: `platforms/libqwayland.so` and
  `wayland-shell-integration/libxdg-shell.so` are staged. Qt 5 has separate
  generic, EGL and XComposite Wayland plugins. Changing Qt major versions
  does not solve dynamic ELF rejection.
- The staged host closure contains only the two selected plugins and the
  ELF dependencies resolved from the executable, those plugins and
  libqalculate. It contains GLX/OpenGL/EGL and X11 libraries required by
  the installed Qt build, despite excluding the XCB platform plugin and
  graphics integration plugins. No Qt Quick or QML is required by this
  closure. This is a host dependency snapshot, not the requested
  portable, graphics-free guest deployment. The executable and some host
  DSOs retain their host RUNPATH; the staged loader listing prioritizes
  the copied libraries. It does not execute application code.
- Upstream icons, flags and UI translations are embedded via qrc, with a
  separate bundle icon. The host has Qt translations and C/C.UTF-8
  locales. The candidate stages one Liberation Sans font and the installed
  libqalculate XML/JSON data, including local exchange rates.
  `QALCULATE_DEFINITIONS_DIR` is available for redirecting definitions.
  Fontconfig setup, bundle-relative data lookup and locale behavior have
  not been exercised in the guest. Upstream still links Qt Network and
  includes clipboard code; no currency update or clipboard test was run.

The successful build sequence, from `target/helios-qalculate`, was:

```sh
mkdir -p build/translations
(cd build && qmake6 ../source/qalculate-qt.pro COMPILE_RESOURCES=yes)
cp build/translations/*.qm source/translations/
(cd build && make -j4)
```

The shadow build first failed because the translation output directory
did not exist and then because the qrc expected generated QM files beside
the source. The two staging steps resolve those build issues without
changing tracked upstream files.

Evidence under `target/helios-qalculate/` includes `build*.log`,
`executable-readelf.log`, `host-dependencies.log` (including libqalculate's
dependencies), `library-origins.txt`, `staged-loader-list.log`,
`bundle-contents.txt` and `SHA256SUMS`. The candidate is staged at
`Qalculate.sunapp/` within that directory. The trace contains no Qalculate
Wayland requests unless a child actually runs. The unchanged fixture's
baseline globals are `wl_compositor`, `wl_shm` and `xdg_wm_base`.

## Guest replay

From the workspace root, after staging the host executable:

```sh
out="$PWD/target/helios-qalculate"
install -Dm644 Qalculate.sunapp/Manifest.toml "$out/Qalculate.sunapp/Manifest.toml"
install -Dm755 "$out/build/qalculate-qt" "$out/Qalculate.sunapp/Program/qalculate-qt"
bash tools/stamp_helios_elf.sh "$out/Qalculate.sunapp/Program/qalculate-qt"
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p helios-wayland-fixture -p sunlight-display --release \
  --features helios-wayland-fixture/qalculate-probe,sunlight-display/helios_wayland_test
services="$PWD/target/x86_64-unknown-none/release"
SUNLIGHT_WAYLAND_SERVER="$services/helios-wayland-fixture" \
SUNLIGHT_QALCULATE_LAUNCHER="$services/qalculate-probe-launcher" \
SUNLIGHT_QALCULATE_ELF="$out/Qalculate.sunapp/Program/qalculate-qt" \
  cargo build -p sunlight-kernel --features helios_qalculate_probe
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel "$out/guest.iso"
python3 tools/helios_qalculate_probe.py "$out/guest.iso" "$out/tcg-replay" tcg
if test -e /dev/kvm; then
  python3 tools/helios_qalculate_probe.py "$out/guest.iso" "$out/kvm" kvm
fi
```

The probe is a rejection test. Its screenshot is a scanout capture after
the launch attempt, not a Qalculate window. The calculation and its before
and after screenshots are gated on a successful launch. No protocol or
transport work is warranted by an ELF failure.

On both two-vCPU KVM and TCG, the stamped bundle passed manifest validation
and resolved to its declared executable. The serial trace records
`spawn_entered`, `[ELF] header rejected: NotStaticExecutable`, and
`SpawnFailed(Failed)`. No child was created, so there is no application
exit code. Qt Core/Gui/Widgets and the platform plugin never loaded, no
Qalculate Wayland globals were requested or received, and there was no SGP
window to remove. See `kvm/` and `tcg-replay/` for the commands, serial
logs, result JSON, and `after-rejection.png` captures.

The initial two-vCPU replays returned `UnsupportedRuntime` before spawn.
The launcher requires a registered Helios bundle and the `HLNX01` ELF marker.
The probe adds the registry entry behind its own feature and stamps a copy
with the existing helper. The raw executable is preserved. Both attempts'
logs are retained in `{kvm,tcg}-first-validation/`.

## Existing controls

The controls were replayed from unchanged retained ISOs on two-vCPU TCG.
These runs do not provide a fresh all-green baseline:

| Control | Replay outcome |
| --- | --- |
| Direct Canvas | Both expected frames were captured. The display owner reaper removed the window, but the gate timed out waiting for the explicit removal marker. |
| Canvas SunApp | Manifest and spawn succeeded; registry dispatch failed, the child returned 1, and the gate timed out waiting for the graphical desktop. |
| Original Wayland fixture | The gate timed out waiting for the graphical desktop; no successful window replay was recorded. |
| Interactive Canvas, standalone libwayland and transport gates | Not rerun. Prior standalone SCM_RIGHTS, shared pool and mmap outcomes are preserved under `target/helios-wayland-lib/regressions/`. |

Serial logs, commands and captures from the control reruns are under
`target/helios-qalculate/controls/`. The control sources and gate criteria
were unchanged.

The smallest next milestone is a separate Helios dynamic ELF startup
probe: a tiny Linux C++ PIE using a compatible interpreter and one shared
library, launched through a SunApp manifest. Preserve the existing static
loader and prove interpreter mapping, auxv, relocation and TLS before
bringing in Qt. After that, configure a guest Qt Widgets runtime whose
Wayland path uses raster buffers without linking the host graphics
stack; supply the session endpoint from the launcher and verify Qt's
actual global requirements before changing the bridge.
