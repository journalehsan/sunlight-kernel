# BTop-18: Replay harness qualification and first divergence

BTop-17 did not run live replay or a guest bisect. Its session encountered a content restriction and the user narrowed the scope to safer tests; the report records archive review and host checks. It does not identify a missing ISO, device, or emulator as the reason. BTop-18 explicitly requests normal interactive replay on the original read-only setup. The original media, QEMU with TCG, and the scanout tools were available.

## Media and runner

| Artifact | Verified SHA-256 |
| --- | --- |
| `target/btop-14/final/guest-ramfs-trace-config.iso` | `4df853e5b65f8ccdec95b9e51d11d8bd714fc98e839e010733ec7774fe903c80` |
| `target/btop-16/final/guest-ramfs-trace-config.iso` | `b15e3726cd5535ff61e7c154aac5fedddb6d4dc39aea33efae15ddf432b4ccc9` |
| `target/btop-01/btop` | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| `target/btop-10/disk.img` | `4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f` |

`target/btop-18/media.sha256` records the hashes. Extraction of
`/boot/sunlight-kernel.elf` from each ISO with `xorriso -osirrox on`
also found the complete pinned btop bytes and identical packaged
`tools/btop-08/btop.conf` bytes in each ELF; see
`target/btop-18/{extract.log,embedded.txt}`. The packaged settings are
`update_ms=1000`, `show_disks=True`, `only_physical=False`,
`use_fstab=False`, `disks_filter="/mnt"`, `show_io_stat=False`,
`io_mode=False`, and `disk_free_priv=False`. Packaging and requested
environment do not establish which configuration btop loaded at runtime.

The runner is the archived BTop-16 paced sequence with a host-only
observer in `target/btop-18/replay.py`, SHA-256
`bd059602f65267d46eb9e458964e3f9c63026f1abd0665737df2945ecbf84456`.
`target/btop-18/harness.sha256` identifies the wrapper, generated
runner and dependencies. It redirects generated code away from the
archive and logs QMP replies, host monotonic timestamps and guest
serial byte counts for key injection and scanout. It adds no guest
instrumentation. Exact argv is in each attempt's `network-command.json`.
QEMU 11.1.3 used this command shape, with the mounted options inserted
before the trace options:

```sh
/usr/bin/qemu-system-x86_64 -cdrom "$iso" -serial "file:$out/serial.log" \
  -display none -m 4096M -smp 4 -device virtio-rng-pci,disable-modern=on \
  -qmp "unix:$socket,server=on,wait=off" -no-reboot -no-shutdown \
  -device virtio-vga,xres=1280,yres=800 -accel tcg -cpu max -boot d -nic none \
  -trace enable=virtio_blk_handle_read -trace enable=virtio_blk_rw_complete \
  -trace enable=virtio_blk_req_complete \
  -netdev user,id=testnet,restrict=on,dhcpstart=10.0.2.42 \
  -device virtio-net-pci,netdev=testnet,disable-modern=on \
  -object "filter-dump,id=frames,netdev=testnet,file=$out/network.pcap"
# Mounted options:
# -drive file=<verified disk.img>,format=raw,if=none,id=fixture,readonly=on
# -device virtio-blk-pci,disable-modern=on,drive=fixture
```

The runner waits for login, sends Enter, types `root`, then sends Tab,
Space and Enter for the built-in hardware TTY. The mounted sequence
waits for MBR partition start 2048 and the FAT mount at `/mnt` and
issues `storagectl devices` and `storagectl mounts`. After a separate
idle interval, both sequences run `/bin/storage-probe` twice (expected
status 3 without the fixture and 0 with it). Then they export
`TERM=xterm`, `LANG=C.UTF-8`, `XDG_CONFIG_HOME=/tmp/btopfixture` and
`XDG_STATE_HOME=/tmp/btopfixture`, and launch `/bin/btop`. Key holds
are 60 ms, with the 150 ms base delay and 250 ms extra pacing. After
three dashboard captures with two-second waits, the runner sends
`h`, asserts Help, sends `h` again, and checks for dashboard return
before sending `q`. It requires TTY status 0 and both btop reaps
before running `/bin/proc-native` separately in the same shell.

## Evidence boundaries

- Host: `qmp-input-scanout.jsonl` records each attempted key and its
  QMP reply. A successful reply means QEMU accepted the command.
- Guest input: the built-in TTY's launch and exit lines show the
  shell received commands. Help opening, Help closing and orderly
  exit show application responses where observed. Per-key byte
  receipt and consumption in btop are not directly observable in
  these images. The bounded syscall trace does not record the
  individual tty bytes or reliably pair a masked wait with each key.
- Redraw: `dashboard-*.{ppm,png,txt}` and `help.{ppm,png,txt}`
  preserve scanout and OCR separately from serial. A timeout only
  means the harness did not establish the requested screen transition.

## CD-only replay

```sh
for version in 14 16; do
  out=target/btop-18/cd$version-first
  python3 target/btop-18/replay.py \
    target/btop-$version/final/guest-ramfs-trace-config.iso "$out" > "$out.log" 2>&1
  printf '%s\n' "$?" > "$out.status"
done
```

