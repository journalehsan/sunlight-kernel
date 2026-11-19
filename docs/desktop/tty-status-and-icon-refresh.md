# TTY status and launcher icon refresh

## Implementation

The native tabbed terminal is `services/tty_server`, not the windowed
`services/sunlight-terminal`. Its old header hardcoded CPU to 15%, cached RAM
and time by UTC minute, and always appended `eth0`. The idle renderer also
updated only once per minute and skipped periodic status rendering while a
foreground program was running.

The header now reuses the TTY server's existing `sunlight_telemetry::Telemetry`
instance and mapped kernel page. `sunlight-top` re-exports this same crate, so
CPU accounting and RAM definitions agree. There is no second sampler, service,
or dependency on launching top. CPU needs a valid interval baseline; missing
baseline, missing mapping, zero RAM denominator, and samples more than three
seconds old render `--`. RAM is bounded to 0–100%. Timezone failure now produces
an explicit UTC clock instead of an empty clock. Both clocks include seconds.

Maintenance prepares a fixed 64-byte header at most once per monotonic second.
Rendering only copies that buffer. Visible TTY maintenance redraws once per
second even when a foreground application is idle, preserving its grid and
prompt ownership. Inactive TTY sessions do not refresh or redraw the header.
The existing renderer, framebuffer selection, resolution negotiation, pointer,
login, tab storage, and session-switch protocol are retained.

Network state reuses the desktop's typed `sunlight_networkd::NetworkClient`
every two seconds: `up`, `config`, `down`, `none`, or `--`. These describe the
service's interface/configuration state, not verified Internet reachability.
An error immediately drops the previous state; cached state expires after three
seconds. The client validates and bounds interface enumeration. Its temporary
vector is allocated per network refresh, never per frame. CPU/RAM sampling and
header formatting allocate nothing. No optional header click actions were added.

## Icons and labels

Overview uses overlapping window outlines with title bars in the existing
16-pixel glyph renderer. Start retains its filled four-tile Applications glyph;
Search retains its magnifier; Settings retains its gear. Overview's tooltip is
`Overview`, and its existing workspace-switcher action is unchanged.

Network actually opens System Preferences (the Settings application); it does not open a network-location browser.
Its stored desktop tooltip metadata now says `Network settings`, correcting that metadata
mismatch. Its asset is the existing theme's blue globe,
`places/symbolic/network-workgroup-symbolic.svg`, replacing
`preferences/22/yast-nfs-server.svg`. SVG parsing/caching still happens through
`DesktopThemeIcons` at initialization. Failure uses a connected-node bitmap in
the existing glyph renderer. Home, Files, Trash, Start and Search retain their
current assets. These labels follow the shell's existing English tooltip
convention; its locale support formats dates/time, rather than translating
launcher strings. No raster assets or new icon infrastructure were introduced.

## Validation

Host:

- `python3 tools/test_tty_status_host.py`: PASS. Compiles the production header
  formatter with deterministic adapters; checks cadence, CPU warm-up, stale and
  unavailable telemetry, every network state, and service loss.
- Telemetry library: 12 passed; TTY library: 42 passed.
- UI library: 277 passed.
- Vortex shell with `native-svg`: 67 passed, one failed:
  `sidebar::tests::telemetry_missing_before_first_sample_is_unavailable`.
  The same failure is documented as baseline in
  `docs/session-manager/desktop-login-tty-switching-ux.md`.
  The added launcher identity, asset parsing, and Network fallback test passes.
- Networkd library: 4 passed after adding the missing test-only `alloc::vec`
  macro import. The initial compile failure is retained in
  `target/tty-status-host.log`; the corrected run is
  `target/tty-status-network-host.log`.
- Native TTY, native SVG shell, and production kernel builds succeed.

Logs are under `target/tty-status-*.log`; generated artifacts are not committed.
Native matrix and final verdict are recorded below after completion.

## Performance and remaining limitations

The refresh policy is bounded and avoids per-keystroke telemetry/clock IPC.
Networkd still uses its existing synchronous bounded IPC client, outside the
render path: lookup has a 50 ms deadline and each of at most eight interface
requests has a 40 ms deadline. Under service failure this can delay event-loop
maintenance; this change does not introduce an asynchronous network transport.
Timezone lookup/request deadlines are each 10 ms and fall back to UTC.
Rendering reuses the existing whole-terminal redraw path once per second;
partial framebuffer damage optimization is outside this milestone.
Native diskless validation has no configured Ethernet device, so `NET none`
is expected; it cannot prove actual external connectivity. Host adapters cover
connection/configuration/error transitions. CPU percentages across independent
tools are interval averages, so exact simultaneous numeric equality is not
expected. No performance benchmark or connected-network native fault injection
is claimed.

