# BTop-PROC-01: process enumeration replay

The retained BTop-22 CD image reproduced the one-row dashboard. In this
boot the collector also received exactly one numeric proc directory entry.
There was an independent transient failure opening that leader's stat file.
This is a baseline replay; a controlled comparison with several eligible
leaders and a fix have not been completed.

## Replay and artifacts

```sh
git -C target/btop-01/src rev-parse HEAD
sha256sum target/btop-01/btop Btop.sunapp/Program/btop
python3 tools/btop-06/guest.py target/btop-22/final/guest-ramfs-trace-config.iso target/btop-proc-01-replay --btop
cargo test -p sunlight-telemetry --target x86_64-unknown-linux-gnu thread_groups_sum_cpu_once_and_share_memory
```

The guest used BIOS/TCG, four vCPUs and a 138×38 TTY. The replay
needed permission to bind its local control socket. The image recipe
embeds `target/btop-01/btop`, v1.4.5 at
`a1099d075fa914452366b28c67ff171e21a1f02e`. That recipe input matched the
required digest. A byte search of the ISO found the complete 2,754,744-byte
payload at decimal offset 33,436,780. Hashing that ISO byte range matched
the required digest. The separate SunApp payload has a
different digest and was not used.

| Artifact | SHA-256 |
|---|---|
| Pinned recipe input | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Separate SunApp payload | `6632acc21bb187f692df3fe09c13a36ce97fe5775698bcc937342603801cce29` |
| Replay ISO | `e7575a42b56655c41b578157b8cf4fa9c2e575dd25b8bffff5d90a8bb79eb148` |
| Replay serial.log | `a5817a75152031ff0aef988b673e46e954dd1d5f33ca99471b7203edfe66aaa4` |
| Replay dashboard-2.png | `51acc651208ba79d3f22f8d66ad5280552c4a051c19ae4680a03ee832e95c017` |

## Registry, directory, and dashboard

- **Registry:** serial records native shell PID 37, btop leader PID 38
  with parent 37, and Linux clone TID 39 with TGID 38. The replay does not
  capture a complete live registry with publication incarnations, or run
  native top in a second view. Native top reads the shared telemetry page
  through `sunlight-telemetry`. It starts with up to 64 scheduler task
  samples and aggregates Linux thread groups after deriving runtime
  deltas. That projection is neither the full scheduler table nor the
  procfs visibility policy. Scheduler slots can also represent idle tasks
  and kernel workers, which are not eligible Linux userspace leaders.
- **Directory:** `serial.log` trace sequences 43–49 show the actual
  btop worker opening `/proc`, getting 208 bytes then EOF, and opening
  `/proc/38/{comm,cmdline,status,stat}`. In the existing directory layout
  the seven fixed entries consume 184 bytes (five 24-byte and two 32-byte
  records); the remaining 24 bytes account for one PID. The reader's paths
  identify it as 38. This is inferred from record sizes and opens; the
  trace does not dump dirent bytes. The stat open returned `-11`
  (`EAGAIN`); sequence 79 and later refreshes opened it successfully.
  The initial error is consistent with the RSS sample being unavailable
  for a new identity, but the trace does not distinguish absent and stale
  RSS. There was no controlled direct lookup of 37 or 39 from this worker.
- **Dashboard:** the final dashboard shows PID 38, `btop`, `/bin/btop`,
  two threads, and RSS around 5.4 MiB. There is no separate row for 39.
  The 38-line terminal has room for the one enumerated PID. The fixture
  config did not enable tree mode or set a filter, and the runner did not
  enter a process filter. It did not explicitly toggle tree mode or
  clear the filter through the UI. CPU and RSS were not compared with a
  matched authoritative sample.

## Source boundary

`kernel/src/linux_procfs.rs` uses the scheduler registry directly.
`visible` requires a live Linux leader with matching UID and either the
caller's TGID or the existing foreground authorization. The root directory
and numeric lookup use that same predicate. Native shell 37 is outside
the documented BTop-02 view even though it has a kernel PID and is the
actual parent. In procfs the parent field falls back to 0 if its PID
is not visible. Linux getpid uses the existing TGID/PID, and workers
already have their own TIDs; there is no separate procfs allocator.
A native kernel PID alone is not authorization to publish a native row.
Native visibility needs a policy that positively identifies eligible
native userspace leaders and preserves the foreground isolation.

