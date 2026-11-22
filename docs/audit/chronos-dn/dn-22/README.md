# DN-22: Direct startup and the New Manager command

**Direct DN.PRG can create and populate both panels through its own UI.** Press
F10, choose **Manager → New**, then select C: in the drive picker. **Ctrl+F3**
opens the same picker. Verified in the Chronos 386 host probe and independently
under FreeDOS. No Chronos implementation or DN binary changed.

## Startup contract

Source references are to the RIT Labs 1.51 tree mirrored at
[maximmasiutin/Dos-Navigator, b268678](https://github.com/maximmasiutin/Dos-Navigator/tree/b268678fad8a9ccb3fd6a9463e1e341ac73985ec).
The archive digest is in [upstream.sha256](upstream.sha256). Exact source to
binary correspondence is unverified; the authentic binary traces and UI
independently corroborate the behavior below.

- `DN.ASM:79–124,138–170`: DN.COM queries for a prior DN handler, saves
  the existing INT 2Fh vector and installs its own with DOS AH=35h/25h.
  The private AX=9900h response returns BX=444Eh, DX=2151h (version), AL
  as run number, AH as first-run byte, and CL as child error level. NumRun
  and FirstRun start at 1. The launcher clears FirstRun after EXEC returns
  successfully. AX=9901h gives ES:BX of the launcher command buffer.
  AX=9905h reads and clears the launcher's timer.
- `DN.PAS:920–950,993–1026`: only a matching BX signature lets DN assign
  AH to the Boolean `RunFirst` and query the command buffer. It separately
  queries the timer even when the signature is absent. After application
  initialization, a true `RunFirst` and no panels found via
  `cmLookForPanels` trigger `cmFirstTimePanel`. Without first-run state,
  the source takes the swap cleanup branch and enters the application loop.
  `/P` in this block selects a palette; it does not enable panel creation.
  Normal filename arguments are handled separately and conditionally in
  `DNUTIL.PAS:732–740`. Both tested command tails were empty.
- Persistent state can affect the outcome independently of that default
  branch. `DN.PAS:330–424` reads config blocks including startup and panel
  defaults from `DN<DNCFG>.CFG`. `TDNApplication.Init`
  (`DNUTIL.PAS:680–717`) tries `DN<number>.SWP`, then
  `DN<DNDSK>.DSK`, and calls `RetrieveDesktop` if present. That path
  validates the stream and can reload windows and a current directory
  (`DNUTIL.PAS:1020–1100`); it is not gated by `RunFirst`. The fresh
  inputs had no such files, and the probes do not establish saved desktop
  round-trip compatibility or behavior for other startup options.
- Chronos installs an end-of-chain guest IRET at 0800:0400
  (`chronos-core/src/runtime.rs:530`); INT 2Fh is routed through the
  guest vector (`chronos-core/src/dos.rs:15`). EXEC suspends the parent and
  leaves its handler callable; child teardown closes its handles, releases
  its vectors and memory and restores the parent context (runtime.rs
  `exec`, `restore_parent`). The launcher collects AH=4Dh and restores
  its predecessor before it terminates. The direct path requires no change
  to this behavior.

| Startup INT 2Fh | Direct DN.PRG, vector 0800:0400 | DN.COM child, vector 1041:0142 |
| --- | --- | --- |
| AX=9900h | At 3,403,544 with BX=5CA8h; returns AX=9900h, BX=5CA8h, DX=0, FLAGS=F046h | At 3,404,439 with BX=5D26h; returns AX=0101h, BX=444Eh, DX=2151h, FLAGS=F046h |
| AX=9901h | Skipped | Returns ES:BX=1041:0295, FLAGS=F046h |
| AX=9905h | AX=9905h, CX:DX=0, FLAGS=F046h | AX=0000h, CX:DX=0, FLAGS=F046h |

Full registers, all multiplex calls and returns, and vector transitions are
in the compressed traces and [interrupt summary](interrupt-summary.log).

## Native DN command

The English menu mapping is `RESOURCE/ENGLISH/DN.DNR:432–433`:
**Manager → New**, `kbCtrlF3` → `cmCreatePanel`. The application dispatch
in `DNUTIL.PAS:1844,1960` calls `OpenWindow(False)` for automatic startup
and `OpenWindow(True)` for New. The latter calls
`SelectDrive` (`FILESCOL.PAS:355–412`); a cancelled picker returns no
drive and no window is constructed. With a valid drive and successful
allocation, both paths create a fresh `TXDoubleWindow`, not a saved
desktop. `TDoubleWindow.Init` gives both views the selected drive (or
the current drive for automatic startup), builds right and left views, and
applies configured left-panel defaults (`DBLWND.PAS:136–155,326–334`).
The command needs a running application, usable drive, and enough memory.
It neither requires an existing manager nor reads the launcher flag.

Layout and data are separate: the drive dialog runs before allocation or
search. The panel calls `ReadDirectory` → `TListDrive.GetDirectory`
(`FLPANELX.PAS:174–192,474–497`; `FILELIST.PAS:106–217`). In the host
trace the picker uses DOS AH=19h/0Eh for drive discovery/selection; after
acceptance there are also AH=47h/3Bh for directory state, AH=1Ah for DTA,
AH=4Eh/4Fh for enumeration, and AH=36h for free space. BIOS calls in the
interaction include INT 16h AH=11h/10h for input and INT 10h for cursor
operations. Chronos has no disk allocation statistics for these drives:
AH=36h returns FFFFh and DN displays 0 free bytes. That is separate from
enumeration. FreeDOS on its FAT volume supplies real free space.

## Controlled runs

The probe mounts only regular sibling files on C:, with C:\ as the
current directory, D: and T: also available, and writes held in guest
memory. No saved desktop, CFG or SWP was supplied. The tail is zero length
then CR; PATH, TMP, TEMP and APPID agree; COMSPEC/SHELL name the launched
program, inherited by its child. [Before](inputs-before.log) and
[after](inputs-after.log) hashes match all [three pins](inputs.sha256).
No-input runs match DN-21 byte for byte in the decompressed traces and
pixel for pixel in the captures ([comparison](baseline-comparison.log)).

| Host checkpoint | CS:IP / FLAGS (hex) | Result |
| --- | --- | --- |
| DN.COM entry | 1041:0100 / 0002 | SS:SP=1041:FFFE, DS=ES=1041 |
| Direct DN.PRG entry | 1051:0000 / 0002 | SS:SP=4C7C:0100, DS=ES=1041 |
| EXEC child entry | 10CF:0000 / 0002 | SS:SP=4CFA:0100, DS=ES=10BF, parent PSP 1041 |
| [Launcher at 10M](launcher.png) | 29EF:0431 / F202 | Two populated panels, parent suspended; 5 FindFirst (2 successes, 3 no match), 40 FindNext (38 successes, 2 exhaustion) |
| [Direct at 10M](direct.png) | 11CB:0036 / F246 | Prompt and input loop, no panels, zero FindFirst/FindNext |
| [Direct at drive picker](menu-picker.png) | 3265:08D1 / F293 | Still zero FindFirst/FindNext |
| [Direct after selecting C:](direct-panels.png), 7M | 1443:18C5 / F246 | Both C:\ panels list real files, including DN.COM and FILES.BBS. Two successful FindFirst, then 36 successful FindNext and two exhaustion returns (CF=1, AX=0012h). First FindFirst at 6,137,445 retired. INT 2Fh unchanged. |
| [Direct exit after creating panels](direct-exit.log.gz) | 3C56:01F0 / F246 | Alt+X, Y; AH=4Ch code 0 at 7,629,147; no suspended parent |
| [Launcher exit](launcher-exit.log.gz) | 1041:0276 / 0096 | Child exits 0, parent resumes at 1041:01F0, FLAGS=0006h at 5,980,599; AH=4Dh, restores INT 2Fh, exits 0 at 5,980,616. A deliberately recorded extra queued Enter is not needed for quit. |

[F10](menu-top.png) opens the menu bar. M selects the Manager mnemonic.
[Down](menu-down.png) highlights Directory tree; [Up](menu-up.png) returns
to New. Enter opens the drive picker, and another Enter accepts the
highlighted C:. [Escape](direct-menu-escape.png) backs out of Manager
without creating panels. The [Ctrl+F3 path](direct-shortcut.png) also
creates both panels once C: is accepted. [Tab](direct-focus.png) moves
focus right to left; [Down after Tab](direct-focus-down.png) changes only
the left selection from DN.COM (1,778 bytes) to FILES.BBS (964 bytes).
The host screenshots are renders of the guest text buffer; full hardware
keyboard compatibility and native event delivery remain untested.

## Independent reference

FreeDOS 1.4 floppy media (kernel 2043, FreeCOM 0.86) in QEMU 11.1.3 with
SeaBIOS, 32 MiB, and a writable FAT C: volume containing a fresh copy of
the distribution. A small [ordinary EXEC wrapper](reference-exec.asm)
requests `DN.PRG` by its original name with an empty tail; it installs
no interrupts and forwards the child's exit code. The build and guest
setup are below. The authentic copied binaries [still match](reference-inputs-after.log).

Direct startup [shows no panels](reference-direct.png). The same menu
sequence reaches [the drive picker](reference-picker.png), then both
C: panels [show FAT entries](reference-panels.png). [Tab and Down](reference-focus.png)
move selection on the left panel, and the batch prints [exit code 0 at
the recovered C:\ prompt](reference-exit.png). The reference monitor log
contains key inputs, sampled CPU registers/EFLAGS and IVT values before and
after the workflow. These are asynchronous system snapshots, not DN entry
registers or INT 2Fh/FindFirst/FindNext return traces; those service-level
comparisons were not instrumented. Reference file counts and free space
are not expected to match the Chronos file map. This is a FreeDOS behavioral
comparison, not native SunlightOS Chronos in QEMU.

## Reproduction

From the repository root, reuse the DN-21 probe patch under a temporary
example name, build it, then remove that example. The case driver writes
audit artifacts; choose a fresh output directory to retain this run.

```sh
d=docs/audit/chronos-dn/dn-22
mkdir -p /tmp/dn22
patch -o chronos-core/examples/dn22_probe.rs chronos-core/examples/dos_probe.rs \
  < docs/audit/chronos-dn/dn-21/interactive-probe.patch
cargo build -p chronos-core --example dn22_probe --target x86_64-unknown-linux-gnu
unlink chronos-core/examples/dn22_probe.rs
DN22_OUT=/tmp/dn22 bash "$d/replay-case.sh" direct DN.PRG '' 10000000
DN22_OUT=/tmp/dn22 bash "$d/replay-case.sh" launcher DN.COM '' 10000000
DN22_OUT=/tmp/dn22 bash "$d/replay-case.sh" direct-panels DN.PRG \
  5000000:4400,5200000:326d,5400000:5000,5600000:4800,5800000:1c0d,6100000:1c0d
sha256sum -c "$d/inputs.sha256"
```

The probe exits 2 for an instruction budget stop; the captured state
distinguishes that from a trap. Each saved trace records its exact inputs.
For the FreeDOS side, see [reference setup](reference-setup.md).

## Validation

| Area | Verdict |
| --- | --- |
| Automatic launcher startup | **PASS** on host |
| Direct startup without saved state | **PASS** for the observed DN contract; no automatic panels |
| DN's own panel command | **PASS** on host for menu and shortcut |
| Actual directory enumeration | **PASS** on host for both paths |
| Keyboard interaction and clean exit | **PASS** for the tested keys and paths; no broad keyboard claim |
| Reference DOS comparison | **PASS** for visible behavior and exit; **PARTIAL** for service-level comparison |
| Focused multiplex, enhanced keyboard, DOS find | **PASS**: 5, 2, and 1 tests ([log](focused.log)) |
| Chronos core, Chronos UI, Sunlight UI | **PASS**: 197, 33, 290 passed; 1 core ignored. MIDTERM and Mines pass in the [unit log](units.log). |
| Core no_std and native release build | **PASS** ([no_std](no-std.log), [release](native.log)) |
| Phase 3.0 QEMU platform | **PASS** ([gate log](platform.log)); no DN workload |
| Sunlight UI doctest | **BLOCKED** by the known unchanged `horizon.rs` diagram parsed as Rust ([log](doctest.log)) |
| Native QEMU DOS Navigator | **NOT_RUN**: 65,536-byte import cap and no 386 profile selection |

The startup difference is expected DN behavior under the fresh-state
conditions tested. No generic DOS or BIOS defect was established by New
Manager. Next, enable complete imports for the authentic MZ/overlay and
select the required 386 profile in the native adapter, then repeat the
launcher and direct UI workflows with native events.