## Native evidence

`bash tools/test_desktop_tty_ux.sh` passed all three diskless QEMU guests:

- `target/desktop-session-ux/desktop/result.json`: 18 checks, including Desktop
  default login, explicit TTY authentication, two tabs, 20 alternating left/right
  Ctrl round trips, no duplicate session/backing allocation, terminal state,
  Start, Search, multiple games, Run, and no background TTY command execution.
- `target/desktop-session-ux/recovery/result.json`: three checks, including fresh
  TTY-first login, preserved tabs, and functioning Ctrl+F1 and shell input after
  controlled compositor termination.
- `target/desktop-session-ux/limine/result.json`: two checks covering the standard
  VGA framebuffer fallback and TTY-first/first-Desktop tab preservation.

Each directory includes `serial.log`, `command.json`, and PNG scanout captures.
`desktop/02-desktop.png` visually shows the globe, overlapping-window Overview,
four-tile Applications, magnifier Search and existing Home icon. The TTY capture
`desktop/05-tty-two-tabs.png` shows measured CPU/RAM and the live clock with
`NET none`, consistent with the guest's lack of configured Ethernet.

The focused gate is `tools/tty_status_icon_gate.py`. Exploratory attempts are
retained under `target/tty-status-icons*` and `target/tty-status-top-network-native`.
They encountered scanout cursor timing, small-text OCR misses, and an incorrect
harness expectation that the Settings window title was “Settings” rather than
“System Preferences”; these runs are not counted as passes. Their PNG captures
visually confirm the respective actions. The gate now uses bounded cursor
retries, cropped OCR, and the application's actual title. Pointer clicks still
traverse real PS/2 input; no shell UI action injection is used.

`target/tty-status-top-network-native/04b-top-network-popover.png` shows the new
top-bar globe and its functioning Network popover.
`target/tty-status-top-network-native/05-network-target.png` shows the desktop
globe launching System Preferences. Final-source replay evidence is recorded
below. `target/tty-status-image-hashes.txt` records native image SHA-256 values.

## Top-panel Network follow-up

The panel used a different asset from the desktop shortcut:
`ICON_SYM_LAN_TGA`, generated from a rack-like bundled font glyph. The panel now
reuses the existing `icon_public.tga` globe, keeping the normal tinted symbol
size and all connection-state colors, hover/focus, hit region and Network
popover behavior. If parsing fails, the panel uses the connected-node glyph;
the state dot is rendered for either path. The icon host regression also checks
that the panel actually selects the globe asset.

The production gate command
`SUNLIGHT_TEST_TIMEOUT=180 bash tools/test.sh phase3.0` passed, including its
native serial expectations. Evidence is `target/tty-status-production-gate.log`.

## Final-source replay and verdict

`python3 tools/tty_status_icon_gate.py target/tty-status-release.iso --out
 target/tty-status-release-native` completed with exit zero and **PASS** for all
five checks:

1. Applications and Overview open distinct surfaces.
2. The top-bar globe opens the existing Network popover.
3. The desktop globe opens System Preferences.
4. CPU/RAM/NET fields render and the clock advances while idle.
5. System Preferences and TTY output survive a desktop/TTY round trip.

Evidence: `target/tty-status-release-native/result.json`, `serial.log`,
`command.json`, and captures `01-desktop-icons.png` through
`09-tty-preserved.png`. In particular, `04b-top-network-popover.png`,
`05-network-target.png`, and `06-tty-status.png` / `07-tty-status-updated.png`
cover the requested visual changes and live clock. The final normal kernel
uses `--no-default-features`; `target/sunlightos.iso` is the same normal image
as `target/tty-status-release.iso`. No source commits or pushes were made.

**Overall verdict: PARTIAL.** The requested visual changes, native interaction
checks, all three existing session matrices, and production boot gate pass.
The full shell host suite still has its documented pre-existing sidebar test
failure. Strictly nonblocking network refresh is not established: the reused
network client retains bounded synchronous IPC, as described above. Connected
Ethernet operation and service-failure input latency need separate native
coverage before claiming the full responsiveness requirement without limits.