The pinned reader in `src/linux/btop_collect.cpp:2860` iterates numeric
paths, reads comm, NUL-delimited cmdline and Uid from status on first
discovery, then reads stat on refresh. It parses state, parent, combined
CPU, nice, threads, start time and RSS. It uses statm only if stat RSS
looks too large; task traversal is not needed for the normal list.
Current stat uses 100 Hz for aggregate CPU and boot-relative start,
and 4 KiB pages for RSS. RSS comes from the native present-mapping
walk; the existing publication only gives procfs its first 64 task
rows, even though telemetry walks more address spaces. That is an
additional source-level limit; this replay did not exercise it.

Root descriptions retain bounded PID/incarnation/address-space identities.
Directory iteration skips stale identities and per-process read rechecks
the identity before returning the saved bytes. This was inspected in
source. No exit-between-operations or PID reuse test was run here.
Per-process stat formatting currently happens under the scheduler lock.
No procfs code, ABI fields, or access rules changed in this pass.

| Area | Outcome |
|---|---|
| Enumeration | **PASS** for baseline `{38}` from the worker trace; multi-leader before/after and create/reap membership **NOT RUN**. |
| Native process visibility | **BLOCKED** by the existing native exclusion; policy expansion and paired native top capture **NOT RUN**. |
| Per-PID data | **FAIL** on first stat open (EAGAIN); later opens **PASS**. Full contents and matched counter validation **NOT RUN**. |
| Lifecycle races | **NOT RUN** for exit between enumeration and lookup/read, and for an old handle after PID reuse. |
| btop acceptance | **PASS** for Help open/return, explicit q exit 0, worker 39 and leader 38 reaped, and separate proc-native exit 0 on the same TTY. Full process-list acceptance **NOT RUN**. |
| Host check | **PASS** for telemetry thread aggregation; that test does not exercise procfs or btop. |

There is no post-change PID set. To distinguish policy exclusions from
missing rows in a larger view, the next fixture needs concurrent eligible
Linux leaders and a worker, and a bounded registry and dirent capture
from the collector scope. Separate foreground commands cannot be assumed
to share that scope. Compare the visible set and stat payloads against
the dashboard before changing policy or the RSS producer.

## Concurrent leader continuation

The original one-row run above is unchanged. The first continuation
tried the original ISO with a disposable FAT image. The volume mounted
at `/mnt` and the host FAT directory contained `RUN.ELF`; guest shell
path resolution returned ENOENT and the runner timed out waiting for
btop spawn. That attempt did not test procfs. Its serial and command
records are in `target/btop-proc-01-multi/run/`.

The fallback packages the fixtures in an independent copy of the
archived ISO. `image.py` checks the source ISO hash and replaces the two
occurrences of each selected test ELF with the new ELF padded to the
original length. It checks that the complete pinned btop byte sequence
remains at the original offset. No kernel source, permissions, config,
or pinned btop bytes changed. The first packaged run under
`target/btop-proc-01-multi/fixture/` showed four concurrent rows but
the fixture worker exited early and the native launcher treated a
parked wait's retry return as a result. That run is not used for
interactive or lifecycle acceptance. The second fixture keeps the
worker alive until the leader stops it and retries parked child waits.

```sh
out=target/btop-proc-01-multi
python3 tools/native-c/cc.py -O2 -Wall -Wextra -Werror tools/btop-proc-01/launch.c -o "$out/launcher-v2.elf"
musl-gcc -static -no-pie -pthread -O2 -Wall -Wextra -Werror tools/btop-proc-01/leader.c -o "$out/leader-v2.elf"
bash tools/stamp_helios_elf.sh "$out/leader-v2.elf"
python3 tools/btop-proc-01/image.py "$out/launcher-v2.elf" "$out/leader-v2.elf" "$out/fixture-v2.iso"
python3 tools/btop-proc-01/replay.py "$out/fixture-v2.iso" "$out/fixture-v2" --btop
```

| Artifact | SHA-256 |
|---|---|
| Fixture ISO v2 | `fc8940b73f06377f908e77da95c3832ea11b785fb6170a8dc20ba3ae169770ec` |
| Launcher v2 | `fd5cecc253c68d648865f4fad546d5b95c2aea99f012962fa6fe4f75e4d4f59b` |
| Leader v2 | `746e085f4858421de273110b07580277f52c754fd8b32866d6ec7e418024e34f` |
| Unchanged ISO btop bytes | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |

The native launcher starts all fixtures and btop as sibling processes
within a single shell foreground scope. The threaded fixture reports
its UID, enumerates numeric entries and logs per-PID file contents
from that scope. Separately the archived syscall trace captures
btop's actual opens and getdents byte counts. A fixture read is not
reported as a btop read. The native shell is a negative control.

