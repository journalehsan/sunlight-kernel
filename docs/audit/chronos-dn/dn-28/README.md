# DN-28: Modifier state and focus transitions

## Policy and boundaries

The PS/2 keyboard service (`services/sunlight-kbd/src/lib.rs`, `Keyboard::process`)
tracks left and right Shift separately on make and break and attaches aggregate
modifier snapshots to each physical event. Ordinary window focus changes do not
clear this service's physical state. The compositor routes events to the
currently focused window. Its separate `release_session_keys` behavior is for
session handoff, not ordinary window focus transfer.

The UI framework returns a `FocusChanged` event before an accompanying key,
retaining that key for a later poll. On focus loss Chronos calls
`Runtime::release_host_keys`: it clears modeled transient modifiers and the
Num Lock make latch, retains the guest Num Lock value, and publishes the flags
to BDA 0040:0017. It does not synthesize physical releases. Focus regain does
not synthesize modifier makes. Physical state and guest state therefore can
intentionally differ when a modifier remains physically held across focus loss.
An aggregate Shift snapshot on a later ordinary key cannot identify a side and
cannot restore either BIOS Shift bit; a fresh side-specific make can. A break
routed elsewhere is not necessary to clear the guest state.

`update_modifiers` publishes mask 2Fh using the same computed state as INT
16h/AH=02h, preserving unmodeled bits. AH=12h additionally reports known
Ctrl/Alt sides. DN-27's actual guest instruction test checks the BDA and BIOS
separately and retains sentinel bits D0h. This policy is not an assertion about
physical keyboard state after a lost hardware break.

