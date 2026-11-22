# DN-26: BIOS modifier status, DOS character reads, and DN file-mask editing

## Baseline and root causes

The [pre-change host suite](baseline-core.log) passed 200 tests with one ignored.
The unchanged [DN-25 desktop replay](../dn-25/desktop.py) completed two DN
sessions and one Mines session on the [existing ISO](baseline-image.sha256),
which does not match the archived DN-25 final ISO hash. The first launch
attempt [could not bind its local QMP socket](baseline-desktop.log); the
[retry](baseline-desktop-retry.log) ran with the required local socket
permissions. See [input sequence](baseline-inputs.jsonl) and
[lifecycle log](baseline-lifecycle.log). The original DN-24 and DN-25 captures
and reference runs remain in their original directories.

- The PS/2 Set 1 decoder (`services/sunlight-kbd/src/lib.rs`) carries E0
  and make/break separately. Its modifier state already tracks each side and
  emits aggregate snapshots. Its E1 Pause filter removes embedded Ctrl and
  Num Lock bytes. The UI decoder (`sunlight-ui/app.rs`) forwards physical
  controls, and the Chronos adapter (`sunlight-chronos/src/main.rs`) hands
  those edges to the runtime. Chronos previously copied only the aggregate
  snapshot and put either Shift in bit 0. The
  [guest probe](baseline-bios-probe.log) returned AX=0201h on left Shift,
  where AH=02h should return AX=0202h. AH=12h was not dispatched at all.
- DOS byte reads consumed a complete BIOS FIFO word, normalized enhanced
  markers to zero, and then lost the scan byte. In the
  [guest probe](baseline-dos-probe.log), the next poll returned AX=0651h
  (the following ASCII Q) in place of AX=0650h (the scan byte for Down).
- The buffered read processed one existing FIFO entry and then parked,
  even when more type-ahead was already queued. Its input bound also
  allowed the text to fill every declared slot before appending CR. That
  could write past the caller's declared buffer.

## BIOS and DOS contracts

