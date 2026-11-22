# DN-23: DOS Navigator 1.51 SunApp

Based on Dos Navigator by RIT Research Labs. The package retains
[the original notice](../../../../DosNavigator.sunapp/Program/README.TXT).

## Packaging and launch

Mines supplies the model: format-1 manifest, a DOS executable in `Program`,
an icon in `Resources`, and static installation under `/Applications` in
the boot filesystem. The desktop Start Menu and search palette map the app
to the existing `sun_exec` registry. The resolver validates the bundle
manifest and paths and spawns the existing `/bin/sunlight-chronos` with the
bundle path, application identity, title, scoped arguments and launch trace.
Chronos reads the DOS files via VFS and uses its normal native window,
keyboard, rendering and exit paths. No new launch mechanism or grant was added.

The application ID is `org.sunlight.chronos.dos-navigator`. The entry
point is `C:\DN.COM`, the original launcher which runs `DN.PRG` and provides
the first-run state used by DN's panel initialization. Guest C: starts at its
root with an empty command tail. The C: base is the bundle's `Program`
directory, with the normal private persistent overlay. No host Documents,
clipboard, printing, serial, or network access is requested.

The old native adapter's 65,536-byte limit truncated both `DN.PRG` and
`DN.OVR`, and the launcher ignored the manifest CPU setting. The common
bundle launch path now forwards the existing CPU profile selection and
imports files from bundled `Program` up to 1 MiB each, rejecting a file
larger than that rather than silently truncating it. The old cap still applies
to direct imports, Documents, dependencies and overlay files. Chronos uses
the existing reclaiming Sunlight allocator for the larger file set. No
guest CPU, BIOS, DOS or process semantics were changed.

## Inventory and integrity

The original `dn151.zip` SHA-256 is
`f6dee3904945a2f390f8576a5d18d83446f4554e43d6f0a3c75f8aa5b570d7d9`.
The complete 63-file, 1,809,437-byte Program tree was copied unchanged from
the already extracted compatibility inputs. It includes the MZ child and
overlay containers, configuration defaults, help, translations, palette
files, screen savers, and documentation, with the original DOS paths intact.
[Inventory](inventory.tsv) lists all filenames, sizes and prepackaging
hashes. The [original](original-checksums.log) and
[packaged](packaged-checksums.log) trees independently match the
[same hashes](program.sha256), including:

| File | Bytes | Original and packaged SHA-256 |
| --- | ---: | --- |
| DN.COM | 1,778 | `ae0863550ec07f21c5cf8d208373a5022b868133e6f644757b1623cd8f7e9905` |
| DN.PRG | 109,613 | `ac0024e269ecf0feb4a8db15f96181092604567a62cf00fb8453d9052c2862c3` |
| DN.OVR | 745,951 | `401a8e631b8627fcbf8c209f01b1d71e3e314347f1c32417a0345d45455e3569` |

