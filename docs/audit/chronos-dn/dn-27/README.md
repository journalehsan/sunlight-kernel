# DN-27: Shift selection in the Select → File mask dialog

This audit separates the guest's source of modifier state, the state Chronos
exposes, and the text selection drawn by DOS Navigator 1.51.

## Baseline and binary evidence

The baseline replay boots the [archived DN-26 native ISO](baseline-image.sha256)
at 4 GiB with two vCPUs, then launches the original SunApp from desktop search.
From DOS Navigator, press **F10 → M → N → C → Tab → Down → Enter** to open
COLORS. Exercise **Ctrl+F3 → Escape**, then press **keypad +** to reach
**Select → File mask**. Clear the initial `*.*` with Home/Delete and type
`abcdef`. Plain Right and Left move the caret without selecting. On this
baseline, both Shift keys also move the caret without selecting:
[20 scanout checks](baseline-scanout.log) read the six on-screen cell backgrounds.
This is a visible failure despite correct arrow delivery.

The [pinned source](source.sha256) declares `ShiftState: Byte absolute
$40:$17` in `ADVANCE.PAS:294`. In `DIALOGS.PAS:827,973–983,1087`,
the input line checks the navigation scan code, masks the two Shift bits,
and extends the selection from its anchor. `DRIVERS.PAS:633–649` also uses
the flags for other keyboard handling. The archived source does not prove
source-to-binary build identity.

A temporary instruction trace watched the guest bus address `00417h`
(`0040:0017`) during normal guest instruction execution and separately logged
INT 16h and keyboard-port I/O. The first trace confirmed the dialog load at
`5191:0CBA` but exhausted the bounded read budget on repeated polling before
Shift was pressed. A second trace coalesced consecutive identical
address/instruction/value/status observations. The instrumentation is removed
from the production sources.

The [guest trace](guest-trace.log) records 31 loads at `5191:0CBA` with
bytes `26 A0 17 00 24 03 08 C0`. [Disassembly](dialog-disassembly.log):
`mov al,es:[0017h]; and al,03h; or al,al`. The bus read resolves to
`00417h`. The application uses both side bits as a single “Shift held”
condition. The trace preserves the original serial line numbers:

| Dialog samples | Trace lines | Read byte | Chronos computed status |
| --- | --- | --- | --- |
| Plain arrows before Shift | 2480–2534 | 00h | 00h |
| Left Shift make (scan 2Ah), then arrows | 2540–2683 | 00h | 02h |
| Left Shift break, then plain arrows | 2687–2778 | 00h | 00h |
| Right Shift make (scan 36h), then arrows | 2784–2927 | 00h | 01h |
| Right Shift break, then plain arrow | 2931–2949 | 00h | 00h |

The 55 successful logged status/read pairs use INT 16h/AH=11h and 10h
at `1249:01CA` and `1249:01D5`; Left/Right return `4BE0h`/`4DE0h`
with FLAGS `F206h`. No AH=02h/12h or keyboard port 60h/64h accesses
were recorded in the diagnostic window. The INT trace omits unchanged AX
polls and does not assert absence outside the replay. Thus the application
uses BIOS for key events and **direct BDA reads for selection modifiers**.
“Computed status” in the trace is the host diagnostic's view, not a guest
BIOS status call or a byte DN could see in the baseline.

## Correction and focused verification

Chronos now mirrors its existing modeled keyboard flags (mask `2Fh`)
to the standard BDA byte. The same status computation still drives
INT 16h: bit 0 right Shift, bit 1 left Shift, bit 2 combined Ctrl,
bit 3 combined Alt and bit 5 Num Lock. The other bits stay as stored
by the guest. Num Lock is applied before publishing the byte; a repeated
make cannot toggle the lock again. Focus loss goes through the same update,
clearing held bits and the Num Lock make latch without dropping the lock.
This does not add a BDA queue, secondary BDA byte support or new lock
emulation. BIOS return values and FLAGS, scan-code translation,
and guest modal logic were not changed.

The new real-mode test executes `MOV AL,[0017h]` with DS=0040h, then
`INT 16h/AH=02h` and compares both results and FLAGS. It [fails before
the fix](focused-before.log) at the first left Shift make (AX=02D0h,
expected 02D2h with test sentinel bits) and [passes after](focused-after.log).
Its 13 comparisons cover startup, Shift overlap and release, fake E0 Shift,
Ctrl/Alt, Num Lock repeat and focus cleanup while preserving unmodeled bits.
The separate DN-26 guest probe still checks AH=12h and FLAGS; the 216-row
DN-25 matrix and all DOS byte service checks remain in the focused run.

