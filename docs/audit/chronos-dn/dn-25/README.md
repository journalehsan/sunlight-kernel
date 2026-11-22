# DN-25: extended navigation and keypad

The target is the original DOS Navigator 1.51 desktop SunApp. The
[package checker](package.log) and the [before](program-before.log) and
[after](program-after.log) checks against [DN-24's pins](../dn-24/program.sha256)
verified all 63 DOS files. See [input hashes](inputs.sha256),
[replayed input hashes](replay-inputs.sha256), [baseline ISO](baseline-image.sha256), [final ISO](final-image.sha256)
and [embedded release binaries](embedded.log).

## Baseline

The unchanged [DN-24 replay](../dn-24/replay.py) ran against the existing
DN-24 ISO before implementation. The [driver output](baseline-desktop.log)
and [serial/scanout evidence](baseline-unchanged/) show two DN launches
returning to the desktop with DOS and native status 0; this capture ends
before Mines. A separate [extended replay on the same ISO](baseline-matrix/)
shows dedicated Down and keypad 2 both returning AX=5000h, despite the
different physical E0 bits, and [no command text](baseline-matrix/dn-1-keypad-command-line-num-on.png)
after toggling Num Lock and sending keypad 1, 2, 3 and decimal. Main Enter,
keypad Enter, Ctrl+F3 and Alt+X retain the DN-24 values and effects. That
replay also reaches Mines guest exit 0 and native status 143; its original
assertion expected the older scheduler wording “reason=exit”, while the
[actual terminal record](baseline-matrix/lifecycle.log) says
“reason=pending-fatal-signal”. The corrected replay accepts both labels
only after observing SIGTERM and reaping. The pre-final image also
[completed a two-run replay](final-desktop.log) before the final Pause and
Alt+decimal corrections; it has a [separate hash](first-built-image.sha256).

The [216-row matrix](keyboard-matrix.csv) has both the **source-derived**
baseline enhanced value for each combination and the **measured** final
Chronos and reference return words. Only cases present in the baseline
native trace are empirical baseline observations. Prior evidence:
[DN-21](../dn-21/README.md), [DN-22](../dn-22/README.md),
[DN-23](../dn-23/README.md), [DN-24](../dn-24/README.md).

## Translation and evidence

The PS/2 receiver forwards set 1 physical identity, make/break, and
aggregate modifier snapshots. The Sunlight UI physical mode retains E0
and does not manufacture keypad text. Chronos had three omissions: its
navigation switch selected only on the scan byte, it had no Num Lock state
or keypad operator mapping, and its adapter dropped translated printable
physical keys. The fix uses the same physical queue path for these keys
and leaves Space and ordinary printable text on their existing commit
path. The DOS byte-oriented console strips BIOS marker bytes, so they
do not become printable characters. The PS/2 decoder also consumes the
E1 Pause sequence instead of delivering its embedded Ctrl and 45h bytes
as independent modifier and lock transitions.

Reference source: [SeaBIOS rel-1.16.2: kbd.c](https://qemu.googlesource.com/seabios/+/refs/tags/rel-1.16.2/src/kbd.c),
specifically `scan_to_keycode`, the two extended Enter/divide rows,
`__process_key` and `dequeue_key`. The source gives Alt precedence over
Ctrl, XORs Shift with Num Lock in the keypad range, marks enhanced
navigation in AL, and normalizes legacy values at dequeue. Alt with
keypad digits or decimal has no immediate queue entry in this reference.
The enhanced arithmetic and Enter words are compared independently of
the dedicated navigation representation.

The [independent boot-sector probe](reference.asm) and [QMP driver](reference.py)
run on the local QEMU SeaBIOS ROM (ROM hash recorded; firmware version
banner unavailable). F12 delimits each input, including empty ones.
The [delimited results](seabios-delimited.csv) and [join script](matrix.py)
compare the two lock states × four modifier states × 27 physical keys.
All 216 fixture rows agree on AH=01h, 11h and 10h words or queue absence.
Nonempty checks returned FLAGS=0207h from 0247h; reads retained 0247h.
The probe preloads BX=ABCDh, CX=1234h and DX=9876h, and records them
unchanged at the enhanced status return. The
[Chronos test](focused-tests.log) captures full register comparisons,
empty status, and depth before consumption. The earlier
[non-delimited attempt](seabios-runtime.csv) could assign a delayed
response to the next key and was excluded. The runtime comparison is
against an independent firmware image, not a determination of its
release tag or physical keyboard parity.

## Desktop path

In [native input](native/inputs.jsonl), QMP sends physical keys through
the emulated PS/2 controller to the native decoder and compositor. The
[native trace](native/keyboard.log) records what the UI handed to
Chronos, the resulting queue key, and the actual return from DN's
INT 16h calls. Examples below use hexadecimal BIOS words:

| Input | UI → queue → guest | Visible application evidence |
| --- | --- | --- |
| Dedicated Down | Scan 50, E0, make, no modifiers → 50E0 → AH=11h and AH=10h, AX=50E0 | [Panel navigation](native/dn-1-dedicated-down.png) |
| Keypad 2, lock off | Scan 50, no E0 → 5000 → both BIOS functions AX=5000 | [Panel navigation](native/dn-1-keypad-down-num-off.png) |
| Keypad 2, lock on | Same physical key → 5032 → both calls AX=5032 | [Numeric text in DN](native/dn-1-keypad-command-line-num-on.png) |
| Shift+keypad 8, lock on | Non-E0 scan 48 with Shift → 4800 → both calls AX=4800 | [Navigation](native/dn-1-shift-keypad-num-on.png) |
| Ctrl+dedicated Up | E0 scan 48 with Ctrl → 8DE0 → enhanced status/read | [Next plain event](native/dn-1-unmodified-after-chords.png) is 48E0 after Ctrl and Alt releases |
| Main / keypad Enter | Scan 1C without/with E0 → 1C0D / E00D → respective enhanced returns | [Launch 1](native/dn-1-directory-opened.png) and [launch 2](native/dn-2-directory-opened.png) both open COLORS |
| Keypad arithmetic | Non-E0 4E, 4A, 37 and E0 35 → 4E2B, 4A2D, 372A, E02F | [DN select dialog and file mask](native/dn-1-keypad-arithmetic.png) |
| Dedicated Insert / Delete | E0 52 / 53 → 52E0 / 53E0, both consumed | [Insert selected a file](native/dn-1-dedicated-insert.png). [Delete](native/dn-1-dedicated-delete.png) was delivered; no file operation was confirmed. |

Representative native status/read pairs return FLAGS=F206h; depth remains
1 on AH=11h and becomes 0 on AH=10h. Modifier breaks and ignored Alt keypad
make events enqueue nothing. Native tracing does not snapshot every
register or empty return; the focused and independent probes cover those
boundaries separately. Both DN runs use normal desktop search and the
original package. See [lifecycle trace](native/lifecycle.log) and restored
desktop captures [one](native/dn-1-restored.png) and
[two](native/dn-2-restored.png). Both DOS processes and both native
Chronos processes exited with code 0 (PIDs 39 and 40), with overlay writes,
owner detach and desktop cleanup. Mines (PID 41) displayed its
[board](native/mines-first.png), then guest Escape completed the DOS
session with code 0. Its default policy kept the window open until
Ctrl+W, which triggered SIGTERM and native code 143 as expected.
The replay ended with [the desktop restored](native/mines-restored.png).
The Pause action between lock-on and keypad input did not emit a lock or
modifier edge in the trace or prevent the numeric entry.

## Regression gates and limits

- [Host suites](final-tests.log): 200 Chronos core passed (one ignored),
  36 Chronos UI and 291 Sunlight UI passed; [keyboard driver](keyboard-driver-tests.log)
  passed 5. Includes the actual MIDTERM batch fixture and marker check,
  Mines MZ workload and interactive exit, empty/FIFO/repeat behavior,
  pending reads, modifier make/break, and the physical adapter test.
  The optional host test of the freestanding keyboard **binary**
  [failed](host-driver-binary-attempt.log) because its panic handler
  conflicts with the Rust test harness; its library and native binary
  were validated separately.
- [Core no_std](no-std.log) and [native release](native-release.log) passed.
  The first platform build [hit ENOSPC](platform-first-build.log.gz).
  The default 1 GiB retry [missed the timezone marker](platform-default-memory.log.gz)
  after service ELF mapping failures. Phase 3.0 [passed](platform.log)
  with `SUNLIGHT_TEST_MEMORY_MB=4096`; see [platform serial](platform-serial.log.gz).
  This gate is separate from the SunApp runtime.
- The unchanged `horizon.rs` drawing [doctest still fails](doctest.log).
- Desktop reproduction:
  `SUNLIGHT_TEST_MEMORY_MB=4096 CHRONOS_KEY_TRACE=1 bash tools/test.sh phase3.0`;
  `python3 docs/audit/chronos-dn/dn-25/desktop.py target/sunlightos.iso target/dn25-replay`.
  Key choices were replayed through QMP in the real desktop, with scanout
  captured after each action. This is not a physical keyboard or HID test.

Scope remains bounded: no global Num Lock/LED synchronization, right/left
BIOS shift-flag parity, Ctrl+Alt firmware actions, Alt-numpad composition,
BIOS Data Area ring/overflow parity or exhaustive DN modal shortcuts.
The next useful comparison is side-specific modifier flags plus
byte-oriented DOS extended-key reads, followed by an authentic DN
editor/dialog probe where physical key behavior can be independently seen.
