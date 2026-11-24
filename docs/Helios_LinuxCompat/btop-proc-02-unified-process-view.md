# BTop-PROC-02: unified process view

## Visibility policy

The scheduler registry is authoritative. Keep the PROC-01 LinuxCompat
discovery rule. For native userspace leaders, require the same guest UID
and a positively shared, nonzero foreground execution scope. The legacy
built-in TTY launch has no such scope. There, require two unscoped tasks
on the same inherited TTY attachment and a live, generation-checked
family relationship: ancestor or sibling. A protected session owner
must be an ancestor; a shared TTY tab alone never authorizes inspection.
Unrelated unscoped services and other foreground scopes are not made
public. Guest native and Linux credential syscalls
use the same kernel UID/GID fields; UID 0 does not bypass this policy.

Discovery and per-file inspection are separate checks with the same
admission rule for the four process files in this phase. Numeric lookup,
directory iteration, and use of an open description must enforce it.
Special nodes such as mountinfo retain their separate rules. Worker TIDs,
kernel/idle entries and host tasks have no process rows. Use the global
guest PID of each leader; report PPID as zero if the recorded parent
incarnation cannot be resolved to a visible leader.

Native top is a bounded shared telemetry projection. Its flat process
display groups Linux TGIDs, but it does not enforce procfs authorization
and its task and row limits differ from procfs.

## Implementation and artifact

- `kernel/src/linux_procfs.rs`: shared discovery and lookup predicate;
  stable parent check and native group accounting. Every use of a saved
  PID description checks the PID, publication incarnation and VM generation
  against the live, authorized registry entry. Reads validate and copy
  while holding the scheduler lock. Root iteration skips dead or replaced
  identities. The existing 512-task scan bound remains.
- `kernel/src/process/spawn.rs`: snapshot the actual native exec argv
  using the same bounded, NUL-delimited format as Linux exec, up to 1 KiB.
- `kernel/src/sched/mod.rs`: keep native reaped worker runtime on its
  retained group owner, just as for Linux thread groups.
- `kernel/src/telemetry.rs`: feed procfs the existing resident walk results
  beyond the native display's 64-row cap.
- `tools/btop-proc-02/`: isolated native and Linux probes, image packaging,
  and TTY replay.

The reader at pinned btop commit
`a1099d075fa914452366b28c67ff171e21a1f02e`, in
`src/linux/btop_collect.cpp`, opens comm, cmdline and status on discovery,
then stat on refresh. It uses the registry name, actual argv and shared
guest UID. Stat reports boot-relative creation in 100 Hz ticks, and
aggregate nanoseconds divided by 10,000,000 for fields 14+15; the existing
formatter puts that total in field 14 and leaves field 15 zero because
accounting does not split user and kernel execution. Count live group
members; include runtime of finished and reaped workers once. RSS is
present user mapping count in 4 KiB pages, not virtual size. A retained
native owner can use a surviving worker's sample from the same VM
generation. The unimplemented virtual size field is zero; the pinned
reader skips it. The reader tries statm only on its suspicious-RSS fallback;
that path is not implemented. A missing or stale RSS sample still yields
EAGAIN. btop skips the stat update on open failure, so enumeration alone
does not establish that the row's metrics were populated.