## Regression commands

Run from the repository root. These tests use host Chronos unless the
row explicitly says QEMU.

| Check | Command | Result |
| --- | --- | --- |
| Focused DOS/BIOS | `cargo test --offline -p chronos-core --lib --target x86_64-unknown-linux-gnu dos::tests` | [39 passed](focused-after.log) |
| Chronos core and workloads | `cargo test --offline -p chronos-core --lib --target x86_64-unknown-linux-gnu` | [205 passed, 1 ignored](core-final.log), including the MIDTERM batch and Mines MZ load |
| Chronos UI | `cargo test --offline -p sunlight-chronos --target x86_64-unknown-linux-gnu` | [36 passed](chronos-ui-final.log), including focus loss and playable Mines frame/input/exit |
| Sunlight UI library | `cargo test --offline -p sunlight-ui --lib --target x86_64-unknown-linux-gnu` | [277 passed](sunlight-ui-final.log) |
| PS/2 keyboard driver | `cargo test --offline -p sunlight-kbd --lib --target x86_64-unknown-linux-gnu` | [5 passed](keyboard-final.log), including Pause, E0 handling and modifier overlap |
| Core `no_std` | `cargo check --offline -p chronos-core --no-default-features --target x86_64-unknown-none` | [PASS](no-std-final.log) |
| Unrelated Sunlight UI doctest | `cargo test --offline -p sunlight-ui --doc --target x86_64-unknown-linux-gnu` | [Known failure](doctest.log): `horizon.rs` diagram parsed as Rust; 1 failed, 6 ignored. Checked on unchanged DN-26 source; no Sunlight UI changes. |

The desktop driver sends PS/2 input through QEMU QMP. In the matrix, press
Shift after the field is active, repeat arrows while holding it, reverse
direction at the original anchor, release Shift before breaking the Right key,
then send an ordinary Left. These are multiple explicit arrow makes and
breaks, rather than a typematic timing measurement. The host Chronos UI
test separately exercises the focus callback. Follow-on native dialog input
and relaunch test the normal lifecycle boundary.

To reproduce the 4 GiB gate and desktop run:

```sh
CHRONOS_KEY_TRACE=1 SUNLIGHT_TEST_MEMORY_MB=4096 SUNLIGHT_TEST_SERIAL_LOG=target/dn27-4g.serial bash tools/test.sh phase3.0
cp target/sunlightos.iso target/dn27-desktop.iso
python3 docs/audit/chronos-dn/dn-27/desktop.py target/dn27-desktop.iso target/dn27-final-desktop
python3 docs/audit/chronos-dn/dn-27/scanout.py corrected target/dn27-final-desktop
```

For the independent default-memory gate:

```sh
env -u SUNLIGHT_TEST_MEMORY_MB CHRONOS_KEY_TRACE=1 SUNLIGHT_TEST_SERIAL_LOG=target/dn27-1g.serial bash tools/test.sh phase3.0
```

The gate builds native Chronos in release mode using the service linker
script. [Embedded evidence](embedded.log) verifies the [release ELF
digest](native-release.sha256) in the 4 GiB image and confirms the
temporary memory watcher is absent. The trace opt-in here enables the
pre-existing DN-26 keyboard logging only.

The [full SunApp manifest](package.sha256) pins 66 files, including both
authentic binaries. [Original package validation](package-original-pins.log)
verifies the 63 program files against the original archived pins. Original
archive extraction was not repeated; the checkout contains the packaged
files and the original manifest of file hashes. The [embedded binary
check](embedded.log) independently matches DN.COM and DN.PRG in the
baseline and corrected kernel images.

## Platform gates

| Memory | Required markers | Service launch | ELF mapping | Explicit `NoMemory` |
| --- | --- | --- | --- | --- |
| Default 1 GiB | **PASS**: all 32 expected strings plus the PMM check, including UTC and native StatPath | Base launch failures and supervised spawn failures; no desktop readiness claim | `uac_service`: segment mapping failed, `ElfLoadFailed` | 11 spawn failures, including powerd, niced, thermald, audiod and sunlight-display |
| 4 GiB | **PASS**: all 32 expected strings plus the PMM check | No matching service launch failures in the gate window | No matching failures in the gate window | None recorded in the gate window |

