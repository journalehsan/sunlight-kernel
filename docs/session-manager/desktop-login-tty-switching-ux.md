# Desktop-first login and native TTY switching

## Scope and ownership

Desktop is the default selection in `sunlight-tty::login::LoginScreen`. The
second selection is **Terminal (TTY)**. Space, Up/Down and the existing pointer
dropdown action toggle the selection; Enter authenticates through UAC. The
initial frame now renders this same state, instead of the compatibility
renderer's formerly hardcoded TTY label. No new login screen, account policy,
startup service, privilege, or boot delay is introduced.

`sunlight-kbd` decodes IRQ1 Set-1 bytes, tracks each modifier side, and sends
hardware events to `tty_server`. TTY intercepts Ctrl+F1/F2 before forwarding
input to `sunlight-display`; the compositor then dispatches other shortcuts and
focused-window input. SessionD owns authenticated session creation and desktop
shell supervision. It does not intercept hardware shortcuts.

## Root cause and repair

The previous Ctrl+F1 handlers already received global keyboard input before
application focus routing. They sent `SESSION_DEACTIVATE` and rendered TTY into
the Limine framebuffer. Deactivation only changed `session_active`; the VirtIO
scanout stayed attached to desktop resource 1. Thus text input could switch
while QEMU continued displaying the last desktop frame. Desktop activation's
one-time scanout guard also assumed TTY never selected another GPU surface.

Native TTY/login now allocates and attaches **resource 3** once, using the same
GPU-reported dimensions as Desktop. Resource 1 remains Desktop and resource 2
remains its cursor. Kernel image provenance chooses the resource; userspace
cannot supply another service's resource ID. TTY selects its resource on return
and flushes its own rendered frames. Desktop reselects resource 1 on activation.
Neither switch reallocates a surface or replaces the other environment's
backing. TTY selects scanout after the bounded deactivation request finishes,
including failure; its completion marker follows the text redraw and flush. TTY keeps its direct Limine renderer when VirtIO is unavailable. It can
select and render its VirtIO surface without a functioning compositor.

The hardware stream's register/pop/unregister syscalls now require the
root-launched embedded keyboard service identity. TTY checks the kernel-stamped
IPC badge against that identity before handling keys. Display accepts keyboard
and activation/deactivation messages only from the kernel-loaded TTY service.
These checks use existing provenance validation; names and client event payloads
cannot establish authority. Existing exec/account-member restrictions withdraw
service identity. Cursor operations remain display-only.

A bounded two-key latch consumes held shortcut repeats and the matching release.
Plain F1/F2 pass through. A bare function key held before Ctrl still delivers
its original release to its application. A second verified routing bug let the
Shell branch forward desktop events, then fall through into the native ASCII
line editor outside that forwarding block. Native text decoding now requires
TTY to be active; desktop typing cannot edit or execute a hidden TTY command.
The regression test exercises text, Enter, control keys, releases and commits.
On deactivation Display drops stale queued keys,
releases held keys to the focused client, then resets compositor keyboard state.
Window order, ownership, workspace, logical focus and application processes stay
intact. The hardware decoder continues tracking real left/right modifier releases.
There is no per-key heap allocation, registry rebuild, new input polling, logout,
application suspension, or service restart in switching.

## TTY lifecycle and first Desktop activation

The TTY remains one console with up to ten internal tabs. Its shell processes,
input buffers, output, viewport, geometry and active-tab index are retained.
Ctrl+F1 returns directly to existing tabs when present; otherwise it displays the
secure login form. It never creates a tab as a side effect of switching.

An existing managed desktop is restored directly by Ctrl+F2. On a fresh TTY-only
login, the previous code activated the compositor without a SessionD desktop
handle. That path now presents the same secure form, focused on Password and
preselected Desktop. Fresh authentication creates a managed desktop through the
existing UAC grant and SessionD path; the consumed TTY proof is never replayed.
TTY tabs stay intact through that first Desktop login. Ctrl+F1 can cancel the
form and return to them. Successful login clears the password buffer.

Desktop startup errors retain the text environment. Failed display activation
reselects the TTY surface; ordinary switching never starts a desktop restart loop.

## Launcher and shortcuts