```sh
out=target/btop-proc-02
python3 tools/btop-proc-01/replay.py target/btop-proc-01-multi/fixture-v2.iso "$out/before" --btop
python3 tools/btop-proc-01/replay.py target/btop-proc-01-multi/fixture-v2.iso "$out/before-retry" --btop
(
  set -e
  git apply target/btop-22/final/ramfs-config-package.patch
  trap 'git apply -R target/btop-22/final/ramfs-config-package.patch' EXIT
  git apply target/btop-22/final/trace-only.patch
  trap 'git apply -R target/btop-22/final/trace-only.patch; git apply -R target/btop-22/final/ramfs-config-package.patch' EXIT
  export SUNLIGHT_LINUX_STARTUP_TRACE=1 SUNLIGHT_HELIOS_SIG01_TRACE=1 SUNLIGHT_BTOP06_WAIT_TRACE=1
  cargo build --package sunlight-kernel --features native_document_viewer
  cp target/x86_64-unknown-none/debug/sunlight-kernel "$out/kernel-v2.elf"
  bash tools/make_hybrid_iso.sh "$out/kernel-v2.elf" "$out/base-v2.iso" target/limine "$PWD"
)
python3 tools/native-c/cc.py -O2 -Wall -Wextra -Werror tools/btop-proc-02/launch.c -o "$out/launcher-v2.elf"
musl-gcc -static -no-pie -pthread -O2 -Wall -Wextra -Werror tools/btop-proc-02/leader.c -o "$out/leader-v2.elf"
bash tools/stamp_helios_elf.sh "$out/leader-v2.elf"
python3 tools/btop-proc-02/image.py "$out/base-v2.iso" "$out/launcher-v2.elf" "$out/leader-v2.elf" "$out/final-v2.iso"
python3 tools/btop-proc-02/replay.py "$out/final-v2.iso" "$out/after-v2" --btop
python3 tools/btop-proc-02/replay.py "$out/final-v2.iso" "$out/accept-v2" --btop
cargo test -p sunlight-telemetry --target x86_64-unknown-linux-gnu thread_groups_sum_cpu_once_and_share_memory
```

| Artifact | SHA-256 |
|---|---|
| Baseline ISO, retained PROC-01 fixture v2 | `fc8940b73f06377f908e77da95c3832ea11b785fb6170a8dc20ba3ae169770ec` |
| Final ISO | `5759b349f00b1b69195dac832bfbf6da52d0bc0284835a35f805ad3b9eeff7c9` |
| Kernel ELF before injecting probes | `a38083014a5a36882254d6451cbbb96649e273f10247fc1bc98ff50397261ab8` |
| Native launcher | `1f1c4752d23841b7f3df2a35bd9171abc098c34e1f50f3ef9c9ffdf0e2fcc969` |
| Linux fixture | `33f8e823c7f0a7435a522fe1c2ab0147cc91a86f3e8299bd0354500799903712` |
| Corrected final ISO (v3) | `18e3c536f914ebef6d0b7f26930a35f17db500c83942dcc0a486b324961dc86a` |
| Corrected Linux fixture (v3) | `4a94684e24dd25308ad691db1a0ad949ca8420f06bb6663df03af7d402901547` |
| btop, 2,754,744 bytes at final ISO offset 33,352,476 | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |

The first baseline replay (`before/`) preserved serial evidence of shell
37, launcher 38, Linux leaders 39–42, and `/proc/37/stat` returning ENOENT.
btop 42 terminated at startup with a filesystem status EAGAIN for its
theme directory. The baseline retry (`before-retry/`) reached a dashboard
with the four original Linux leaders but the driver timed out waiting for
39 to reap. The original PROC-01 successful before sets remain
`{39,40,41,42}`, then `{39,40,41,42,45}`, then `{40,41,42,45}`.
The original one-row replay remains unchanged.

The first changed image (`after/`) was rejected: the built-in TTY launch
did not carry a foreground execution scope, so scope-only admission did
not show the shell. Its Help return screenshot exposed a separate
fixture bug: the dashboard showed 3.38 GiB while the OCR check required
3.39 GiB. The retry checks for dashboard content without assuming RAM size.

## Final guest evidence

```sh
musl-gcc -static -no-pie -pthread -O2 -Wall -Wextra -Werror tools/btop-proc-02/leader.c -o "$out/leader-v3.elf"
bash tools/stamp_helios_elf.sh "$out/leader-v3.elf"
python3 tools/btop-proc-02/image.py "$out/base-v2.iso" "$out/launcher-v2.elf" "$out/leader-v3.elf" "$out/final-v3.iso"
for run in accept-v3 accept-v3-retry accept-v3-third; do
  python3 tools/btop-proc-02/replay.py "$out/final-v3.iso" "$out/$run" --btop
done
dd if="$out/final-v3.iso" bs=1 skip=33352476 count=2754744 status=none | sha256sum
```

