# Qalculate! Qt Widgets on Helios

## VFS authority repair and native replay: BLOCKED

The ordinary `./tools/test.sh` gate failed before build or QEMU: 71
inventoried ring-0 VFS references and four additions to the reviewed baseline.
The [authority audit](../filesystems/helios-vfs-authority-audit.md) identifies
each site and the migration dependency. No production paths, guard rules,
baseline, or bundle files changed.

Two guard unit tests and the configured kernel check with
`kfp2a5_transaction_test,helios_runtime_mmap` passed. The feature check is
compile-only. Guest mmap regressions, failure injection, and lifecycle and
resource accounting are NOT_RUN. Full pages beyond EOF still zero-fill
instead of raising SIGBUS.

Runtime binding and all authentic candidate milestones are NOT_RUN, including
Qt Gui discovery, dynamic linking, application entry, Wayland, visible GUI,
calculations, and cleanup. No guest milestone is confirmed for this candidate.
The historical exit 127 applies only to the earlier image. See the
[machine-readable results](helios-qalculate-vfs-results.json) for evidence paths.

## Recursive closure and native validation verdict: PARTIAL

The new candidate resolves **78 unique ELF dependency objects** (including
the interpreter), across 274 edges and 81 objects total including the
executable and two explicitly declared plugin roots. There are zero unresolved
edges, SONAME conflicts or missing version definitions in the declared
profile. The resolver records ELF headers, ABI notes, PT_INTERP, DT_NEEDED
order, SONAME, search paths before and after ORIGIN expansion, version
requirements, segment headers, and aliases. It runs readelf only. Static
validation does not verify actual symbol binding, late runtime discovery,
or guest ABI behavior.

The candidate is 122,816,354 bytes of regular file payload (117.1 MiB)
and preserves 70 symlink hops in the resolved graph. The private Qt libraries
are Core, DBus, Gui, Network, WaylandClient and Widgets, all Qt 6.12.0.
The plugin roots are `platforms/libqwayland.so` and
`wayland-shell-integration/libxdg-shell.so`. The graph includes libqalculate,
GNU libc and the C++ runtime, Fontconfig, FreeType, HarfBuzz, ICU, PCRE2 and
the GL/EGL/X11 libraries required by these particular ELF inputs. The
interpreter and libc match the same glibc package; the pinned backend
payload is owned by libqalculate 5.13.2-1 in the byte-matched package set.
The build version in the historical section below describes its build input.
Staging keeps the manifest unchanged, adds no capabilities, restores package
alias chains and copies the authentic executable without an ELF stamp.
No guest base-image files were added. Existing application, font and
available package license files and package provenance are in `Licenses/`.
Eleven installed package directories lacked package license files; the
provenance inventory flags them with empty lists. Redistribution review
remains open.

Fresh evidence is under `target/helios-qalculate/closure/`:
`candidate/dependency-graph.json`,
`candidate/Qalculate.sunapp/Licenses/provenance.json`, and
`execution-milestones.json`. The staging instructions are in the
[bundle README](../../Qalculate.sunapp/README.md). Two independent staging
runs produced identical graph and provenance reports; the candidate
executable and manifest match their originals byte for byte.

The ordinary boot gate **FAIL** is the first new observed blocker:
`tools/test.sh` stops before build/QEMU when
`tools/audit_vfs_cutover.py --no-growth` reports 71 ring-0 VFS references
and four new references: `syscall.rs:4177`, `linux_procfs.rs:173`,
`process/mmap.rs:272` and `storage.rs:121`. The guard source locations
and exact diagnostics are in `closure/boot.log`. The mmap reference is
the pre-existing private file read path in the supplied worktree, so the
failure cannot be classified solely as unrelated platform drift.
This step did not bypass the guard or run isolated guest diagnostics.
KVM accepted an idle QEMU initialization probe; that is not a guest boot.

