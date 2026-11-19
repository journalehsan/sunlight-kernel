# DN-21: Enhanced keyboard read and direct startup

Two independent findings explain the observed behavior. First, DN.PRG creates
its initial panels on the launcher's first-run path. The direct run skips the
launcher handshake and enters the application loop without the panels. Second,
both paths could see an available key but stopped when they tried to read it
with INT 16h/AH=10h. DN-21 fixes the read. It leaves program loading, process
ownership, text rendering and the launcher's private protocol unchanged.

## Baseline and execution paths

Before editing the dispatcher, both original no-input runs reproduced DN-20:
decompressed traces matched byte for byte and screenshots matched pixel for
pixel ([direct comparison](baseline-direct-comparison.log),
[launcher comparison](baseline-launcher-comparison.log)).
The same three [SHA-256 pins](inputs.sha256) passed
[before](inputs-before.log) and [after](inputs-after.log).

| Entry, hex | CS:IP | SS:SP | DS=ES | FLAGS | Process |
| --- | --- | --- | --- | --- | --- |
| DN.COM | 1041:0100 | 1041:FFFE | 1041 | 0002 | PSP 1041; environment 1000; no parent |
| Direct DN.PRG | 1051:0000 | 4C7C:0100 | 1041 | 0002 | PSP 1041; environment 1000; no parent |
| DN.PRG through EXEC | 10CF:0000 | 4CFA:0100 | 10BF | 0002 | PSP 10BF; environment 107E; suspended parent 1041 |

Both MZ entries use the same relative entry and stack, relocated by the PSP
difference of 007Eh paragraphs. All tails are length zero followed by CR.
The child inherits the launcher's environment variables, including
`COMSPEC=C:\DN.COM` and `SHELL=C:\DN.COM`; the direct probe sets those
variables to `C:\DN.PRG`. PATH, TMP, TEMP and APPID match. EXEC copies the
environment variables and writes the child's executable path into its
environment trailer. There is no evidence that the empty tail, environment
variable differences or MZ relocation cause the missing panel layout.

The earliest observed panel-relevant difference is a guest protocol. Before
EXEC, DN.COM uses DOS AH=35h/25h to get and replace INT 2Fh, pointing it at
`1041:0142`. EXEC transfers to DN.PRG at 895 retired instructions. DN.PRG
queries INT 2Fh with AX=9900h at the same relative entry location
(`PSP+10h:15A3`) on both paths:

| Path | Before the INT | At return to DN.PRG |
| --- | --- | --- |
| Direct | 3,403,544; BX=5CA8; vector 0800:0400 | 3,403,546; AX=9900 BX=5CA8 DX=0000 FLAGS=F046; immediately skips AX=9901h |
| Launcher child | 3,404,439; BX=5D26; vector 1041:0142 | 3,404,456; AX=0101 BX=444E DX=2151 FLAGS=F046; then calls AX=9901h for the launcher command buffer |

The upstream RIT Labs source at revision
`b268678fad8a9ccb3fd6a9463e1e341ac73985ec` explains those observed return
values: `DN.ASM` returns its run number and first-run byte in AL and AH;
`DN.PAS` checks the DN signature, copies AH to `RunFirst`, and guards
`cmFirstTimePanel` with that flag. The source is corroboration, not a
reproducible build of the tested binaries. The trace shows the actual
launcher signature and first-run byte, and the direct guest's missing
signature and skipped follow-up. The missing prerequisite for automatic
panels is DN's own launcher state. No missing DOS loader contract was
established for that divergence; DOS should not invent an installed
multiplex handler for a directly started guest.

The launcher issues five FindFirst calls (three normal no-match returns and two
successes), then forty FindNext calls (38 successes, two AX=0012h/CF=1
exhaustion returns). Direct has zero calls to either service across all
recorded runs. The launcher also queries free space twice and gets AX=FFFFh;
“0 free bytes” is DN's rendering of the unavailable result. See the full
[direct trace](baseline-direct.log.gz), [launcher trace](baseline-launcher.log.gz)
and [interrupt totals](interrupt-summary.log) for DOS inputs and returns.

## Keyboard and visible guest response

With no input, the first AH=11h calls retire at 3,809,845 (direct) and
4,463,619 (child). Both return AX=1100h with ZF set (FLAGS=F246), queue 0.
Across 10 million guest instructions there are 7,209 direct and 6,313 child
polls and no AH=00h, AH=01h or AH=10h. Both guests remain Running: polling
and doing guest work between polls, with frequent INT 10h cursor operations.
There is no evidence of a blocked DOS service or a blank host presentation
hiding initialized guest panels. Both finish with cursor at column 4, row 23,
shape 0607h. Direct stays at the prompt and shortcut strip; the launcher
has both populated panels.

Before the change, at 5,000,000 retired instructions the probe queued
one F10 (4400h) or Down (5000h). AH=11h returned the key in AX, cleared ZF
and left queue depth at 1. The next AH=10h stopped with AX=1000h, FLAGS=F206,
and the key still queued. Direct trapped at 5,000,256 retired
(`11CB:01D5`, CPU IP advanced to 01D7). The launcher child trapped at
5,000,330 (`1249:01D5`); the runtime restored its parent at `1041:01F0`
with the launcher's vector restored. The probe stops at the recovered trap
boundary. See [direct F10 before](direct-f10.log.gz) and
[launcher Down before](launcher-down.log.gz).

