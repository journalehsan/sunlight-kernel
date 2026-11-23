# BTop-17: Interactive regression evidence

The interactive baseline is still unresolved. This pass checked preserved
evidence and ran host tests. The live fault replay and guest bisect were
not run under the revised safer-test scope. No production code or guest
artifact changed.

## Artifact identity and commands

| Artifact | Verified SHA-256 |
| --- | --- |
| `target/btop-01/btop` | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| BTop-14 ISO | `4df853e5b65f8ccdec95b9e51d11d8bd714fc98e839e010733ec7774fe903c80` |
| BTop-16 ISO | `b15e3726cd5535ff61e7c154aac5fedddb6d4dc39aea33efae15ddf432b4ccc9` |
| Read-only fixture | `4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f` |

```sh
sha256sum target/btop-01/btop target/btop-{14,16}/final/*.iso target/btop-10/disk.img
mkdir -p target/btop-17
python3 tools/btop-16/verify.py target/btop-16/readonly-paced > target/btop-17/verify-mounted.log
python3 tools/btop-16/verify.py target/btop-16/none-paced > target/btop-17/verify-cd.log
cargo test --offline --locked --target x86_64-unknown-linux-gnu -p sunlight-fs -p sunlight-fat -p sunlight-block -p sunlight-compat-linux -p sunlight-virtio > target/btop-17/focused.log 2>&1
python3 tools/test_tty_status_host.py > target/btop-17/tty-host.log 2>&1
git diff --check
```

This report is the only new source-tree file; test outputs are under
`target/btop-17/`. Historical launch commands and artifacts are preserved
in [BTop-16](btop-16-lifetime-read-accounting.md#reproduction),
[BTop-15](btop-15-read-only-io-display.md), and
[BTop-14](btop-14-partition-busy-time.md). The BTop-16 paced runs used
4096 MiB and four vCPUs. See each run's `network-command.json`,
`attached.json`, `phases.jsonl`, and `serial.log` for emulator arguments,
attachment state, workload timing, and guest output. The original
`tools/btop-16/replay.py` adds 250 ms after each key operation;
`tools/btop-07/replay.py` specifies the requested environment
(`TERM=xterm`, `LANG=C.UTF-8`, both XDG directories at
`/tmp/btopfixture`), three dashboard captures two seconds apart,
then `h`, `h`, and conditionally `q`. Requested keys and settings
do not prove the guest received them. No new guest configuration or
TTY geometry was measured.

## Archived failure classification

- Mounted: `target/btop-16/readonly-paced/serial.log:5433` identifies
  leader 42. At line 5554 it took a ring-3 general-protection exception
  (vector 13, RIP `0x60a3ef`, RSP `0x7fffffffe850`, error 0).
  The scheduler recorded the reason and code 139 at line 5557, reaped
  the leader at 5561, and the TTY reported 139 at 5565. That is a
  guest fault and exit, not a host timeout; the log does not establish
  delivery of Linux SIGSEGV, a Linux-encoded parent wait status, or a
  faulting data address. A config-path warning was captured. No completed
  dashboard refresh, Help open, worker reap, or TTY reuse was established.
- CD-only: `target/btop-16/none-paced/serial.log:1462` identifies leader
  40 and line 1712 records worker 41 reaping. The Help capture met the
  open assertion; the second `h` was requested and the dashboard
  return assertion timed out. The archive does not prove key delivery
  or consumption, the signal mask, the return of `pselect6`, a handler
  or return-path fault, or an independently observed dashboard redraw.
  No leader exit status or reap was observed before the harness stopped;
  whether the leader was blocked at that point was not established.

## Bisect evidence

| Factor | Evidence and limit |
| --- | --- |
| Mounted versus CD-only | Archived BTop-16 mounted and CD-only runs used the exact same ISO but different disk and workload outcomes. Different symptoms; controlled replay **NOT RUN**. |
| Lifetime accounting on versus off | **NOT RUN**. The trace switch gates logging and does not disable accounting. |
| Storage query versus no query | **NOT RUN**. The archived mounted script queried devices and mounts; no matched query-free run. |
| Help versus dashboard-only q | **NOT RUN**. The paced script requires passing Help return before it sends q. |
| BTop-14 versus BTop-16 | Historical comparison only. BTop-14 mounted and CD-only serial logs show separate btop and native exits of 0; the BTop-16 runs failed. Image-only bisect with the same runner and input timing **NOT RUN**. |

The cause is **BLOCKED** on controlled guest evidence. No minimal
guest reproducer, production fix, or fault-specific regression was
validated.

## Results

Guest results below describe archival observations on the BTop-16 ISO.
All fresh interactive guest acceptance checks are **NOT RUN**.

| Gate | Mounted | CD-only |
| --- | --- | --- |
| btop startup | **FAIL**: leader faulted at startup; no refresh established | **PASS**: launched and reached Help; full refresh acceptance unverified |
| Help open | **FAIL**: no Help observed after launch fault | **PASS**: Help capture |
| Help return | **NOT RUN** | **FAIL**: return assertion timed out |
| Normal q exit/status 0 | **NOT RUN** | **NOT RUN** |
| Leader and worker reaping | **PASS** for leader cleanup after fault; worker **NOT RUN** | **PASS** for worker cleanup; leader **NOT RUN** |
| Same-TTY `/bin/proc-native` and its status 0 | **NOT RUN** | **NOT RUN** |
| Lifetime accounting | **PASS** for archived serialized reads, independently rechecked offline; live regression **NOT RUN** | **PASS** for archived no-disk isolation, rechecked offline |
| Native/musl parity on this ISO | **NOT RUN** | **NOT RUN** |

Host tests: **PASS**, 70 block, 29 Helios, 10 FAT, 113 filesystem, and
31 VirtIO tests; TTY status host checks also passed. Guest TTY and
signal behavior and same-image parity were **NOT RUN**. BTop-16's
historical parity pass used the separate ISO
`6a98b4a0ae50256723d840e916ca0743f1a19d3f5a0c91c91d09f2d5d797c9e8`.

The archived mounted run retains **1,304 completions / 1,304 successful
512-byte sectors**. Linux ABI equivalence remains **BLOCKED**: the
10 ms clock does not resolve early boot or sub-tick latency, and
successful sectors do not establish Linux retired-byte semantics.
Linux storage row export, writable guest tests, and btop I/O display
replay remain **NOT RUN**. BTop-12's historical **FAIL** and BTop-13's
guest-discovery **BLOCKED** result stand.
