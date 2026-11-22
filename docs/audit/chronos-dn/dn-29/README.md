# DN-29: Dual Shift overlap and independent breaks

No production code or SunApp payload was changed in the workspace. DN-29 adds
two host regressions, a stock desktop replay, and a separate real-mode probe.
The [DN-28 audit](../dn-28/README.md) describes the focus policy and the previous
native trace cutoff.

## Input policy and host matrix

`Keyboard::process` tracks the PS/2 Shift sides independently (left
`2A/AA`, right `36/B6`). Its routed event includes the edge's key identity and
an aggregate Shift snapshot. Ordinary compositor window focus changes do not
reset the keyboard service. The UI delivers the focus transition ahead of the
corresponding key. Chronos clears transient guest modifiers on loss and does
not infer a guest Shift side from the aggregate snapshot on regain.

The driver test checks the internal side bits and routed event at each edge.
The Chronos UI test supplies edge identities and aggregate snapshots to the
actual UI callback. Its guest instructions write the unmodeled `D0h` sentinel,
read `0040:0017`, and execute both INT 16h services at each checkpoint. This is
host execution of guest instructions; the native probe is separate.

| Case | Input order | Physical sides after each event | Guest Shift bits |
| --- | --- | --- | --- |
| A | L make, R make, L break, R break | L, LR, R, none | `02 03 01 00` |
| B | R make, L make, R break, L break | R, LR, L, none | `01 03 02 00` |
| C | L make, R make, loss, regain, L break, R break | L, LR, LR, LR, R, none | `02 03 00 00 00 00` |
| D | R make, L make, loss, regain, R break, pause, L break | R, LR, LR, LR, L, L, none | `01 03 00 00 00 00 00` |
| E | first make twice, second make, first break, first make, second break, first break (both side orders) | first, first, LR, second, LR, first, none | first, first, `03`, second, `03`, first, `00` |

The host driver does not receive a focus event in C/D. The host UI test runs
the two callbacks and checks the state before the delayed break arrives. It
also tests both fresh cycles after cleanup. Windows may receive different
events while unfocused; the table is for breaks routed back after regain.
In the host guest probe, AH=02h returns AX=`0200h | bits`, AH=12h returns
AX=`0000h | bits` for Shift-only input, and FLAGS stays `0247h`. The BDA is
`D0h | bits`; neither BIOS query enqueues a key.

## Regression commands and counts

Run from the repository root. Host tests use
`cargo test --offline --target x86_64-unknown-linux-gnu` with these arguments:

| Arguments | Result |
| --- | --- |
| `-p sunlight-chronos shift_focus_cleanup_and_fresh_edges_follow_physical_snapshots` | 1 passed, pre-extension DN-28 baseline |
| `-p chronos-core --lib` | 205 passed, 1 ignored; includes DN-25's 216-case matrix, DN-26 byte and buffered input, DN-27 BDA/BIOS, MIDTERM batch and Mines MZ load |
| `-p sunlight-chronos` | 38 passed; includes the DN-28 recovery and DN-29 overlap tests and Mines interaction |
| `-p sunlight-ui --lib` | 277 passed |
| `-p sunlight-kbd --lib` | 6 passed |
| `-p sunlight-display focus` | 9 passed, 2 auxiliary target tests filtered out |
| `-p sunlight-display` | 77 passed, 6 failed in the compositor binary; all 2 auxiliary target tests passed |

`cargo check --offline -p chronos-core --no-default-features --target x86_64-unknown-none`
passed. The native release build passed:

```sh
env -u CHRONOS_KEY_TRACE RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' cargo build --offline -p sunlight-chronos --release --target x86_64-unknown-none
```

The compositor failures reproduce DN-28 and were left alone:
`ambient_shadow_falloff_is_wider_and_monotone`,
`compact_close_hit_test_only_exposes_close_button`,
`compact_close_minimize_hit_test_exposes_minimize_and_close_only`,
`inactive_workspace_windows_do_not_receive_hit_tests`,
`maximize_vs_restore_hit_zone_same_rect`, and
`move_resize_and_hide_damage_complete_old_effect_region`.
The unrelated [`horizon.rs` doctest failure](../dn-27/doctest.log) was not rerun.
Host transcripts are under `target/dn29/`.

## Phase 3.0

```sh
env -u SUNLIGHT_TEST_MEMORY_MB SUNLIGHT_TEST_SERIAL_LOG=target/dn29/phase-1g.serial bash tools/test.sh phase3.0
SUNLIGHT_TEST_MEMORY_MB=4096 SUNLIGHT_TEST_SERIAL_LOG=target/dn29/phase-4g.serial bash tools/test.sh phase3.0
```