The v3 packaging log hashes the bytes directly at that offset. The first
v3 boot stalled waiting for TTY shell capability lookup. The second boot
hit the theme-directory EAGAIN again. In the third boot, the process
filter was empty, the list flat, and the footer showed all 6, 7, and 5
rows within the viewport. The guest PID and Linux-visible PID match:
native shell 37, native launcher 38, transient native 47, Linux leaders
39, 40, 41, 42 and short-lived observer 48. Restricted launcher 43 and
its child 45 exited before the stable capture; their UID excluded them
from the root collector's same-UID view. Workers 44 and 46 belong to 40
and 42 respectively; neither is an independent process row. Unrelated
native services lack the required foreground or TTY family relation.
Kernel/idle and host tasks are outside the published userspace view.

| Window | A: fixture lives reconstructed from scheduler events | B: sibling numeric entries and btop observations | C: btop rows |
|---|---|---|---|
| Stable, uptime 36 s | 37, 38, 39, 40, 41, 42 | 37, 38, 39, 40, 41, 42; worker 46 returns 328 bytes | 37, 38, 39, 40, 41, 42 |
| Native appeared, uptime 53 s | 37, 38, 39, 40, 41, 42, 47 after observer exit | Same seven in the later samples; btop returns 352 bytes after observer exit | 37, 38, 39, 40, 41, 42, 47 |
| Native removed, uptime 72 s | 37, 38, 40, 41, 42 | 37, 38, 40, 41, 42 in sibling samples; btop trace cap reached | 37, 38, 40, 41, 42 |

A is based on fixture creation, clone and reaping records, not an
independent full scheduler registry dump. B combines same-scope sibling
reads and bounded btop syscall traces; the traces do not dump dirent
payloads. The strict A=B=C gate therefore remains incomplete.

```text
[HELIOS-THREAD] clone tgid=40 tid=44 tls=0x1000022f38 stack=0x1000022ef8
[PROC-RESTRICTED] reader=45 uid=1234
[PROC-RESTRICTED] entry=43
[PROC-RESTRICTED] entry=45
[PROC-RESTRICTED] path=/proc/37/stat fd=-1 errno=2
[PROC-FIX] path=native-start pid=47 result=0
[PROC-FIX] path=native-exit pid=47 result=0
[PROC-FIX] path=observer-exit pid=48 result=15
[PROC-FIX] path=btop-exit pid=42 result=0
```

Raw serial, QEMU commands, and captures are in
`target/btop-proc-02/accept-v3-third/`. The restricted reader got ENOENT
for all four files at the known shell PID. Status reports native
38's PPID as 37, and Linux 39 and 40's PPID as 38. The shell's parent is
outside the view (PPID 0). Stat reports two threads for 40, matching
one leader plus one worker and the displayed count.

Stat RSS was 878 pages for native 37 (3.43 MiB, displayed 3.4M),
532 pages for native 38 (2.08 MiB, displayed 2.1M), and 567 for Linux
40 (2.21 MiB, displayed 2.2M). The shell's unchanged 16 ticks in the
nearby samples are consistent with displayed 0.0%. PID 40's ticks rose
25 to 38 across probe samples, but those samples do not bracket btop's
refresh or capture the denominator in its CPU calculation. Neither
the normalized CPU percentages nor the independent memory counters
were quantitatively validated. Stat EAGAIN recurred independently:
worker 46 first opened stat for 47 at trace sequence 807 (EAGAIN),
then succeeded at 883. No cause or fix is inferred from recovery.

| Gate | Result |
|---|---|
| Native visibility | **PASS** for this fixture: shell and launcher once each; 47 appears and disappears after exit/reap. The runner captures about five host seconds after spawn and four after wait returns, plus capture overhead. |
| LinuxCompat regression | **PASS** for stable fixture leaders, worker exclusion and thread count. |
| Identity and PPID | **PASS** for fixture IDs, native parent/child and mixed-runtime parent/child. Full registry dump and live native top cross-check **NOT RUN**. |
| Permissions | **PASS** for UID 1234 discovery and four direct opens of the protected shell. Separate same-UID cross-scope negative control **NOT RUN**. |
| Telemetry | **PASS** for sampled stat RSS conversion and displayed rounding; matched CPU/independent RSS comparison **NOT RUN**. Initial stat opens **FAIL** with transient EAGAIN; later opens succeed. |
| Exit and PID reuse | **PASS** for visible row removal. Stale FD observer **FAIL**: polling ended with exit 15 before it demonstrated a post-exit read. Deterministic PID reuse **NOT RUN**. |
| Interactive acceptance | **PASS** for Help open and return, explicit q exit 0, worker 46 and leader 42 reaped, and separate same-TTY `/bin/proc-native` PID 49 exiting 0. |
| Focused host test | **PASS**, telemetry thread group aggregation; it does not exercise procfs identity. |