The reference is [SeaBIOS rel-1.16.2, kbd.c](https://qemu.googlesource.com/seabios/+/refs/tags/rel-1.16.2/src/kbd.c),
blob `15e5ae789cd3a8c0aefb0f9c8d9db054e973445d`. Compare
`handle_1602`, `handle_1612` and `__process_key`: AH=02h exposes
the low flag byte, including side-specific Shift and combined Ctrl/Alt;
AH=12h combines left Ctrl/Alt in AH bits 0–1 with right Ctrl/Alt in bits
2–3. Extended fake Shift in Print Screen sequences is ignored. Chronos
uses the upstream aggregate snapshots to keep Ctrl/Alt active while
either side remains held, with side flags learned from physical edges.
This comparison uses the pinned source; DN-25 measured navigation/keypad
words against a local firmware image, but that probe did not measure
modifier status. Native focus loss clears held sides and the Num Lock
make latch; a new guest starts clear. EXEC is within the same keyboard
session and does not clear physical modifier state.

The [Microsoft MS-DOS Encyclopedia](https://msarchive.pcjs.org/mspl13/msdos/encyclopedia/section5/)
documents a zero prefix and second byte for 01h, 06h, 07h and 08h; 01h
echoes input, 06h/DL=FFh polls with ZF, and 07h/08h wait without echo.
07h and 06h return raw Ctrl-C; 01h/08h/0Ah have console break handling
in DOS. Function 0Ah counts text without the CR, but the declared
capacity includes it. MS-DOS console redirection and the richer DOS line
editor are separate contracts.

Chronos now uses one pending DOS scan byte and one BIOS event FIFO.
The DOS byte services share the pending byte, consume it before another
FIFO entry, and never present it to INT 16h. INT 21h/AH=0Bh reports it;
AH=0Ch flushes it; a DOS process switch discards an incomplete pair.
The 0Ah line reader consumes existing type-ahead and reserves space for
CR, preserving the following FIFO event. The separate 3Fh console
handle path and its existing CR/LF behavior are unchanged.

[Guest instruction tests](focused-dos-bios.log) execute actual INT
instructions, including a blocking 07h with a subsequent E0 Down, a
BIOS peek at the following Q, 06h receiving the scan tail, 08h
receiving Q, and a subsequent empty 06h. They capture AX and FLAGS
plus BX/CX/DX sentinels. Additional tests assert 01h echo behavior,
0Bh/0Ch, raw Ctrl-C through 06h/07h, buffered editing/capacity/FIFO,
and AH=02h/12h modifier edges including overlapping release, repeat,
and E0 Print Screen make/break isolation. The existing driver test
covers the full Pause byte sequence. Neither the side-bit probe nor the
DOS character-byte probe requires the authentic binary to issue those calls.

Limits: 01h/08h/0Ah do not invoke DOS INT 23h; the host Ctrl-letter path
does not supply BIOS control-letter text, and Ctrl-Break has no guest
INT 1Bh delivery. Raw Ctrl-C tests inject byte 03h through the core.
The legacy DOS calls bypass stdin handle redirection even though 3Fh
supports file handles. AH=0Ah does not implement DOS template recall,
special editing keys, or the full bell/echo behavior. BIOS lock flags
beyond Num Lock, BDA queue/state access and firmware actions such as
INT 05h and SysRq are outside the tested contract.

## Authentic DN dialog and desktop replay

Selected probe: the **Select → File mask** dialog. The pinned source snapshot
([file hashes](dn-source-files.sha256)) maps Gray Plus to `SelectFiles`
(`FLPANELX.PAS:1359`); `FLTOOLS.PAS:349` changes only selected entries
after `SelectDialog`; `FILECOPY.PAS:577` instantiates the input line.
`DIALOGS.PAS:990–1051` handles Ctrl navigation, arrows, Home/End,
Backspace, Delete and Insert. Source and binary build equivalence is
unverified; the screenshots and trace confirm this path exists in the
packaged binary.

From desktop search, launch DOS Navigator. Send
**F10 → M → N → C → Tab → Down → Enter** to open COLORS on the focused
panel. The replay exercises Ctrl+F3 and Escape as in DN-25, then sends
**keypad +** to open Select. Inspect the following scanouts from the actual
1.51 SunApp:

| Action | Guest-visible result |
| --- | --- |
| Keypad + | [Select dialog with the populated default mask `*.*`](native/dn-1-mask-initial.png) |
| Home, repeated Delete | [Empty mask](native/dn-1-mask-empty.png) |
| A, B, B, C | [`abbc`, including the repeated letter in order](native/dn-1-mask-typed.png) |
| Left, Backspace | [`abc`; the middle duplicate was removed](native/dn-1-mask-backspace.png) |
| Home, Delete, Right, End | [`bc`, caret at the end when visible](native/dn-1-mask-delete.png) |
| Home, Insert, Z | [`zc`; overwrite replaced the first byte](native/dn-1-mask-insert-mode.png) |
| Right Ctrl+Left, Right Alt+Right | [`zc` retained](native/dn-1-mask-chords.png); the trace shows 73E0h and 9D00h respectively. The chord pair was captured together; the Alt gesture has no independently established visible effect. |
| Escape | [Dialog closed and no files selected](native/dn-1-mask-cancel.png) |
| Keypad +, Enter | [Reopened with `*.*`](native/dn-1-mask-reopened.png), then [13 selected files](native/dn-1-mask-accepted.png) |

[QMP event order](native/inputs.jsonl), [keyboard trace](native/keyboard.log)
and [full serial](native/serial.log.gz) link physical native events to the
BIOS words consumed by the application. For example, keypad + returned
4E2Bh, dedicated Left/Right returned 4BE0h/4DE0h, and the right-hand
chords kept their E0 modifier identities. Selected status/read pairs
show FLAGS=F206h and queue depths 1→0. The native trace records DN reading
enhanced INT 16h words. DOS byte service results are from the separately
executed real-mode probes, not inferred from the DN dialog.

The [desktop replay](desktop.log) completed two DN launches (PIDs 39 and
40), both with guest exit 0, natural Chronos exit 0, overlay flush,
window removal, and [native process reaping](native/lifecycle.log).
See [first](native/dn-1-restored.png) and [second](native/dn-2-restored.png)
restored desktops. DN-25 emitted 50E0h for dedicated Down, 5000h for
keypad Down with Num Lock off, 5032h for keypad 2 with Num Lock on,
1C0Dh/E00Dh for main/keypad Enter, and 6000h for Ctrl+F3. The
[numeric keypad scanout](native/dn-1-keypad-command-line-num-on.png) and
the [second Enter scanout](native/dn-2-directory-opened.png) retain the
application-visible behavior. Mines displayed its [board](native/mines-first.png)
and exited the DOS guest on Escape; its different lifecycle policy kept
Chronos open until Ctrl+W (SIGTERM, exit 143, reaped, desktop restored).

No disposable file was required and no file edit was confirmed or
requested. All 63 installed program files pass the original pins both
[before](program-before.log) and [after](program-after.log). The
[package check](package.log) and [kernel ELF inspection](embedded.log)
verify the package and record authentic DN.COM/DN.PRG hashes in the tested
image. The [native image](native-image.sha256) is the 4 GiB gate build.
The independent [1 GiB build](platform-1g-image.sha256) came after
test-only additions and formatting; its embedded Chronos hash differs.
Both images contain the same production changes and the same DN binaries.

## Regression gates and next boundary

| Gate | Result | Evidence |
| --- | --- | --- |
| Focused BIOS/DOS | PASS | [38 guest service tests](focused-dos-bios.log); original 216-row DN-25 case matrix also passed |
| Core, MIDTERM and Mines workloads | PASS | [204 core tests, 1 ignored](host-tests.log), including the bundled MIDTERM batch and Mines MZ load |
| Chronos UI and Sunlight UI library | PASS | [36 Chronos tests](chronos-ui-tests.log); [277 default-feature UI tests, 5 keyboard driver tests](host-tests.log) |
| Core `no_std` and native release | PASS | [Core check](no-std.log), [native build](native-release.log) |
| Phase 3.0, 1 GiB default | PASS for the gate | [Gate output](platform-1g.log), [serial](platform-1g-serial.log.gz), [build](platform-1g-build.log.gz). UTC timezone and StatPath markers found. UAC service ELF mapping failed; powerd, thermald, niced, audiod, sunlight-display and additional services failed with NoMemory. This does not establish default-memory desktop readiness. |
| Phase 3.0, 4 GiB | PASS | [Gate output](platform-4g.log), [serial](platform-4g-serial.log.gz), [build](platform-4g-build.log.gz). UTC timezone and StatPath markers found; no matching ELF mapping or NoMemory failures in the captured window. An unsupported PCI Ethernet backend and the enabled-state file's NoEntry are separate observations. |
| Unrelated UI doctest | Known failure | [Drawing in `horizon.rs` is parsed as Rust](doctest.log) |

Reproduce the desktop run with
`CHRONOS_KEY_TRACE=1 SUNLIGHT_TEST_MEMORY_MB=4096 bash tools/test.sh phase3.0`
then `python3 docs/audit/chronos-dn/dn-26/desktop.py target/sunlightos.iso target/dn26-replay`.
Run the platform gate separately without `SUNLIGHT_TEST_MEMORY_MB` for the
default check.

The next concrete DN boundary is **direct BDA modifier state**. The
source in `DRIVERS.PAS:633` checks 0040:0017 and `DIALOGS.PAS:974`
uses Shift state for selection. Confirm that the packaged binary reads
the BDA on Shift+Left/Right and measure selection in this same dialog
before changing BDA behavior. The next DOS console boundary is
Ctrl-C/Ctrl-Break delivery and redirected stdin for the legacy services.
This run does not validate the full editor or every DN dialog.
