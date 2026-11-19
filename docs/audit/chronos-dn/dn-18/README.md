# DN-18 audit

The saved `dn-17-*-original.log` files are byte-identical to the DN-17
traces. DN-18 changes were already present in the worktree when reviewed.

## Boundary diagnosis

Direct DN.PRG reached `CD 16` at `11CB:01CA`, retired count 3,809,844.
The original trace records AX=1100, BX=0000, CX=0000, DX=0000, SI=4F09,
DI=4DE0, BP=SP=4D98, DS=4117, ES=SS=4C13, FLAGS=F246 (ZF=1).
The upper halves in AX/CX/DX/BX/SP/BP/SI/DI order were
`0000 0000 004C 0000 0000 0000 0000 0000`. The new trace confirms
zero queued keys at the same entry state. The earlier dispatcher handled
00h (read), 01h (peek), and 02h (shift flags); it rejected function 11h
after fetching the interrupt number. No key is injected by the host probe.
The input queue stores translated scan code and character pairs, including
ASCII-zero special keys. There is no enhanced translation/filtering or
separate queue. Function 11h uses the existing peek, preserves AX when
empty, and changes only ZF; any returned AX comes from the queue head.

The DN.COM child reached `D4 0A` at `3CD4:24C5`, retired count 3,909,219,
AH=AL=00 and FLAGS=F206. The original `UnsupportedOpcode` recorded CPU IP
24C6 (one byte into the instruction), confirming that byte dispatch rejected
D4 before reading the immediate. In the 80386 real-mode profile the default
operand size is 16 bits; this instruction operates on AL and AH regardless.
The normal dispatcher now consumes 0Ah as the base and leaves IP at 24C7.
Base zero uses the existing divide trap at the instruction start. The test
matrix covers alternate bases, prefix handling, result and FLAGS preservation,
and both clear and set SF outcomes.

## Reproduction

Run from the repository root. The host probe imports the original files
into a private in-memory drive; the checksum manifests cover both
executables and the overlay.

```sh
cargo build -p chronos-core --example dos_probe --target x86_64-unknown-linux-gnu
probe=target/x86_64-unknown-linux-gnu/debug/examples/dos_probe
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3809844 "$probe" target/dos-navigator/dn151/DN.PRG
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3909219 "$probe" target/dos-navigator/dn151/DN.COM
sha256sum -c docs/audit/chronos-dn/dn-18/inputs.sha256

cargo test -p chronos-core -p sunlight-chronos -p sunlight-ui \
  --target x86_64-unknown-linux-gnu --lib --bins
cargo test -p sunlight-ui --target x86_64-unknown-linux-gnu --doc
cargo check -p chronos-core --target x86_64-unknown-none
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-chronos --release --target x86_64-unknown-none
bash tools/test.sh
```

## Results

The fresh [direct](direct-rerun.log) and [launcher](launcher-rerun.log)
traces reproduce the boundary and stop states recorded in the earlier
DN-18 captures. The direct probe exits 2 at its instruction budget; the
launcher probe exits 2 because it recorded a recovered child trap. Neither
exit code indicates a failure at the original DN-17 boundary. The
before/after checksum logs match for DN.PRG, DN.COM, and DN.OVR.

| Independent check | Verdict |
| --- | --- |
| AAM CPU implementation | **PASS**. Focused tests and authentic launcher retirement from 3,909,219 to 3,909,220. |
| INT 16h/AH=11h BIOS implementation | **PASS** for the modeled queue. Focused empty and queued checks, including repeated peeks and flag preservation; authentic direct retirement from 3,809,844 to 3,809,845. |
| Regression suite | **PASS** for the requested unit tests and workloads: core 191 passed / 1 ignored, including DOS shell MIDTERM and Mines; Chronos UI 33 passed, including Mines interaction; UI library 290 passed. Core no_std and the native Chronos release build passed. |
| UI doctest | **FAIL**, separately: the unchanged indented titlebar diagram in `sunlight-ui/horizon.rs` is parsed as Rust; rustdoc rejects the pipe on line 6. |
| Platform Phase 3.0 QEMU boot gate | **PASS**. The `helios-note` and `wiseowl` artifacts were present before the gate; no separate prerequisite rebuild was needed. |
| Native QEMU DN | **NOT_RUN**. The adapter still caps host files at 65,536 bytes and uses the default CPU profile. The platform gate does not run DN. |
| DN panels | **UNCONFIRMED**. The host text dumps do not show file panels. |