| Raw evidence in `accept-v3-third/` | SHA-256 |
|---|---|
| serial.log | `76ebb2822e7bfb622bfc776ec9e444012e899a7be0c724d7650f86b11f8633e2` |
| stable.png | `0c0c7264e42a5d7ea066f09c4298e5628f01b0c1bfdcebfe0ebde12e3e30bbc4` |
| appeared.png | `960f76c1439dada104a9e44148154c333d3b87a9196dce97b78d9ccc5082fbbc` |
| removed.png | `64ec9486ab9bcf5b853cbe39832c96dbedfe996f0a3a96ffb7914303819d29be` |
| help.png | `08de557362a2f2a329ab6848fb36326ee9e9bc539d8b1c2eebd9001ad9ef38f7` |
| dashboard-return-0.png | `aae9bb682ca3a3b65f6567280352d31a6d787b2ef1b204c016a58580843420` |

## Follow-up, November 24, 2026

The fixture PASS results above remain specific to final-v3 and its
accepted replay. PROC-01 is unchanged. This follow-up changed only
`tools/btop-proc-02/leader.c` (numeric directory probes and bounded
lifetime checkpoints), `tools/btop-proc-02/launch.c` (separate timing
control, native target lifetime 20 to 12 seconds), and this report.
Both probe images use the original base-v2 kernel and change only
fixture payloads.

```sh
out=target/btop-proc-02
run=$out/followup
mkdir -p "$run"
sha256sum "$out/final-v3.iso"
python3 tools/btop-proc-02/replay.py "$out/final-v3.iso" "$run/baseline" --btop
musl-gcc -static -no-pie -pthread -O2 -Wall -Wextra -Werror tools/btop-proc-02/leader.c -o "$run/leader.elf"
bash tools/stamp_helios_elf.sh "$run/leader.elf"
python3 tools/btop-proc-02/image.py "$out/base-v2.iso" "$out/launcher-v2.elf" "$run/leader.elf" "$run/probe.iso" | tee "$run/package.log"
python3 tools/btop-proc-02/replay.py "$run/probe.iso" "$run/replay" --btop
python3 tools/native-c/cc.py -O2 -Wall -Wextra -Werror tools/btop-proc-02/launch.c -o "$run/launcher-short.elf"
python3 tools/btop-proc-02/image.py "$out/base-v2.iso" "$run/launcher-short.elf" "$run/leader.elf" "$run/probe-short.iso" | tee "$run/package-short.log"
python3 tools/btop-proc-02/replay.py "$run/probe-short.iso" "$run/replay-short" --btop
```

Each replay directory contains the full serial log and exact QEMU arguments
in `command.json`. Both packaging logs hash the embedded btop bytes at
offset 33,352,476 (length 2,754,744):
`bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702`.

| Artifact under `followup/` | SHA-256 |
|---|---|
| leader.elf | `92f5a9b3e1d28c79f79d5e9985e7433e3a021aa510def304a59da11d9d83a993` |
| launcher-short.elf | `af7c93095d351dd88b0892402e211630c13e67302f656fa69816f29c39882dca` |
| probe.iso | `213be5c6821351babde5ea32a48ecd6117e519d41a6026d9cc56b7b41f3e9c2d` |
| probe-short.iso | `cd29920a39a315700a43a35b3becff142fa59cbcfbd18ebdb62513237d797c02` |
| baseline/serial.log | `e19d5fca062250fdd09e3ad5f7267cea376360f46b098629d2449e5d0e4f3a5e` |
| replay/serial.log | `e909983664e447800005a57c1fc1979e4cc8e93975592560dc0f29b0f140cd1c` |
| replay-short/serial.log | `b6e1d23f696d6b0d0f72ff1337250696b7509a35a0f2ec4f5c6a0e3a4f7c52ec` |
| replay/help.png | `090f5801e48604bc9502457c1e84316f016c2803bdf6abac4bbc4d0e19855719` |
| replay/dashboard-return-0.png | `d92b2a9b897e73052d8bca92d7271a1df2c1a06dbd0af72214a2b19465d21e9d` |

