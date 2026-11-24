# Helios infrastructure baseline — November 24, 2026

Scope: transport, static execution and the two-frame Wayland lifecycle.
No loader, bridge, manifest, test assertion, or timeout changes were made.
Qalculate! and Canvas input were not rerun.

## Checkout and reconstruction

HEAD was `089850a5e05adadc97101d50a855fe1c3fd3d178`, with seven modified
files and existing untracked dynamic ELF and Qalculate probe work.
`target/helios-baseline-2026-11-24.MQ0n4Y/inventory/` contains the exact
starting status, tracked binary patch, untracked file hashes, prior audit
and evidence directory inventory, host versions and command logs. In the
rest of this document, **E** is that uniquely named baseline directory;
**T** is `E/checkout/target`.

The copied checkout inherits the repository's Cargo config. The first build
exposed an unstaged `helios-note` prerequisite; it was built and stamped
using the commands in `tools/build.sh` (see `inventory/helios-note.log`).
The initial copy also loaded the kernel linker script twice through Cargo's
ancestor config merge; the resulting section overlaps were isolation
errors. Moving the duplicate config to the inventory restored the single
script and static relocation flags. Early failed attempts remain in the
inventory. Historical scripts, sources and artifacts were left in place.

Kernel builds use `x86_64-unknown-none`, dev profile (panic abort, opt-level
1); fixture and display services use release (panic abort, opt-level 3,
LTO, one codegen unit, strip) with the service linker script,
`x86-64-v2`, static relocation and no red zone. Standalone client
builds use static non-PIE C/musl and the Helios stamp; the Canvas Rust
library uses release on `x86_64-unknown-linux-musl`. The host uses Rust
1.98.0-nightly (`8954863c8`), Cargo 1.98.0-nightly (`0b1123a48`) and
QEMU 11.1.3. The host and guests are x86-64.

The packaged images were generated in the isolated checkout; the audited
fixtures were built by their runners except where explicitly noted.
The repository's normal kernel build can reuse valid embedded services.
Unrelated top-level freestanding release executables, Yazi and Limine were
seeded from the original target directory, with service hashes recorded
before test builds. This is a fresh image baseline with identified retained
inputs, not a build of every embedded program from scratch. The initramfs
is embedded in the kernel; the ISO and packed kernel hashes identify it.

`qemu-system-x86_64 -accel help` lists TCG, MSHV, Nitro and KVM. The
character device `/dev/kvm` exists and is accessible. The probe
`timeout 2 qemu-system-x86_64 -accel kvm -smp 2 -machine q35 -nodefaults -display none -S`
returned 124 because QEMU stayed alive until the timeout. The matrix uses
two vCPUs. TCG replay means a fresh boot of the same ISO without rebuilding;
it does not use QEMU deterministic record/replay.

## Entry points and evidence

Run commands from `E/checkout`. The inventory's `*.cmd`, traced `*.log`
and `*.exit` files identify the exact attempts and build environment.
Transport traces include the exact QEMU command. GUI and static diagnostic
run directories contain `command.json` with the QEMU arguments and
`serial.log`. GUI captures come from the repository's QMP gate.

| Entry point | Arguments and launch path |
| --- | --- |
| `bash -x tools/test_native_unix_listener.sh` | No positional arguments; `SUNLIGHT_TEST_ACCEL=tcg\|kvm`; embedded native/Helios fixtures start at boot. |
| `bash -x tools/test_fd_shared_mmap.sh MODE` | `mmap` (default), `scm-rights`, `pool`; `SUNLIGHT_TEST_ACCEL`, `SUNLIGHT_TEST_OUTPUT`; embedded fixture pair starts at boot. |
| `bash -x tools/test_helios_wayland.sh tcg kvm` | Accelerator list; static embedded client starts at boot. |
| `bash -x tools/test_helios_libwayland.sh [--canvas] tcg kvm` | Standalone or direct Canvas; `--canvas-input-elf` is a build-only input variant, out of scope. |
| `bash -x tools/test_helios_canvas_sunapp.sh ACCEL` | `tcg` or `kvm`; optional `--input` excluded. Launcher invokes `/Applications/SunlightCanvas.sunapp`; script verifies a pinned client hash before staging. |
| `python3 tools/helios_wayland_gate.py ISO OUT tcg [MODE]` | Same-ISO replay. Default static fixture; explicit `libwayland`, `canvas`, or `canvas-sunapp`. The GUI gate submits Return, `root`, Return at login and waits for the graphical session. |
| `bash -x tools/helios-dynamic-elf/build.sh`; `python3 tools/helios-dynamic-elf/replay.py ACCEL` | Existing diagnostic containing the static non-PIE SunApp. `tcg` writes `tcg-replay`; `kvm` writes `kvm`; `--saved` validates existing logs and was not used. |