| Memory | Required markers and PMM | ELF segment map failures | `NoMemory` | Service spawn failures | Desktop |
| --- | --- | ---: | ---: | --- | --- |
| Default 1 GiB | 32/32 markers; 145/428 MiB PMM free | 1 | 11 | 4: gcd, uac_service, rand_service, timed | Not demonstrated in gate; **PARTIAL** |
| 4 GiB | 32/32 markers; 3217/3499 MiB PMM free | 0 | 0 | 0 in gate window | Gate did not drive login; **PASS** for the boot gate only |

Counts are serial-line counts, and some reports refer to the same failed
spawn. The 4 GiB interactive replay below uses a separately pinned desktop ISO.

## Stock desktop and DOS Navigator

```sh
python3 docs/audit/chronos-dn/dn-28/desktop.py target/dn27-desktop.iso target/dn29/baseline
python3 docs/audit/chronos-dn/dn-28/scanout.py target/dn29/baseline
python3 docs/audit/chronos-dn/dn-29/desktop.py target/dn27-desktop.iso target/dn29/overlap-final
```

The unchanged DN-28 replay ran first. It held Left, then Right, then both
across real Alt+Tab focus transfers to Mines and back past Welcome. With
each individual side, Left selected `c` before loss, Right on return was
ordinary while the side was still physically held, and a fresh make plus
Left selected `b`. Left make, post-regain Right and Left break appear at
serial lines 2850, 2879 and 2891; the Right-side sequence is at 3027, 3055
and 3068. The original replay again exhausted the DN key trace after both
makes (3204, 3207) and before their breaks; the last traced event in that
first DN process is at 3223. The [baseline input stream](baseline/inputs.jsonl),
[serial](baseline/serial.log.gz), and [scanout assertions](baseline/scanout.log)
preserve that limit.

The new replay searched for DOS Navigator from the desktop and opened
[Select → File mask](desktop/file-mask.png) with the keypad Plus shortcut.
For each make/break order, the guest field contained `abcdef`, with the cursor
anchored after `c`. With both keys down, Left, Right, Right selected `c`,
then nothing, then `d`. Releasing the first key and pressing Right selected
`de` ([Left released](desktop/shift-one-held.png),
[Right released](desktop/shift_r-one-held.png)). After releasing the second
key, plain Left cleared selection ([left-first](desktop/shift-both-released.png),
[right-first](desktop/shift_r-both-released.png)). A fresh independent Shift
make and Left selected `d` again
([Right](desktop/shift-fresh.png), [Left](desktop/shift_r-fresh.png)).
All 16 scanout assertions passed. The [QMP input and frame log](desktop/inputs.jsonl)
contains byte offsets in the [serial log](desktop/serial.log.gz).

The [key trace](desktop/keyboard.log) independently logs both makes in
order, then breaks with aggregate Shift true after the first and false
after the second. Left-first: lines 2226, 2228, 2317 and 2329. Right-first:
2400, 2402, 2436 and 2448. The Right arrow between each pair of breaks
also reports Shift true. These are emulated PS/2 events observed by Chronos;
the keyboard driver's actual two-bit mask is asserted by the host driver test.
Neither the trace nor screen pixels constitute a direct BIOS/BDA read.

[Escape canceled](desktop/mask-canceled.png); reopening and pressing
[Enter accepted](desktop/mask-accepted.png). Both runs exited DOS with
code 0; Chronos exited naturally with code 0, the kernel reaped PIDs 39 and
40, and the desktop removed both instances. A [second search](desktop/search-2.png)
and launch succeeded in the same desktop session. See
[lifecycle](desktop/lifecycle.log) and [restored desktop](desktop/restored-2.png).
The DN-28 replay also launched the original Mines package, played
and exited the guest with code 0. Its native window closed through Ctrl+W
(Chronos exit 143), was reaped, and disappeared
([play](baseline/mines-play.png), [guest exit](baseline/mines-guest-exit.png),
[restored](baseline/mines-restored.png), [lifecycle](baseline/lifecycle.log)).

Two earlier overlap attempts reached the correct dialog but used scanout
coordinates from the DN-28 three-window layout; the new two-window layout
required a different sample location. Those attempts stopped before overlap
input and are retained in `target/dn29/overlap{,-retry}`.

## Native guest BIOS and BDA

