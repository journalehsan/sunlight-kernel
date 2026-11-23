# Helios BTop-02: telemetry audit and procfs

The acceptance binary is upstream btop v1.4.5 at
`a1099d075fa914452366b28c67ff171e21a1f02e`, with the BTop-01 static musl
configuration and existing Helios ELF stamp. Its SHA-256 is
`bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702`.
No btop source or fixture binary was patched to accept this view. The retained
baseline is `target/btop-01/HANDOFF.md` and
`target/btop-01/ramfs-config-traced/serial.log`: version exit 0, real TTY,
and completed syscall 4 at sequence 590 returned ENOENT for `/proc`.
A parked VFS IPC replay in that log is not a syscall completion.

The GUI Tasks Monitor (`services/sunlight-tasks/src/main.rs`), terminal
`sunlight-top`, `memoryctl`, the TTY/desktop dashboards and niced consume
`sunlight-telemetry`. Its reader maps the versioned kernel page via
`SYS_MAP_TELEMETRY`, validates the magic/version, and retries the sequence
check. There is no userspace telemetry server on this path. The BSP captures
scheduler/PMM state about once a second
(`kernel/src/arch/x86_64/interrupts.rs`), then
`kernel/src/telemetry.rs` publishes raw core counters, task runtime,
present-user-mapping counts, physical accounting, and legacy totals.
The reader derives percentages, grouping and history. The native process
memory additions use `ipc/src/telemetry.rs` v5. The same page also provides
legacy aggregate frame-proxy bytes; those are counted at the kernel's
frame proxy syscalls and have different boundaries than NIC driver counters.
The native hardware counters are already available separately via
`NetInfo` / `NetDeviceInfo` to networkd, net_server, and networkctl.
There is **no native wire schema change**.

| Consumer / fields | When btop needs it | Producer, published data and units | Linux mapping, availability and decision |
|---|---|---|---|
| `/proc` directory type, read access | Hard startup gate in `Shared::init()`; enumeration for process refresh | No existing VFS node | Linux-only virtual directory. Shared file descriptions support stat, read, seek and bounded directory entries. No file copied to RAMFS. |
| `/proc/stat` aggregate and numbered CPU lines | Startup and repeated CPU/process collection | `sched/accounting.rs` seqlock samples cumulative elapsed/busy/halt-idle **ns**. Telemetry v4 publishes raw; Monitor derives rates. | Reuse live bounded producer; divide by 10 ms (USER_HZ=100) and sum rounded per-core rows. Busy is the single measured non-idle total, carried in the system column. User/kernel/nice/IRQ/etc. split is unmeasured; zero placeholders in those columns are not split measurements. CPU snapshot contention/uninitialized sample: EAGAIN. |
| `/proc/uptime` seconds; optional `btime` in `stat` | Uptime is a hard startup dependency and used for process age | BSP boot-relative 100 Hz ticks and per-core halt-idle clocks; RTC base and that same timekeeper for realtime. Raw in kernel, uptime in shared page. | Live decimal seconds and sum of rounded idle times. `btime` uses realtime minus boot-relative seconds, omitted if RTC unavailable. |
| `/proc/meminfo`: MemTotal, MemFree, MemAvailable; SwapTotal/Free, Cached | MemTotal is hard at startup; other fields drive panels | PMM managed/free page counts published as totals and in physical memory accounting; zram logical configured/stored pages distinct from allocator bytes. Native cache categories aren't a Linux page-cache reclaim model. | Live 4 KiB frames → KiB. MemAvailable conservatively equals physically free PMM bytes. Zram totals/free describe logical configured slot space and don't promise compressed capacity; allocator consumption remains in physical accounting. Cached omitted. |
| `/proc/[pid]/{comm,cmdline,status}`: name, argv, UID | Process refresh; read on first discovery by btop | Scheduler live identity and credentials, native telemetry name and group; original exec argv captured only for Linux processes, at most 1 KiB, no environment. | Same-UID Linux leaders further scoped by the existing foreground relationship check. No privileged/native service rows, cross-UID metadata, or telemetry-service identity. Cmdline is the original exec sample; subsequent user changes to their argv are not tracked. `/proc/self` resolves to the caller's TGID; arbitrary `fd`, `exe`, and `environ` links are absent. |
| `/proc/[pid]/stat`: state, PPID, CPU time (14/15), nice and threads (19/20), start (22), RSS (24) | Process refresh | Scheduler lifecycle and `effective_runtime_at`, cumulative ns; worker time retained after reap. Boot ticks at process creation. Native v5 already walks present user mappings approximately once a second and reports mapped bytes. | CPU runtime in 100 Hz ticks in stime (unsplit); start relative to the same boot clock. Only visible parent IDs are reported. RSS in 4 KiB pages, including shared/aliased present mappings with the native walk's semantics. Procfs reuses that existing walk through a kernel-only compact table keyed by PID + full incarnation + address-space generation. Missing, pre-first-sample, or older than 3 seconds: EAGAIN. |
| `/proc/net/dev`; actual btop discovery/counters via `getifaddrs()` and `/sys/class/net/<if>/statistics/{rx,tx}_bytes` | Optional network panel; `/proc/net/dev` is a standard counter probe, **not read by this btop** | Retained VirtioNet/VMXNET3 Ethernet backend, `NetDeviceCounters`, already published by kernel `NetInfo` and consumed by native tooling. | One selected backend is Linux `eth0`, named independently of service restarts. Since device initialization, RX = delivered frames/bytes, TX = submitted frames/bytes, excluding virtio headers; errors use producer error/bad completion counters and RX drops use driver drops. No inferred TX drops or loopback. Untracked fixed-format cells zero. Device removal/no selected device returns ENODEV, lock contention EAGAIN, counters reset if backend is reinitialized. Linux link/address discovery and sysfs are deferred. |
| `/proc/self/mountinfo`, `/proc/filesystems`, `statvfs`, `/sys/dev/block/<major>:<minor>/stat` | Optional disks; pinned source does not use `/proc/diskstats` as fallback | VFS retains mount records; FAT knows volume geometry/allocation but has no unified capacity ABI across the VFS routes. Virtio/USB report individual operation outcomes and geometry; no common cumulative sectors/timing counters. | Deferred: requires authorized mount/capacity and stable device identity across VFS routes, plus actual block accounting before I/O can be shown. No RAMFS/VFS operation counts exported as disk I/O. |
| `cpuinfo`, `sysinfo` load average, frequency/thermal/battery/GPU, `smaps` and per-task I/O | Optional, except where the collector fails to handle an unavailable interface | Native RTC/topology/thermal services are separate; no Linux sysfs topology/sensor schema, load average or equivalent smaps/I/O accounting on this path. | Deferred. No fabricated cache, temperature, frequency, GPU, load or per-process I/O counters. |