The [package check](package.log) verified both the checked-in bundle and a
fresh archive-based rebuild. The builds compare byte for byte. See the
[application guide](../../../apps/dos-navigator.md#dn-23-desktop-sunapp)
for reproduction, installation and manual test steps.

## Host and build validation

- [Regression log](regressions.log): 197 Chronos core tests passed (one
  pre-existing ignored), 33 Chronos UI tests, 290 Sunlight UI tests, and
  30 app-state tests. These include the DOS shell MIDTERM and Mines
  workloads, with the Mines startup, input and clean guest exit checks.
- [Resolver tests](launcher-units.log): nine focused launch registry and
  path/argument tests passed. A preliminary broader libc sweep
  ([log](units.log)) reported an allocator test failure and was stopped
  with two secret-store tests still pending; it isn't a passing libc suite.
- [Core no_std check](no-std.log) and [native Chronos release
  build](native-final.log) passed.
- The unrelated Sunlight UI [doctest](doctest.log) still fails because the
  `horizon.rs` diagram is parsed as Rust.
- The existing DN-22 host probe loaded the packaged `DN.COM`, reached
  native DOS code in the child, then finished with DOS code 0 and no
  suspended parent ([trace](host-package-exit.log)). The probe imports
  sibling files and sends BIOS key words directly. This is package-byte
  evidence; it does not test the Sunlight launcher, adapter or keyboard.

## Native desktop interaction

The [Phase 3.0 platform gate](platform.log) passed independently. The
graphical checks booted the same ISO in a separate diskless QEMU guest,
using its real PS/2 keyboard and display scanout. The [desktop search
result](02-search.png) shows the registered bundle. The
[first launch](03-dn-first.png) shows both C: panels populated by DN with
the actual Program contents, including `DN.COM`, `DN.PRG` and `DN.OVR`.
The [serial trace](desktop-first-boot.log) records launcher resolution,
Chronos startup and drive seeding, and a window made visible by the
compositor.

| Native action | Observation |
| --- | --- |
| F10 | [Menu bar opened](04-f10.png). |
| Manager → New | [M opens Manager](05-manager.png); [N opens the picker](14-new-mnemonic.png). |
| Drive picker | [C accepts C:](15-c-selected.png) and returns to populated panels. |
| Tab and Down | [Tab shifts focus to the left panel](16-tab.png). [Down selects COLORS](17-arrow.png). |
| Enter | [Main Enter did not open COLORS](18-enter.png); [keypad Enter did](19-kp-enter.png), enumerating real nested palette files. |
| Ctrl+F3 | [No drive picker opened](20-ctrl-f3.png). |
| Quit | [File → Exit](22-file-menu.png), [Yes](23-quit-confirm.png), then [guest code 0](24-guest-exit.png). Alt+X did not open the confirmation in the earlier run. |

The tested input boundary is the native Chronos adapter: the UI's main
Enter is delivered as a text event, which Chronos passes with scan code 0.
Keypad Enter reached the scan-code path. The current function-key path
does not translate Ctrl+F3 to DN's extended BIOS key. The package leaves
those input paths intact. The screenshots also show missing glyphs for
some DOS box-drawing characters.

Escape [dismissed the Manager menu](31-after-escape.png) and returned
focus to the panels. All keyboard inputs were sent as QEMU PS/2 events,
not BIOS words or injected Chronos state. The
[keyboard sequence](desktop-keyboard-inputs.jsonl), [keyboard serial
trace](desktop-keyboard.log), and [lifecycle input
sequence](desktop-lifecycle-inputs.jsonl) retain the attempts.

## Process lifecycle and Mines

Both DN launches in one desktop session exited the guest with code 0
([first](32-first-exit.png), [second](42-repeat-exit.png)). Chronos
displayed the exit status and kept its window open. Closing it with
the normal desktop shortcut Ctrl+W [restored the desktop](33-desktop-restored.png).
The [second launch](41-repeat.png) opened independently and reimported
the private overlay, including the history file written by the first
run. The [lifecycle trace](lifecycle-summary.log) shows distinct PIDs
39 and 40, both reaped and removed from the display registry, and
matching native frame and page-table cleanup counts. The compositor
closed both with SIGTERM (native code 143). Automatic Chronos exit
and a clean native exit code were therefore not demonstrated. The
test found no stale process or window from these two runs; it was not
a general leak or persistence soak test.

The same image [launched Sunlight Mines](45-mines.png) from desktop
search and displayed its board under Chronos. The separate host workload
also verified game input and guest exit. The [full lifecycle
trace](desktop-lifecycle.log) includes PID 41 and its native teardown.

| Area | Verdict |
| --- | --- |
| Bundle creation and byte integrity | **PASS** — all 63 original files; all hashes match. |
| Manifest validation | **PASS** — package check and actual native resolver launch. |
| Installation and launcher discovery | **PASS** — installed in the image and found by normal desktop search. |
| Chronos launch integration | **PASS** — packaged DN.COM reached interactive DN under native Chronos. |
| Manual keyboard interaction | **PARTIAL** — F10, menu mnemonics, C, Tab, arrows, keypad Enter and Escape passed; main Enter, Ctrl+F3 and Alt+X did not. |
| Panel creation and real directory enumeration | **PASS** — automatic C: panels; Manager → New reached and accepted C:; keypad Enter showed real nested files. |
| Clean exit and repeated launch | **PARTIAL** — two guest exits with code 0, desktop restoration, and native reap; Chronos remained open until Ctrl+W and terminated with code 143. |
| Mines regression | **PASS** — native desktop launch and host Mines workload. |
| Native QEMU DOS Navigator | **PARTIAL** — actual interactive native launches verified; input and close limitations above. |
| Phase 3.0 platform gate | **PASS** — independent boot and VFS gate. |