Start draws four native rounded application tiles using the panel theme's text
color. The existing button bounds and dock spacing remain unchanged. Hover and
open state highlight the button; its tooltip reads **Applications**, following
the shelf's existing English tooltip convention. No raster asset is added.
Start still opens the application menu. Its keyboard handler now accepts the
native UI's `Event::Key('\n')` / carriage-return representation of Enter and
uses the same activation handler as raw Enter. Previously only `KeyPress`
activated a tile, so native Enter could leave the filtered menu open. A focused
unit test covers native Enter and verifies the menu closes after selecting
Sunlight Empires. Start and Search keep their existing
catalogs, shared `AppId` values and common `open_app_from_ui` launch path,
including native SunApp resolution. No additional application index is created. A separate verified Search-chord
routing bug let the palette intercept K-up before the compositor cleared its
consumed-chord latch; the next Super+K could then be swallowed. The existing
release suppression now runs before desktop-overlay routing, with a focused
regression test. Query retention and Search matching remain unchanged.

| Shortcut | Action |
|---|---|
| Ctrl+F1 | Native TTY or its secure login form |
| Ctrl+F2 | Preserved Desktop; secure first Desktop login from a fresh TTY session |
| Super alone | No existing application-menu binding was found; none added |
| Super+R / Ctrl+Space | Existing Run dialog |
| Super+K | Global Search, including over a focused application |
| Ctrl+K | Search when no application owns focus; otherwise application-local |
| Ctrl+T / Ctrl+W in native TTY | Create / close an internal tab |
| Ctrl+1…9 / Ctrl+0 in native TTY | Select tabs 1…9 / 10 |

The login footer also displays Ctrl+F1/F2. Session shortcuts do not overlap TTY tab
shortcuts. Legacy kernel keyboard-injection gates explicitly choose TTY when
needed; desktop gates exercise the new default.

## Reproduction and evidence

Build freestanding keyboard, TTY and Display with the usual service linker flags;
build the native SVG shell with `bash tools/build_svg_apps.sh -p
sunlight-vortex-shell`. Build the normal kernel and an isolated hybrid ISO.
Do not treat a build as a native test result.

```sh
cargo test -p sunlight-tui -p sunlight-tty -p sunlight-kbd -p sunlight-sessiond \
  --lib --target x86_64-unknown-linux-gnu
cargo test -p sunlight-display --target x86_64-unknown-linux-gnu --no-default-features
cargo test -p sunlight-vortex-shell --features native-svg --target x86_64-unknown-linux-gnu
python3 tools/desktop_tty_ux_gate.py target/session-ux-final-validation.iso
python3 tools/desktop_tty_ux_gate.py target/session-ux-final-validation.iso --tty-first \
  --out target/desktop-tty-first
python3 tools/desktop_tty_ux_gate.py target/session-ux-final-validation.iso --failure \
  --out target/desktop-tty-recovery
```

`bash tools/test_desktop_tty_ux.sh` builds all changed services and runs the three
isolated native scenarios above. It stops on any build error and therefore
cannot validate an old service binary after a failed rebuild. Other embedded
services must already exist from a normal repository build.

The native gate requires Python/Pillow, ImageMagick, Tesseract, QEMU and OVMF; no
new production dependency or privileged tooling is added. It uses QMP hardware
key events and real PS/2 mouse events, captures
QEMU scanout, and retains serial output and JSON results under `target/`.
`--failure` terminates only the compositor in an isolated guest with no writable
persistent disk, then checks Ctrl+F1 and actual text-shell input.

## Validation results

Validation on 2026-11-18 used fresh service binaries and a production kernel
without keyboard-injection features. Builds alone are not counted as tests.

| Check | Result |
|---|---|
| Keyboard decoder host tests | PASS, 4 tests; both Ctrl sides and function-key releases |
| Native TTY library host tests | PASS, 42 tests; includes chord debounce, login selection and inactive-seat input isolation |
| SessionD library host tests | PASS, 27 tests |
| TUI host tests | PASS, 14 tests; includes tab hit targets and capacity |
| Native Terminal host tests | PASS, 28 tests |
| Start host tests | PASS, 9 tests; includes native Enter launching Empires |
| Search host tests | PASS, 19 tests |
| Compositor host suite | 77 passed / 6 failed, plus 2 library tests passed; both new routing/handoff tests pass |
| Complete shell host suite | 66 passed / 1 failed; new Enter regression passes |
| Native desktop/session matrix | PASS, all 18 recorded checks |
| Fresh TTY-first login, VirtIO | PASS; preserves both tabs through first authenticated Desktop activation |
| Controlled compositor termination | PASS; Ctrl+F1 selects native scanout and executes `echo recoveryalive` |
| Fresh TTY-first login, Limine without VirtIO | PASS; existing text fallback and desktop return work |
| Production Phase 3.0 gate | PASS with existing `SUNLIGHT_TEST_TIMEOUT=180` override; preceding default 60-second run ended with empty serial output |
| Legacy TTY-login gate | PASS on final-source replay with 180-second timeout and its standard 2 GiB guest; all expected serial markers matched |