Visibility is checked again on reads, fstat, seek and directory enumeration.
Open directory snapshots retain identity and skip rows that exit or become
hidden; reopening obtains a fresh view. Relative lookup is supported for
the proc root and network directory. Relative traversal from a numeric PID fd
returns EOPNOTSUPP; it must not resolve a new exec image behind a stale fd.
File payloads stay stable for short reads and dup shares the offset. Reopen
to resample. A sampled process file still becomes inaccessible after exit or
image replacement. The implementation caps 128 open descriptions, 128 PID
entries per listing, 512 task slots for lookup and up to 8 KiB per record;
overflows return an error. RSS reuses the first 64 native rows. No VFS or
scheduler lock is held across IPC.

Stock btop uses pselect for keyboard input. This is independent of procfs
and wasn't reachable in BTop-01. A passing ELF load, successful procfs
opens, a dashboard frame and successful collectors are separate observations.

## Evidence and replay

The final fixture boot is BIOS/TCG, 4096 MiB, 4 vCPUs, virtio VGA, authenticated
framebuffer console, real PS/2 events, 138×38 terminal. The tracked tree does
not embed the acceptance payload or enable syscall tracing. The fixture build
temporarily packages the pinned binaries in both existing boot filesystem
views and adds bounded trace instrumentation. It undoes these patches on exit.

```sh
bash tools/btop-02/build-replay-image.sh
python3 tools/btop-02/replay.py target/btop-02/guest-ramfs-trace-config.iso target/btop-02/tty-replay
python3 tools/btop-02/replay.py target/btop-02/guest-ramfs-trace-config.iso target/btop-02/net-replay --network
python3 tools/btop-02/verify-network.py target/btop-02/net-replay
python3 tools/btop-02/replay.py target/btop-02/guest-ramfs-trace-config.iso target/btop-02/tasks-replay --tasks-monitor
bash tools/btop-02/probe-run.sh target/btop-02/probe-replay
bash tools/runtime-parity/run.sh target/btop-02/parity-replay
cargo check -p sunlight-kernel --features native_document_viewer
cargo test -p sunlight-telemetry -p sunlight-net -p sunlight-block -p sunlight-fat --target x86_64-unknown-linux-gnu
cargo test -p sunlight-tasks --target x86_64-unknown-linux-gnu
objdump -d --start-address=0x60dc62 --stop-address=0x60dc7b target/btop-01/btop
sha256sum -c target/btop-02/final.sha256
```