Evidence: [1 GiB gate](phase-1g.log), [summary](phase-1g-summary.log),
[serial](phase-1g-serial.log.gz), [ISO digest](phase-1g-image.sha256);
[4 GiB gate](phase-4g.log), [summary](phase-4g-summary.log),
[serial](phase-4g-serial.log.gz), [ISO digest](final-image.sha256).
The 4 GiB gate also reports an unsupported PCI Ethernet backend and the
enabled-state file's `NoEntry`, separate from the checks above.
[Native release without trace](native-release.log) also builds using
`RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' cargo build --offline -p sunlight-chronos --release --target x86_64-unknown-none`
with `CHRONOS_KEY_TRACE` unset.

## Authentic SunApp result

The desktop boot used the [4 GiB image](desktop-image.sha256), OVMF, two
vCPUs and QMP PS/2 inputs. See the [command](native/command.json),
[inputs](native/inputs.jsonl) and [replay](native/replay.log). The
20-frame [baseline result](baseline-scanout.log) and
[corrected result](native/scanout.log) independently compare foreground
glyphs and selection background, without using the emulator's internal
screen buffer. Plain arrows showed no highlight. At the anchor between
`c` and `d`, the same arrow sequence under either Shift side produced:

| Arrow sequence | Visible selected text after each arrow |
| --- | --- |
| Left, Left | `c`, `bc` |
| Right, Right | `c`, empty (back to anchor) |
| Right, Right | `d`, `de` (crossed the anchor) |
| Left | `d` |
| Right make, Shift break, Right break | `de` (make was already handled before the break; break did not extend the selection) |
| Plain Left | empty |

Compare [baseline Left Shift+Left](baseline/shift-1-left.png) with
[corrected Left Shift+Left](native/shift-1-left.png), and
[baseline Right Shift+Right](baseline/shift_r-5-right.png) with
[corrected Right Shift+Right](native/shift_r-5-right.png). The
[return to anchor](native/shift-3-right.png) and [right Shift released
before arrow break](native/shift_r-released-before-arrow.png) show the
intermediate states. The six glyphs stayed unchanged in all 20 frames.
The final desktop run shows the correction through the application;
the direct post-fix memory values are verified separately by the guest
instruction test, since the temporary address watcher was removed.

The replay exercises both Shift makes and breaks with DN already focused.
Native focus cleanup is observed at app removal and relaunch, with
subsequent input in the desktop and the new app. A lost-focus gesture
while a Shift key is still physically held was not separately forced in
QEMU; the host Chronos UI test exercises the focus-loss callback, and
the core test checks BDA publication on cleanup.

On the corrected first launch, [Escape](native/dn-1-mask-cancel.png)
returned to the panels without selecting files. [Reopening](native/dn-1-mask-reopened.png)
restored `*.*`; [Enter](native/dn-1-mask-accepted.png) closed the dialog
and selected 13 files. No guest file operation was confirmed. The
[numeric keypad](native/dn-1-keypad-command-line-num-on.png) still typed
digits with Num Lock on. The core matrix independently rechecked the
precise BIOS words.

The [second search launch](native/dn-2-search.png) in the same desktop
opened COLORS via [keypad Enter](native/dn-2-directory-opened.png). It
replayed DN-26's mask edits: typing `abbc` then Left/Backspace left
[`abc`](native/mask-regression-backspace.png); Home/Delete left
[`bc`](native/mask-regression-delete.png); Insert and Z overwrote the
first byte to [`zc`](native/mask-regression-insert.png). Right Ctrl+Left
and Right Alt+Right retained [`zc`](native/mask-regression-chords.png);
the trace records `73E0h` and `9D00h`, respectively. This does not
prove a standalone visible Alt action.

Both DN launches (PIDs 39 and 40) accepted Alt+X → Y, exited the DOS
guest and native Chronos with code 0, flushed the overlay, detached the
window and reaped the process. See the
[first](native/dn-1-restored.png) and [second](native/dn-2-restored.png)
restored desktop scanouts and [native lifecycle](native/lifecycle.log).

The same desktop launched Sunlight Mines (PID 41) and displayed its
[board](native/mines-play.png). Escape completed the Mines DOS guest with
code 0. Its ordinary policy left Chronos open; Ctrl+W then requested
SIGTERM, native exit 143 was reaped, and the desktop was
[restored](native/mines-restored.png). This is distinct from DN's
natural Chronos exit.

[Native keyboard events](native/keyboard.log) and
[complete serial](native/serial.log.gz) accompany the scanouts.
All **66** SunApp hashes matched
[before](package-before-final.log) and [after](package-after-final.log)
the run; the [63 original program pins](package-original-pins-after.log)
also reverified afterward. The [baseline post-run check](package-after-baseline.log)
matched the same hashes. No original package file changed.
