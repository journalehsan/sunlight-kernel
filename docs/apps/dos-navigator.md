# DOS Navigator / Chronos compatibility audit

DN-10 update: see the final section for current evidence. VGA text attribute
blink/background-intensity switching (`INT 10h/AX=1003h`) now works; authentic
startup advances past it and stops at an unsupported byte read of I/O port
`0x61`. Earlier sections retain historical baselines.

Date: 2026-11-18. Overall status: **PARTIAL / BLOCKED**. The requested usable
two-panel file manager has **not** been achieved. No native DOS execution,
working panel, file operation in DN, or successful desktop launch is claimed.

## Version, license, and provenance

Selected diagnostic candidate: original RIT Labs DOS Navigator **1.51**,
dated April 19, 1999. Source inspected at
[maximmasiutin/Dos-Navigator](https://github.com/maximmasiutin/Dos-Navigator/tree/b268678fad8a9ccb3fd6a9463e1e341ac73985ec),
revision `b268678fad8a9ccb3fd6a9463e1e341ac73985ec`.

`README.TXT` permits source/binary redistribution with retained notices,
attribution and an advertising acknowledgement; it includes a warranty
disclaimer. This is the upstream RIT Labs license, not a license substitution.
Individual source headers, including `DN.ASM`, also prohibit changing the
license of derivatives. Preserve those headers if distributing modified source.
Based on Dos Navigator by RIT Research Labs.

The documented toolchain is Borland Pascal 7.0, TASM and TLINK. `BUILD.BAT`
invokes `BUILDDN.BAT asm ver help resource`; the latter invokes
`bpc dn /dDN /GP /Q /B`, builds the assembly COM launcher with TASM/TLINK,
builds resource/help tools, and optionally invokes PKLITE. `/GP` in that
script is a target-mode issue that must be resolved before claiming a
reproducible **real-mode** source build. No source build was attempted or
succeeded: BPC, TASM and TLINK are unavailable in this environment. Native
`fpc` is installed; `ppc8086` is not on PATH. An FPC rewrite was not substituted.
No compiler/linker components were downloaded or redistributed.

Initial probe uses the publisher's prebuilt
[dn151.zip](https://www.ritlabs.com/download/files3/dn/dn151.zip).
It identifies itself as 1.51; exact correspondence to the source revision is
**UNVERIFIED**, not a bit-for-bit source build. The archive's `DN.PRG` has a
UPX signature, whereas the source build script mentions PKLITE. UPX 5.2.0
cannot unpack this old format; the interpreter does execute its unpacker.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| dn151.zip | 786272 | `f6dee3904945a2f390f8576a5d18d83446f4554e43d6f0a3c75f8aa5b570d7d9` |
| DN.COM | 1778 | `ae0863550ec07f21c5cf8d208373a5022b868133e6f644757b1623cd8f7e9905` |
| DN.PRG | 109613 | `ac0024e269ecf0feb4a8db15f96181092604567a62cf00fb8453d9052c2862c3` |
| DN.OVR | 745951 | `401a8e631b8627fcbf8c209f01b1d71e3e314347f1c32417a0345d45455e3569` |

The launcher is a 16-bit COM; the main image is an MZ executable with a runtime
unpacker and external overlays. MZ alone does not establish real-mode-only
requirements throughout the program. Protected-mode/DPMI requirements remain
unverified beyond the reached code. Language/dialog/help and configuration
files are present in the original distribution and retained by the packager.

Alternative inspected: official FreeDOS
[DN2 2.14 beta, package revision 2025-05-27.3](https://ibiblio.org/pub/micro/pc-stuff/freedos/files/repositories/latest/html/en/apps/dn2/20250527.3/index.html).
Metadata calls its policy OpenSource, similar to BSD, and publishes SHA-1
`6b5bb5bf8b7c58ec237308449103f6a089f1aecf`, size 2,761,048 bytes.
This metadata is not sufficient license or real-mode verification. Downloads
from ibiblio and a mirror timed out; the GitLab archive clone returned HTTP
403. Its exact source revision, actual license text, binary format, build
dependencies and suitability remain **UNVERIFIED**. It was not substituted
for DN 1.51. FreeDOS's [DN help](https://help.fdos.org/en/hhstndrd/util/dn.htm)
describes the intended two-panel/viewer/copy workflows, not Chronos results.

## Repository audit and compatibility matrix

Evidence: `chronos-core/src/{runtime,dos,fs,loader,mz,arena,memory}.rs`,
`sunlight-chronos/src/main.rs`, `sunlight-libc/src/{lib,sun_exec}.rs`,
`kernel/src/ipc/vfs_registration/file.rs`,
`kernel/src/capability/vfs_grants.rs`, and
`services/vfs_server/src/main.rs`.

“Supported” means the existing subset is implemented/tested; it does not mean
DN has successfully exercised it. Guest-specific completeness is unverified.

| Required feature | Status | Evidence / limit |
| --- | --- | --- |
| Private real-mode memory | Supported | Bounds-checked 1 MiB, 20-bit wrapping, no native guest code execution |
| CPU execution | Partial | 8086/186/286 profiles; unsupported instructions trap; no protected mode/DPMI |
| COM loading | Supported | PSP, command tail, environment, arena allocation |
| MZ loading | Partial | Relocations, initial segments/stack, min allocation; extended formats rejected; overlay loading unverified |
| Conventional allocation | Supported | AH=48/49/4A, ownership and failure tests |
| Environment/termination | Partial | PSP and environment modeled; one suspended parent only |
| Shell → DN.COM → DN.PRG | Missing | Requires a second suspended parent; `Runtime::exec` explicitly rejects nesting |
| DOS drives/current directory | Partial | C/D/T and normalized drive-relative paths; root navigation needs review |
| Open/create/close/read/write/seek | Partial | Guest-memory files and DOS handles; native snapshot/persistence limitations below |
| Find first/next | Partial | DTA output; at most 512 search results; native import only reads 64 entries per directory |
| Attributes | Partial | Attribute bits modeled; write/create/delete semantics need additional read-only validation |
| File timestamps | Missing | No AH=57 dispatch / stored per-file timestamps |
| Rename/delete/mkdir/rmdir | Partial | Guest overlay/tombstones exist; persistence does not apply tombstones to host |
| Path/wildcard handling | Partial | Traversal/device-name rejection tests exist; 8.3 aliases and malformed paths need broader coverage |
| AH=52 list-of-lists | Partial (new) | Only terminating SFT capacity chain, not full DOS internal structures |
| INT 2F multiplex | Partial (new) | AX=9900 absent-resident check; installed guest handler runs and IRETs; unknown requests still trap |
| Software-FPU interrupt vectors | Partial (new) | Installed INT 34–3F handlers run as guest code; authoritative IVT includes direct guest writes; arithmetic completeness unverified |
| INT 10 text/cursor | Partial | 80×25, attributes, CP437, scroll up, cursor shape/position; BIOS hook chaining remains unimplemented |
| Direct B8000 | Supported | Authoritative character/attribute memory rendered by native window |
| INT 16 keyboard | Partial | AH=00/01/02, queue and modifier translation; DN function-key workflows unverified |
| DOS console | Partial | AH=01/02/06/07/08/09/0A/0B/0C and standard handles |
| INT 11 equipment | Supported subset (new) | BDA-backed virtual color display with absent coprocessor / physical disk controllers / serial / parallel devices |
| INT 1A clock | Missing | No BIOS clock dispatcher |
| DOS Ctrl-Break state | Partial (new) | AH=33 AL=00/01 guest-local get/set flag; actual Ctrl-Break interrupt delivery unimplemented |
| DOS country information | Missing | AH=38 is the current main-program blocker |
| DOS date/time | Supported subset | AH=2A/2C backed by host-supplied guest time |
| Mouse | Partial | INT 33 polling modeled; DN integration unverified |
| Coprocessor startup latch | Partial (new) | Only byte OUT F0h,0 accepted as an inert absent-coprocessor clear; no physical I/O |
| Scheduler cooperation | Supported subset | Bounded slices, INT 28 timer yield, blocked input; busy polling still bounded but not automatically asleep |
| SunApp schema/shared runtime | Supported | Existing format=1 / runtime.type=chronos / Program / permissions schema reused |
| Native CPU selection | Unverified | Manifest parser does not forward `[chronos].cpu`; host probe explicitly selects 286 |
| Closure/abnormal cleanup | Unverified for DN | Core child-trap recovery exists; native persistence happens on Exited, not verified on window closure/trap |

## Security and Ring 3 VFS implications

The host probe imports only explicitly selected sibling regular files, without
following directory symlinks, and never persists writes. Guest code cannot
request host opens. Neither the probe nor DOS changes add native syscalls,
unrestricted mounts, host port access or capability bypasses.

The production adapter imports approved Program/Dependencies, a private C:
overlay, and D: only when the launcher passes a Documents root. T: is in-memory.
`sunlight-libc` operations enter the existing syscall/VFS transport. The Ring 3
file transport carries authenticated identity; service-side descriptor checks
and existing capability admission remain required. A manifest field alone is
not evidence that every native request has the intended capability confinement.
End-to-end DN capability enforcement remains **NOT_RUN**.

The current adapter is a snapshot plus exit-time writeback, not a live DOS-to-VFS
bridge. `MAX_HOST_FILE=64*1024` silently truncates imported files. This truncates
both DN.PRG and DN.OVR; the host probe deliberately imports their full bytes,
so its progress cannot predict native success. `persist_drive` writes uppercase
guest paths, ignores write errors, and does not process tombstones. Host deletion,
rename, case preservation, read-only denial propagation, concurrent changes,
resource bounds and closure during I/O must be fixed and tested before granting
DN real Documents access. No such grant was added here.

## Actual execution and implemented changes

All following guest execution was **host interpreter execution**, not QEMU:

1. Baseline DN.COM prints the authentic copyright banner and traps after five
   instructions on INT 21/AH=52.
2. Added the minimal SFT capacity view, uninstalled AX=9900 multiplex probe and
   guest INT 2F handler dispatch with FLAGS/CS/IP frame and IRET. DOS vector
   setters now mirror guest IVT words. Other BIOS hook dispatch stays unchanged:
   an attempted general hook dispatch regressed the shell because native BIOS
   chaining stubs are absent; that approach was narrowed and tests pass again.
3. The next launcher blocker was opcode E0 (LOOPNE). Added LOOPNE/LOOPE with
   CX wrapping, conditional ZF use, unchanged flags and existing LOOP behavior.
4. The launcher then starts the original DN.PRG. Its unpacker reaches a byte
   OUT F0h,0. Added only that inert absent-coprocessor clear. Adjacent ports,
   reads, word operations and nonzero values still trap.
5. The next observed blockers were INT 11h (equipment), DOS AH=33 (Ctrl-Break
   state), and INT 37h (a software-FPU call). Added a BDA-backed virtual equipment
   word `0020h`, guest-local get/set of the Ctrl-Break flag, and dispatch of
   installed INT 34–3F guest handlers. The guest IVT is now authoritative for
   both DOS vector calls and direct guest memory writes; the duplicate vector
   array was removed. No host FPU or hardware interrupt is exposed.
6. DN.PRG now traps on **INT 21h/AH=38h** (country information), after
   3,401,904 retired instructions when launched directly. DN.COM starts DN.PRG
   and recovers the same child trap. The probe stops on the first observed
   recovered child trap, at 3,409,999 retired instructions in the final run.
   Its parent state is still Running, which is not acceptance success.
   Neither execution shows panels.

The equipment word describes the virtual machine; protected-mode features and
overlay behavior remain unverified. Next is the country-information interface,
then another actual guest probe. The current changes are an incremental
compatibility milestone, not completion of the requested integration.

## Packaging and reproduction

`tools/dos_navigator_prepare.py` verifies the archive SHA-256 before extracting,
rejects unsafe ZIP paths/symlinks, preserves the original distribution and
license, adds attribution and per-file hashes, and prepares an explicitly
**BLOCKED diagnostic** bundle under `target/dos-navigator/package/`.
It uses the existing manifest schema, references the shared Chronos runtime,
and sets `permissions.documents = "none"`. No desktop registration, installation
or publication is performed. This is not the finished application bundle:
native imports, profile forwarding, nested EXEC, shell return and usable guest
workflows remain unresolved. C: would use the existing private overlay policy;
T: remains the appropriate synthetic-test drive for future functional tests.

```sh
python3 tools/dos_navigator_prepare.py --archive target/dos-navigator/dn151.zip
cargo run -p chronos-core --example dos_probe --target x86_64-unknown-linux-gnu -- target/dos-navigator/dn151/DN.COM
cargo run -p chronos-core --example dos_probe --target x86_64-unknown-linux-gnu -- target/dos-navigator/dn151/DN.PRG
```

Omit `--archive` to download the pinned publisher archive. Preparation refuses
to overwrite an existing diagnostic bundle. Probe exit code 2 means blocked,
nonzero exit, waiting or execution-budget exhaustion, never acceptance success.

## Validation and outstanding acceptance

| Validation | Result |
| --- | --- |
| Baseline core tests | PASS: 105 passed, 1 pre-existing ignored |
| Final core tests | PASS: 113 passed, 1 pre-existing ignored; no new skips |
| Chronos UI host tests | PASS: 30 passed (includes Mines and shell regressions) |
| Core no_std target check | PASS: `cargo check -p chronos-core --target x86_64-unknown-none` |
| Original DN launcher/main execution | PARTIAL: authentic bytes executed; blockers above |
| Checksum-pinned diagnostic packaging | PASS: generated with original licenses, no Documents grant |
| Reproducible source compilation | BLOCKED: original toolchain unavailable, target mode unresolved |
| DN native QEMU execution | NOT_RUN: native import/runtime prerequisites unresolved |
| Two panels, navigation, tab, viewer, mkdir, copy, rename, confirmed delete, exit to shell | NOT_RUN: guest startup blocked |
| DN launch/exit soak, large directory, memory failures, permission denial, malformed paths/handles, window closure during I/O, concurrent desktop load, resource-growth checks | NOT_RUN for DN |

Core tests cover the new SFT view, multiplex default, guest interrupt stack/IRET,
direct IVT software-FPU dispatch, equipment/BDA consistency, guest-local
Ctrl-Break state, conditional loops and narrow coprocessor port handling.
Existing Mines, shell,
file-service, memory-owner and scheduler tests passed. This is regression
coverage for the additions, not proof of DN functionality or native capability
enforcement. Retained text evidence is in `docs/audit/chronos-dn/`; no screenshot
is supplied because no working panel/native-window milestone was reached.

No commits or pushes were made.


## DN-2: country information and next demonstrated blocker (2026-11-18)

Overall **PARTIAL / BLOCKED**. AH=38h is **PASS** in host execution; first
panels and keyboard responsiveness have not been reached. Native QEMU DN
launch, boot validation for this change, desktop cleanup and screenshot are
**NOT_RUN**. This milestone stops at the next independently reproduced CPU gap,
as permitted by the completion criteria. No native success is inferred from
host tests. The existing ISO was not rebuilt or used to claim current results.

### Identity and reproduction

The same publisher archive and main executable were independently rehashed:

- `target/dos-navigator/dn151.zip`: SHA-256
  `f6dee3904945a2f390f8576a5d18d83446f4554e43d6f0a3c75f8aa5b570d7d9`.
- `target/dos-navigator/dn151/DN.PRG`: 109613 bytes, SHA-256
  `ac0024e269ecf0feb4a8db15f96181092604567a62cf00fb8453d9052c2862c3`.

The unmodified DN.COM launcher also reaches the same new opcode gap in its
child; its hash and provenance remain as recorded above. No binary patch,
signature-based behavior or instruction skip was used.

Reproduce the diagnostic with:

```sh
CHRONOS_TRACE=1 cargo run -p chronos-core --example dos_probe \
  --target x86_64-unknown-linux-gnu -- target/dos-navigator/dn151/DN.PRG
# Substitute DN.COM to exercise the original launcher as well.
```

Exit status 2 denotes an incomplete/trapped diagnostic, including a recovered
child trap. Trace mode steps at most 10 million guest instructions and records
software-interrupt registers before/after, progress, and country buffer bytes.
It imports sibling regular files only, into private guest memory; no Documents,
network, native persistence, global installation or new permissions are used.
Raw evidence is under `docs/audit/chronos-dn/dn-2/`.

### AH=38h diagnosis

`baseline.log` independently reproduces the missing dispatch branch. At retired
count 3,401,904, before INT 21h at **35A6:3239**:

```text
AX=3800 BX=0000 CX=0000 DX=AC1C SI=2644 DI=2314
BP=4FFA SP=4EEE CS=35A6 DS=4116 ES=1000 SS=4C12
FLAGS=F046 (CF=0)
```

The destination is **4116:AC1C**, physical **4BD7C**, fixed length **34**
bytes (no length register in this API). Baseline dispatch immediately returns
`UnsupportedInterrupt { interrupt: 33, function: 56 }`; IP advances to 323B,
registers and FLAGS remain unchanged, and no DOS error code is returned.
The runtime traps before any buffer write or subsequent guest call. This is an
unimplemented service, not a demonstrated retry loop or invalid memory access.

`country.log` shows the corrected return at the same location/count: state
Running, BX=0001, CF=0, AX and all other registers preserved, IP=323B.
The trace includes the complete 34-byte result. Subsequent INT 21h calls are
AX=3000, 3001 and 3000 at retired counts 3,401,909, 3,401,918 and 3,401,947.
Each returns DOS version AX=0005. No repeated country-information pattern was
observed before the next trap.

### Generic implementation

The DOS layer now exposes country 1 (US), using the
[documented DOS 3+ AH=38h interface and layout](https://fd.lod.bz/rbil/interrup/dos_kernel/2138.html).
Current-country and explicit selectors are supported, including AL=FF/BX;
unavailable countries return AX=2 with CF set. DX=FFFF selects country setting
without writing a buffer; only the installed country is accepted.
The result contains US date/currency/separator/time settings and reserved zeros.
It writes exactly 34 bytes using the existing bounded, 20-bit-wrapping guest
memory API. No native pointers or filesystem calls are exposed.

A FAR-callable CP437 uppercase routine in DOS-reserved low memory at 0800:0200
maps representable extended lowercase characters, preserving flags and all
registers except AL. Other bytes remain unchanged. This is separate from the
existing SFT region and below the conventional allocation arena. The routine
executes ordinary guest instructions, without privileged host callbacks.

### Next blocker and validation

At retired count **3,403,088**, DN.PRG traps at **28D7:0135**, bytes
**C1 E7 02 31**: the first three encode **SHL DI,2**. Trap CPU state:

```text
AX=0099 BX=AA40 CX=0000 DX=28D7 SI=AA40 DI=0001
BP=4FFA SP=4FFA CS=28D7 DS=4116 ES=4116 SS=4C12
FLAGS=F093; fetched IP=0136
```

The interpreter dispatch implements D0-D3 shifts, but no C0/C1 immediate-count
shift dispatch, even with the diagnostic's explicit I80286 profile. A synthetic
`C1 E7 02` test independently reproduces the trap with DI unchanged. This test
documents the gap rather than claiming shift support. The original launcher
hits the same child instruction at **2913:0135**, retired **3,403,982**;
`launcher.log` records the recovered child trap. Its Running parent is not a
successful application result. Screens remain empty; no usable panels appeared.

- **PASS**: core host suite, 117 passed, 0 failed, 1 ignored. Coverage includes
  existing Pascal shell, Mines, MZ relocation, isolated drives, guest memory,
  keyboard, interrupts, waits/yields and child recovery tests.
- **PASS**: native UI/shared runtime host suite, 30 passed, 0 failed. This is
  host adapter testing, not a native DN launch or end-to-end VFS security proof.
- **PASS**: `cargo check -p chronos-core --target x86_64-unknown-none`.
- **PASS**: new country layout/register/boundary, selector/error/set-operation,
  exhaustive FAR callback tests, and explicit immediate-shift gap reproducer.
- **NOT_RUN**: native DN launch, keyboard workflows, close/exit desktop
  stability, first-panel screenshot, and QEMU boot gate. No unrelated QEMU or
  runner failure was demonstrated.

Recommended next milestone: implement and test generic 186+ C0/C1 shifts with
profile gating, operand widths, count and flag semantics, then rerun the exact
verified image. Next, resolve the already documented native 64 KiB snapshot
limit before treating native DN startup as equivalent to the full-image host
probe, and perform an isolated temporary QEMU launch. Native profile forwarding
also remains unresolved. Nested EXEC and native truncation were not changed;
no evidence in this run requires them to reach the next instruction.

Existing workspace edits were preserved. No commit or push was made.

## DN-3: immediate shifts and authentic startup progress (2026-11-18)

| Deliverable | Status |
| --- | --- |
| CPU immediate shift/rotate support | **PASS**: C0/C1, all seven documented operations, byte/word register and memory operands |
| Authentic DN startup | **PARTIAL**: passes the prior CPU blocker; stops at BIOS INT 10h/AH=12h |
| DOS Navigator panels | **BLOCKED**: no panels or keyboard workflow reached |
| SunApp packaging | **PARTIAL**: existing isolated diagnostic bundle retained, Documents=none; no install or publish |
| Native QEMU validation | **NOT_RUN**: host startup has not reached a stable interface; native import/profile prerequisites still unresolved |
| Regression suite | **PASS**: 122 core tests, 30 UI tests, core no_std and native adapter checks |

### Reproduction and CPU root cause

The archive and DN.PRG were rehashed before execution and match the DN-2
SHA-256 values above. `docs/audit/chronos-dn/dn-3/baseline.log` reproduces
exactly the unsupported instruction at retired count 3,403,088:

```text
28D7:0135  C1 E7 02   SHL DI,2
AX=0099 BX=AA40 CX=0000 DX=28D7 SI=AA40 DI=0001
BP=4FFA SP=4FFA CS=28D7 DS=4116 ES=4116 SS=4C12 FLAGS=F093
```

This was an **opcode dispatch gap**, before ModR/M or immediate decoding:
only the opcode had been fetched, leaving IP=0136 and DI unchanged. The
complete encoding is three bytes. E7 is mod=11 (register direct), extension
/4 (SHL/SAL), r/m=111 (DI), word width from C1, followed by imm8=02.
Expected result: DI=0004, IP=0138, CF=0, ZF=0, SF=0, PF=0; OF and AF
are undefined for this count. No displacement or memory operand is involved.

C0/C1 now enter the existing D0-D3 group handler. It uses the same ModR/M,
displacement, segment override, operand access, and read-modify-write paths;
the count byte is fetched after any displacement. C0 is byte, C1 is word.
8086 rejects immediate-count encodings through the existing profile trap.
Reserved group extension /6 now traps for all six opcodes instead of silently
executing SHR (including when count is zero). /4 covers both SHL and SAL.

### CPU generation, counts and flags

Chronos retains its real-mode 8086, 80186 and 80286 profiles, without claiming
a complete implementation of any CPU. DN diagnostics explicitly select 80286.
8086 CL counts use all eight bits; 186/286 counts use the low five bits.
Immediate-count operations require 186 or later. Through-carry rotations on
186/286 additionally reduce counts modulo 9 or 17. Zero effective counts
preserve the operand and every flag while still consuming the full encoding.
Full-width ROL/ROR retain the final rotated-out carry; rotations preserve
ZF/SF/PF/AF and control flags. The loops are bounded by 255 iterations on
8086 and 31 on later profiles, within the existing bounded instruction slices.

The flag formulas follow the Intel instruction references for
[shifts](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2b-manual.pdf)
and rotates. CF records the final shifted-out bit where defined. Shifts update
ZF/SF/PF, with parity from the low result byte. One-bit SHL/ROL/RCL use
result MSB XOR CF; SHR uses the original MSB; SAR clears OF; ROR/RCR use
the two highest result bits. This also fixes existing shared SHR/ROR/RCR
one-bit overflow calculations for D0-D3.

Undefined flags are deterministic implementation details: AF is preserved,
multi-bit OF is preserved except when through-carry reduction leaves one
iteration (where the one-bit formula is used), and out-of-range SHL/SHR CF
comes from the bounded bit loop. Tests do not assert undefined CF/OF/AF values
as architectural requirements. No guest-specific flag or instruction behavior
was added.

### Authentic execution after the fix

`main.log` records the first corrected execution at the same retired count:

```text
before: 28D7:0135 DI=0001 FLAGS=F093
 after: 28D7:0138 DI=0004 FLAGS=F012 state=Running
```

All other registers are preserved (full before/after state is in the log).
The same shift continues successfully through the initialization loop. The
next blocker is an unsupported **INT 10h/AH=12h** at retired count
**3,403,475**, 387 instructions beyond the previous trap. No new unsupported
CPU opcode, guest memory error, or stall is observed before it:

```text
11CA:2301  CD 10
AX=121A BX=AA10 CX=0000 DX=26C5 SI=AA40 DI=0004
BP=4FFA SP=4FFA CS=11CA DS=4116 ES=4116 SS=4C12 FLAGS=F046
UnsupportedInterrupt { interrupt: 16, function: 18 }; fetched IP=2303
```

AH=12h with BL=10h is the next video compatibility interface to investigate;
this milestone does not fabricate a video result or broaden BIOS behavior.
There are no new DOS calls between the previous shift blocker and this BIOS
call. `launcher.log` independently reaches the same child BIOS request at
1206:2301, retired count **3,404,369**. The launcher recovers the child trap;
its Running parent and copyright banner do not establish successful startup.
The direct main screen remains empty. Both probes exit 2 as intended.

The trace runner now records immediate shift states in addition to interrupts.
The ten-million-instruction cap, private sibling-file import, no persistence,
no Documents access, and no guest-triggered host filesystem access remain.
Reproduction uses the DN-2 commands above; logs here use the same inputs.
Nested EXEC, native file truncation, and capability policy were not changed.

### Validation and remaining limits

Raw logs are in `docs/audit/chronos-dn/dn-3/`:

- `core.log`: **122 passed**, zero failed, one pre-existing ignored. Six focused
  tests replace the old gap reproducer and cover all operations, register and
  displaced memory operands, byte/word widths, immediate/one/CL counts,
  zero/masked/width/beyond-width counts, overflow/truncation, defined flags,
  unrelated state, instruction lengths, ES override and 20-bit memory wrap,
  reserved /6, 8086 gating and D0-D3 regressions. Existing DOS/BIOS, Pascal
  shell, Mines, MZ, drive/filesystem, wait/yield and bounded soak tests pass.
- `ui.log`: **30 passed**, zero failed, including shell and Mines regressions.
- `no-std.log`: core `x86_64-unknown-none` check **PASS**.
- `native-check.log`: sunlight-chronos `x86_64-unknown-none` check **PASS**;
  dependency warnings remain.
- `native-build.log`: sunlight-chronos release ELF build **PASS**, using
  `tools/build.sh` userspace linker and x86-64-v2 flags, without invoking its
  workspace-wide version update or ISO creation. Dependency warnings remain.

Native QEMU boot, display, keyboard, desktop stability, clean guest exit and
resource reclamation are **NOT_RUN** for DN-3. No native screenshot or host
rendering substitute is supplied. Existing native 64 KiB import and profile
forwarding limitations remain, alongside the newly precise BIOS blocker.
Existing workspace changes are preserved; no commit or push was made.

## DN-4: EGA configuration query and next heap blocker (2026-11-18)

| Deliverable | Status |
| --- | --- |
| Video BIOS compatibility | **PARTIAL**: AH=12h/BL=10h passes; deliberately limited adapter subset |
| DN startup | **PARTIAL**: previous trap resolved; authentic main exits on its low-memory check |
| Two-panel interface | **BLOCKED**: no panels reached |
| Keyboard interaction | **NOT_RUN**: no interactive DN interface reached |
| Native QEMU | **NOT_RUN**: host startup exits; native import/profile prerequisites remain |
| SunApp packaging | **PARTIAL**: existing diagnostic bundle retained, Documents=none, no installation |
| Regression validation | **PASS**: 128 core tests, 30 UI tests, no_std check and native release build |

### Confirmed diagnosis and contract

`docs/audit/chronos-dn/dn-4/baseline.log` independently reproduces the same
checksum-verified main image at retired count **3,403,475**, before dispatch:

```text
11CA:2301 CD 10
AX=121A BX=AA10 CX=0000 DX=26C5 SI=AA40 DI=0004
BP=4FFA SP=4FFA CS=11CA DS=4116 ES=4116 SS=4C12 FLAGS=F046
```

The existing dispatcher rejects AH=12h as unsupported. Only IP advances to
2303; all other registers and FLAGS remain unchanged. The runtime traps before
executing another instruction. BDA and B800 bytes are logged before and after:
mode 03h, 80 columns, page zero, cursor (0,0), shape 0607h, blank text memory.
Baseline BDA rows/font/CRTC fields were uninitialized and page stride was 4000.
There is no evidence of a retry loop, a register-return bug in an existing
implementation, or an unrelated CPU trap at this point.

The implementation follows IBM's
[PS/2 and PC BIOS Interface Technical Reference, May 1988, INT 10h Alternate Select, printed page 2-33](https://www.ardent-tool.com/docs/pdf/15F0306_PS2_and_PC_BIOS_Interface_Technical_Reference_May88.pdf).
AH selects 12h and BL selects 10h; **AL is not a selector** for this service.
BH returns color (00h) or monochrome (01h); BL encodes installed video memory
as 64/128/192/256 KiB for values 0/1/2/3. CH holds adapter feature bits and CL
holds the simulated EGA switches. All other registers are preserved here,
including AX and FLAGS. No memory writes occur during the query. The unrelated
AX=1A00h display-combination service is not used to interpret AL=1Ah.
The [SeaBIOS implementation](https://qemu.googlesource.com/seabios-hppa/+/refs/tags/rel-0.4.2/vgasrc/vga.c)
also separates BL dispatch from AL and uses switch value 09h for enhanced
350/400-line color configurations.

### Conservative virtual adapter and audited BIOS services

Chronos exposes an **EGA-compatible configuration-query subset**, with a
64 KiB graphics aperture, color display, no feature connector and enhanced
color switches: **BX=0000, CX=0009**. The existing Mode 13h implementation
remains available alongside text mode 03h. This is a limited virtual adapter,
not a complete physical EGA or VGA: it does not claim 256 KiB planar memory,
extra display pages, arbitrary graphics modes, font loading, or full VGA
identification. AX=1A00h remains an explicit unsupported-function trap;
there is no misleading full-VGA display combination code. The fixed text
profile remains 80×25 with a 16-scanline font geometry. No font service or
alternate row count was added. Full VGA BIOS compatibility remains PARTIAL.

The configuration constants are shared by the query and BDA initialization;
there is no new mutable adapter state or alternate screen buffer.

| Service | Audited result / change |
| --- | --- |
| AH=00h | Existing modes 03h/13h retained; others explicitly trap |
| AH=01h | Cursor shape and BDA stay synchronized |
| AH=02h | Existing cursor position handling, clamped to 80×25; nonzero pages trap |
| AH=03h | Corrected to preserve BH; nonzero pages explicitly trap instead of silently querying page zero |
| AH=05h | Added page-zero selection; nonzero pages explicitly trap |
| AH=06h | Existing validated scroll-up window behavior retained |
| AH=07h | Added scroll-down using backwards copy; AL=0 clears; rectangle errors trap |
| AH=08h | Reads authoritative B800 cell; nonzero pages now explicitly trap |
| AH=09h | Corrected sequential character/attribute writes to preserve cursor and avoid scrolling; writes clip at the supported page boundary |
| AH=0Eh | Existing teletype behavior retained |
| AH=0Fh | Existing mode/columns/page-zero results retained |
| AH=12h, BL=10h | Added generic configuration query, independent of AL or input BH |
| AX=1A00h | Audited; remains unsupported, tested explicitly |

Text BDA now reports mode 03h, 80 columns, **1000h page stride**, zero page
start/active page, color CRTC 03D4h, 24 rows minus one, 16 font points and
switches 09h. Mode 13h retains its 40-column/FA00h visible-page convention,
with 25 character rows and 8 font points. Mode changes reset all eight BDA
cursor slots; only page zero is supported through BIOS/rendering. Cursor
shape and page-zero position use the existing runtime/BDA synchronization.

BIOS output and direct B800 writes use the existing authoritative memory and
renderer. Existing dirty-row/generation handling is unchanged; no instruction
polling, redraw loop, or per-instruction full-screen scan was introduced.
Regression tests check direct text writes against BIOS reads and cells, cursor
shape/position, page rejection, mode/BDA agreement, bounded writes and scrolling.
The synthetic query test compares all 1 MiB before/after dispatch, including
repeated requests with differing AL/BH inputs and full CPU state preservation.

### Authentic guest progress and exact next blocker

`main.log` records the successful response at the same CS:IP/count:

```text
after INT 10: Running, IP=2303, AX=121A BX=0000 CX=0009
DX=26C5 SI=AA40 DI=0004 BP=4FFA SP=4FFA
CS=11CA DS=4116 ES=4116 SS=4C12 FLAGS=F046
```

The BDA/text bytes remain unchanged across the query. DN subsequently reads
BDA font height at 0040:0085, receiving 16. There are no further INT 10h
calls or mode transitions before exit, no unsupported opcode and no stall.
The next BIOS/DOS call is INT 21h/AX=3533h (query mouse vector), then DOS
console writes and cleanup/vector restoration, ending in AH=4Ch/AL=CBh.
No panel text, borders, directory entries, cursor BIOS operations or interactive
keyboard requests appear. Visible text comes from the authentic guest:

```text
Not enough memory for Navigator.
Please check if 400K memory is available
```

Main finishes **Exited { code: 203 }**, retired **3,404,686**, at
**3C55:01F0**. The launcher independently reaches the same low-memory exit
and returns code 203 after **3,405,597** instructions (`launcher.log`).
This is an orderly guest error exit, not startup completion.

The bounded instruction trace pinpoints the next issue: at retired
**3,403,530**, the Pascal heap routine returns to **1050:1536** with
**DX:AX=0000:0020** (32 bytes available). At 1536 the bytes are
`83 FA 01 7C 07 7F 4D 3D A0 86 73 ...`; the signed high-word comparison
branches to the low-memory message at 1542. The preceding routine subtracts
heap pointer segment **5CA7h** from heap end **5CA9h**, finding only two
paragraphs. Initial PSP **1040h** reports end **5CA9h** at PSP:0002.
The published `DN.PAS` lines 916–919 have the matching `memAvail < 100000`
check and `Halt(203)` (source/binary identity remains unverified as above).

`load_mz_in_arena` allocates only PSP + image + **e_minalloc**, ignoring the
parsed **e_maxalloc** when choosing process size. This is the leading loader
compatibility cause to address next; the exact small heap and branch are
confirmed by guest execution. No AH=48h allocation request occurs between the
video query and this check, so this is not a demonstrated DOS allocation-call
failure. See `mz-header.log` for the original header values. The loader and
allocation policy were not changed in this video milestone. Recommended next
milestone: implement correct bounded MZ min/max allocation semantics, with
parent/child allocation and shell-return regressions, then rerun both images.

### Validation, evidence and native limits

Reproduction (diagnostic exit status 2 is intentional for the guest error):

```sh
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3403480 \
  cargo run -p chronos-core --example dos_probe \
  --target x86_64-unknown-linux-gnu -- target/dos-navigator/dn151/DN.PRG
# Substitute DN.COM for the original launcher; omit CHRONOS_DETAIL_START
# to suppress the bounded 1000-instruction detailed window.
```

All evidence is under `docs/audit/chronos-dn/dn-4/`. Checksums match the pinned
archive and executable recorded above. The probe adds before/after video BDA,
text bytes and initial PSP extent to its existing complete CPU traces.

- `core.log`: **128 passed, 0 failed, 1 pre-existing ignored**, including six
  new video tests plus CPU, DOS/BIOS, shell, Mines keyboard/display, isolation,
  filesystem and scheduler regressions.
- `ui.log`: **30 passed, 0 failed**, including DOS Shell and Mines integration.
- `no-std.log`: core `x86_64-unknown-none` compilation **PASS**.
- `native-build.log`: sunlight-chronos release build **PASS**, with existing
  dependency warnings and the established userspace linker/x86-64-v2 flags.
- `native-prerequisites.log`: QEMU binary is available; native importing still
  caps files at **65536 bytes**, below both DN.PRG and DN.OVR sizes. CPU profile
  forwarding remains unresolved. These are adapter prerequisites, not a
  demonstrated QEMU infrastructure fault. No QEMU changes were made.

Native DN boot/launch/render/keyboard/shutdown/capability validation and QEMU
boot gates remain **NOT_RUN**: host startup exits before an interface and the
native adapter cannot import the verified full image. No native screenshot is
claimed or substituted with a host rendering. The diagnostic SunApp remains
isolated and uninstalled, with no Documents grant, no new capabilities and no
host writeback. Existing workspace edits were preserved; no commit or push.

## DN-6: guest multiplex ownership and authentic loader execution (2026-11-18)

Overall **PARTIAL / BLOCKED**: the AX=9905h root cause is identified, the
loader's authentic private handler executes and returns, and interrupt
regressions **PASS**. First panels remain blocked by an operand-size prefix.
Earlier milestone statements above are historical baselines.

### Reproduction and root causes

Evidence is in `docs/audit/chronos-dn/dn-6/`. `identity-native.txt` records
unchanged SHA-256 values and sizes for DN.COM, DN.PRG and DN.OVR, matching the
provenance table. Both programs import only sibling regular files into private
guest memory. The diagnostic bundle, capabilities and guest files were unchanged.

Two distinct baseline failures were reproduced:

- `main-baseline.log`: direct DN.PRG starts with IVT 2Fh at physical
  **000BC/000BE = 0000:0000** and never installs a multiplex handler before
  AX=9905h. At retired **3,403,557**, instruction **1051:15D7** is
  `CD 2F 89 16 8E 6D 89 0E`. Registers are AX=9905 BX=5C00 CX=0000 DX=0000
  SI=0000 DI=5CA8 BP=4FFE SP=4DFC CS=1051 DS=4117 ES=0040 SS=4C13
  FLAGS=F046 (IF=0, TF=0). The trace includes twelve stack words and PSP=1041.
  The existing dispatcher advances IP to 15D9 then reports
  `UnsupportedInterrupt { interrupt: 47, function: 153 }`. There is no installed
  handler to dispatch. This is **not evidence of a broken INT frame or IRET**.
- `launcher-baseline.log`: authentic DN.COM does install **1041:0142** through
  DOS AH=25h, and the main program's private calls already execute there.
  At retired **3,406,020**, the unrelated AX=D44Dh query at **3625:2856**
  enters this handler. Its non-private branch far-jumps to the saved predecessor
  **0000:0000**, then traps on bytes `D4 3C 00 00` at **0000:0002**. The loader
  needs a valid end of the DOS multiplex chain. This is the confirmed generic
  runtime defect blocking the proper launcher path.

The original dispatcher restricted guest execution to vectors 2Fh and 34h–3Fh.
It also contained a host AX=9900h special case. That special case did not supply
an actual callable predecessor for a guest handler to chain to.

### Generic changes and dispatch boundaries

Runtime initialization now installs an ordinary guest **IRET** at
**0800:0400**, in DOS-reserved low memory separate from the SFT and country
case-mapping routine, and initializes IVT 2Fh to it. It preserves registers and
saved FLAGS for arbitrary unclaimed multiplex requests. Guest tail jumps and
PUSHF/CALL FAR chaining both work with this predecessor. No private DN protocol
is implemented in the host; the old AX=9900h branch was removed.

Nonzero IVT entries for interrupts without built-in host services now execute
through normal guest control flow, including vectors beyond the former
2Fh/34h–3Fh whitelist. INT saves return IP after its immediate byte, CS and
FLAGS (reserved bit 1 set), and clears IF/TF. Existing guest IRET restores
IP/CS/FLAGS. PUSHF/POPF, far calls/returns, nested INTs and direct IVT writes
continue using the same private one-megabyte memory and 20-bit wrapping model.
There is no native code execution, port access or added privilege.

Dispatch distinctions remain explicit:

- Host services: 10h, 11h, 16h, 20h, 21h, 28h and 33h retain existing host
  dispatch. Guest hooks on these vectors are still **unsupported**, because
  chainable host BIOS/DOS gateways are not modeled. This limitation is **PARTIAL**,
  not a claim of universal vector fidelity.
- Guest handlers: other nonzero IVT entries execute guest instructions and IRET.
  The initial multiplex predecessor is one such handler, with no function decoder.
- Unhandled vectors: zero entries without host services explicitly trap. A
  direct guest write of zero to 2Fh also produces this trap.

Real mode has no invalid segment:offset bit pattern: addresses wrap within guest
memory. A pointer to unsupported code traps through the CPU engine; no new
allocation-based execute permission or native pointer interpretation was added.

### Vector history, process ownership and private protocol

Final `launcher.log` records these observations:

| Event | Retired count | Vector / result |
| --- | ---: | --- |
| Before execution, PSP=1041 | 0 | 0800:0400 |
| Loader queries predecessor (AH=35h), then installs itself (AH=25h) | 32 after installation | 1041:0142; saved predecessor 0800:0400 |
| Child installation query AX=9900h | 3,404,439 | Guest handler returns AX=0101 BX=444E CX=0000 DX=2151 |
| Child command pointer AX=9901h | 3,404,463 | Guest handler returns ES:BX=1041:0295 |
| Child getter AX=9905h | 3,404,494 | PSP=10BF; vector remains 1041:0142 |
| Getter IRET completed | 3,404,508 | AX=0000 CX:DX=0000:0000, return 10CF:15D9, SP=4DFC, FLAGS=F046 |
| AX=D44Dh query | 3,406,021 | Chains through loader to valid IRET predecessor; execution continues |
| Child trap recovery | 3,408,442 | Parent vector 1041:0142 survives; child FPU vectors restored to zero |

Before the getter, child registers are AX=9905 BX=0200 CX=0000 DX=0000 SI=0000
DI=5D26 BP=4FFE SP=4DFC CS=10CF DS=4195 ES=1041 SS=4C91 FLAGS=F046.
The stack begins `4FFE 4E44 502E 4752` (the preceding PUSH BP is included).
INT builds its frame at SS:4DF6. The detailed instruction window shows the
actual loader reading **CS:01C9/01CB**, clearing both words, and executing
IRET at **1041:01AC**. The saved return address and flags are restored.

The inspected upstream `DN.ASM` handler (`Int2Fentry`, approximately lines
80–134) and `DN.PAS` startup (approximately lines 925–955) explain this sequence.
The source/binary build correspondence remains unverified as recorded above;
the executed machine instructions independently establish the getter behavior.
The [RBIL getter entry](https://fd.lod.bz/rbil/interrup/shells/2f9905.html)
and [installation entry](https://fd.lod.bz/rbil/interrup/shells/2f9900.html)
support the protocol identification; the source and execution provide the
stronger evidence for this particular binary.

| Function | Upstream handler / caller finding |
| --- | --- |
| 9900h | Loader execution count, first-run state, child exit code, signature and version |
| 9901h | Loader-owned command-line pointer |
| 9902h | Records child-requested action/errorlevel |
| 9903h | Records command execution method |
| 9904h | Stores CX and DX in loader-owned DNTimer |
| 9905h | Reads DNTimer into CX and DX, clears it, sets AX=0 |
| 9906h | Reads DN_Misc; optionally replaces it when DX bit 15 is set |

`RUNCMD.INC` sends 9904h before requesting a loader-controlled restart;
`DNUTIL.PAS` uses 9906h for miscellaneous UI state. No 9904h precedes the
first getter in either captured run. Zero is the observed initial value,
not evidence of lost setter state. Nonzero timer persistence across a full
DN command/restart cycle is **NOT_RUN**. DN.COM, already in the diagnostic
bundle, supplies the needed parent handler; no separate TSR was fabricated.
Nested EXEC is not required by the observed DN.COM → DN.PRG startup and was
not expanded. Shell → DN.COM → DN.PRG still exceeds the one-parent limit.

### Lifecycle audit and cleanup

Each process now snapshots entry vectors. Before normal exit or recovered
child-trap memory reclamation, vectors pointing into that process's owned
allocations are restored to their entry values. Vectors backed by a suspended
parent's memory survive. AH=49h free and AH=4Ah shrink also remove vectors
whose backing range is released; a snapshot pointing into that same released
range is replaced with an unhandled vector (or the default multiplex IRET).
Direct IVT writes receive the same cleanup as AH=25h installations. The trusted
arena metadata, not guest-writable MCB ownership, determines backing ranges.

Tests cover repeated children, foreground termination, explicit release and
shrinking, while the real launcher trace demonstrates child-trap cleanup with
the parent handler preserved. This is cleanup of released allocations, not
full DOS TSR support. AH=31h and INT 27h remain unsupported and trap; no
successful resident termination is acknowledged and then freed. TSR lifecycle,
resident allocation transfer and asynchronous hardware interrupt delivery are
**NOT_RUN / unsupported**. No resident implementation was needed by this trace.

### Next reproducible blocker and validation

Authentic startup now reaches **3CD4:3FBE**, bytes **66 C1 E0 10**
(`SHL EAX,16` with a 32-bit operand-size prefix), at retired **3,408,442**
on the launcher path. Trap registers: AX=005F BX=0000 CX=0400 DX=0000 SI=0081
DI=44B2 BP=48DA SP=46BC DS=4195 ES=4C91 SS=4C91 FLAGS=F046.
The CPU trap reports fetched IP=3FBF. This is a new CPU compatibility boundary:
the diagnostic explicitly selects I80286 and the engine lacks 386 operand-size
support. Adding a 32-bit register/operand model is deferred to a focused CPU
milestone rather than skipping or patching the instruction.

`main.log` also progresses through the generic absent-multiplex IRET, but
it **does not execute the DN loader's private protocol**. It traps on the same
bytes at **3C56:3FBE**, retired **3,407,159**. Direct execution is diagnostic;
DN.COM remains the correct path for demonstrating private handler execution.
Neither run reaches panels, input waiting, a clean application exit or DOS
error text. The launcher probe stops on the recovered child trap with a Running
parent; that is not application success. Video logs retain BDA and B800 bytes;
the final launcher screen contains only the authentic copyright banner.

| Validation | Result |
| --- | --- |
| Core CPU, DOS/BIOS, allocation, Pascal Shell and Mines regressions | **PASS**: 141 passed, 0 failed, 1 pre-existing ignored |
| New machine-code interrupt cases | **PASS**: DOS install/query/restore, exact frame, IF/TF and IRET, nested INT, PUSHF/far-call and tail-jump chains, direct IVT writes, wrapped pointer/stack, unsupported code/unhandled pointer, repeated launches and ownership cleanup |
| Shared native UI host suite | **PASS**: 30 passed, including shell and Mines |
| Core no_std target check | **PASS** |
| sunlight-chronos native release ELF build | **PASS** with existing dependency warnings and userspace linker flags |
| Authentic DN private handler and continued startup | **PARTIAL**: getter executes; next opcode blocks panels |
| DN panels, keyboard navigation, file workflows, clean termination | **NOT_RUN**: startup trapped |
| Native QEMU DN launch / boot gate | **NOT_RUN**: native adapter still truncates files at 65536 bytes and does not forward the explicit diagnostic CPU profile |

QEMU exists at `/usr/bin/qemu-system-x86_64`; its presence does not resolve the
adapter prerequisites. No new QEMU infrastructure fault is demonstrated.
No ISO rebuild, global installation, Documents grant or host persistence was
performed. Native release compilation is not native execution validation.

Reproduction (diagnostic status 2 means blocked, including recovered child traps):

```sh
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3404494 cargo run -p chronos-core \
  --example dos_probe --target x86_64-unknown-linux-gnu \
  -- target/dos-navigator/dn151/DN.COM
cargo test -p chronos-core --target x86_64-unknown-linux-gnu
cargo test -p sunlight-chronos --target x86_64-unknown-linux-gnu
cargo check -p chronos-core --target x86_64-unknown-none
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-chronos --release --target x86_64-unknown-none
```

Existing workspace edits were preserved. No commit or push was made.

## DN-7: partial 80386 real-mode foundation (2026-11-18)

The minimum DN-7 chain **PASS**: 32-bit register model → operand override →
`SHL EAX,16` → correct authentic execution → legacy regressions. Overall
compatibility remains **PARTIAL / BLOCKED**; no usable panels are claimed.
Evidence: `docs/audit/chronos-dn/dn-7/`.

### Architecture and instruction audit

The old engine stored eight `u16` GPRs, decoded only 16-bit effective addresses,
used byte/word memory helpers and a 16-bit stack/IP/FLAGS, and did not recognize
66h/67h. Many DOS/BIOS paths assign AX and other low words directly.

`CpuState` now retains those public low words plus eight upper words in x86
encoding order (EAX, ECX, EDX, EBX, ESP, EBP, ESI, EDI). `reg32`/`set_reg32`
compose/update the complete register. This is one split representation, not a
second register cache. Existing word assignments, byte setters and DOS/BIOS
results automatically preserve upper halves. Full CPU snapshots, including
suspended-parent state, include the upper words. Segment registers and IP remain
16-bit; only low FLAGS are modeled. No EIP/EFLAGS, FS/GS, protected mode,
paging, V86, FPU or CPUID support is claimed.

`CpuProfile::I80386RealMode` explicitly selects the **partial integer subset**.
I8086/I80186/I80286 retain their existing behavior. The host probe now selects
the 386 subset; the runtime default remains I8086. Profile selection does not
change slice budgets, yielding, timer wakeups or guest instruction scheduling.
No complete 386 CPU identity is advertised to guests.

| Family | 32-bit implementation in this milestone |
| --- | --- |
| MOV | B8–BF immediate GPR, 89/8B ModR/M, C7 /0 immediate register/memory |
| SHL/SAL, SHR, SAR, ROL, ROR, RCL, RCR | Shared 8/16/32-bit executor; immediate, CL and one-bit counts; register/memory |
| SHLD/SHRD | 0F A4/A5/AC/AD, immediate/CL, register/memory destination |
| ADD/ADC/SUB/SBB, AND/OR/XOR, CMP/TEST | Existing byte/word paths only; 32-bit forms explicitly trap |
| INC/DEC, PUSH/POP, LEA, XCHG | Existing 16-bit paths only; 32-bit forms explicitly trap |
| MUL/IMUL/DIV/IDIV | Existing byte/word paths only; 32-bit forms explicitly trap |
| Near/far control transfers, strings, sign/zero extension | Existing subset only; operand-overridden forms explicitly trap |

The 32-bit dispatcher is deliberately an allowlist. It never silently executes
an unsupported dword operation through the word executor. Operand override on
implemented byte MOV/group-2 operations keeps byte semantics; 66h NOP remains
inert. REP on the dword dispatcher is unsupported and traps explicitly.

### Prefixes, operands and FLAGS

80386 real-mode defaults stay 16/16. Local 66h state selects the supported dword
forms and expires at the next instruction. Repeated 66h selects 32 bits rather
than toggling. Segment overrides compose with it; effective addresses still use
all existing 16-bit ModR/M displacement/default-segment rules. 67h is recognized
but **staged unsupported**: every address override traps before operand execution,
including combined 66h/67h. SIB, 32-bit offsets and address-size strings are not
implemented. LOCK is unsupported and traps. Prefix chains are bounded at 15
bytes; general enforcement of the total 15-byte instruction limit across every
legacy executor remains future work.

Dword register/memory access helpers are reusable independently of opcode
execution. Memory accesses are little-endian and allow unaligned operands.
The complete four-byte segment extent is checked before either half is written;
offsets above FFFCh produce `SegmentLimit`, including for zero-count shifts.
Each underlying byte remains inside the private one-megabyte guest memory with
A20 disabled (20-bit physical wrapping). FFFCh is valid; FFFDh is not. The old
word helper's segment-end wrapping policy is retained for legacy compatibility;
full profile-dependent word-boundary exception fidelity is not claimed.
Execution is single-threaded: no concurrent guest observer can see a half-written
dword. LOCK/bus atomicity and architectural exception-frame delivery are not
modeled; deterministic runtime traps are used instead.

All 386 shift counts use `count & 31`; carry rotations use the appropriate ring.
The shared executor steps at most 31 bits on this profile, avoiding host shifts
by the host operand width. Zero-effective-count operations preserve FLAGS.
Shifts update SF/ZF/PF and CF, rotations preserve SF/ZF/PF, and OF is updated
only for count one. SHLD/SHRD update CF/SF/ZF/PF and count-one OF. AF and
multi-bit OF are undefined and retained; no correctness claim depends on them.
The existing 8086 unmasked CL behavior and later 16-bit profile tests still pass.

### Authentic execution and next exact blocker

The unchanged checksum-verified DN.COM launcher is the primary test. At retired
**3,408,442**, **3CD4:3FBE**, bytes `66 C1 E0 10`, EAX begins at **0000005F**.
The next trace shows EAX **005F0000** (upper word 005F, AX=0000), FLAGS
**F046 → F006**, and IP **3FBE → 3FC2**. CF=0, ZF=0, SF=0, PF=1;
undefined multi-bit OF and AF are not used as correctness criteria.

The next instruction `66 0F AC D0 10` (SHRD EAX,EDX,16) executes at 3FC2,
producing EAX=0000005F. SHL ECX,16 and SHRD ECX,EBX,16 also execute, restoring
ECX=00000400. No DOS/BIOS calls intervene in these four 386 instructions.

The new reproducible boundary is **3CD4:3FD0**, bytes **66 F7 E9**:
**one-operand signed IMUL ECX**, which requires the 64-bit signed product in
EDX:EAX. At the trap, EAX=0000005F, ECX=00000400, EDX=00000000, FLAGS=F006.
The following bytes begin `66 0F A4 C2 10` (SHLD EDX,EAX,16, now supported).
The generic 32-bit multiplication family is the next bounded dependency;
no instruction skipping or DN-specific behavior was added.

Final retired count is **3,408,446**. The existing runtime recovers the child
trap into the suspended launcher at **1041:01F0**, state Running. This is not
an application exit, successful cleanup workflow, or panel startup. The screen
still contains only the authentic copyright banner. DOS/BIOS calls and video
BDA/B800 snapshots are retained in `launcher.log`; the final instruction window
records all newly reached prefix/opcode combinations and full split GPR state.
The probe remains bounded to ten million trace steps and a 1000-instruction
window; guest output is not persisted to host files.

### Validation and separate outcomes

Five new guest-machine-code tests cover all GPR widths/aliases, upper-word
preservation, exact IP, prefix lifetime/segment composition, dword MOV and
unaligned memory, nested INT/IRET with upper halves, all seven shifts/rotates,
immediate/CL/one-bit counts, 0/1/16/31/32/33/255 boundaries, defined flags,
SHLD/SHRD, staged 67h/LOCK/unsupported opcode rejection, and segment limits.
Expectations use explicit bit operations and independent 64-bit concatenation
for double shifts. Undefined FLAGS are excluded. Existing tests cover shell,
Mines, BIOS/DOS, loaders, allocation, keyboard, rendering, interrupts and slices.

| Deliverable | Result |
| --- | --- |
| 32-bit GPR storage and aliases | **PASS** |
| Operand-size override | **PARTIAL**: supported families above; other dword forms trap |
| Address-size override | **PARTIAL**: recognized and deterministically rejected; no 32-bit EA support |
| 32-bit shifts and double shifts | **PASS** for implemented forms |
| 32-bit arithmetic | **BLOCKED** at IMUL ECX; general arithmetic not implemented |
| Authentic DN startup | **PARTIAL**, 3,408,446 retired instructions |
| Two-panel rendering and interactive workflows | **BLOCKED** by IMUL; workflows **NOT_RUN** |
| Legacy regressions | **PASS**: 146 core passed, 1 pre-existing ignored; 30 UI passed |
| Core no_std check | **PASS** |
| Native release ELF compilation | **PASS**, existing dependency warnings |
| Native QEMU execution | **NOT_RUN** |

Native prerequisites were inspected again: `MAX_HOST_FILE` remains 65536 bytes,
below DN.PRG and DN.OVR, and the adapter does not forward the profile. Host
execution has not reached a stable interface. No supported full-image isolated
native launch is therefore available here. The isolated diagnostic SunApp is
retained without global installation, Documents access, unrestricted filesystem
grants, image modification or host writeback. No QEMU screenshot or host render
is presented as native validation.

Reproduction:

```sh
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3408438 cargo run -p chronos-core \
  --example dos_probe --target x86_64-unknown-linux-gnu \
  -- target/dos-navigator/dn151/DN.COM
```

Exit status 2 denotes the recovered child compatibility trap. Check commands
remain those in DN-6, with fresh logs in `dn-7/`. SHA-256 and native prerequisite
evidence are in `identity-native.txt`.

### Expansion path

Next extend the common integer operand/flag helpers for full-product IMUL/MUL
with signed-overflow tests, then add only evidenced arithmetic dependencies.
Unify dword stack, strings and control transfers after specifying 16-bit stack
address semantics separately from operand width. Add 32-bit EA/SIB decoding with
full segment-limit checks before enabling 67h. Complete total instruction-length
validation, EFLAGS/exception semantics and broader conformance before claiming
an 80386 profile is complete. Reserve 486/Pentium profiles for separately tested
instruction/feature additions (including CPU identification and FLAGS behavior),
not aliases of the current partial profile. Scheduling/speed configuration stays
separate. No commit or push was made; pre-existing workspace edits were retained.


## DN-8: width-aware integer multiplication (2026-11-18)

Minimum completion chain **PASS**: `66 F7 E9` → correct signed full product →
EDX:EAX verified → regressions pass. Evidence is retained in
`docs/audit/chronos-dn/dn-8/`; diagnostic logs are host interpreter runs.

### Root cause and arithmetic semantics

`baseline.log` reproduces the DN-7 trap at retired 3,408,446. The 66h prefix
selects the explicit 32-bit allowlist, which rejected F7 before decoding ModR/M.
The captured CPU IP was 3FD2 (prefix and opcode fetched); neither ModR/M nor
arithmetic/writeback had executed. E9 is mod=3, reg=5, r/m=1: register ECX,
single-operand signed IMUL, operand size 32, default real-mode address size 16.
The legacy F6/F7 byte/word multiply paths already existed and were correct;
they now share the same full-product helper as dword execution.

Unsigned operands use u64 multiplication; signed operands are sign-extended
at the selected width before i64 multiplication. The complete product is kept
before splitting AL×operand into AX, AX×operand into DX:AX, or EAX×operand
into EDX:EAX. CF and OF agree: unsigned overflow means a nonzero upper half;
signed overflow means the full product cannot be represented at the operand
width. PF/AF/ZF/SF and unrelated FLAGS retain their prior values, the existing
deterministic undefined-flag policy. No EFLAGS upper-word model was added.

Two/three-operand IMUL shares this helper but writes only the truncated
register destination. 6Bh sign-extends its byte immediate; 69h fetches the
selected word/dword immediate. Source operands are captured before writeback,
including aliases of accumulator/high-result registers. Shared ModR/M readers,
segment overrides and dword extent validation remain authoritative. 67h and
LOCK remain explicitly rejected. Byte F6 multiplication retains byte width
under 66h. 69h/6Bh require 80186 or later; 0F AF requires the partial 386 profile.

| Instruction family | Compatibility / test coverage |
| --- | --- |
| F6 /4,/5 MUL/IMUL r/m8 | Supported and tested |
| F7 /4,/5 MUL/IMUL r/m16 | Supported and tested |
| 66 F7 /4,/5 MUL/IMUL r/m32 | Supported and tested |
| 0F AF IMUL r16,r/m16; 66 0F AF r32,r/m32 | Supported and tested |
| 69/6B IMUL r16/r32,r/m16/r/m32,immediate | Supported and tested |
| SHL/SHR/SAR, rotates, dword SHLD/SHRD | Supported and tested subset from DN-7 |
| Byte/word DIV/IDIV | Supported but incompletely tested; unchanged |
| Dword DIV/IDIV, general ALU, stack/control/string overrides | Unsupported |
| 67h address decoding, EFLAGS, protected mode | Unsupported |
| Other 386 instruction families not audited here | Not yet investigated |

Three new execution-level tests use machine code and explicit boundary tables:
positive/negative/zero, signed minima/maxima, unsigned maxima, overflow and
non-overflow, full result halves, unchanged registers/undefined flags, immediate
sign extension, register and segment-overridden memory operands, FFFCh valid
and FFFDh invalid dword extents, profile gates, prefix expiration and exact IP.
Existing shift/SHLD/SHRD, DOS/BIOS, guest interrupts, MZ/COM loaders, arena,
Pascal Shell and Mines tests remain in the core/UI regression suites.
These are focused conformance tests, not exhaustive 80386 certification.

### Authentic startup and next reproducible boundary

Checksums in `checksums.txt` match the original archive and all three executed
artifacts documented above. No guest binary was modified. `launcher.log` shows:

- Before IMUL at 3CD4:3FD0: EAX=0000005F EBX=00000000 ECX=00000400
  EDX=00000000 ESI=00000081 EDI=000044B2 EBP=000048DA ESP=000046BC.
  DS=4195 ES=4C91 SS=4C91, low FLAGS=F006; upper EFLAGS unmodeled.
- After IMUL, retired 3,408,447, IP=3FD3: EDX:EAX=00000000:00017C00
  (95×1024=97280), CF=OF=0, FLAGS remain F006. Unrelated state is intact.
- Subsequent `66 0F A4 C2 10` SHLD executes, leaving DX=0001, AX=7C00.
  Execution returns to Pascal startup; no DOS/BIOS call intervenes in IMUL/SHLD.

The new exact blocker is **1249:02AF**, bytes **CD 10**, **AX=1130h BH=00h**,
at **3,409,991 retired instructions**: BIOS font-information query, trapped as
`UnsupportedInterrupt { interrupt: 16, function: 17 }` (decimal fields).
`next-blocker.log` retains the final bounded instruction window. Before INT:
EAX=00011130 EBX=00000000 ECX=00000607 EDX=00000100 ESI=00004D00
EDI=00004B66 EBP=00004FFE ESP=00004DF2 DS=4195 ES=4C91 SS=4C91 FLAGS=F046.
The INT opcode/immediate are fetched before dispatch; child recovery returns to
1041:01F0. The diagnostic exits with status 2; no clean DN exit occurred.

Newly reached execution includes IMUL ECX and the following SHLD, followed by
ordinary word instructions/far returns. Later DOS file-open calls return AX=2,
CF=1 (file not found); these guest-visible failures do not immediately stop
startup. INT 10h AH=03h and AH=0Fh complete before AX=1130h blocks it.
The logs retain DOS/BIOS calls and before/after BDA and B800 snapshots. Final
video text remains the authentic copyright banner; no panels or new error
screen are observed. No keyboard response or file-manager workflow is claimed.

| Outcome | Status |
| --- | --- |
| Multiplication semantics | **PASS**, focused 8/16/32-bit tests |
| 32-bit CPU execution | **PARTIAL**, explicit implemented integer subset |
| Authentic DOS Navigator startup | **PARTIAL**, now 3,409,991 retired |
| Two-panel interface | **BLOCKED**, BIOS font query |
| Keyboard interaction | **NOT_RUN**, no panels reached |
| Regression validation | **PASS**, 149 core passed / 1 pre-existing ignored; 30 UI passed |
| Core no_std check | **PASS** |
| Native release build | **PASS**, existing dependency warnings |
| Native QEMU | **NOT_RUN** |

Native QEMU prerequisites remain blocked: the adapter still caps imported files
at MAX_HOST_FILE=65536 (DN.PRG/DN.OVR exceed it) and does not forward a selected
386 profile. Host execution has not reached a stable interface, so the conditional
native launch was not attempted. Existing supported native workflow cannot yet
reproduce this full-image diagnostic. The isolated bundle remains at
`target/dos-navigator/package/DosNavigatorDiagnostic.sunapp`; no global install,
Documents grant, additional privilege, native filesystem access or persistence
was added. No commit or push was made; existing workspace edits were retained.

Reproduce with the DN-7 command, setting CHRONOS_DETAIL_START=3408444 for the
multiply result or 3409980 for the next blocker. Validation commands are unchanged
from DN-6; fresh core/UI/no_std/native build logs are in `dn-8/`.

## DN-10: VGA blink/intensity switching and next startup blocker (2026-11-18)

Overall status: **PARTIAL**. The requested `INT 10h/AX=1003h` compatibility gap
is resolved and authentic DN 1.51 advances beyond the previous stop. The
two-panel interface is still **BLOCKED**, now by a generic hardware-port read
rather than a VGA service. Native QEMU DN validation remains **NOT_RUN**.

| Deliverable | Status |
| --- | --- |
| `INT 10h/AX=1003h` dispatch | **PASS**, generic BL=00h/BL=01h handling |
| Shared text-attribute bit-7 semantics | **PASS**, one decoder for BIOS/renderer |
| Blink presentation | **PARTIAL**, timed host renderer phase; native QEMU not exercised |
| BIOS and renderer consistency | **PASS**, raw attributes unchanged in guest memory |
| Authentic DN startup | **PARTIAL**, advances past AX=1003h, stops at port `0x61` read |
| Two-panel interface | **BLOCKED**, no panels reached |
| Regression suite | **PASS**, 165 core / 33 UI / no_std / native release build |
| Native QEMU DN launch | **NOT_RUN**, adapter limits below |

### Reproduction and exact input

The unchanged publisher archive and executables were rehashed before execution;
values match the DN-8 table (`DN.PRG` SHA-256
`ac0024e269ecf0feb4a8db15f96181092604567a62cf00fb8453d9052c2862c3`).
Evidence is retained in `docs/audit/chronos-dn/dn-10/`.

`baseline.log` (pre-change) and `direct.log` (post-change) reproduce the same
guest call site at retired **3,408,733**:

```text
CS:IP=35A7:06E4  bytes CD 10
AX=1003 BX=0000 CX=0010 DX=5018 SI=4D00 DI=4B66 BP=4DF6 SP=4DF4
DS=4117 ES=4C13 SS=4C13  FLAGS=F097 (CF=0, IF=1)
video mode=03h, active page=00h, cursor=(0,0), shape=0607
text attribute mode=Blink, generation=0
B800:0000=[20 07 20 07 ...] (unchanged before/after)
```

The actual inputs are **BL=00h** and **BH=00h**. DN requests background
intensity, not blinking. The pre-change dispatcher had no `AH=10h` branch, so it
returned `UnsupportedInterrupt { interrupt: 16, function: 16 }`; IP advanced to
`06E6` and no register, BDA, page, or `B8000h` byte changed. This is a rejected
service, not a later display-state failure.

### Implemented video-state behavior

- `chronos-core/src/dos.rs` dispatches `AX=1003h` generically: `BL=00h` selects
  background intensity, `BL=01h` selects blinking. `BH` is documented as `00h`,
  is not an output, and is preserved; other `BL` values are undefined input and
  leave the state unchanged without trapping.
- `chronos-core/src/text_mode.rs` owns `TextAttributeMode` and
  `TextAttribute::decode`. The raw attribute byte remains authoritative in guest
  `0xB8000`; only the derived foreground/background/blink view changes.
- `Runtime` stores the selected mode plus a generation counter. The generation
  advances on a real mode change without writing guest memory, so the renderer
  can repaint existing cells when their interpretation changes.
- A VGA BIOS mode set resets the virtual Attribute Controller mode-control bit to
  its documented default, blinking. No hardware-port Attribute Controller was
  implemented; this is one virtual mode-control bit, not a duplicate color path.

### Text attribute bit-7 interpretation

| Mode | Bits 0-3 | Bits 4-6 | Bit 7 |
| --- | --- | --- | --- |
| Blink | foreground | background (0-7) | blink marker |
| Background intensity | foreground | background bits 0-2 | background bit 3 (0-15) |

Foreground intensity remains bit 3 in both modes. `0x8F` therefore means
white-on-black blinking in blink mode and white-on-bright-black (background 8)
in intensity mode. CP437 mapping and line-drawing glyphs are unchanged.

### Blink presentation

The native Chronos renderer now derives colors through the shared decoder and
uses a 500 ms presentation phase. In blink mode, the off phase paints the glyph
in the background color; guest attributes are never modified. A repaint is
requested only when the phase changes and the visible page actually contains a
bit-7 attribute under blink mode, so there is no per-instruction redraw and no
guest busy wait. Focus loss does not alter guest state. This is a bounded
presentation feature, not a compositor subsystem. It was validated in host
renderer tests; native QEMU visual confirmation is **NOT_RUN**.

### Post-change authentic startup

`direct.log` shows the same call completing:

```text
before: attr_mode=Blink, generation=0
after:  state=Running, IP=06E6, AX/BX/CX/DX/SI/DI/BP/SP/DS/ES/SS/FLAGS unchanged
        attr_mode=BackgroundIntensity, generation=1
        B8000:0000 identical
```

The guest then executes five more instructions and stops at retired
**3,408,738**:

```text
CS:IP=35A7:2458  bytes E4 61 24 FC E6 61 CB
E4 61 = IN AL,61h
Trap::UnsupportedIoPort { operation: Read, port: 0x61, width: Byte, cs: 35A7, ip: 2458 }
```

Port `0x61` is the keyboard-controller port B / PC-speaker latch, not a VGA
service. The surrounding sequence (`IN AL,61h; AND AL,0FCh; OUT 61h,AL; RETF`)
is a generic speaker-off helper. It was **not** implemented in this milestone;
doing so is a separate hardware-model change. This is the next independently
reproducible compatibility issue.

`launcher.log` exercises the original DN.COM launcher. It reaches the same
`AX=1003h` call at retired **3,410,016**, completes it, and recovers the child
trap at the same port `0x61` read at retired **3,410,021**. The authentic
copyright banner remains on screen; no panels appear.

Video BIOS calls observed in `direct.log`: `AX=121Ah`, `AX=0100h`, `AX=0300h`,
`AX=0F00h`, `AX=1130h`, `AX=1003h`. The font-information service still returns
`CX=16`, `DL=24`, and `ES:BP=C000:0400`.

### Regression coverage

New focused tests exercise the service through real guest `INT 10h`
instructions and cover: dispatch, BL=00h, BL=01h, actual BH input and undefined
BL, unrelated-register/FLAGS preservation, foreground and background decoding,
bit 7 in both modes, mode switching without `B8000h` rewrites, existing screen
contents, BIOS AH=09h writes, direct guest video-memory writes, page/cursor
state, mode-set reset, font-information regression, CP437 line drawing, and
blink-phase rendering. Existing cursor, scroll, Pascal shell, Mines, CPU,
interrupt, MZ/COM loader, and scheduler tests remain green.

| Validation | Result |
| --- | --- |
| Chronos core tests | **PASS**, 165 passed, 0 failed, 1 pre-existing ignored |
| Chronos UI host tests | **PASS**, 33 passed, 0 failed (includes shell and Mines) |
| Core `no_std` check | **PASS**, `cargo check -p chronos-core --target x86_64-unknown-none` |
| Native release build | **PASS**, `bash tools/build_native_std.sh build -p sunlight-chronos --release` |
| `sunlight-chronos` rustfmt | **PASS** |
| `chronos-core` rustfmt | **PARTIAL**, pre-existing uncommitted DN-9 formatting diffs remain in untouched lines |
| Native QEMU DN launch | **NOT_RUN**, see below |
| Two panels, colors, keyboard, tab, clean exit | **NOT_RUN**, guest blocked at port `0x61` |

Native QEMU prerequisites are unchanged: the adapter still caps imported files
at `MAX_HOST_FILE=65536`, which truncates `DN.PRG` (109,613 bytes) and `DN.OVR`
(745,951 bytes), and it does not forward a selected 386 profile to the guest.
The isolated diagnostic SunApp was not reinstalled, no Documents grant or new
capability was added, and the authentic binary was not modified. Host
interpreter execution is not native SunlightOS validation.

Next milestone: model the generic keyboard-controller port B / speaker latch
read at `0x61` (or document why it must remain absent), then rerun the exact
verified image to find the next blocker. No commit or push was made; existing
workspace edits were retained.

## DN-11: PC System Control Port B (2026-11-18)

**PASS** for the Port B control model; **PARTIAL** for authentic startup.
DN now stops at BIOS `INT 15h / AX=1022h`. Panels and clean exit remain
**BLOCKED**. The publisher archive and DN.COM, DN.PRG, and DN.OVR matched
the recorded DN-10 SHA-256 values before and after testing. The three files
in the existing diagnostic SunApp's `Program` directory were byte-for-byte
identical to those verified inputs. Runs below are from the host interpreter
using those files, without reinstalling or launching the SunApp natively.

| Observation | Trace |
| --- | --- |
| Direct baseline, retired 3,408,738 | `35A7:2458 E4 61` (`IN AL,61h`, byte); `AX=1003 DX=5018 FLAGS=F097`. The immediate port operand supplies `61h`; AL is the destination. The unsupported read leaves AL at 03h and the instruction does not retire; decoder IP advances to 245Ah. |
| Surrounding code | `1051:166E 9A 58 24 A7 35` calls the helper. At `35A7:245A`, `24 FC` masks off gate and speaker bits; at `245C`, `E6 61` writes AL back; at `245E`, `CB` returns. No conditional branch, status test or polling loop is involved. |
| Preceding I/O | The only earlier access in the direct trace was `OUT F0h,AL` with AL=00h at `3C56:3003`, retired 3,401,561. No Port B writes or accesses to PIT ports `40h–43h` preceded the read. |
| Direct after change | Read at the same site returns `00h` (`AX=1000`), all control/status bits clear. The mask leaves AL=00h; the write at retired 3,408,740 succeeds, and the far return executes. No other new I/O appears before the next stop. |
| New direct boundary | Retired **3,469,869** at `11CB:04B3`, bytes `CD 15 09 DB 74 1C B4 01`, `AX=1022 BX=0000 CX=0000 DX=5019 FLAGS=F046`. The BIOS call returns the diagnostic trap `UnsupportedInterrupt { interrupt: 21, function: 16 }` (decimal fields); post-decode IP is 04B5h. |
| Original DN.COM launcher | Read at `3625:2458`, retired 3,410,021, also returns 00h; write at `3625:245C`, retired 3,410,023, succeeds. The child reaches `1249:04B3` at retired **3,476,252** with the same BIOS call and AX. Chronos recovers the child trap and restores the launcher at `1041:01F0`; probe exits with status 2. |

The helper disables both speaker controls; it preserves the error-check
control bits. It does not consume the refresh or PIT output bits. The shell
probe reached its input wait and the standalone Mines probe ran to the
10,000,000-step trace limit without showing this port sequence. Those traces
are startup samples; the workload regressions below exercise input and exit
paths separately.

Chronos has a mixed PC-compatible device subset (including VGA and the AT
coprocessor latch clear), not a full XT or AT machine. This device shares the
AT-compatible layout across CPU profiles. See [PC System Control Port B](chronos.md#pc-system-control-port-b)
for the control/status split, absent PIT/IRQ0 and deterministic refresh policy.
No guest binary, adapter permissions, host access, or other I/O port changed.

| Validation | Result |
| --- | --- |
| Core/CPU/I/O: `cargo test -p chronos-core -p sunlight-chronos --target x86_64-unknown-linux-gnu` | **PASS**: core 167 passed, 1 pre-existing ignored; UI 33 passed. New device tests cover all low-nibble combinations including active-low readback, ignored status/strobe writes, 10,000 stable reads, instance isolation, video reset independence and neighboring/word traps. Existing guest IN/OUT and VGA DAC tests passed. |
| DOS shell / Mines | **PASS**, host workload tests: shell prompt, commands and child return; Mines initial and continued frames, pointer input, restart and Escape. |
| `cargo check -p chronos-core --target x86_64-unknown-none` | **PASS** |
| `bash tools/build_native_std.sh build -p sunlight-chronos --release` | **PASS**, native build only |
| Native QEMU DN / boot | **NOT_RUN**. The adapter still limits each imported file to 65,536 bytes, truncating DN.PRG (109,613 bytes) and DN.OVR (745,951 bytes), and does not forward the diagnostic 386 profile. The generic kernel boot gate does not exercise DN. |

Reproduce the direct host trace with:

```sh
sha256sum -c docs/audit/chronos-dn/dn-10/checksums.txt
CHRONOS_TRACE=1 CHRONOS_TRACE_IO=1 CHRONOS_DETAIL_START=3469865 \
  cargo run -p chronos-core --example dos_probe --target x86_64-unknown-linux-gnu -- \
  target/dos-navigator/package/DosNavigatorDiagnostic.sunapp/Program/DN.PRG
```

Use detail start `3408730` for the direct Port B call site or pass `DN.COM`
to run the launcher. The opt-in host probe reports up to 128 I/O instructions
per run; reads do not add logging or counters to the virtual device.

## DN-12: INT 15h and TopView GETVER (2026-11-18)

**PASS** for the generic BIOS fallback; **PARTIAL** for authentic startup.
Both unchanged entry paths returned from GETVER and reached a later VGA
BIOS call. Panels and clean exit remain **BLOCKED**.

The DN-11 baseline reproduced with direct DN.PRG at `11CB:04B3`, retired
3,469,869, and the DN.COM child at `1249:04B3`, retired 3,476,252. The IVT
entry at physical `00054h` was `0000:0000` from startup to the trap in
both runs. There was no BIOS handler, Chronos predecessor, or observed guest
hook or chain for INT 15h. No DOS `AX=2515h` calls occurred. The generic
dispatcher already read non-host vectors and could enter a guest handler;
with no vector it raised `UnsupportedInterrupt { interrupt: 0x15, function:
0x10 }` after decoding `CD 15`. The call did not retire or create a frame,
and no DN instructions executed after the failure. The launcher's reported
post-trap CPU belonged to the restored parent at `1041:01F0`.

The pre-call state was identical before and after the fix for each entry
path. Both had bytes `CD 15 09 DB 74 1C B4 01`, and the complete shared
register state was:

```text
EAX=00011022 EBX=00000000 ECX=00000000 EDX=00005019
ESI=00000000 EDI=00000000 EBP=00004B14 ESP=00004B12
FLAGS=F046 (CF=0, IF=0, TF=0)
```

| Path | CS:IP | DS / ES | SS:SP | Retired |
| --- | --- | --- | --- | ---: |
| Direct | 11CB:04B3 | 4117 / 4117 | 4C13:4B12 | 3,469,869 |
| Launcher child | 1249:04B3 | 4195 / 4195 | 4C91:4B12 | 3,476,252 |

[Ralf Brown's Interrupt List, INT 15h GETVER](https://fd.lod.bz/rbil/interrup/quarterdeck/151022bx0000.html)
and the [DOS Interrupts Reference Guide, p. 35](https://ic.unicamp.br/~ducatte/mc404/Lampiao/docs/dosints.pdf)
document `AX=1022h`, `BX=0000h` as inputs. A loaded compatible environment
returns nonzero BX, with BL major and BH minor; TaskView returns `0001h`
and DESQview 2.00+ returns `0A01h`. This does not prescribe how a plain BIOS
handles the call. The [IBM PC AT BIOS V3 listing](https://github.com/Gawlas/IBM-PC-BIOS/blob/master/IBM%20PC%20AT/AT%20BIOS%20V3/BIOS1.ASM)
branches from `CASSETTE_IO_1` to `C1` for `AH<80h`, setting `AH=86h` and
CF. Its implementation uses STI and a far return that discards saved FLAGS;
that instruction sequence is specific to the IBM ROM. Chronos's diagnostic
386 setting selects CPU decoding, not a full AT ROM or chipset. Its existing
AT-compatible device subset now uses that error status with an explicit IRET
policy that preserves non-CF caller flags.

This was **Case A**, a missing BIOS predecessor on an otherwise functioning
IVT path. Chronos now installs the generic unsupported-service handler at
`F000:E000`. There were no existing INT 15h subservices to preserve. The
handler contains no function or register-value checks. DN's BX remains zero
because the BIOS preserves BX, and there is no advertised TopView version.
Both final traces saw only the firmware vector from startup to the next trap.

At entry, Chronos pushed next IP `04B5`, caller CS and FLAGS `F046`;
SP became `4B0C`, and IF/TF were clear. The handler saved BP, used it to
address the saved FLAGS through SS, set CF there (`F047`), restored BP,
set AH to `86h`, and executed IRET at `F000:E00A`. The exact caller state
on return was the input state above with `EAX=00018622`, `FLAGS=F047`
(CF=1, IF=0, TF=0) and `IP=04B5`; CS, SP, segments and all other GPRs
were unchanged. The return retired at count 3,469,876 direct and 3,476,259
via the launcher (counts at the first instruction after IRET).

In both paths, `04B5: 09 DB` (`OR BX,BX`) set ZF and cleared CF. At
`04B7: 74 1C`, `JZ` took the branch to `04D5`. DN then set AH=03h
and BH=00h and called its video wrapper; it did not take the nonzero-BX
path or attempt a TopView operation. Subsequent calls were all INT 10h:
`AX=0322h` (read cursor), `0122h` (set cursor shape), `1003h`
(attribute mode) and [`1C00h` (VGA video-state size query)](https://fd.lod.bz/rbil/interrup/video/101c.html). No DOS interrupts
or port I/O were observed after GETVER before this stop.

| Path | Next instruction that traps | Retired | Result |
| --- | --- | ---: | --- |
| Direct | `11CB:09B7 CD 10`, `AX=1C00 BX=4B00 CX=0007` | 3,474,021 | Unsupported INT 10h function 1Ch, decoder IP 09B9h |
| Launcher child | `1249:09B7 CD 10`, same AX/BX/CX | 3,480,404 | Same child trap; launcher restored at `1041:01F0` |

The next compatibility investigation is the VGA state query. DN-12 does not
implement it. No guest binary, launcher, checksum, adapter permission or
host file access changed. The firmware code takes six guest instructions to
return, uses only the guest stack and adds no allocation, logging or host
clock dependency. It never dispatches another interrupt.

| Validation | Result |
| --- | --- |
| Chronos core/CPU/BIOS/interrupt and UI: `cargo test -p chronos-core -p sunlight-chronos --target x86_64-unknown-linux-gnu` | **PASS**: core 170 passed and 1 pre-existing ignored; UI 33 passed. New tests cover the entry frame, IF/TF/CF, full GPR and segment preservation, read-only firmware, guest interception, both chaining styles, 10,000 repeated calls, and instruction-budget handling of guest recursion. |
| DOS shell and Mines | **PASS** in the host suite: shell prompt, commands, child return; Mines first and continued frames, pointer input, restart and clean Escape. |
| `cargo check -p chronos-core --target x86_64-unknown-none` | **PASS** |
| `bash tools/build_native_std.sh build -p sunlight-chronos --release` | **PASS**, native build only |
| Platform boot gate: `bash tools/test.sh` (phase3.0) | **BLOCKED**: kernel build exited 101 with `No space left on device` writing a compiler output file; QEMU did not start. |
| Native QEMU DN | **NOT_RUN**: the adapter still truncates DN.PRG and DN.OVR at 65,536 bytes and does not forward the diagnostic 386 profile. The diagnostic SunApp was neither reinstalled nor given new capabilities. |

[DN-12 traces and build logs](../audit/chronos-dn/dn-12/) use the existing
diagnostic SunApp's `Program` files in the host interpreter. Before and
after the run, the archive and extracted DN.COM, DN.PRG and DN.OVR matched
the DN-10 SHA-256 manifest, and each of the three packaged files matched its
verified extracted counterpart. A separate byte comparison matched all 63
packaged regular files to members of the verified archive. The host probe
exited with status 2 at the expected trap in both baseline and final runs. To reproduce the final trace:

```sh
sha256sum -c docs/audit/chronos-dn/dn-10/checksums.txt
CHRONOS_TRACE=1 CHRONOS_TRACE_INT15=1 CHRONOS_TRACE_IO=1 CHRONOS_DETAIL_START=3469867 \
  cargo run -p chronos-core --example dos_probe --target x86_64-unknown-linux-gnu -- \
  target/dos-navigator/package/DosNavigatorDiagnostic.sunapp/Program/DN.PRG
```

Use `CHRONOS_DETAIL_START=3476250` and `DN.COM` for the launcher run.

## DN-13: VGA Video State

**PASS** for the generic BDA state interface and for advancing beyond
`AX=1C00h`. **PARTIAL** for DN startup: both paths stop at the same
subsequent BIOS call, and the panels are not visible.

Both entry paths reach `CD 10 3C 1C 74 0F B8 00` with
`EAX=00011C00 EBX=00004B00 ECX=00000007 ESI=00000FA0 EDI=00000FA0
EBP=00004B14 ESP=00004B10 FLAGS=F093` (CF=1, IF=0).
The IVT entry is `0000:0000`; Chronos handles INT 10h in the generic
dispatcher without entering a guest handler. The adapter is in mode 03h,
page zero, 80×25 with 16 scan lines per character, cursor shape `2000h`,
and background intensity enabled (generation 1). The direct cursor is
`(0,0)`; the launcher trace shows `(0,1)`.

| Entry | CS:IP | EDX / DS | SS:SP | ES:BX (physical) | Retired before INT |
| --- | --- | --- | --- | --- | ---: |
| Direct | `11CB:09B7` | `00004117 / 4117` | `4C13:4B10` | `68D3:4B00 (6D830h)` | 3,474,021 |
| Launcher child | `1249:09B7` | `00004195 / 4195` | `4C91:4B10` | `6951:4B00 (6E010h)` | 3,480,404 |

`CX=0007h` requests hardware, BIOS data, and DAC state together.
Chronos has a guest BDA for the current mode, page-zero cursor and shape,
and geometry. It models text attributes and blink/intensity, but lacks
complete sequencer, CRTC, attribute-controller and graphics-controller
register files. The mode 13h DAC has 256 palette entries and indexed RGB
I/O, but lacks the complete mode, pixel-mask and color-select controls.
Only the BDA mask `0002h` supports all three subfunctions. The buffer is
`20h + 60h = 80h` bytes, or two 64-byte blocks; see [Chronos](chronos.md)
for layout and rejection policy. Multiple-area sizing is inapplicable to
this profile because every mixed request includes an unsupported area.

Both calls return `AX=1C00h BX=4B00h` with the other registers and flags
preserved; the query does not use the buffer. At `09B9h` DN executes
`CMP AL,1Ch`; the `JZ` at `09BBh` is not taken. There is no check of
the returned BX, allocation, save, or restore before the next boundary.
DN falls through to `MOV AX,1200h`, `MOV BL,32h`, then `INT 10h`
at `09C2h`, with `BX=4B32h`. Both children trap there on the
unimplemented `AH=12h` service; retired counts are 3,474,026 direct
and 3,480,409 via the launcher. The direct decoder stops at
`11CB:09C4`; the launcher reports the child trap and resumes the
parent at `1041:01F0`.

| Validation | Verdict |
| --- | --- |
| DN-12 recorded core and UI regressions, including interrupt hook chaining and flag restoration | **PASS**: 170 core passed, 1 ignored; 33 UI passed |
| DN-13 reviewed host suite, including BIOS, CPU, guest memory, interrupt dispatch, DOS shell, and Mines | **PASS**: 179 core passed, 1 ignored; 33 UI passed |
| Independent video tests | **PASS**: query and flag/register preservation, rejected masks/subfunctions, exact header and BDA bytes, text and graphics round trips, repeatability, and invalid RAM ranges and buffers |
| Core `no_std` and native release build | **PASS** |
| Platform boot gate | **NOT_RUN** for the reviewed revision at report time; the existing invocation was still building and was left running |
| Native QEMU DN | **NOT_RUN**: diagnostic adapter file-size and CPU-profile limits remain |

The reviewed host runs used the diagnostic SunApp's unchanged `Program`
files. The archive and three DN binaries passed the DN-10 checksum
manifest before and after the run; the packaged binaries also matched
the verified extracted files byte for byte.
[Reviewed traces and build logs](../audit/chronos-dn/dn-13/).

## DN-14: VGA CPU Address Decoding

**PASS** for the modeled VGA service and for advancing the unchanged
DN 1.51 binaries past DN-13's `INT 10h / AX=1200h / BX=4B32h`
boundary. **PARTIAL** for DN startup: the diagnostic shows the startup
banner and a shell prompt, without visible DN panels.

The preceding `AH=1Ch / CX=0007h` query still returns `AL=00h`.
Both entry paths then reach `CD 10 3C 12 74 04 29 C0` below.
The selector is AH=12h/BL=32h; the request is AL=00h (enable),
with BH=4Bh. The common input is
`EAX=00011200 EBX=00004B32 ECX=00000007 ESI=00000FA0 EDI=00000FA0
EBP=00004B14 ESP=00004B10 FLAGS=F097` (CF=1, IF=0). Chronos's
real-mode CPU does not expose FS/GS values in this trace.

| Entry | CS:IP | EDX / DS | ES | SS:SP | Retired before INT |
| --- | --- | --- | --- | --- | ---: |
| Direct | `11CB:09C2` | `00004117 / 4117` | `68D3` | `4C13:4B10` | 3,474,026 |
| Launcher child | `1249:09C2` | `00004195 / 4195` | `6951` | `4C91:4B10` | 3,480,409 |

The INT 10h IVT entry is `0000:0000` on both paths. The generic
dispatcher handles the call synchronously without a guest interrupt
frame or IRET. The diagnostic uses the 80386 real-mode CPU setting
and Chronos's single color VGA profile. At entry, CPU decoding is
enabled; both paths report mode 03h, page zero, 80×25, cursor shape
2000h and background-intensity generation 1. The direct cursor is
(0,0); the launcher cursor is (0,1). Both report DAC generation 0,
checksum `710945753CE5A529h`, and framebuffer generation 0. These
video values are unchanged after the call.

Both resume at `09C4h` with `EAX=00011212` and all other registers
and FLAGS unchanged. DN executes `CMP AL,12h`, then takes `JZ`
at `09C6h` to `09CCh`. There it sets AX=0001h, executes RETF at
`09CFh`, and tests the wrapper result with `OR AL,AL` at `0687h`.
DN does not test BIOS carry. The first post-return data accesses
visible from the instruction trace are the far-return stack reads
and the word read at DS:883Ch (value 001Ah). No IN/OUT was observed
after the BIOS return; the trace does not log every data access.

The next failure is an unsupported ES-prefixed XLAT (`26 D7`;
following bytes `08 C0`). The decoder reports the fault at
`1443:0022` direct (3,745,436 retired; decoded IP `0024h`) and
`14C1:0022` in the launcher child (3,751,834 retired; decoded IP
`0024h`). The launcher recovers to parent `1041:01F0`. The
paths have the same behavior after accounting for relocation.

See [the VGA decoder contract](chronos.md#vga-cpu-address-decoding) for
the references, limits, and unsupported-input policy. The DN call tests
the acknowledgement on an already enabled adapter. Independent tests
exercise both directions by observing CPU reads and writes, DAC reads
and writes, retained display data, register and flag preservation,
and guest interrupt interception.

| Validation | Verdict |
| --- | --- |
| DN-12 recorded regressions | **PASS**: 170 core, 33 UI, 1 core ignored |
| DN-13 recorded regressions | **PASS**: 179 core, 33 UI, 1 core ignored |
| DN-14 core/CPU/BIOS/interrupt, DOS shell including MIDTERM, Mines, and UI | **PASS**: 181 core, 33 UI, 1 core ignored |
| Final core `no_std` check and native release build | **PASS** |
| Platform boot gate | **NOT_RUN** for QEMU at report time: DN-14 invocation still building |
| Native QEMU DN | **NOT_RUN**: adapter still caps files at 65,536 bytes and does not forward the diagnostic 386 setting |

The first host suite caught a MIDTERM regression from enabling hooks
for all INT 10h services. The final change limits interception to
`AH=12h / BL=32h`, preserving legacy routing for the other services.
Before and after the host probe, the DN-10 manifest verified the
archive and binaries, and the diagnostic SunApp binaries matched
the extracted files byte for byte. See the
[baseline and final diagnostic logs](../audit/chronos-dn/dn-14/).

The DN-13 gate process was active at the first inspection and was left
alone. It later exited without a completion marker in its redirected log.
The standalone DN-14 platform gate was then started and left running.
It was still in the kernel build, including the bundled Yazi build, at
the last inspection. The separate launch-regression QEMU process was
left untouched.

## DN-15: XLAT and Segment Override Semantics

DN-14's authentic boundary was `26 D7` at `1443:0022` (direct, retired 3,745,436) and `14C1:0022` (launcher child, retired 3,751,834). The DN-15 host diagnostic used the unchanged extracted DN 1.51 files with the 80386 real-mode profile. SHA-256 remained `DN.PRG` `ac0024e269ecf0feb4a8db15f96181092604567a62cf00fb8453d9052c2862c3`, `DN.COM` `ae0863550ec07f21c5cf8d208373a5022b868133e6f644757b1623cd8f7e9905`, and `DN.OVR` `401a8e631b8627fcbf8c209f01b1d71e3e314347f1c32417a0345d45455e3569`.

The instruction bytes are `26 D7`; lookahead is `08 C0`. Root cause: **unsupported opcode dispatch**. The generic prefix decoder accepted ES and delivered `D7` to execution, but XLAT had no implementation. The prefix was not rejected, and no incorrect segment, effective offset, guest-memory fault, or invalid execution mode caused the original boundary.

At the first XLAT, both paths used 16-bit address and operand sizes. Unsigned `AL=03h` and `BX=8738h` yield offset `873Bh`.

| State | Direct DN.PRG | DN.COM launcher child |
| --- | --- | --- |
| Retired before | 3,745,436 | 3,751,834 |
| CS:IP before | `1443:0022` | `14C1:0022` |
| EAX / EBX / ECX / EDX | `00010103 / 00008738 / 00000050 / 00006A9A` | `00010103 / 00008738 / 00000050 / 00006B18` |
| ESI / EDI / EBP / ESP | `00006A9A / 00000000 / 00004682 / 0000467E` | `00006B18 / 00000000 / 00004682 / 0000467E` |
| CS / DS / ES / SS | `1443 / 4117 / 4117 / 4C13` | `14C1 / 4195 / 4195 / 4C91` |
| FS / GS | not modeled | not modeled |
| Prefix / instruction | `26 / D7` | `26 / D7` |
| FLAGS before | `F293` | `F293` |
| Offset / selected segment | `873B / ES:4117` | `873B / ES:4195` |
| Linear address / loaded byte | `498AB / 04` | `4A08B / 04` |
| EAX / FLAGS after | `00010104 / F293` | `00010104 / F293` |
| CS:IP / retired after | `1443:0024 / 3,745,437` | `14C1:0024 / 3,751,835` |

Both direct and launcher runs execute beyond XLAT. The next unsupported operation is `66 99` at direct `3C56:400F` and launcher child `3CD4:400F`, at retired counts 3,749,458 and 3,755,856; `66 F7` follows. The launcher recovers to parent `1041:01F0`. At the final trap, the direct text dump shows `C:\>` and `F1 Help` but no startup title; the launcher dump shows the DN 1.51 title, `C:\>`, and `F1 Help`. The file panels are **unconfirmed**, since they are not established by the text dump.

At that next boundary, both paths have `EAX=001EAB90`, `EBX=00000000`,
`ECX=0000005C`, `EDX=0000001E`, `EDI=00000050`, `EBP=00004B56`,
`ESP=00004B4E`, and `FLAGS=F206`. Direct has `ESI=00006FE6`,
`CS:IP=3C56:400F`, `DS=4117`, `ES=6FE0`, `SS=4C13`; the launcher child
has `ESI=00007064`, `CS:IP=3CD4:400F`, `DS=4195`, `ES=705E`, `SS=4C91`.
The trap records decoded IP `4011` after consuming `66 99`. The next trace
bytes are `66 F7`; execution stops at the unsupported `66 99` boundary.

XLAT now uses the normal instruction dispatch and guest memory read path. Default addressing is `DS:(BX + zero-extended AL)`; the decoded segment override selects ES, CS, SS, or DS without altering offset calculation. Address-size 32 uses EBX and validates the result against the real-mode 64 KiB segment limit. Operand-size override does not change XLAT's byte width. AL is updated through its register alias; EAX's upper bits, BX/EBX, FLAGS, and retirement behavior are preserved. The CPU state does not model FS/GS. Repeated segment prefixes keep the existing last-prefix-wins behavior, with prefix state local to the instruction.

| DN-15 validation | Verdict |
| --- | --- |
| Root cause and authentic host reruns | **PASS** |
| XLAT and prefix tests | **PASS**: DS/ES/CS/SS, segment default, conflicts, size-prefix interaction, 16-bit wrap, 32-bit EBX, state preservation, retirement, segment limit |
| Core tests, including MIDTERM and Mines | **PASS**: 184 passed, 1 ignored |
| UI tests | **PASS**: 33 passed |
| Core `no_std` check | **PASS** |
| Native SunlightOS release build | **PASS**: `bash tools/build_native_std.sh build -p sunlight-chronos --release` |
| Platform boot gate | **BLOCKED**: `./tools/test.sh` exited 101 without diagnostic output; no gate process remains active |
| Native QEMU DN | **NOT_RUN**: adapter truncates files above 65,536 bytes and does not forward the diagnostic 386 profile |
| DN panels | **UNCONFIRMED** |

See [DN-15 host traces](../audit/chronos-dn/dn-15/). DN executables, launcher, and checksums were not modified.

## DN-16: CWD/CDQ and Operand-Size Override

DN-15's direct and launcher-child traces both reached `66 99` in the
80386-real-mode profile. The profile's default operand size is 16 bits;
`66h` selects 32 bits for this instruction. Prefix decoding consumes both
bytes, then dispatches opcode `99h` to `execute32`. The failure was in
execution dispatch: `execute32` did not implement opcode `99h`, even though
the normal 16-bit dispatcher already implemented CWD. This was not a decoder
rejection, and the instruction is handled generically without DN-specific
behavior.

At direct `3C56:400F` and launcher child `3CD4:400F`, the effective operand
size was 32 bits. Both had `EAX=001EAB90`, `EDX=0000001E`, and `FLAGS=F206`.
The trap's CPU state records IP `4011`, confirming that the prefix decoder
consumed exactly `66 99`; no prefix or instruction-byte issue caused the
trap. The runs share the same accumulator, data register, flags, and logical
instruction. Their segment and stack addresses differ as expected for the
separately loaded direct executable and launcher child.

Opcode `99h` now sign-extends EAX into EDX in the 32-bit dispatcher, leaving
EAX and FLAGS unchanged. The existing 16-bit dispatcher performs CWD by
sign-extending AX into DX. The tests cover positive, negative, zero, and
signed-boundary inputs, register and flags preservation, IP advancement,
override consumption, and an unprefixed CWD immediately after CDQ.

Both authentic runs used the unchanged files. Their SHA-256 checksums remain
`DN.PRG` `ac0024e269ecf0feb4a8db15f96181092604567a62cf00fb8453d9052c2862c3`,
`DN.COM` `ae0863550ec07f21c5cf8d208373a5022b868133e6f644757b1623cd8f7e9905`,
and `DN.OVR` `401a8e631b8627fcbf8c209f01b1d71e3e314347f1c32417a0345d45455e3569`.

| Path | Before / after | Result |
| --- | --- | --- |
| Direct DN.PRG | retired `3,749,458` → `3,749,459`; EAX `001EAB90` → `001EAB90`; EDX `0000001E` → `00000000`; FLAGS `F206` → `F206`; `3C56:400F` → `3C56:4011` | CDQ executed. Next unsupported bytes are `66 F7` at `3C56:4011`; direct run traps there. |
| DN.COM launcher child | retired `3,755,856` → `3,755,857`; EAX `001EAB90` → `001EAB90`; EDX `0000001E` → `00000000`; FLAGS `F206` → `F206`; `3CD4:400F` → `3CD4:4011` | CDQ executed. The child traps at `66 F7` (`CS:IP=3CD4:4011`); launcher restores its parent at `1041:01F0`. |

The direct screen still shows only `C:\>` and `F1 Help`. The launcher run
shows the DN 1.51 title, `C:\>`, and `F1 Help`. File panels are **unconfirmed**;
these text traces do not establish that they rendered.

| DN-16 validation | Verdict |
| --- | --- |
| CPU implementation | **PASS** |
| Regression suite | **PASS**: Chronos core 186 passed / 1 ignored; MIDTERM and Mines included; Chronos UI 33 passed; UI library 277 passed; core `no_std` check and native release build passed |
| Platform boot gate | **FAIL**: exited 101 before QEMU; kernel build lacks embedded `helios-note` and `wiseowl` artifacts (details in DN-16 audit log) |
| Native QEMU DN | **NOT_RUN**: the adapter still truncates files above 65,536 bytes and does not forward the diagnostic 386 profile |
| DN panels | **UNCONFIRMED** |

See [DN-16 host traces](../audit/chronos-dn/dn-16/). Native QEMU validation
remains explicitly unrun because the adapter still truncates the authentic
executables and cannot forward the required 386 profile.

## DN-17: Signed dword division

The DN-16 trap window was `66 F7 F9 66` on both paths. The first three
bytes form the complete instruction; the last byte prefixes the following
instruction. ModR/M `F9h` splits into `mod=3`, `reg=7`, `r/m=1`: Group 3
`IDIV ECX`, with ECX holding `0000005C`. Both runs were in the 80386
real-mode profile with 32-bit operand size from `66h` and the default
16-bit address size. There was no segment override, SIB, displacement,
effective address, or data memory access. The prefix and ModR/M decoders
had consumed all three bytes (trap CPU IP `4014h`), but the 32-bit
execution dispatcher accepted only multiplication suboperations at
`F7h` and returned `UnsupportedOpcode` before it could read the divisor.
The instruction did not retire in DN-16.

| Entry state (hex except retired) | Direct DN.PRG | DN.COM launcher child |
| --- | --- | --- |
| CS:IP | `3C56:4011` | `3CD4:4011` |
| Retired | 3,749,459 | 3,755,857 |
| EAX / EBX / ECX / EDX | `001EAB90 / 00000000 / 0000005C / 00000000` | `001EAB90 / 00000000 / 0000005C / 00000000` |
| ESI / EDI / EBP / ESP | `00006FE6 / 00000050 / 00004B56 / 00004B4E` | `00007064 / 00000050 / 00004B56 / 00004B4E` |
| FLAGS | `F206` | `F206` |

The new `F7 /7` path uses the common ModR/M decoder and checked dword
operand read. It divides signed EDX:EAX by signed r/m32, then checks that
the quotient fits EAX before writing EAX and EDX. It preserves FLAGS as
Chronos already does for IDIV; Intel leaves the arithmetic status flags
undefined. The existing word form now also uses checked division for
`80000000h / -1`, which previously could panic in the host. Divide errors
report the instruction start and leave registers and FLAGS intact.
Tests exercise both widths, signed results and limits, zero divisors,
non-retirement on faults, an out-of-segment memory operand, and an
unprefixed word memory divide immediately after a prefixed dword divide.

Both authentic inputs and the overlay matched their DN-16 SHA-256 hashes
before and after the runs. After IDIV, both paths had `EAX=00005557`,
`EDX=0000004C`, and `FLAGS=F206`. The immediate next instruction was
`66 8B CA` (MOV ECX, EDX) at offset `4014h` and retired on both paths.

| Path | IDIV retirement | Next stop |
| --- | --- | --- |
| Direct DN.PRG | 3,749,459 → 3,749,460; `3C56:4011` → `3C56:4014` | At 3,809,844 retired, `CD 16` at `11CB:01CA` with `AX=1100` traps as unsupported BIOS `INT 16h/AH=11h`; CPU IP advances to `01CC`. |
| DN.COM launcher child | 3,755,857 → 3,755,858; `3CD4:4011` → `3CD4:4014` | At 3,909,219 retired, the child traps on `D4 0A` (AAM) at `3CD4:24C5`. The parent returns to Running at `1041:01F0`; its INT 2Fh vector is restored to `1041:0142`. |

The final text dumps show prompt/help and clock text, plus the DN title on
the launcher path. They do not establish that the file panels rendered.

See the [DN-17 audit directory](../audit/chronos-dn/dn-17/) for the host
traces, input hashes, and validation logs.

| DN-17 validation | Verdict |
| --- | --- |
| CPU implementation | **PASS**: both authentic host paths retired IDIV, and focused word and dword tests passed. |
| Regression suite | **PASS** for the requested unit and workload tests: 188 core passed / 1 ignored, including MIDTERM and Mines; 33 Chronos UI passed, including Mines interaction; 290 UI library tests passed. Core `no_std` and the native Chronos release build passed. The additional UI doctest run **failed** on the unchanged `horizon.rs` diagram, which rustdoc treats as Rust. |
| Platform boot gate | **PASS**: DN-16's two missing embedded binaries were still absent at entry. Built and staged `helios-note` and `wiseowl` with the normal build commands, then the default Phase 3.0 gate passed in QEMU. This gate does not exercise DN. |
| Native QEMU DN | **NOT_RUN**: the native adapter still caps imports at 65,536 bytes and does not select the diagnostic 386 profile. |
| DN panels | **UNCONFIRMED**. |

## DN-18: AAM and enhanced keyboard status

The unchanged 80386 real-mode host runs passed both DN-17 boundaries
and then diverged. No input was injected.

| Path | Boundary passed | Subsequent execution |
| --- | --- | --- |
| Direct DN.PRG | `INT 16h/AH=11h` at `11CB:01CA` retired at 3,809,845. AX stayed `1100`, FLAGS stayed `F246` (ZF set), and the queue was empty. | Next: `B8 00 00` at `11CB:01CC`. The program continued polling the empty queue and was still Running at the 10,000,000 instruction limit, `11CB:0036`. |
| DN.COM child | `D4 0A` at `3CD4:24C5` retired at 3,909,220. AX stayed `0000` and FLAGS changed from `F206` to `F246`. | Next: `D2 E4` (SHL AH, CL) at `3CD4:24C7`. At 3,913,219 retired the child stopped at `5F40:1337` on `INT 21h/AH=36h`, with AX=`3600`, DX=`4103`, FLAGS=`F206`. The launcher restored the parent at `1041:01F0` and its INT 2Fh vector at `1041:0142`. |

No shared later stop was observed. The direct text dump shows the DOS
prompt, clock, and help/menu labels; the launcher also shows the DN 1.51
title. Neither dump establishes that the file panels appeared.

See the [DN-18 audit](../audit/chronos-dn/dn-18/README.md) for the preserved
DN-17 traces, full register snapshots, new traces, input hashes, and
independent validation verdicts.

| DN-18 validation | Verdict |
| --- | --- |
| AAM CPU implementation | **PASS** independently in focused tests and the launcher child. |
| INT 16h/AH=11h BIOS implementation | **PASS** for Chronos's translated key queue in focused tests and the direct run. |
| Regression suite | **PASS**: core 191 passed / 1 ignored, Chronos UI 33 passed, UI library 290 passed; MIDTERM, Mines, core no_std, and native Chronos release build passed. The separate UI doctest still fails on the unchanged titlebar diagram. |
| Platform boot gate | **PASS**: Phase 3.0 in QEMU; embedded prerequisites were already available. |
| Native QEMU DN | **NOT_RUN**: the 65,536-byte import cap and absence of 386 profile selection still apply. |
| DN panels | **UNCONFIRMED**. |

## DN-19: DOS free disk space

The unchanged DN.COM child now passes INT 21h/AH=36h at
`5F40:1337`. It explicitly requests C: with DL=03h. Chronos returns
AX=FFFFh because the mounted file map has no allocation statistics.
BX=4382, CX=0000, DX=4103, and FLAGS=F206 are preserved. The call
retires at 3,913,220 and execution resumes at `5F40:1339` with a
comparison against FFFFh. The child repeats the query once, reaches
empty keyboard polling, and is still active at 10,000,000 instructions.

Direct DN.PRG again passes its original INT 16h/AH=11h boundary and
is still polling with no input at 10,000,000 instructions. It never
calls AH=36h during the run.

The [launcher capture](../audit/chronos-dn/dn-19/launcher.png) shows
two C: file panels in the final host guest text buffer. The
[direct capture](../audit/chronos-dn/dn-19/direct.png) shows no panels.
The launcher's “0 free bytes” label is guest behavior after receiving
the unavailable result, not a measurement. The captures are offline
renders of guest text cells; they do not establish native QEMU DN
behavior or interactive panel compatibility.

See the [DN-19 audit](../audit/chronos-dn/dn-19/README.md) for traces,
register state, and independent validation results.

## DN-20: DOS directory enumeration

DN-19's launcher already populated both C: panels through INT 21h
AH=4Eh/4Fh: two FindFirst calls and 38 FindNext calls succeeded,
and both searches ended with carry set and AX=0012h. No enumeration
blocker was observed. DN-20 fixes generic overlay filtering, classic filename
delivery, DTA bounds, and process search lifetime. The file map does
not supply timestamps or short aliases.

| Authentic 386 host path | Panel layout | Actual file entries |
| --- | --- | --- |
| Unchanged DN.COM launcher | Both panels visible | Real C: entries visible in both panels |
| Unchanged direct DN.PRG | No panels visible | None visible; no FindFirst/FindNext calls |

Both guests were still running at 10,000,000 retired instructions.
No unsupported DOS or CPU stop was observed; the launcher parent was
still suspended. See the [DN-20 audit](../audit/chronos-dn/dn-20/README.md)
for fresh screenshots, traces, register returns and regression results.
Native QEMU DN remains **NOT_RUN**.

The DN-20 verification rerun reproduced both host traces and captures
exactly, with all three input hashes unchanged.

## DN-21: Enhanced keyboard reads and panel startup

Both unchanged no-input runs reproduced DN-20. Controlled host input exposed
a separate problem: both guests saw the key through INT 16h/AH=11h, then
stopped at unsupported AH=10h. Chronos now uses its existing BIOS FIFO read
for AH=10h. F10 opens the menu on both paths; Down moves the launcher's
right-panel selection from DN.COM to FILES.BBS. Those responses reproduced
in guest traces and text captures. Both paths also exited cleanly after
the quit sequence, with normal parent recovery on the launcher path.

| Area | Host result |
| --- | --- |
| BIOS input and controlled guest response | **PASS** for the tested key cases |
| Launcher panel layout and directory enumeration | **PASS**: both panels show actual C: entries |
| Direct startup | **PARTIAL**: menu and exit work; no automatic panels or FindFirst/FindNext |
| Native QEMU DN | **NOT_RUN**: import cap still 65,536 bytes; required 386 profile is not selectable |

The panel divergence precedes any key input. DN.COM installs the guest
INT 2Fh handler that returns the DN signature and first-run flag. Direct
DN.PRG has no such handler and skips the startup path that creates default
panels. The keyboard change leaves that launcher contract intact.
See the [DN-21 audit](../audit/chronos-dn/dn-21/) for hashes, before/after
traces and captures, regression logs, and the Phase 3.0 platform gate,
which passed independently of DN.

## DN-22: Create panels from a direct launch

Direct DN.PRG starts without automatic panels on a fresh desktop. The
unchanged program can create both itself: press **F10**, choose
**Manager → New**, then accept C: in the drive picker. **Ctrl+F3** also
opens the picker. The host run enumerated C: and displayed actual entries
in both panels. Tab switched focus and Down moved the selection in the
newly focused panel. Both launcher and direct paths exited cleanly.

The launcher provides the first-run byte that enables automatic creation.
New Manager uses the same window constructor after drive selection, with
no dependency on the launcher. Saved desktop restoration is a separate
startup path. FreeDOS reproduced direct startup without panels, then
populated both panels through the menu and returned exit code 0. Chronos
implementation remained unchanged. See the [DN-22 audit](../audit/chronos-dn/dn-22/)
for source references, captures, and traces.

Native QEMU DN is **NOT_RUN**. The native adapter still needs to import
files above 65,536 bytes and select the 386 profile.

## DN-23: Desktop SunApp

The interaction and shutdown observations in this section are the DN-23
baseline. See DN-24 below for the current controls and exit procedure.

[DosNavigator.sunapp](../../DosNavigator.sunapp) includes the original 1.51
distribution, based on Dos Navigator by RIT Research Labs. Its original
redistribution notice is in `Program/README.TXT`.

Build a normal SunlightOS image with the repository build workflow
(`bash tools/build.sh`, or the Phase 3.0 gate for a test ISO). The boot image
installs the package at `/Applications/DosNavigator.sunapp` alongside Mines.
Log into the graphical desktop and choose **DOS Navigator 1.51** in Start
Menu → All apps, or search for **DOS Navigator** in the desktop search palette.
From a Sunlight terminal the equivalent command is
`sun-exec /Applications/DosNavigator.sunapp`.

Chronos starts the original `C:\DN.COM` from `C:\` with the existing 386
real-mode profile and no arguments. The complete `Program` directory is the C:
base. Writes use the app's private persistent overlay; no Documents, clipboard,
printing, serial, or network grant is requested.

For a manual pass, start from a fresh guest. Check for two file panels and real
entries such as `DN.COM` and `DN.OVR`. Press **F10**, **M**, **N** to open
**Manager → New**, then **C** to choose C:. Use **Tab** to change panels, arrow
keys to select a directory, and **keypad Enter** to open it. **Escape** dismisses
menus. Main Enter, Ctrl+F3 and Alt+X did not perform their DN actions in the
native desktop test; the menu mnemonics and keypad Enter are working alternatives.

Use **F10**, **F**, **X**, then **Y** to select **File → Exit** and confirm.
After Chronos displays **Exited with code 0**, press **Ctrl+W** to close
the window. The desktop reaps Chronos through its standard window-close
path. Launch a second time from the desktop to repeat the check.

Verify the checked-in bundle or reproduce a fresh copy from the same pinned
archive used for the compatibility tests:

```sh
python3 tools/dos_navigator_package.py --archive target/dos-navigator/dn151.zip
python3 tools/dos_navigator_package.py --archive target/dos-navigator/dn151.zip \
  --output target/dos-navigator/rebuilt/DosNavigator.sunapp
```

See the [DN-23 audit](../audit/chronos-dn/dn-23/README.md) for the integrity
inventory and separate host, platform, and desktop results.

## DN-24: Native controls and shutdown

On the Sunlight desktop, open **DOS Navigator 1.51** from search or All
apps. Both C: panels show the bundled files. Use **Tab** to switch panels
and the arrows to select an item. **Enter** and **keypad Enter** open the
selected directory. **F10** opens the menu; **Manager → New** (**M**, **N**)
or **Ctrl+F3** opens the drive picker. Press **C** to select C:, or
**Escape** to dismiss the picker or menu.

With the file panels active, **Alt+X** opens DN's quit confirmation.
You can also choose **File → Exit** from the menu. Press **Y** to
confirm. The packaged session flushes its private overlay and closes
Chronos automatically when the original DOS launcher finishes. The
desktop reaps Chronos after its ordinary native exit. **Ctrl+W** is
no longer part of the DN exit procedure. Other Chronos bundles retain
their existing exit behavior unless their manifest opts into closing
the window after the final DOS process exits.

The [DN-24 audit](../audit/chronos-dn/dn-24/) separates the native
keyboard and lifecycle evidence, and records the remaining limits.

## DN-25: Compatibility matrix

These statuses describe the tested packaged DOS Navigator 1.51 workflow
in Sunlight QEMU. The [DN-25 audit](../audit/chronos-dn/dn-25/README.md)
records the [216-case BIOS matrix](../audit/chronos-dn/dn-25/keyboard-matrix.csv)
separately from native application evidence. A host check alone cannot
upgrade a desktop result.

| Capability | Test method | Result | Evidence | Remaining limitation |
| --- | --- | --- | --- | --- |
| Startup and packaging | Verify the package and launch twice from desktop search | PASS | [Package](../audit/chronos-dn/dn-25/package.log), [launches](../audit/chronos-dn/dn-25/native/lifecycle.log) | Fresh diskless session; no installed-disk soak |
| Panel creation | Check launch, then F10 → Manager → New → C on both runs | PASS | [Both panels](../audit/chronos-dn/dn-25/native/dn-1-new-panels.png) | Saved layouts and other drives not covered |
| Directory enumeration | Inspect populated C: panels and open COLORS | PASS | [Opened directory](../audit/chronos-dn/dn-25/native/dn-1-directory-opened.png) | Bundled files only |
| Keyboard input overall | PS/2 input through the desktop; inspect DN BIOS reads and scanout | PARTIAL | [Native keyboard trace](../audit/chronos-dn/dn-25/native/keyboard.log) | Full modal and hardware parity untested |
| Dedicated E0 navigation | Send all ten keys; inspect enhanced words and panel selection | PARTIAL | [Page Down](../audit/chronos-dn/dn-25/native/dn-1-dedicated-pgdn.png), [Insert](../audit/chronos-dn/dn-25/native/dn-1-dedicated-insert.png), [Delete](../audit/chronos-dn/dn-25/native/dn-1-dedicated-delete.png) | Delete was consumed; file deletion was not verified |
| Keypad navigation | Keypad 8/2 with lock off; Shift+keypad 8 with lock on | PASS | [Lock off](../audit/chronos-dn/dn-25/native/dn-1-keypad-down-num-off.png), [Shift override](../audit/chronos-dn/dn-25/native/dn-1-shift-keypad-num-on.png) | Native test exercised representative navigation keys |
| Keypad digits and decimal | Toggle session Num Lock and enter all ten digits plus decimal | PASS | [DN command line](../audit/chronos-dn/dn-25/native/dn-1-keypad-command-line-num-on.png) | Command text only; no command executed |
| Keypad operators | Send keypad +, −, ×, ÷ and inspect enhanced reads and select dialog | PASS | [Select dialog](../audit/chronos-dn/dn-25/native/dn-1-keypad-arithmetic.png), [reads](../audit/chronos-dn/dn-25/native/keyboard.log) | DN interprets the operators in context; no claim about every dialog |
| Main and keypad Enter | Open COLORS with main Enter on run 1 and keypad Enter on run 2 | PASS | [Run 1](../audit/chronos-dn/dn-25/native/dn-1-directory-opened.png), [run 2](../audit/chronos-dn/dn-25/native/dn-2-directory-opened.png) | Unmodified app behavior; modified words checked separately |
| Shift, Ctrl, Alt and release | Shift keypad reversal, Ctrl+Up, Alt keypad suppression, then plain Up | PARTIAL | [Input and BIOS trace](../audit/chronos-dn/dn-25/native/keyboard.log) | One modifier at a time; no complete left/right or combined modifier coverage |
| Menu navigation | F10, Manager → New, Ctrl+F3, Escape | PASS | [F10](../audit/chronos-dn/dn-25/native/dn-1-f10.png), [Ctrl+F3](../audit/chronos-dn/dn-25/native/dn-1-ctrl-f3.png) | Other menus and modal contexts not exhausted |
| File selection | Tab, arrows, Insert; observe selected filename/count | PASS | [Selected file](../audit/chronos-dn/dn-25/native/dn-1-dedicated-insert.png) | No copy, move or confirmed deletion in this pass |
| DOS guest exit | Alt+X → Y in both launches | PASS | [Quit dialog](../audit/chronos-dn/dn-25/native/dn-1-alt-x.png), [code 0](../audit/chronos-dn/dn-25/native/lifecycle.log) | Tested from active file panels |
| Chronos session completion | Observe overlay flush, window close and native exit code 0 twice | PASS | [Lifecycle](../audit/chronos-dn/dn-25/native/lifecycle.log) | No persistence or resource leak soak |
| Desktop cleanup | Observe owner detach, process reap and restored desktop twice | PASS | [Run 1](../audit/chronos-dn/dn-25/native/dn-1-restored.png), [run 2](../audit/chronos-dn/dn-25/native/dn-2-restored.png) | Desktop Ctrl+W was unnecessary for DN |
| Native QEMU workflow | Run packaged DN and Mines from the desktop on the final ISO | PASS | [Inputs](../audit/chronos-dn/dn-25/native/inputs.jsonl), [desktop driver](../audit/chronos-dn/dn-25/verified-desktop.log) | OVMF, PS/2, two vCPUs, 4 GiB; physical hardware untested |
| Phase 3.0 platform boot | Run independent boot gate | PASS | [4 GiB result](../audit/chronos-dn/dn-25/platform.log) | Default 1 GiB attempt missed timezone after mapping failures; see audit |

Num Lock in these tests is local to the Chronos session and starts off.
With the lock on, Shift temporarily restores keypad navigation. The
guest input setting does not follow the host keyboard LED.

## DN-26 keyboard contract and dialog compatibility

See the [DN-26 audit](../audit/chronos-dn/dn-26/README.md) for the baseline,
implementation, and limits. Guest service tests here execute real-mode INT
instructions in the host Chronos runtime; they are separate from the authentic
SunApp desktop run.

| Capability | Test method | Result | Evidence | Remaining limitation |
| --- | --- | --- | --- | --- |
| BIOS modifier status | Execute INT 16h/AH=02h and 12h after physical modifier events; assert FLAGS and returned side bits | PASS | [Guest INT probe](../audit/chronos-dn/dn-26/focused-dos-bios.log) | Host guest probe; other BIOS locks, BDA mirroring and firmware actions remain untested |
| Left/right make and break | Overlap Shift/Ctrl/Alt sides, repeat a make, release one side, inject Print Screen edges and clear focus state; run the driver Pause test | PASS | [Guest INT probe](../audit/chronos-dn/dn-26/focused-dos-bios.log), [driver suite](../audit/chronos-dn/dn-26/host-tests.log) | Physical keyboard hardware and modifier flags for makes missed before focus were not measured |
| DOS byte-oriented reads | Execute INT 21h/AH=01h, 06h/DL=FFh, 07h, 08h, 0Bh and 0Ch with ASCII and extended events; check byte order, wait/poll, echo, flush and FLAGS | PASS | [Guest INT probe](../audit/chronos-dn/dn-26/focused-dos-bios.log) | Guest console, not redirected handle input; no assertion that DN uses these calls |
| DOS break handling and redirection | Check raw byte 03h through 06h/07h; inspect dispatch and handle lookup | PARTIAL | [Guest INT probe](../audit/chronos-dn/dn-26/focused-dos-bios.log), [scope](../audit/chronos-dn/dn-26/README.md) | No INT 23h delivery, Ctrl-Break or redirected character service coverage |
| DOS buffered input | Execute AH=0Ah with live input and prequeued input; verify BS, capacity, CR, and the following FIFO entry | PARTIAL | [Guest INT probe](../audit/chronos-dn/dn-26/focused-dos-bios.log), [live input test](../audit/chronos-dn/dn-26/host-tests.log) | Simple console line editing only; DOS template and extended editing, overflow bell and redirection remain unsupported |
| Select dialog text editing | From the COLORS panel send keypad +, edit `*.*` through empty, `abbc`, `abc`, `bc` and `zc`; Escape, reopen, then Enter | PASS | [Dialog trace and scanouts](../audit/chronos-dn/dn-26/README.md#authentic-dn-dialog-and-desktop-replay) | Enter selected 13 files; no file editor or saved edits tested |
| Select dialog Ctrl/Alt navigation | Send right Ctrl+Left and right Alt+Right; compare BIOS reads and scanout | PARTIAL | [Trace](../audit/chronos-dn/dn-26/native/keyboard.log), [scanout](../audit/chronos-dn/dn-26/native/dn-1-mask-chords.png) | Both chords reached DN; no separate visible Alt effect established |
| DN-25 keyboard and authentic desktop | Launch packaged DN twice in QEMU via desktop search, replay keypad/navigation and exit | PASS | [Desktop replay](../audit/chronos-dn/dn-26/desktop.log), [keyboard trace](../audit/chronos-dn/dn-26/native/keyboard.log) | 4 GiB, two vCPUs, QMP to PS/2; native traces establish enhanced BIOS reads, not DOS byte services |
| Natural DN and Chronos exit | Confirm guest code 0, native code 0, window cleanup and reaping twice | PASS | [Lifecycle](../audit/chronos-dn/dn-26/native/lifecycle.log) | Two launches with the existing opt-in completion policy |

## DN-27: Shift selection in the file mask

The packaged DOS Navigator reads the Shift bits directly from guest BIOS
Data Area byte `0040:0017` while handling the arrow keys in **Select → File
mask**. It uses enhanced INT 16h to obtain the arrow events. The
[DN-27 audit](../audit/chronos-dn/dn-27/README.md) records the executing
instruction, the stale baseline byte and the visible dialog checks.

Chronos mirrors its modeled right/left Shift, combined Ctrl/Alt and Num Lock
bits into that guest byte when processing keyboard and focus state. No DOS
Navigator package files changed. The other BDA bits and the BIOS keyboard
service interface retain their behavior.

In the 4 GiB QEMU desktop, either Shift key with Left/Right extends or
contracts the visible selection around its original anchor. Releasing
Shift ends extension; the next plain arrow clears the highlight. See the
[audit screenshots and scanout checks](../audit/chronos-dn/dn-27/README.md#authentic-sunapp-result).

## DN-28: Focus and modifier recovery

Chronos clears its guest-visible transient modifier bits when its window loses
focus. The keyboard service continues tracking physical make and break edges;
focus regain does not reconstruct a Shift side from an aggregate modifier
snapshot. A fresh Shift make restores that side, and the BIOS flags and modeled
BDA bits use the same state. The [DN-28 audit](../audit/chronos-dn/dn-28/README.md)
separates this policy and host regression evidence from actual desktop results.

In the DN-28 QEMU replay, left Shift, right Shift and both Shift keys were
held across a real desktop focus cycle. The first Right after return cleared
the selection, and a fresh Shift cycle selected again. Native direct BIOS/BDA
readback and a break lost to another window remain separate open checks; see
the [evidence and limitations](../audit/chronos-dn/dn-28/README.md#native-qemu-focus-replay).

## DN-29: Overlapping Shift keys

In the stock QEMU desktop replay, either Shift key can be released first
in Select → File mask. The remaining key continues extending selection;
after both releases, a plain arrow clears it. A fresh Shift make extends
selection again. A separate native real-mode probe verified that both INT
16h modifier queries agree with the modeled bits at `0040:0017`, preserve
the expected AX and FLAGS behavior, and show no stale Shift after focus
loss or regain. The PS/2 trace still shows the remaining physical side held
between the delayed breaks. See the [DN-29 audit](../audit/chronos-dn/dn-29/README.md)
for keyboard traces, native readbacks, scanout, and package hashes.