## Matrix

The status cells describe this run. Evidence paths are relative to **T**
unless stated otherwise.

| Test | TCG | KVM | TCG replay | Evidence and comparison |
| --- | --- | --- | --- | --- |
| Native named AF_UNIX listener | passed | passed | unavailable | `native-unix-listener-{tcg,kvm}-retry/`; exchange, close, refusal and rebind match the listener audit. |
| Wrong-UID listener rejection | passed | passed | unavailable | Same runs; non-root native registration denied, wrong-UID Linux connect refused and queue left empty. |
| Shared mmap coherence and invalid-map checks | passed | passed | passed | `mmap-{tcg,kvm}-retry/`, `mmap-tcg-replay/`; same PFN across personalities, bidirectional update, invalid maps rejected. |
| Original global-page mmap, final post-exit read and waitpid | passed | passed | passed | Same files; final read proves the original wait assertion passed. Earlier TCG assertion and KVM no-child did not recur in these attempts. |
| Single-FD SCM_RIGHTS | passed | passed | unavailable | `scm-rights-{tcg,kvm}-retry/`; transfer, mapping update, read-only rights and queue discard match the SCM_RIGHTS audit. |
| Multipage memfd pool | passed | passed | unavailable | `pool-{tcg,kvm}-retry/`; unchanged from the pool audit, including independent mappings of different sizes. |
| FD lifetime and cleanup | passed | passed | unavailable | Same SCM_RIGHTS and pool runs; close preserves live mappings, unmap frees the two objects and queued-reference discard returns to zero live pages/objects. |
| Static non-PIE ELF and static SunApp launcher | not rerun | passed | passed | `helios-dynamic-elf/{kvm,tcg-replay}/`; one static payload covers ELF execution and manifest launch. TCG ran through the repository replay entry point only. |
| Static child allocation, exit, waitpid and reclamation | not rerun | passed | passed | Same diagnostic; spawn published PID 747 (KVM) and 744 (TCG), child printed the expected line, exited zero and returned `Ok(0)`; 517 user frames and 8 page tables reclaimed for each PID. Matches earlier diagnostic. |
| Dynamic diagnostic rejection controls | not rerun | passed | passed | Same diagnostic; unchanged expected ELF rejections: PIE `NotStaticExecutable`, dynamic ET_EXEC `HasDynamicSection`. Neither published a child. |
| Original static Wayland fixture | passed | passed | passed | `helios-wayland/{tcg,kvm,tcg-replay}/`; all three captured both frames and removal for window 2. The retained-image desktop timeout did not recur. |
| Standalone libwayland | passed | passed | failed | `helios-wayland-lib/{tcg,kvm,tcg-replay}/`; replay first failure: **scanout cleanup**. |
| sunlight-ui Canvas Wayland / original direct Canvas control | failed | passed | failed | `helios-wayland-lib/canvas/{tcg,kvm,tcg-replay}/`; TCG and replay first failure: **Wayland protocol**. These are the same direct client and gate, so no duplicate runs are counted. |
| Canvas SunApp with newly rebuilt ELF | blocked | blocked | not rerun | `inventory/canvas-sunapp-{tcg,kvm}.log`; first block: **initramfs/bundle installation**, before staging, because the new ELF fails the existing input digest pin. |
| Canvas SunApp with original pinned ELF | failed | passed | passed | `helios-sunapp-pinned-tcg/{tcg,tcg-replay}/` and `helios-sunapp-pinned-kvm/kvm/`. First TCG failure: **Wayland protocol**. |
| Qalculate! rejection (historical only) | not rerun | not rerun | not rerun | See `helios-qalculate.md` and the original `target/helios-qalculate/` evidence. The stamped binary reached ELF validation and no child was published. |
| Canvas input | not rerun | not rerun | not rerun | Out of scope; prior input evidence is unchanged. |