```sh
python3 docs/audit/chronos-dn/dn-29/native_probe.py target/dn27-desktop.iso target/dn29/probe-readback.iso target/dn29/probe-readback
python3 docs/audit/chronos-dn/dn-29/probe_scanout.py target/dn29/probe-readback
```

The diagnostic ISO is a disposable copy of the pinned stock desktop ISO.
The runner substitutes [the assembled COM](modifiers.asm) for the two
embedded copies of the Mines payload (offsets 91320608 and 184506406, each
3178 bytes, padded with NOPs). It launches through the existing Mines desktop
entry and the unmodified native Chronos loader. No DOS Navigator bytes,
source package bytes, native executable code or repo fixtures are edited.
This diagnostic run is separate from the authentic Mines and DN runs.
Earlier attempts in `target/dn29/` stopped at an unavailable File Lab
search result, captured the Welcome window covering the readout, or failed
to locate the mouse pointer sprite. The final run uses Alt+Tab and places
the readout below Welcome's window.

The guest writes `D0h` to the BDA as an unmodeled-bit sentinel, loads
`0040:0017` from real-mode guest memory, invokes INT 16h/AH=02h and AH=12h
with AL=`FFh` and FLAGS=`0247h`, then prints the byte, both AX values, and
both FLAGS values through guest B800 memory. The scanout verifier decodes
the displayed digits using the guest's hexadecimal glyph row. All
[25 samples](probe/readback.log) matched, including the following checkpoints:

| Checkpoint | BDA | AX, AH=02h | FLAGS | AX, AH=12h | FLAGS |
| --- | --- | --- | --- | --- | --- |
| [Both down](probe/shift-both.png) | `D3` | `0203` | `0247` | `0003` | `0247` |
| [Left released, Right held](probe/shift-first-break.png) | `D1` | `0201` | `0247` | `0001` | `0247` |
| [Right released, Left held](probe/shift_r-first-break.png) | `D2` | `0202` | `0247` | `0002` | `0247` |
| Both released | `D0` | `0200` | `0247` | `0000` | `0247` |
| [Both held, loss](probe/shift-focus-lost.png) | `D0` | `0200` | `0247` | `0000` | `0247` |
| [Both held, regain](probe/shift-focus-regained.png) | `D0` | `0200` | `0247` | `0000` | `0247` |
| First break, Right arrow while second still held, then second break (both orders) | `D0` at each | `0200` | `0247` | `0000` | `0247` |
| [Fresh Left](probe/shift_r-fresh-make.png) / [fresh Right](probe/shift-fresh-make.png) | `D2` / `D1` | `0202` / `0201` | `0247` | `0002` / `0001` | `0247` |

The [QMP input log](probe/inputs.jsonl) and [routed keyboard trace](probe/keyboard.log)
are independent of the probe display. In the left-first focus cycle, the
first break at serial line 1999 leaves aggregate Shift true, the subsequent
Right at 2003 still reports true, and the second break at 2008 reports false.
The right-first equivalents are 2086, 2090 and 2095. See
[full serial](probe/serial.log.gz). The entire delayed interval is after
regain; a break permanently lost or delivered to another window was not
injected. Duplicate makes/interleaved case E was covered by the host
regressions. The native traces exercise QEMU's PS/2 device, not a physical
host keyboard. The stock DN replay and the native probe are separate sessions.

All 66 hashes in the [original manifest](../dn-27/package.sha256) matched
[before](package-before.log) and [after](package-after.log) via
`sha256sum -c docs/audit/chronos-dn/dn-27/package.sha256`. Both ISO digests,
the probe COM digest and the manifest digest are in [images.sha256](images.sha256).

## Verdicts

| Check | Status |
| --- | --- |
| Dual-Shift physical tracking | PASS (PS/2 in QEMU; direct side bits in host driver test) |
| Independent Left Shift break | PASS |
| Independent Right Shift break | PASS |
| Focus cleanup with both held | PASS |
| BIOS modifier readback | PASS |
| Native BDA readback | PASS |
| BIOS/BDA consistency | PASS |
| Fresh modifier cycles | PASS |
| Visible DN selection | PASS |
| DN-25 through DN-28 regression | PASS |
| Mines regression | PASS (guest exit and native close/reap) |
| Compositor host suite | FAIL (six existing failures) |
| Phase 3.0 default 1 GiB | PARTIAL (markers pass; service/ELF/allocation errors) |
| Phase 3.0 4 GiB | PASS (boot gate scope) |
| Authentic SunApp desktop validation | PASS |
| Natural guest and Chronos exit | PASS (DOS Navigator, two runs) |