| Window | Live eligible leaders (guest spawn and reap records) | Same-scope numeric entries | btop screenshot rows |
|---|---|---|---|
| Stable, guest uptime ~35 s | 39, 40, 41, 42 | 39, 40, 41, 42 (fixture elapsed 4 and 8 s) | 42, 40, 41, 39 |
| After launch, uptime ~53 s | 39, 40, 41, 42, 45 | 39, 40, 41, 42, 45 (elapsed 24 s) | 42, 40, 45, 41, 39 |
| After reap, uptime ~74 s | 40, 41, 42, 45 | 40, 41, 42, 45 (elapsed 44 s) | 42, 40, 45, 41 |

PIDs 39, 40, 41 and 45 were `masked-select`,
`masked-select-original`, `btop-reference` and `route-scope`.
Their numeric identities came from native spawn and the matching Linux
PID/TGID fields in status. btop worker 44 returned 280 bytes for
`getdents64` at sequence 69 (184 fixed bytes plus four 24-byte numeric
records). At sequences 70–85 it opened those four PIDs' comm, cmdline,
status and stat. At 794–797 it opened all four files for PID 45; directory
returns in that window grew to 304 bytes. The 1,024-event trace cap
precedes the post-reap window, so the removal set is corroborated by
the sibling enumeration and screenshot rather than by a post-reap
btop syscall trace.

The fresh config leaves tree mode off and the filter empty; the runner
also sends Delete. The screenshots show flat PID/name rows with room
below the last row. Manual image inspection accounts for each PID once,
including PID 40 where OCR misreads the truncated name. Native shell
37 and native launcher 38 never appear; `/proc/37/stat` returns
ENOENT in the sibling view. Clone records bind TID 43 to TGID 40 and
TID 44 to TGID 42. Neither worker has a row. For example, the 4 s
sample for PID 40 contains:

```text
[HELIOS-THREAD] clone tgid=40 tid=43 tls=0x1000022f38 stack=0x1000022ef8
[PROC-FIX] sample=4 reader=40 uid=0
[PROC-FIX] reader=40 path=/proc/40/stat bytes=40 (masked-select-original) R 0 0 0 0 0 0 0 0 0 0 0 19 0 0 20 0 2 0 2883 0 567 0 ...
```

Both the stat thread field and btop show two threads for PID 40.
RSS 567 pages at 4 KiB/page agrees with its displayed 2.2 MiB;
the 19-to-23 tick increase from 4 to 8 s measures aggregate CPU time,
not a matched comparison with btop's CPU percentage.

**Transient stat open:** The first packaged run records btop
`EAGAIN` on `/proc/{39,40,41,42}/stat` at sequences 76, 80, 84
and 88, and on `/proc/45/stat` at 832. These leaders were live.
The initial leaders opened successfully on later refreshes; the
same-scope probe also later read PID 45 successfully, but btop recovery
for 45 in that run was not established. In v2 none of the captured
btop per-PID stat opens or fixture samples returned EAGAIN, including
the first open for 45. This does not diagnose the earlier EAGAIN.
PID 39's removal follows the explicit exit/reap record; no failed open
against a live PID is used as evidence of removal.

| Area | Fixture v2 outcome |
|---|---|
| Multi-leader enumeration | **PASS** for the tested scope; four initial eligible leaders and five after launch. |
| btop display | **PASS** for the three PID sets, with all rows in view. |
| Thread aggregation | **PASS**: TGID 40 has two threads and one row; neither worker is listed. |
| Appearance and removal | **PASS**: 45 shown after creation; 39 absent after reap, before btop exits. Runner allows five host seconds after spawn and four after reap before screen capture, plus the capture delay. |
| PID reuse | **NOT RUN**: no supported deterministic reuse setup with a surviving observer fd was identified. Ordinary slot reuse is insufficient. |
| Native visibility | **PASS** for shell exclusion under current policy; native inclusion remains outside it. |
| Interactive acceptance | **PASS** on fixture ISO v2: Help open/return, explicit q, native parent observes btop exit 0, btop worker 44 and leader 42 reap, launcher exits 0 and separate `/bin/proc-native` exits 0 on the same TTY. |

Raw evidence is under `target/btop-proc-01-multi/fixture-v2/`.

| Evidence | SHA-256 |
|---|---|
| serial.log | `9159bddb739c3e7c00290f0b6e0e235fc6c34a25e16db78c92a96002a01e8a2a` |
| stable.png | `42e7596b3a37d006ae8e2a1261385eb7efa581b1d531858d756585d22f6a7189` |
| appeared.png | `dff4cff661b794e69e371a4ea99706291e23b3db97aace150abf1c09c86fdff1` |
| removed.png | `8af6e65d39e463be909d65df4cc73121aa8f38c10f3b26171e2edf413d2e0c20` |