The listener, SCM_RIGHTS and pool scripts rebuild on each invocation and
have no retained-ISO replay option. The global-page mmap replay uses the
historical two-vCPU TCG command and the original marker and PFN checks
against the fresh TCG ISO; see `mmap-tcg-replay/command.log`.

The fresh libwayland TCG and KVM results match the earlier passing
two-frame runs. Both have matching SGP window creation ID 2, two redraw
markers, explicit removal and all three PNGs. The new same-image TCG
replay produced the two frames, then the display owner reaper recorded
one removed window for PID 716. The gate timed out waiting for the explicit
`[WL-SGP] removed window=` marker and never took the after-disconnect
capture, so the replay fails. This reproduces the removal-evidence
failure shape in the retained direct-Canvas control, for a different
client; it does not prove both have the same cause.

The freshly built direct Canvas client has SHA-256
`5c7b5b51a6281b96506f252fb97054334a7f01bc40a4f3db10ee7a2f2420b314`.
KVM has matching window create/redraw/removal for ID 2 and all three
captures; this fills the prior KVM evidence gap in the Canvas audit.
The initial TCG log records login success followed by client registry
dispatch failure (`errno=0`); the harness reported the missing
GraphicalDesktop marker. On replay the desktop marker was present, but
the bridge logged `FAIL desktop unavailable` and the client failed
registry dispatch with `errno=32`. Neither TCG attempt produced frames.
These attempts stop earlier than the retained direct-Canvas control,
which drew both frames and then lacked the explicit removal marker.

The historical KVM `no-child` issue remains open. The recorded
failure in `target/mmap-regression-kvm/serial.log` was an instrumented
wait after the child exited and its process slot was reused. The fresh
KVM mmap and static launcher each passed their wait assertion; neither run
establishes that it exercised that interleaving. The two TCG mmap
observations also do not resolve the original failure.

The pinned SunApp control deliberately reused the archived ELF
(`577ac203166af6171f5ae2789a4a9da9d39ae40dc6d801188288703a8995ba9e`)
in freshly built guests. The first TCG launch reached spawn, failed
registry dispatch (`errno=0`) and returned `Ok(1)` to the launcher,
reproducing the failure described in the Qalculate audit. The same ISO
passed TCG replay with three captures, explicit removal and waitpid
`Ok(0)`, matching the earlier successful SunApp replay. KVM passed
with the same evidence and fills the KVM gap in the original SunApp
audit. The window creation trace for KVM is interleaved with the
kernel phase marker; the creation event, ID 2 and registration are
visible in serial. Neither passing run validates the freshly built
ELF through the SunApp runner.

## Evidence integrity and next milestone

The follow-up [Wayland stabilization audit](helios-wayland-baseline-fix.md)
records a separate evidence root, preserves the failures above, and tracks
the loader decision against the new matrix.

`E/inventory/artifacts.sha256` lists ISOs, fixture ELFs, bundle files,
and both PNG and raw frame hashes. `packed-kernel.sha256` hashes the
embedded-initramfs kernel extracted from each ISO. `bundles.sha256`
hashes each sorted per-file SHA-256 manifest; these are directory
identities, not archive hashes. `boot-inputs.sha256` records firmware
and Limine inputs. The indexed commands and per-run capture evidence
distinguish a new build, a same-image replay, and a new image with a
pinned retained ELF.

The first build attempts are historical within this baseline: the
missing unstaged input and duplicate-linker failures classify as
**guest build** and did not reach QEMU. The final matrix reports the
separate corrected attempts. Earlier retained ISOs and the old direct
Canvas screenshots describe their own builds only; no retained GUI ISO
was used to fill a fresh pass. The main checkout's tracked diff and
pre-existing untracked file hashes still match the starting inventory.

**Decision C: split out a focused Wayland baseline reliability task.**
Make the Canvas SunApp artifact pin reproducible from source and
investigate registry and removal-marker timing on the same two-vCPU
TCG image. Keep the historical waitpid race open as a separate
lifecycle issue. The next loader milestone after that baseline work is
trusted interpreter lookup and transactional dual-image mapping, with
rollback and the static controls preserved. This baseline does not
establish dynamic ELF execution.
