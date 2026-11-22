# DN-24: native keyboard translation and SunApp completion

The test target is the authentic DOS Navigator 1.51 SunApp launched from
the Sunlight desktop. The [package checker](package.log) and
[SHA-256 check](program-check.log) verify the original 63 Program files
against the [DN-23 pins](program.sha256). Only the bundle's manifest
changed. The original launcher, MZ child and overlay are unchanged.

## Before

The baseline used the existing Phase 3.0 ISO, before the implementation
was edited. Both sessions booted through the login screen and launched DN
from the normal desktop search. The [keyboard replay](baseline/keys/inputs.jsonl)
includes the original DN-23 menu sequence ending in Alt+X, then the
Manager → New sequence. The [lifecycle replay](baseline/lifecycle/inputs.jsonl)
used the normal File → Exit confirmation and Ctrl+W. The full serial
logs are preserved [for keys](baseline/keys/serial.log.gz) and
[for lifecycle](baseline/lifecycle/serial.log.gz).

| Action | Baseline observation |
| --- | --- |
| Main Enter on COLORS | The selected directory does not open; the panel stays at C:\ after [main Enter](baseline/keys/18-enter.png). |
| Keypad Enter | The [panel enters COLORS](baseline/keys/19-kp-enter.png). |
| Ctrl+F3 | [No drive picker](baseline/keys/20-ctrl-f3.png). |
| Alt+X after Escape | [Panel view stays open](baseline/keys/13-alt-x.png); no quit confirmation. |
| Quit through File → Exit | [DOS code 0 stays visible](baseline/lifecycle/32-first-exit.png). Ctrl+W [returns to the desktop](baseline/lifecycle/33-desktop-restored.png); the compositor requests SIGTERM and the scheduler reports code 143 and reaps the native process. |

The old image does not have the DN-24 per-key BIOS trace. Baseline BIOS
words described below are deductions from the original source path.
The independent [DN-21 BIOS traces](../dn-21/README.md) and
[DN-22 reference run](../dn-22/README.md) are host and FreeDOS evidence,
respectively. DN-23's [native run](../dn-23/README.md) independently
established original launch, real panels, repeated exit and Mines.

## Keyboard

1. QEMU sends PS/2 set 1 make/break events. The Sunlight keyboard driver
   strips the break bit, tracks both sides of the modifiers, publishes
   modifier snapshots, and sets the physical extended bit on E0 events.
   Main Enter has code 1Ch, an ASCII CR, and no extended bit; keypad Enter
   has code 1Ch, the extended bit and no ASCII payload. The compositor
   routes these to the focused window; neither Ctrl+F3 nor Alt+X is a
   graphical desktop shortcut in this path.
2. The UI previously collapsed main Enter to a newline text event. Chronos
   then injected CR with scan 0 (000Dh). Keypad Enter stayed physical; the
   old translator ignored the extended bit and produced 1C0Dh. Chronos
   now opts into physical controls so the physical identity reaches the
   adapter. Printable text commits still go through the text channel once.
3. The old adapter carried Ctrl and Alt but the translator ignored them
   when constructing the word. Ctrl+F3 became plain F3 (3D00h); Alt+X,
   a physical event with no text commit, returned no key at all. The
   translator now handles modified F1–F10 and Alt plus alphabetic keys.
   Releases update modifier state without enqueuing keys. Focus loss clears
   Chronos's aggregate modifier snapshot; the next key supplies a fresh one.
4. INT 16h keeps one FIFO. AH=01h/11h peek at its head, modify only ZF
   besides a successful AX result, and leave AX untouched when empty.
   AH=00h/10h consume the head or wait; their key return preserves FLAGS.
   Enhanced reads and peeks return 1C0Dh for main Enter and E00Dh for
   keypad Enter; legacy returns 1C0Dh for either. Ctrl+F3 is 6000h, and
   Alt+X is 2D00h (AL=0). See SeaBIOS 1.16.2, `src/kbd.c`,
   `scan_to_keycode`, `key_ext_enter`, and `dequeue_key` for the reference.
