# DN-20: DOS directory enumeration

The [DN-19 launcher screenshot](../dn-19/launcher.png) already shows
both C: panels populated with real files, including DN.COM (1,778 bytes),
DN.PRG, FILES.BBS, and README.TXT. The
[DN-19 launcher trace](../dn-19/launcher.log.gz) has two successful
FindFirst calls and 38 successful FindNext calls. Both searches end
with CF set and AX=0012h. The first successful FindFirst was at
`39E6:0162`, retired count 3,822,559 before the INT; the first
successful FindNext was at `39E6:017F`, 3,824,323 before the INT.
These calls precede DN-19's disk-space query. The three earlier
FindFirst calls return the normal no-match status. There was no
observed enumeration blocker, path failure in the panel searches, or
later unsupported instruction or DOS service stopping that run.

The dispatcher already handled AH=1Ah/2Fh (set/get DTA), AH=4Eh/4Fh
(search), drive selection, CHDIR and get-current-directory. DOS paths
are normalized against the selected drive and its current directory;
the map resolves overlay entries and tombstones before exposing bytes
to file operations. The host probe imports sibling regular files into
C:; it does not traverse host folders on guest request. The native
adapter can populate C:, D: and T:, but has different import limits.

DN-20 fixes gaps found in the generic search implementation:

- Enumeration uses the effective overlay entry before applying the
  attribute mask. A base file hidden by an overlay attribute update
  no longer leaks back into a search that excludes hidden files.
- Classic records omit names without a representable 8.3 name. The
  map has no short aliases, so truncation would advertise a name the
  guest could not open. Sizes and attributes come from the visible
  entry. Date and time are zero because the map has no timestamps;
  the prior date field contained a fixed January 1, 1980 value.
- A search writes one 43-byte DTA record after checking that it fits
  in the segment. A rejected FindNext writes nothing and leaves
  its cursor available for retry. Guest physical addresses still
  follow Chronos's existing real-mode memory model.
- Each DOS process has one cursor. A new FindFirst replaces it even
  on failure; exhaustion and exit discard it. EXEC suspends the
  parent's cursor, starts the child with an empty cursor, and
  restores the parent when the child exits. Switching DTAs within
  one process does not create independent cursors; the reserved
  DTA bytes do not contain a portable search token.

The existing case-insensitive wildcard matcher supports `*`, `?` and
`*.*`. It matches against the displayed name and is not a complete
DOS padded-field matcher. The 512-entry cap remains. The map lacks
timestamps, volume labels, and generated short aliases; the host
probe also lacks host subdirectory imports. No missing files,
directories, timestamps or disk-space values were invented.

## Authentic host runs

The probe used the 80386 real-mode profile, no key injection, and
the original DN programs and overlay; the [before](inputs-before.log)
and [after](inputs-after.log) hashes match. See the
[host import inventory](import-inventory.log) and
[register excerpts](interrupt-summary.log) for traceable details.
Counts in the table are **after** the instruction retired.

| Path or event | Retired | Register and state evidence |
| --- | ---: | --- |
| Launcher first unmatched FindFirst | 3,475,838 | AX=0012, FLAGS=F047 |
| Launcher first successful FindFirst | 3,822,560 | AX=4E00, BX=3F10, CX=00F7, DX=41D0, FLAGS=F246 |
| Launcher first successful FindNext | 3,824,281 | AX=4F00, BX=0000, CX=0000, DX=433E, FLAGS=F246 |
| Launcher first and second exhausted FindNext | 3,899,711 / 4,005,487 | AX=0012, BX=0022, CX=0000, DX=433E, FLAGS=F247 |
| Launcher C: disk-space queries | 3,912,360 / 4,012,466 | AX=FFFF, BX=4382, CX=0000, DX=4103, FLAGS=F206; capacity unavailable |
| Launcher first observed enhanced keyboard poll | 4,463,619 | AX=1100, FLAGS=F246, empty queue |
| Direct first enhanced keyboard poll | 3,809,845 | AX=1100, FLAGS=F246, empty queue |

- **DN.COM:** [Screenshot](launcher.png) verifies both panel layouts
  and real entries in both. The [trace](launcher.log.gz) records the
  same 2 successful FindFirst and 38 successful FindNext calls as
  DN-19, plus 2 end-of-search returns. DN renders a date/time even
  from unset DTA fields; that display is not file metadata. EXEC
  entered the child from PSP=1041; at 10,000,000 retired, PSP=10BF,
  CS:IP=29EF:0431, state Running. The parent remained suspended
  and INT 2Fh remained at its handler, 1041:0142.
- **DN.PRG:** [Screenshot](direct.png) shows the C: prompt, clock and
  F10 menu label; there are no panels or visible file entries.
  The [trace](direct.log.gz) has no AH=4Eh/4Fh. At 10,000,000 retired,
  PSP=1041, CS:IP=11CB:0036, state Running, INT 2Fh=0800:0400.
- No unsupported DOS/CPU stop or recovered child trap was observed
  within either budget. Probe exit status 2 means the guest was still
  Running at the instruction limit. These screenshots render the
  actual final guest cells and font offline; interactive and native
  panel behavior were outside these runs.

## Reproduction and regression

```sh
cargo build -p chronos-core --example dos_probe --target x86_64-unknown-linux-gnu
probe=target/x86_64-unknown-linux-gnu/debug/examples/dos_probe
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3809844 CHRONOS_TEXT_CAPTURE=direct.ppm \
  "$probe" target/dos-navigator/dn151/DN.PRG > direct.log
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3822559 CHRONOS_TEXT_CAPTURE=launcher.ppm \
  "$probe" target/dos-navigator/dn151/DN.COM > launcher.log
sha256sum -c docs/audit/chronos-dn/dn-20/inputs.sha256

cargo test -p chronos-core -p sunlight-chronos -p sunlight-ui \
  --target x86_64-unknown-linux-gnu --lib --bins
cargo test -p sunlight-ui --target x86_64-unknown-linux-gnu --doc
cargo check -p chronos-core --target x86_64-unknown-none
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-chronos --release --target x86_64-unknown-none
bash tools/test.sh phase3.0
```

[Host suites](units.log) passed, with the same counts on [recheck](units-recheck.log): 196 Chronos core (1 ignored), 33 Chronos
UI, and 290 UI library tests. This includes the bundled MIDTERM batch
and Mines load, first frame, interaction and exit. The focused DOS
tests cover wildcard matching, no match, overlay attributes, hidden,
system and directory masks, continuation, exhaustion, invalid drives
and paths, DTA bounds, and parent and child search lifetime.
[Core no_std](no-std.log) and [native release](native.log) passed.

[UI doctest](doctest.log) separately fails on the unchanged
`horizon.rs` diagram parsed as Rust. Native QEMU DN is **NOT_RUN**:
the adapter still caps each file import at 65,536 bytes and does not
select the required 386 profile.

The [Phase 3.0 QEMU platform gate](platform.log) **passed**. It boots
the platform and does not run DOS Navigator.

A fresh check of the existing DN-20 source reproduced both authentic
386 host runs. The [direct trace](verification-direct.log.gz) and
[launcher trace](verification-launcher.log.gz) match their saved
counterparts byte for byte after decompression. The
[direct capture](verification-direct.png) and
[launcher capture](verification-launcher.png) match the earlier guest
images [pixel for pixel](verification-comparison.log). The three
original inputs still match their SHA-256 pins. Host suites, core
no_std and the native release build passed again; the unrelated
doctest reproduced its failure. See the
[rerun status](verification-status.log).

The [Phase 3.0 platform rerun](verification-platform.log) also passed.
It did not exercise native DN.