In `replay/serial.log`, restricted Linux reader 44 (UID 1234) enumerated
only 43 and 44. Direct numeric directory opens succeeded for self 44
and its native parent 43 and returned ENOENT for live native shell 37.
The separate four-file open check for shell 37 returned ENOENT for
comm, cmdline, status and stat.

```text
[PROC-RESTRICTED] reader=44 uid=1234
[PROC-RESTRICTED] entry=43
[PROC-RESTRICTED] entry=44
[PROC-DISCOVERY] path=/proc/44 fd=3 errno=0
[PROC-DISCOVERY] path=/proc/43 fd=3 errno=0
[PROC-DISCOVERY] path=/proc/37 fd=-1 errno=2
[PROC-LIFETIME] checkpoint=open pid=47 enumerated=1 fd=3 errno=0
[PROC-LIFETIME] checkpoint=initial-read pid=47 result=1 errno=0
[PROC-LIFETIME] checkpoint=lookup pid=47 attempt=0 fd=4 errno=0
[PROC-LIFETIME] checkpoint=lookup pid=47 attempt=34 fd=4 errno=0
[SCHED] process_mark_finished pid=47 name='tty-select' reason=exit code=0
[SCHED] process_reaped pid=47 name='tty-select' user_frames=532 page_tables=10 swap_blocks=0 ipc_cleared=0
[PROC-FIX] path=native-exit pid=47 result=0
[PROC-FIX] path=observer-exit pid=48 result=15
```

The unchanged final-v3 replay also returned observer exit 15 (observer
49, target 48). The instrumented no-reuse run localizes the exhausted
budget to waiting for lookup to report disappearance: all 35 opens
returned successfully before the target exit and reap records. The
launcher logged the observer's status after its own target wait; that
line does not place observer termination after target reap. There was
no stale-read-start checkpoint. The documented live identity check
requires ENOENT from an old handle after reap, including if a different
incarnation occupies the PID. No deterministic live reuse seam was
found: the allocator uses max retained PID plus one; the boundary
test in `kernel/src/process/pid.rs` does arithmetic only.

Stat EAGAIN recurred on an **open**: Linux reader 40, syscall 2, for
`/proc/38/stat` at sequence 20 returned -11; sequence 225 returned fd 4,
and the sibling poll emitted stat contents. The per-PID renderer has
EAGAIN branches for absent or aged RSS sampling and for a missing
identity in the RSS sample. The trace does not distinguish the branch
taken or establish its underlying cause.

CPU accounting uses 100 ABI ticks per second (10 ms per tick).
For a window of elapsed guest time T seconds, the per-core expectation
for a process delta D ticks is 100 × D / (100 × T) percent.
The pinned btop collector uses the delta of summed aggregate
`/proc/stat` fields as its denominator and multiplies by the detected
core count only with `proc_per_core` enabled. It rounds the computed
value to tenths before clamping; dashboard formatting can reduce
display precision. The recorded sibling samples and screenshots do
not align scheduler counters, process stat, and btop refresh windows
over idle and sustained load.

| Follow-up gate | Result |
|---|---|
| Registry-backed completeness | **BLOCKED**: no independent registry snapshot and no complete collector-credential B/C comparison across the full list. Fixture rows and sibling samples remain partial evidence. A discoverable PID with an EAGAIN stat open is a read failure, not evidence of a missing leader. |
| Restricted discovery and lookup | **PASS** for the UID 1234 fixture, with separate open-denial results above. |
| Stale-handle read, no PID reuse | **BLOCKED**: target exited and reaped after the last lookup; no post-reap read. The shorter target run stopped at the host's btop spawn wait before reaching the probe. |
| Deterministic PID reuse | **NOT RUN**. |
| Matched CPU telemetry | **NOT RUN**: no aligned idle/load measurement. |
| EAGAIN root cause | **BLOCKED**: operation and later success captured; branch and cause unresolved. |

Interactive acceptance passed on the baseline replay and separately on
`probe.iso`: the runner captured Help and return, sent explicit q and
observed btop leader 42 exit 0, with worker 46 and leader 42 reaped in
the probe log. The separate same-TTY `/bin/proc-native` exited 0
(probe serial line 4705). `probe-short.iso` did not reach these checks
and has no interactive acceptance result.