5. At the pinned upstream DN revision cited in DN-22, `DRIVERS.PAS`
   reads with AH=11h/10h and explicitly converts E00Dh to `kbEnter`.
   `FLPANELX.PAS` dispatches `kbEnter` on the selected item. The English
   `DN.DNR` binds Ctrl+F3 to New Manager and Alt+X to Quit. The
   bindings' effects still depend on DN's active view and modal state.

Queue behavior was inspected separately. Display's pending ring keeps 32
events and drops the oldest on overflow. Chronos's guest queue is a
VecDeque with no hardware slot limit. Neither overflow policy changed.
The focused tests check repeat ordering and consumption; they do not
establish real hardware overrun or BIOS Data Area ring parity. The next
keyboard boundary is the distinction between dedicated E0 navigation
keys and keypad navigation/digits under Num Lock and modifiers.

## DOS exit, Chronos, and desktop ownership

DOS AH=4Ch ends the active DOS process. Child termination restores its
parent, releases the child's allocations and records its result for
AH=4Dh. Only termination of the last process sets
`GuestState::Exited { code }`, and only that state completes the session.
The [DN-23 host package trace](../dn-23/host-package-exit.log) shows
DN.PRG returning to DN.COM and final AH=4C00h with no suspended parent;
the earlier [launcher trace](../dn-21/README.md#keyboard-and-visible-guest-response)
also records the parent collecting the result and restoring its interrupt
vector. It was not a still-running DOS shell after exit.

The prior window loop had no completion policy. It was intentional to
leave the exited text view on screen by default; additionally, overlay
persistence was checked in startup and tick paths, rather than at the
common execution boundary where a keyboard-triggered slice can also
terminate. No native child process exists for the emulated DOS child.
Ctrl+W previously closed the window from outside Chronos. The compositor's
ordinary last-window policy immediately requested SIGTERM, giving the
native status 143; scheduler cleanup happened after that signal.

Chronos now acknowledges the final status once, at the end of its
common scheduled slice, after attempting the existing overlay flush.
A bundle may set `[chronos] on_exit = "close-window"`; omitted or
`"keep-window"` retains the prior window behavior. The launcher passes
the close policy as a flag, not a guest argument. Chronos uses the
compositor's existing bounded close grace, requests its own normal
window close, and passes the final DOS code to native ProcessExit. The
desktop's supervision and scheduler policies did not change. Mines and
the DOS shell leave the new policy unset; an emulated launcher that
returns to a waiting parent does not request window close.

## Validation

- [Host unit and workload log](units-final.log): 199 Chronos core passed,
  one ignored; 35 Chronos UI and 291 Sunlight UI passed. This includes
  MIDTERM's guest PASS marker, Mines, physical Enter identity, modifier
  make/break and focus loss, peek/read registers and FLAGS, repeat FIFO,
  child resource ownership, parent recovery, and final codes 0 and 42.
- [Launcher tests](launcher-tests.log): 9 passed. [Core no_std](no-std.log)
  and [native traced release build](native-final-traced.log) passed.
- [Separate UI doctest](doctest.log): the unchanged `horizon.rs` ASCII
  diagram still fails parsing. It is separate from the UI unit suite.

To record comparable native keyboard evidence, build the test ISO with
`CHRONOS_KEY_TRACE=1 bash tools/test.sh phase3.0`, then run:

```sh
python3 docs/audit/chronos-dn/dn-24/replay.py target/sunlightos.iso target/dn24-final
```

The opt-in build records at most 128 native keyboard events and 256
nonempty INT 16h returns per process. Ordinary builds leave it off.

## Native keyboard results

The [replay](replay.py) booted a fresh diskless two-vCPU QEMU instance with
the [built ISO](native-image.sha256) and the normal PS/2 and compositor
routes. Both launches were selected from normal desktop search. The
[inputs](native/inputs.jsonl), [keyboard log](native/keyboard.log) and
[full serial log](native/serial.log.gz) record one desktop session and
two different DN processes. The first exploratory run also exited
naturally; its [driver](desktop-driver.log) falsely timed out on a
split-line session log, so its [evidence](first-desktop-attempt/) is
retained separately. The replay now checks the actual log format and
the [two-run driver](desktop-verified-driver.log) finished successfully
on the same ISO.

| Interaction | Native effect |
| --- | --- |
| Main Enter | [COLORS selected](native/dn-1-directory-selected.png) → [directory open](native/dn-1-directory-opened.png), showing actual palette files. The host event had code 1Ch without E0; AH=11h and AH=10h returned AX=1C0Dh. |
| Keypad Enter | [COLORS selected](native/dn-2-directory-selected.png) → [directory open](native/dn-2-directory-opened.png). The host event had the same code with E0; both BIOS calls returned AX=E00Dh. |
| Ctrl+F3 | The Ctrl make precedes the F3 make; Ctrl remains set through F3 release and clears on its own release. Both BIOS calls returned AX=6000h. The [picker](native/dn-2-ctrl-f3.png) opens in both runs and [Escape dismisses it](native/dn-2-escape.png). |
| Alt+X | Alt remains active across the X make/break. Both BIOS calls returned AX=2D00h and DN opened its [quit confirmation](native/dn-2-alt-x.png) from the active file panels after Escape. Y confirmed the quit. No claim is made for other modal states. |
| F10, Manager → New, C | [F10 opens the menu](native/dn-1-f10.png). M, N open the [drive picker](native/dn-1-manager-new.png); C closes it and leaves [both panels populated](native/dn-1-new-panels.png). This repeated on launch two. |

The native status/read pairs show the same AX, FLAGS=F206h, and queue
depth changing from 1 to 0 on the consuming call. The full register
preservation, empty checks, legacy form and repeats are covered in host
tests. The extended-navigation distinction, BIOS Data Area flags/ring
parity, and overflow under sustained native input remain unverified.

## Native lifecycle results

| Workload | DOS/session result | Native process and desktop |
| --- | --- | --- |
| DN first launch, PID 39 | Alt+X → Y, final DOS session code 0 | Owner closes window 3, then exits 0. Scheduler reaps 2,819 user frames and 13 page tables. Desktop removes the app and [restores the prior window](native/dn-1-restored.png). |
| DN repeat launch, PID 40 | Same sequence, final DOS session code 0 | Owner closes window 4, then exits 0; same cleanup counts and [restored desktop](native/dn-2-restored.png). |
| Mines, PID 41 | [Board appears](native/mines-first.png); Escape completes the DOS session with code 0. The [Mines window stays open](native/mines-guest-exit.png). | Ctrl+W still invokes the original SIGTERM policy and native status 143. The scheduler reaps it and the [desktop is restored](native/mines-restored.png). |

The [lifecycle extract](native/lifecycle.log) shows both DN overlay
writes before the session completion messages, owner window detach,
native status and scheduler cleanup, then desktop instance removal.
The input log contains no Ctrl+W for either DN run. The compositor
never requested SIGTERM or forced termination for either DN PID.
Mines' retained window can still show the old Running footer after
the DOS session stops, as in [DN-23](../dn-23/46-mines-exit.png); the
new session log distinguishes that display from the actual guest result.
This is two successive launches, not a leak or persistence soak test.

The normal [native release build](native-release.log), the
[independent Phase 3.0 gate](platform.log) and the actual desktop replay
all passed. The [platform build](platform-build.log.gz) and
[platform serial](platform-serial.log.gz) logs are independent of the
[desktop traces](native/serial.log.gz). The
[embedded ELF check](embedded-chronos.log) verified that the rebuilt
kernel contained the traced release binary used for the desktop run.