The sole runtime change pairs AH=10h with AH=00h at the BIOS dispatcher.
Like SeaBIOS's enhanced read, AH=10h waits if empty and consumes the oldest
available key. Within Chronos's existing unified key model it preserves the
scan/ASCII word in AX and leaves FLAGS and other registers unchanged. The
focused instruction test exercises FIFO order, an ASCII-zero scan code, a
printable key, preservation, empty wait, and completion on delivery. The
existing enhanced-status test verifies nonconsuming reads and ZF. The
runtime's VecDeque has no fixed hardware FIFO overflow rule; these runs do
not establish IBM keyboard-controller or BDA circular-buffer parity. No new
translation, overflow policy, or AH=12h implementation was added.

After the change, AH=10h retires one instruction later than the original
stop (5,000,257 or 5,000,331). Queue depth goes from 1 to 0, FLAGS stays
F206h, and AX is the same key word that status reported. The following are
actual text-cell changes:

| Host input | Visible response | Stop (retired, CS:IP, FLAGS) |
| --- | --- | --- |
| [Direct F10](final-direct-f10.png) | Opens the top menu; no panels | Running, 7,000,000, 3C56:3FFB, F246 |
| [Direct Down](final-direct-down.png) | Consumes key, no visible change at the prompt | Running, 7,000,000, 1443:11E1, F206 |
| [Launcher F10](final-launcher-f10.png) | Opens the top menu over both populated panels | Running, 7,000,000, 394E:0826, F246 |
| [Launcher Down](final-launcher-down.png) | Right-panel selection moves from DN.COM to FILES.BBS; detail line changes from 1,778 to 964 bytes | Running, 7,000,000, 394E:07DC, F286 |

Direct F10 and launcher Down repeated with byte-identical traces and
pixel-identical images. The final no-input images also match their baselines
exactly ([comparison](final-comparison.log)). Final no-input direct stopped
Running at 10,000,000, `11CB:0036` FLAGS=F246, PSP=1041. The launcher
stopped Running at the same count, `29EF:0431` FLAGS=F202, PSP=10BF, parent
still suspended. No later unsupported DOS service, BIOS service or CPU
instruction was observed in the fixed runs.

The [direct exit trace](final-direct-exit.log.gz) records Alt+X then Y,
DOS AX=4C00h, and exit 0 at 5,335,016, `3C56:01F0` FLAGS=F246. The
[launcher exit trace](final-launcher-exit.log.gz) records F10, Escape,
Alt+X and Y, then a normal child exit at 5,680,799. The parent resumes at
`1041:01F0` FLAGS=0006, collects status through AH=4Dh, restores INT 2Fh
to `0800:0400`, and exits 0 at 5,680,816, `1041:0276` FLAGS=0096.
An extra queued Enter was still pending across child exit and parent
recovery. This is consistent with the runtime-wide FIFO; EXEC and parent
recovery reset pending input for the old context without dropping queue
contents. This run did not exhaust the queue or test lifetime across
separate VM instances.

Captures render actual guest B8000 text cells with the Chronos BIOS font
offline. The probe records cell checksums, cursor state, registers, FLAGS,
PSP and queue depth. The native frontend separately translates keyboard
events and updates its text presentation from guest state on scheduled slices.
No native event delivery or presentation was inferred from these images.

## Reproduction and validation

Use DN-20's commands for the baseline. To reproduce the opt-in host
interaction, apply the saved [probe patch](interactive-probe.patch) to a
temporary second example; the regular diagnostic runner was left unchanged.

```sh
d=docs/audit/chronos-dn/dn-21
patch -o chronos-core/examples/dn21_probe.rs chronos-core/examples/dos_probe.rs < "$d/interactive-probe.patch"
cargo build -p chronos-core --example dn21_probe --target x86_64-unknown-linux-gnu
rm chronos-core/examples/dn21_probe.rs
probe=target/x86_64-unknown-linux-gnu/debug/examples/dn21_probe
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=5000000 DN21_KEYS=5000000:4400 \
  CHRONOS_TEXT_CAPTURE=target/direct-f10.ppm "$probe" target/dos-navigator/dn151/DN.PRG
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=5000000 DN21_KEYS=5000000:5000 \
  CHRONOS_TEXT_CAPTURE=target/launcher-down.ppm "$probe" target/dos-navigator/dn151/DN.COM
sha256sum -c "$d/inputs.sha256"
```

| Area | Verdict |
| --- | --- |
| Unchanged authentic baselines | **PASS**: both reproduce DN-20 |
| Enhanced BIOS read / status | **PASS** for the tested Chronos FIFO contract |
| Launcher panel layout and actual enumeration | **PASS** on host: two populated C: panels |
| Direct startup and panel enumeration | **PARTIAL** on host: starts, opens menu and exits; no automatic panels or enumeration |
| Controlled interactive guest response | **PASS** on host for F10 on both paths and Down on launcher |
| Child termination, parent recovery and clean exit | **PASS** on host for the observed paths |
| Core, Chronos UI, Sunlight UI | **PASS**: 197/33/290 passed, one core ignored; [unit logs](units.log) include MIDTERM and Mines |
| Core no_std and native release | **PASS**: [no_std](no-std.log), [release](native.log) |
| Phase 3.0 platform QEMU gate | **PASS**: [boot log](platform.log); does not run DN |
| Unrelated Sunlight UI doctest | **BLOCKED**: [horizon.rs diagram parsed as Rust](doctest.log) |
| Native QEMU DN | **NOT_RUN**: the adapter caps imports at 65,536 bytes and does not select the required 386 profile |

For native validation, the next blocker is the adapter: it must import the
complete authentic files and select the required CPU profile before repeating
the host workflows with real native input. For direct launch, the next useful
test is to invoke DN's own panel command or compare direct startup under
reference DOS to establish the intended user workflow without manufacturing
the launcher handshake.