BTop-14 control qualification: **PASS**. Both guests reported 138
columns × 38 rows. For each, the two `h` commands and the final
`q` were accepted by QMP. The captured Help text includes
“Description:” and “Quits program.”; `dashboard-return-0.png` shows
the dashboard again and its OCR includes “3.39 GiB” without the Help
marker. BTop-16 also passed each lifecycle gate. There was no
observable divergence in this sequence: the historical CD-only
Help timeout is **NOT REPRODUCED**, and its cause remains unknown.
The launch argv differ only in ISO and per-run evidence/socket paths.

| Gate | BTop-14 CD-only | BTop-16 CD-only |
| --- | --- | --- |
| Help open and dashboard return | **PASS**, `help.png` and `dashboard-return-0.png` | **PASS**, same capture names |
| Normal `q` exit | **PASS**, status 0, serial line 2732 | **PASS**, status 0, line 2118 |
| Leader and worker reaping | **PASS**, leader 40 and worker 41, lines 2716 and 2728 | **PASS**, leader 40 and worker 41, lines 2102 and 2114 |
| Same-TTY `/bin/proc-native` | **PASS**, status 0, line 2755 | **PASS**, status 0, line 2141 |

Both first attempts and their outer status 0 are preserved at
`target/btop-18/cd14-first*` and `target/btop-18/cd16-first*`.
The archived BTop-16 timeout is under `target/btop-16/none-paced/`;
the original second `h` was requested, but the archive does not
prove consumption or a hang.

## Mounted read-only replay

```sh
for version in 14 16; do
  out=target/btop-18/mounted$version-first
  python3 target/btop-18/replay.py \
    target/btop-$version/final/guest-ramfs-trace-config.iso "$out" \
    --disk target/btop-10/disk.img > "$out.log" 2>&1
  result=$?
  printf '%s\n' "$result" > "$out.status"
  if [ "$version" = 14 ] && [ "$result" != 0 ]; then break; fi
done
```

BTop-14 mounted control qualification: **PASS**. The fixture appears
with `ro: true` in `target/btop-18/mounted14-first/attached.json`.
Serial shows the FAT mount, 138×38 geometry, and both read probes
exiting 0. Help opened and returned to the dashboard in `help.png`
and `dashboard-return-0.png`. Worker 44 and leader 42 were reaped
(serial lines 4329 and 4341); the TTY reported btop exit 0 at line
4345 and separate `/bin/proc-native` exit 0 at line 4371. The outer
harness status was 0.

BTop-16 mounted fault reproduction: **NOT REPRODUCED**. The same
verified fixture and device layout reported `ro: true` in
`target/btop-18/mounted16-first/attached.json`. Help open and return
**PASS** (`help.png`, `dashboard-return-0.png`); normal `q` exit
**PASS** (TTY status 0, serial line 7078); worker 44 and leader 42
reaping **PASS** (lines 7060 and 7074); same-TTY reuse **PASS**
(`/bin/proc-native` status 0, line 7105). The outer harness status
was 0. No new guest fault or Linux signal delivery was observed.
Both mounted runs retain `serial.log`, `qemu.log`, `phases.jsonl`,
`block-*.json`, and scanout under `target/btop-18/mounted{14,16}-first/`.
All four first attempts were retained; no retries were run. The media
and harness hashes were checked again after the runs.

The archived mounted BTop-16 failure remains at
`target/btop-16/readonly-paced/{serial.log,qemu.log,phases.jsonl,dashboard-0.png}`.
Leader 42 was in startup; the last PID-tagged syscall in the bounded
trace was Linux `stat` on `/proc/self/exe` returning -2 (line 5550),
followed by untagged mmap records. The first fault was ring-3 vector 13
at RIP `0x60a3ef`, RSP `0x7fffffffe850`, error 0 (lines 5554–5555).
No worker was identified at the fault. The scheduler recorded code 139
and reaped the leader; the TTY reported 139. There is no separately
captured raw parent wait word or proof of Linux SIGSEGV delivery. The
outer replay log records a Help assertion failure. The captured startup
warning about the config path does not establish the fault's cause.

There is no evidence tying the archived fault to the CD-only timeout.
If the timeout recurs and existing evidence again stops at QMP and
scanout, the smallest useful addition is a bounded trace for the
foreground btop TTY: record when the key reaches the input queue and
when a read dequeues it, with PID/TID and a monotonic timestamp. For a
recurring mounted startup fault, retain the first exception frame and
the parent's wait result with the same task identity. Keep any build
with added diagnostics separate from these exact-ISO runs.

## Historical accounting

[BTop-16](btop-16-lifetime-read-accounting.md) retains the archived
mounted result of 1,304 read completions and 1,304 successful 512-byte
sectors; the archive verifies zero fixture events in CD-only mode.
BTop-17 verified those archived traces offline. This replay does not
freshly validate lifetime accounting. Linux storage ABI equivalence
remains **BLOCKED**: the guest's 10 ms clock leaves early and sub-tick
latency unresolved, and successful sectors do not establish Linux
retired-byte semantics. Same-image native/musl parity is **NOT RUN**;
the separate native TTY command is a reuse check.