The focused host UI test runs these sequences for left Shift, right Shift and
both: make (BIOS 02h, 01h or 03h), focus loss (00h), focus regain, ordinary
key with held physical snapshot (00h), side breaks (00h), fresh make (02h or
01h), fresh break (00h). It also omits the original break across another focus
loss, then checks a plain key and a fresh make/break. The host test invokes the
actual Chronos event callback and BIOS guest instructions; it does not exercise
physical hardware or compositor routing. It reads the guest BDA and compares
it with the guest INT 16h result at each asserted boundary, preserving D0h.
The existing core test covers BDA/BIOS equality for Shift overlap, releasing
one side, Ctrl/Alt overlap, repeated Num Lock make, and focus cleanup. See
[Chronos UI results](chronos-ui-final.log), [core results](core.log), and the
[DN-27 test description](../dn-27/README.md#correction-and-focused-verification).
Only a regression test was added; no production behavior was changed.

## Regression results

Commands run at the repository root:

| Command | Result |
| --- | --- |
| `cargo test --offline -p chronos-core --lib --target x86_64-unknown-linux-gnu dos::tests` | 39 passed, baseline before edits |
| `cargo test --offline -p chronos-core --lib --target x86_64-unknown-linux-gnu` | 205 passed, 1 ignored; includes DN-25 216-case matrix, DN-26 byte and buffered input, DN-27 BDA, MIDTERM and Mines MZ |
| `cargo test --offline -p sunlight-chronos --target x86_64-unknown-linux-gnu` | 37 passed, including the new focus test and Mines interaction |
| `cargo test --offline -p sunlight-ui --lib --target x86_64-unknown-linux-gnu` | 277 passed |
| `cargo test --offline -p sunlight-kbd --lib --target x86_64-unknown-linux-gnu` | 5 passed |
| `cargo test --offline -p sunlight-display --target x86_64-unknown-linux-gnu focus` | 9 passed |
| `cargo test --offline -p sunlight-display --target x86_64-unknown-linux-gnu` | 77 passed, 6 failed in unrelated shadow, hit testing and damage tests; [log](display-tests.log) |
| `cargo check --offline -p chronos-core --no-default-features --target x86_64-unknown-none` | PASS |
| `env -u CHRONOS_KEY_TRACE RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' cargo build --offline -p sunlight-chronos --release --target x86_64-unknown-none` | PASS |

The known unrelated `horizon.rs` doctest failure was not rerun or modified;
see [DN-27](../dn-27/doctest.log).

`env -u SUNLIGHT_TEST_MEMORY_MB CHRONOS_KEY_TRACE=1 SUNLIGHT_TEST_SERIAL_LOG=target/dn28-1g.serial bash tools/test.sh phase3.0` failed: 31 expected strings plus PMM found, timezone missing; PMM reported 145/427 MiB free. Five ELF segment mapping failures, five `NoMemory` entries and five service `failed to spawn` entries appear in the gate window. [Gate](phase-1g.log), [serial](phase-1g-serial.log.gz).

`CHRONOS_KEY_TRACE=1 SUNLIGHT_TEST_MEMORY_MB=4096 SUNLIGHT_TEST_SERIAL_LOG=target/dn28-4g.serial bash tools/test.sh phase3.0` passed all required markers and PMM (3217/3499 MiB free), with no matching ELF mapping, `NoMemory` or service spawn failures in the gate window. [Gate](phase-4g.log), [serial](phase-4g-serial.log.gz). These gates do not prove desktop input or application behavior.

## Native QEMU focus replay

The replay boots the original DN-27 4 GiB desktop ISO, whose SHA-256 is
`c19500133ec20f7454ac2b83eb95e3b1a78c1f96e9390d1a3efe913ab8836cae`
([digests](images.sha256)). It launches Mines as a second real window and
DOS Navigator from desktop search. In Select → File mask it enters `abcdef`,
positions at the third character, sends QMP PS/2 Shift make and Left, then
Alt+Tab to Mines and two more Alt+Tab cycles through the existing Welcome
window back to DN while keeping Shift down. It sends Right, Shift break, Left,
and a fresh Shift make/Left/break/Right. This is emulated PS/2 input, not a
measurement of a physical host keyboard. The procedure runs left, right and
both Shift independently. With both held, it releases left then right, but
sends no ordinary key between the two breaks.

[Before loss](native/shift-before-loss.png), [Mines focused](native/shift-lost.png),
[DN regained](native/shift-regained.png), [ordinary key while held](native/shift-ordinary-held.png),
[fresh Shift make](native/shift-fresh-make.png), and corresponding
[right Shift](native/shift_r-regained.png) and [both Shift](native/both-regained.png)
frames are retained. The [scanout check](scanout.log), run with
`python3 docs/audit/chronos-dn/dn-28/scanout.py target/dn28-cycle`, checks six
cell background pixels in every named frame: `c` selected before loss and
at regain, empty after the first Right, and `b` after fresh Shift+Left.
It checks selection backgrounds, not guest memory or physical switch state.
The two breaks in the both-Shift case were not separated by a navigation key;
the existing bounded native key trace runs out before those breaks, so the
remaining physical-side state is not independently demonstrated there.
No missing-break scenario was induced in QEMU. The host callback test covers
the latter case, not the hardware/desktop path.

Earlier attempts are preserved: [search shortcut](failed-search-gesture/shift-lost.png)
did not transfer focus because the shortcut rejects Shift; the
[two-step Alt+Tab](failed-two-window-cycle/shift-regained.png) reached Welcome
rather than DN. Neither is treated as recovery evidence.

The input stream and captures were produced by
`python3 docs/audit/chronos-dn/dn-28/desktop.py target/dn27-desktop.iso target/dn28-cycle`.
See [QEMU command](native/command.json), [all input edges](native/inputs.jsonl),
[bounded Chronos key trace](native/keyboard.log), [full serial](native/serial.log.gz),
and [desktop driver log](desktop-cycle.log). In the serial, the left Shift
make is line 2811; the post-regain Right make reports `shift: true` at 2898,
followed by the physical left break at 2911. The right Shift sequence has the
corresponding make at 3051, post-regain Right with `shift: true` at 3081, and
break at 3094. Those snapshots show the emulated keyboard service still
reported Shift held after the focus cycle; the scanout shows DN treated that
Right as unshifted. They do not directly read native Chronos's internal state,
INT 16h/AH=02h, or BDA at the transition. The host test checks those layers.

Escape canceled the first mask and Enter closed the reopened mask
([cancel](native/dn-1-mask-cancel.png), [accept](native/dn-1-mask-accepted.png)).
Both DN runs completed the guest with code 0, exited Chronos naturally with
code 0, removed the desktop window and reaped PIDs 40 and 41. The
[second search](native/dn-2-search.png) happened in the same session.
[Lifecycle](native/lifecycle.log), [first restored](native/dn-1-restored.png),
[second restored](native/dn-2-restored.png). A separately launched Mines guest
(PID 42) exited with code 0; its Chronos window then required the normal
Ctrl+W termination, exited 143, was reaped and removed
([board](native/mines-play.png), [guest exit](native/mines-guest-exit.png),
[restored](native/mines-restored.png)). The original Mines focus peer was
still running when the QEMU session ended.

All 66 original SunApp file hashes in the DN-27 manifest matched
[before](package-before.log) and [after](package-after.log) this replay using
`sha256sum -c docs/audit/chronos-dn/dn-27/package.sha256`. No package file was
modified. Physical host hardware, a missing-break desktop path, the state of
the second held Shift after just the first break, and native direct BIOS/BDA
readback at focus boundaries remain unverified.
