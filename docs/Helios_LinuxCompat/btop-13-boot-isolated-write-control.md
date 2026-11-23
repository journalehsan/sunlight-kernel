# BTop-13: Boot-isolated disposable write control

The disposable disk was absent throughout legacy BIOS boot and attached
read-only after the kernel entered the scheduler. QEMU exposed the disk on
PCI, but Helios did not claim or access it. The end-to-end isolation gate
remains **BLOCKED** and no writable guest was launched.

## Read-only qualification

From the repository root, using the preserved BTop-11 write fixture ISO:

```sh
qemu-system-x86_64 --version
python3 target/btop-13/probe.py
sha256sum target/btop-11/write.iso target/btop-13/readonly/disposable.img
git diff --check
```

The runner creates a new run directory exclusively and refuses to overwrite
the retained run. Its complete QEMU argv is in
`target/btop-13/readonly/command.json`; exact QMP commands and replies
are in `qmp.jsonl` in the same directory. QEMU 11.1.3 ran q35 with TCG,
4096 MiB, four vCPUs, standard VGA, a legacy VirtIO RNG, no NIC, and an
empty PCI bridge at 00:05.0. QEMU started paused with `-S`; the only
media backend was `ide2-cd0` → raw → file → read-only write ISO
(SHA-256 `187be09182a894494a94f6e454f1372e15d11113d3671d976717b83eeb2b92d3`).
There was no root disk or host block device attached.

QMP queried the block backends, named nodes and PCI graph before `cont`
and again after readiness, before creating any test node. Both snapshots
lack the fixture. Serial records the Limine boundary (line 6), the negative
VirtIO block scan (122), four CPUs (127), and
`[PROC] Entering scheduler — dropping to Ring 3` (342).

| Event | Host monotonic ns | Evidence |
| --- | ---: | --- |
| Before QEMU launch | 121876328443416 | QMP log line 1 |
| Boot resumes | 121876381624386 | `cont` sent |
| Scheduler marker observed | 121882693438957 | QMP log line 14 |
| Read-only disk node requested | 121882695321445 | `blockdev-add`, line 21 |
| PCI attach accepted | 121882698101243 | `device_add` reply, line 24 |
| Host shutdown requested | 121892701633933 | QMP `quit`, line 35 |
| QEMU exited with status 0 | 121892709393062 | QMP log line 38 |

After attach, the additional graph was legacy VirtIO block
`fixture-dev` at 01:01.0 (1af4:1001, configured serial `BTOP13-RO`)
→ BlockBackend → raw node `fixture` → file node `fixture-file` →
fresh regular file `target/btop-13/readonly/disposable.img`. Both nodes
were read-only. QEMU reported 4,194,304 bytes, equal to 8192 sectors at
512 bytes. The authorized ranges for a future invocation are LBA 62
[31744, 32256) and LBA 63 [32256, 32768), in bytes. These are host and
fixture checks; Helios did not confirm identity, capacity or the marker.
The post-attach PCI snapshot shows both endpoint BARs unassigned.

The unfiltered capture of all enabled events in `qemu.log` spans
process launch through exit, alongside `serial.log` and the monotonic QMP
log. There were no VirtIO block submissions or completions, and no
BlockBackend or block-node write events. The shutdown
`vm_stop_flush_all ret 0` is a host event and does not imply a guest
flush request. QMP fixture node counters were zero for reads, writes
and flushes at attach and after ten seconds. No fixture was invoked.

## Exact blocker

`kernel/src/main.rs:717` calls `init_block_and_fat` once at boot.
Its VirtIO probe at line 2120 returns on the absent-device branch before
queue setup or the fixture call at 2183. Source inspection found no
runtime rescan or bind route to register this endpoint and hand it to
the fixture after readiness. The existing `sfs5b4_test::run` also has
no no-payload mode: after the capacity and LBA 0 marker checks it
unconditionally writes LBA 62 (`kernel/src/sfs5b4_test.rs:74`).
The prerequisite is an existing authorized path that binds a disk after
the boot chain, checks its identity, capacity and marker in the guest,
and can qualify it with payload writes disabled. No alternate boot mode
was qualified.

## Sector comparison and results

The whole raw disk had SHA-256
`e56d69a36098d85c525133521a6a1772902eb1812363b06a92d1f4d5b360016f`
before attach and after exit. After shutdown the runner compared all
8192 logical sectors individually and found zero changed sectors.
Before and after digests for LBA 0, 62 and 63 are in QMP log line 39.
LBA 0 remained
`dfcc9e057063284d024fa2be1d235a16de44968232c82a32e759b1e751289357`;
62 and 63 remained all zero, as did every sector except the marker sector.
This establishes byte equality for the read-only probe.

| Gate | Result |
| --- | --- |
| Boot isolation | **PASS** for the observed read-only topology. The disk entered the block graph only after kernel readiness. |
| Guest device discovery | **BLOCKED**. QEMU attached it; Helios did not bind it or verify identity or capacity. |
| Read-only fixture invocation without payload | **NOT RUN**. No guest binding and no no-payload fixture entry. |
| Writable no-payload control | **NOT RUN**. |
| Writable write-range safety | **NOT RUN**. Neither isolated write nor the two-write sequence was attempted. BTop-12's historical out-of-range request remains **FAIL**. |
| Writable payload verification | **NOT RUN**. |
| Partition attribution and busy-time accounting | **BLOCKED**; unchanged. |
| btop I/O display | **NOT RUN**. |

Only this report was added to the source tree. The runner, image, and
logs are under ignored `target/btop-13`. Earlier reports, failed runs,
images and traces were left in place. No unrelated worktree changes,
guest drivers, write grants, Limine, protected mount policy, or
fail-closed mountinfo behavior were changed. BTop-11 read/repeat/idle
and no-mount regressions, focused storage and Helios tests, native/musl
parity, service-view checks, and pinned btop interactive baselines were
not rerun. No commit or push.