Native Wayland fixture replay and authentic candidate execution are
**NOT_RUN**, including entry, relocation, TLS, constructors, Qt,
Wayland, presentation, calculations, cleanup and repeated launch.
There is no new linker exit status. The last confirmed native result is
the historical two-vCPU TCG run below: GNU ld mapped Widgets including its
BSS and exited 127 on missing Gui. Static closure now contains Gui;
native discovery remains unverified.

| Regression | Result | Log under `closure/` |
| --- | --- | --- |
| Static resolver | PASS, 7/7 | `resolver-tests.log` |
| Linux compatibility, ELF loader, kernel transactions | PASS, 30 + 14 + 7 targeted tests | `host.log`, `transaction-host.log` |
| Kernel retirement and memory host library | PASS, 213/213 including the seven transactions | `mm2a-host.log` |
| Filesystem host tests | PASS, 114/114 | `mm-fs-host.log` |
| SunApp launch and manifest host tests | PASS, 10/10 selected | `manifest-host.log` |
| Kernel check: transaction and runtime mmap features | PASS | `feature-check.log` |
| Guest mmap matrix (offsets, privacy, protections, fixed mappings, partial page, BSS, teardown, rollback) | NOT_RUN | Boot gate blocked |

Full pages beyond EOF still zero-fill instead of raising Linux SIGBUS.
No mmap semantics changed in this phase.

Next: resolve the VFS ownership gate through the established VFS authority
and rerun the ordinary boot and Wayland fixture gates. Then validate generic
Helios launch plumbing for an untouched Linux ELF: the existing Qalculate
probe checks for `HLNX01`, its GNU interpreter is a build-time resource,
and its historical image stages only Widgets globally. The new candidate
deliberately uses none of those staging shortcuts. Bind the verified private
runtime through the existing launch and namespace policy, use the native
fixture socket with the existing display service, and replay the real loader,
using KVM if available. Stop at the first demonstrated runtime defect.

## Historical execution and builds

The upstream Qt Widgets UI builds dynamically on the host. In the original
guest image, the static ELF loader requires `ET_EXEC` and rejects
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
python3 tools/helios_qalculate_probe.py "$out/guest.iso" "$out/tcg-replay" tcg \
  --expect-static-loader-rejection
if test -e /dev/kvm; then
  python3 tools/helios_qalculate_probe.py "$out/guest.iso" "$out/kvm" kvm \
    --expect-static-loader-rejection