## Registry-backed process-list coverage, November 24, 2026

**PASS for displayed leader coverage in the sampled boot.** The retained
PROC-02 `closeout/registry.iso` already contains the read-only
`SUNLIGHT_PROC02_REGISTRY_TRACE` diagnostic. Its debug-log hook walks
the scheduler registry under lock and emits every slot with identity,
lifetime, group, UID, scope and TTY metadata. The separate `/proc`
observer triggers the dump before and after each directory scan. No
image was rebuilt. This result applies to the registry ISO.

```sh
out=target/btop-proc-02
sha256sum "$out/closeout/registry.iso" "$out/coverage-replay.py"
dd if="$out/closeout/registry.iso" bs=1 skip=33356924 count=2754744 status=none | sha256sum
python3 "$out/coverage-replay.py" "$out/closeout/registry.iso" "$out/coverage" --btop
sha256sum "$out/coverage/serial.log" "$out/coverage/stable.png"
```

| Artifact | SHA-256 |
|---|---|
| Registry ISO | `8a8a7bef469bda49d17228f24d6a9398d791adbf0f3bf3ddfabb6804f5298f1a` |
| Embedded btop, matching earlier PROC-02 | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Host replay | `11dfa2d55b67a78d6c938d7117445ae9e115a28a662bb378dbd20a53b92f1494` |
| Serial log | `353b16bb3080271ea9d669dc00baabbf2a9a9696602fddeae1754267ae132f6a` |
| Dashboard capture | `75ba3b80f89faa03e0635bac7e805bb7b6e8cb3c6052a154325daee9c49ecbed` |

The host replay uses the existing hardware TTY driver. It ran
`/bin/tty-select 37 registry`, sent Delete to clear the filter, waited
for observer round 1, and captured the dashboard at guest uptime 36 s.
The serial snapshots before and after capture span round 2. The replay
stopped after the round 3 scan, without driving the launcher’s later
lifetime scenario. Exact QEMU arguments are in
`coverage/network-command.json`.

| Set | Linux-visible PIDs in the stable window |
|---|---|
| A: scheduler registry, applying btop caller policy | `{37, 38, 39, 40, 41, 42}` |
| B: numeric directory entries from equivalent LinuxCompat observer | `{37, 38, 39, 40, 41, 42}` |
| B: successful per-PID opens traced in btop collector TID 46 | `{37, 38, 39, 40, 41, 42}` |
| C: all btop rows | `{37, 38, 39, 40, 41, 42}` |

Both full round 2 registry dumps contain 41 slots. The six admitted
leaders retained the same identities through capture: native shell 37
and launcher 38 are generation-matched TTY ancestors; LinuxCompat
leaders 39–42 are live, UID 0 and unscoped. PID 41 is the diagnostic
caller and a regular visible leader, so it is included. TIDs 44 and 46
belong to leaders 40 and 42. PID 43 had already left the registry and
45 was an exiting UID 1234 process. The other registry PIDs
`{1, 3, 4, 6, 7, 8, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36}`
lacked the native TTY family admission. No admitted leader exited in
the capture window.

Observer 41 and btop's collector share UID 0, zero foreground scope,
TTY attachment and the same native ancestor chain. They both see the
unscoped LinuxCompat leaders under the same policy. Observer rounds
1–3 returned the same exact numeric set and opened and read all four
files for each PID without errors. Shell 37's cmdline read returned
zero bytes, leaving its Command column blank. The btop trace shows
successful opens of comm, cmdline, status and stat for every PID in
the set. Earlier collector stat opens for `{38, 39, 40, 41, 42}`
returned EAGAIN before succeeding; these are read-path failures during
startup, with no corresponding leader missing in the stable window.
The trace records btop's getdents return lengths; the exact numeric
payload and read lengths were independently captured by the equivalent
observer, not decoded from btop's own syscalls.

The dashboard shows an empty filter and a flat list (tree off). The
footer reads `0/6` and all six rows fit, so there were no additional
rows to scroll to. Display order was 42, 41, 40, 38, 37, 39. Neither
comparison lost or added a PID in the stable window. Earlier results
and unverified statuses remain as recorded.