| Acceptance area | Result | Witness |
|---|---|---|
| Startup | PASS (initial frame) | Version exit 0; plain btop passes directory stat/read access, opens CPU/memory/uptime, and draws initial panel chrome. The frame is not a complete dashboard. |
| Refresh | FAIL | Two collector attempts reopen CPU/memory, then fault before completing dashboard draws. Captures at three intervals are not three successful refreshes. |
| Keyboard / normal exit | FAIL | `h`, Esc and `q` sent; `pselect6` (270) is unsupported. Application exits 1; PID 38 is reaped, then `echo btopafter`, the musl probe, and the native reference exit 0 on the same TTY. |
| CPU | PASS in procfs; BLOCKED at btop display | Four sequential rows, aggregate equals the sum of rows, increasing ticks consistent with guest uptime, btop opens the file in both worker attempts. |
| Memory | PASS in procfs; BLOCKED at btop display | Isolated Linux probe and native v5 mapping agree at 4,103,456 KiB total and 4,051,724 KiB free. TTY probe and native map agree at 3,560,404 KiB total; free moves between samples. btop opens meminfo; no validated populated panel. |
| Processes | PASS in procfs; BLOCKED in btop | UID 1000 fixture PID 100 enumerates itself, checks read-only and hidden PID 1, gets EAGAIN before its RSS publication, then reads 536 RSS pages and increasing CPU ticks. It is reaped. PID 101 sees PID 100 absent through direct lookup and enumeration. Old btop PID 38 is absent from the later TTY probe. Forced stale RSS, live cross-UID Linux pair and synthetic PID reuse are NOT RUN. |
| Network | PASS in procfs; BLOCKED in btop | Restricted QEMU network and a ping to the local QEMU gateway: four ICMP pairs, one ARP exchange and two background outbound datagrams. Exact before/after pcap prefixes: RX 1,244 B / 3 frames → 1,700 B / 8; TX 810 B / 5 → 1,392 B / 12. No Internet needed. |
| Storage | BLOCKED | No mount/capacity ABI or device counters in this change. The collector logs its missing mountinfo path. A disposable block-I/O run is NOT RUN. |

The first missing syscall after the procfs gate is `sysinfo` (99, used
by musl `getloadavg()`); btop continues. The fatal collector sequence is
`socket(AF_NETLINK, SOCK_RAW|SOCK_CLOEXEC, 0) = -EAFNOSUPPORT`, then
fault address 0x8 with RIP 0x60dc68. The original link map locates that
instruction in musl `freeifaddrs`. The pinned `IfAddrsPtr` destructor
unconditionally passes its member to `freeifaddrs`, even when
`getifaddrs` returns an error. Neither inventing `/proc/net/dev` rows
nor populating mountinfo would resolve that missing discovery path.
Linux route-netlink discovery and atomic signal-mask waits are separate
compatibility work.

Evidence lives under `target/btop-02`. Use `tty-verified/serial.log` and
`tty-verified/03-dashboard-{0,1,2}.png` for startup and refresh attempts;
`tty-verified/04-help.png` and `tty-verified/05-after.png` for keyboard
and console reuse. `net-verified/{serial.log,network.pcap,counter-check.log,
02-network.png}` retain the independent network comparison. The isolated
probe's exact input and output are `probe-final-cases.json`,
`probe-verified.log`, and `probe-verified/serial.log`; the exact replay
uses `python3 tools/native-c/guest.py --cases target/btop-02/probe-final-cases.json --mixed-abi --memory-mb 4096 --cpus 4 --timeout 120 --out target/btop-02/probe-verified`
with a new evidence directory. Initial failed btop and probe attempts
remain under their original directories. Each QEMU run records
`command.json`.

`parity-final/run.log` records both native and musl passing file/TLS/join
checks and the ABI-routing assertions. The telemetry, network driver,
block and FAT host suites passed 120 tests; Tasks Monitor passed 13 host
tests. Native Tasks Monitor also opened from desktop search on the final
image. `tasks-desktop/serial.log` records PID 39 and the window-visible
event; `tasks-desktop/02-tasks-monitor.png` and `03-tasks-monitor.png` show
its populated system overview. Host test builds of networkd and net_server cannot use the std test
runner because their native `panic_impl` conflicts with std. The network
services did run in the isolated QEMU proof.

The final ISO SHA-256 is
`0de6757f291038a58b47f4685bc9d9ab2a347844bede83cff248a09d2d969405`.
`target/btop-02/final.sha256` contains the kernel and both probe hashes as
well as the ISO and unchanged btop hashes.

The production changes are in `kernel/src/linux_procfs.rs`,
`kernel/src/arch/x86_64/{syscall.rs,syscall_procfs.rs}`,
`kernel/src/process/{fd_table.rs,mod.rs,spawn.rs}`,
`kernel/src/sched/mod.rs`, `kernel/src/telemetry.rs`, and the module
registration in `kernel/src/main.rs`. The probes, image build/replay
scripts, and counter verifier are in `tools/btop-02/`. The pre-existing
runtime parity worktree changes were preserved. Nothing was committed or
pushed.