fi
```

The replay above checks the historical static-loader rejection. By default
the probe checks the child process outcome and exits nonzero on a launch
failure, wait failure, or timeout. It records the ISO digest, spawn and
wait evidence, and a guest scanout capture at the observed outcome. A
successful process exit alone does not verify Qt rendering or calculation.

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

## Replay with Helios C runtime fixtures

A fresh two-vCPU TCG image built with `helios_qalculate_probe,helios_runtime_c_dso`
reached `spawn_entered` for Qalculate and failed at `PT_INTERP policy`. The
debug handoff accepts `/lib/helios/ld-handoff.so.1`, while this executable
requests `/lib64/ld-linux-x86-64.so.2`. No Qalculate process was published.
The image embeds the manifest and executable, and the staged host library
closure remains outside the guest. There is no guest Qt, Wayland window,
or calculation result from this run. KVM was unavailable.

To reproduce the combined image, build the launcher and display service as
above and generate the C runtime fixture artifacts if needed:

```sh
bash tools/helios-dynamic-elf/build.sh c-dso qalculate-replay
fixtures="$PWD/target/helios-runtime-c-dso/qalculate-replay"
out="$PWD/target/helios-qalculate/current-handoff"
services="$PWD/target/x86_64-unknown-none/release"
SUNLIGHT_DYNAMIC_PROBES="$fixtures/bundles" \
SUNLIGHT_HELIOS_HANDOFF_INTERPRETER="$fixtures/ld-handoff.so.1" \
SUNLIGHT_DYNAMIC_PROBE_LAUNCHER="$services/dynamic-elf-probe" \
SUNLIGHT_WAYLAND_SERVER="$services/helios-wayland-fixture" \
SUNLIGHT_QALCULATE_LAUNCHER="$services/qalculate-probe-launcher" \
SUNLIGHT_QALCULATE_ELF="$PWD/target/helios-qalculate/Qalculate.sunapp/Program/qalculate-qt" \
  cargo build -p sunlight-kernel --features helios_qalculate_probe,helios_runtime_c_dso
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel "$out/guest.iso"
python3 tools/helios_qalculate_probe.py "$out/guest.iso" "$out/tcg" tcg
```

The current replay used the existing `matrix` fixture artifacts. See
`target/helios-qalculate/current-handoff/tcg/result.json` and `serial.log`.
Completing this port requires a compatible interpreter and libc/C++ ABI
for the Qt dependency closure, or a Qt and libqalculate build targeting a
guest-supported runtime. The pinned host closure's ABI requirements are
documented in the [runtime inventory](../../tools/helios-dynamic-elf/RUNTIME-ABI-INSPECTION.md).

## GNU interpreter handoff and first Qt library

The debug-only `helios_gnu_interpreter_probe` feature selects the
build-time GNU interpreter for the exact Linux interpreter path. The
kernel maps the executable and interpreter and supplies the startup stack;
GNU ld owns dynamic relocation. The existing fixture path still uses its
bounded kernel relocation logic.

On two-vCPU TCG, GNU ld ran and reported that `libQt6Widgets.so.6`
was missing (`gnu-handoff/tcg/`). A second image staged that one library
at `/usr/lib`, the executable's existing RUNPATH. GNU ld found the file
and requested a mapping with flags `0x802` (private file mapping with
`MAP_DENYWRITE`). Helios rejected it and the child exited 127
(`gnu-dso/tcg/`). The current compatibility translator requires anonymous
memory for private mappings. Supporting the flags alone would be
insufficient: the VM must populate the mapping from the opened file.
The next gate is a private file mapping with correct offsets and
permissions, followed by replay of the real loader's segment mappings.
Neither replay reached the application entry point, Qt initialization,
Wayland, or calculation. KVM was unavailable.

To replay the two variants, reuse the combined-image commands above.
Set `SUNLIGHT_HELIOS_GNU_INTERPRETER` to the staged
`Qalculate.sunapp/Program/lib/ld-linux-x86-64.so.2` and add
`helios_gnu_interpreter_probe` to the kernel features. To include the
first library, also set `SUNLIGHT_QALCULATE_QT_WIDGETS` to the staged
`Qalculate.sunapp/Program/lib/libQt6Widgets.so.6` and add
`helios_qalculate_dso_probe`. Use separate output directories; exit 127
is recorded as a failure by the existing process gate.

## Private file mappings

A fresh two-vCPU TCG replay with the same first-library image inputs
mapped the initial read-only range of `libQt6Widgets.so.6`, followed by
its fixed RX, R, and RW remaps and the anonymous BSS extension. GNU ld
continued to `libQt6Gui.so.6`, which this image does not stage, and the
child exited 127. Qalculate entry, window creation, and calculation remain
unverified. KVM was unavailable.

Helios accepts GNU ld's advisory mmap flags and copies file bytes from
page-aligned offsets into private frames. The existing VM protection and
fixed-replacement transaction apply to those frames, including the
`0x524000` executable remap. The mapping is eager and currently zero-fills
pages beyond EOF, including the reservation tail GNU ld overlays with BSS.
Linux's SIGBUS behavior for accesses to whole pages beyond EOF remains
unimplemented.

Use the same feature and environment settings as the first Qt library
replay above. The new run is under
`target/helios-qalculate/private-mmap/tcg-replay/`. The serial log records
five successful mappings starting at the `0x7a5ac0` initial request,
followed by the `libQt6Gui.so.6` diagnostic. The result JSON records
`child-status` with status 127.

## VFS-G0 follow-up

The ordinary boot gate still fails at the unchanged ownership guard. Prior native qualification of Btop storage telemetry blocks the proposed deferral. The private mmap implementation and its historical replay were retained. Qalculate startup was outside this phase and was NOT_RUN. [VFS-G0 report](../filesystems/vfs-g0.md)
