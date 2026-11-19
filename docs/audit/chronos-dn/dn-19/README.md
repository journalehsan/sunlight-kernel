# DN-19 audit

## Diagnosis

DN-18's [launcher trace](../dn-18/launcher-rerun.log) stops before
retiring `CD 21` at `5F40:1337` (3,913,219 retired). The saved
entry matches DN-19:

```text
AX=3600 BX=4382 CX=0000 DX=4103 SI=43B9 DI=0008 BP=3F54 SP=3F54
CS:IP=5F40:1337 DS=4195 ES=7225 SS=4C91 FLAGS=F206
upper halves (AX CX DX BX SP BP SI DI): 0000 0000 004C 0000 0000 0000 0000 0000
```

DL=03h explicitly selects C:, which also happens to be current. The
host probe imports sibling files into C:'s base map, with guest writes
private to the overlay. D: and T: are also in the default drive table.
The native adapter populates C: from program files, dependencies and its
overlay; D: can import documents, and T: provides temporary storage.
Neither adapter supplies disk geometry or capacity. Chronos tracks DOS
paths, byte vectors, attributes, directories and deletions; file lengths
and import limits do not tell it which clusters exist or are free.

The pre-change dispatcher had no AH=36h, 1Bh or 1Ch. AH=0Eh selects a
mapped drive and returns the mounted count; AH=19h returns the current
zero-based drive number; AH=44h/AL=00h reports handle type. None exposes
allocation statistics. Unmapped drive numbers and existing mappings
without a current capacity snapshot now both return AX=FFFFh for AH=36h.

With supplied statistics, AH=36h returns the four counts directly after
checked 16-bit conversions. Zero geometry, the reserved AX=FFFFh value,
and free counts above total are rejected. There is no multiplication,
truncation or inferred geometry. Local map mutations invalidate capacity
snapshots.

## Authentic runs

Both runs used the same original executables and overlay. See the
[before](inputs-before.log) and [after](inputs-after.log) SHA-256 checks.
The compressed [direct](direct.log.gz) and [launcher](launcher.log.gz)
logs are full host traces. Both probe exit codes were 2 because the
guests were still Running at the budget; the probe returns zero only
for a clean guest exit.

- Direct: DN-18's initial INT 16h/AH=11h again retired at 3,809,845
  (AX=1100, FLAGS=F246, empty queue). The next instruction at
  `11CB:01CC` was `B8 00 00`. No AH=36h was reached; there are no
  disk-space return registers. The guest kept polling and ended Running
  at 10,000,000, `11CB:0036`, PSP=1041, INT 2Fh=`0800:0400`.
- Launcher: AAM again retired at 3,909,220. The two AH=36h calls retired
  at 3,913,220 and 4,014,186. Both returned AX=FFFFh, BX=4382,
  CX=0000, DX=4103, FLAGS=F206. The first resumed at `5F40:1339`
  with `3D FF FF` (CMP AX,FFFFh). Empty enhanced keyboard polling was
  observed at 6,600,816 (`1249:01CA`), and the child stayed Running
  through 10,000,000 (`14C1:2E2C`, PSP=10BF). The parent did not
  resume because the child neither stopped nor trapped. INT 2Fh still
  pointed at the parent's `1041:0142`; parent recovery was not triggered.
  The DN-18 baseline separately shows the old trap restoring parent
  execution at `1041:01F0` and the same INT 2Fh vector.

The final [direct](direct.png) and [launcher](launcher.png) images use
the actual 80×25 guest cells and Chronos CP437 BIOS font, with an EGA
palette and blink visible. The launcher image shows two file panels;
the direct image shows a prompt, clock and menu/help labels. These are
host diagnostic images, not captures of the native UI.
The launcher's “0 free bytes” is text the guest draws after the
unavailable result. Chronos did not report zero free clusters.

## Reproduction

```sh
cargo build -p chronos-core --example dos_probe --target x86_64-unknown-linux-gnu
probe=target/x86_64-unknown-linux-gnu/debug/examples/dos_probe
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3809844 CHRONOS_TEXT_CAPTURE=direct.ppm \
  "$probe" target/dos-navigator/dn151/DN.PRG > direct.log
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3913219 CHRONOS_TEXT_CAPTURE=launcher.ppm \
  "$probe" target/dos-navigator/dn151/DN.COM > launcher.log
sha256sum -c docs/audit/chronos-dn/dn-19/inputs.sha256

cargo test -p chronos-core -p sunlight-chronos -p sunlight-ui \
  --target x86_64-unknown-linux-gnu --lib --bins
cargo test -p sunlight-ui --target x86_64-unknown-linux-gnu --doc
cargo check -p chronos-core --target x86_64-unknown-none
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-chronos --release --target x86_64-unknown-none
bash tools/test.sh phase3.0
```

## Verdicts

| Check | Verdict |
| --- | --- |
| DOS AH=36h | **PASS** for drive selection, supplied capacity, unavailable capacity, register width checks, and register/FLAGS preservation. Current adapters supply no allocation statistics. |
| Regression suite | **PASS**: 193 core passed / 1 ignored; 33 Chronos UI passed; 290 UI library passed. Includes DOS shell MIDTERM and Mines load, first frame, and interaction checks. Core no_std check and native Chronos release build passed. |
| UI doctest | **FAIL**: the unchanged diagram in `sunlight-ui/horizon.rs` is parsed as Rust and errors at the pipe on line 6. |
| Phase 3.0 QEMU platform boot gate | **PASS**; this gate does not exercise DN. |
| Native QEMU DN | **NOT_RUN**: the native adapter still limits imports to 65,536 bytes and does not select the required 386 profile. |
| Direct DN.PRG | **PASS** for reproducing the expected polling state through the host budget. AH=36h was not reached. |
| DN.COM child | **PASS** for passing the DN-18 boundary and continuing through the host budget. Parent recovery was not triggered; INT 2Fh remained at the parent's handler. |
| DN panels | **OBSERVED** in the host launcher capture; absent in the final direct capture. Native and interactive DN panel behavior remain unverified. |