The native desktop matrix covers default Enter login, explicit TTY login,
20 consecutive round trips alternating Ctrl sides, two retained TTY tabs and an
unfinished command, Welcome, idle/unfocused desktop, Terminal input, Start,
Search over a focused game, Sunlight Empires from both launchers, Silicon Echoes,
Rappid Rabbit, multiple open applications and the Run dialog. Plain F1/F2 do not
switch. Its last capture verifies desktop queries never reached the background
TTY. The normal matrix contains no kernel panic, exception or fault markers.

Final native evidence:

- `target/desktop-session-last/result.json`, `serial.log`, `command.json` and
  captures `01-login-default.png` through `28-no-background-tty-input.png`.
- `target/desktop-recovery-last/result.json`, its serial log and captures through
  `08-independent-recovery-tty.png`.
- `target/desktop-limine-last/result.json`, its serial log and five captures.
- Host logs: `target/session-ux-host-review.log`, `session-ux-terminal-review.log`,
  `session-ux-display-last-host.log`, `session-ux-shell-review.log`,
  `session-ux-start-last.log` and `session-ux-search-last.log`.
- Production gate: `target/session-ux-final-production-replay.log` and
  `target/session-ux-final-production-replay-serial.log`.
- Final TTY gate: `target/session-ux-tty-last-gate.log`,
  `target/session-ux-tty-last-serial.log` and exit code zero in
  `target/session-ux-tty-last-exit.txt`.

The same six compositor failures were reproduced with `HEAD`'s original Display
source: ambient-shadow falloff, two compact-control hit tests, inactive-workspace
hit testing, maximize/restore hit testing, and move/resize/hide damage. The shell's
`sidebar::tests::telemetry_missing_before_first_sample_is_unavailable` failure was
also reproduced with the original shell source. Baseline comparison logs remain
at `target/session-ux-display-baseline-host.log` and
`target/session-ux-shell-baseline-host.log`. These failures were not changed as
part of session switching.

The legacy TTY-login gate also missed its `whoami -> root` serial-output marker
in earlier runs, although `whoami` launched and exited zero, `id` returned root,
and the tab/sysfetch/hostname expectations passed. Native interactive commands
and identity handoffs pass independently. The final-source replay passed every
expectation, including `whoami -> root`; earlier failing runs remain retained
and are not counted as passes. The 180-second override affects the test deadline,
not boot services or their startup ordering.

After testing, the normal kernel was rebuilt with `--no-default-features` and
`target/sunlightos.iso` regenerated. It has no automatic test keystrokes. Build
logs are `target/session-ux-normal-kernel.log` and
`target/session-ux-normal-iso.log`. The final native test image and source hashes
are recorded in `target/session-ux-evidence.json`.

## Resource observations and limits

At 1280×800, TTY reports 4,096,000 bytes of page-aligned backing, allocated once.
The final native 20-round-trip run recorded no additional GPU attachment, TTY shell
creation, or SessionD session creation during switching. This is object-count
and fixed-backing evidence, not a claim that all OS memory remains constant.
The GPU mode is sampled at startup, matching the desktop's existing behavior;
live host-window resize is not implemented. TTY accepts at most 16 MiB of GPU
backing and otherwise uses its boot-framebuffer fallback.

## Remaining limitations and verdict

Light-theme and high-DPI screenshots were not separately exercised; the icon
uses scalable native drawing and the panel's theme text color. Dynamic GPU mode
changes, device loss after initialization and compositor-resource recreation
were not redesigned. Controlled runtime compositor termination was tested;
allocation-failure injection was not added. Native success is functional and
lifecycle evidence, not an exhaustive memory-safety proof.

**Final verdict: PARTIAL.** All requested native session, recovery and launcher
scenarios pass. The complete host regression suites retain seven confirmed
baseline failures. Both boot gates and the complete native UX matrix pass on
the final production changes. No unrelated stacking, filesystem, network, game,
or authentication-policy repair is bundled with this milestone.
