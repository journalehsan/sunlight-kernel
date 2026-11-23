# Helios BTop-03: TTY readiness and interface discovery

BTop-03 is partial. The real guest TTY passed unmasked `pselect6`; the pinned
btop still has two independently reproduced blockers. No baseline files or
pinned binary were changed. No commits or pushes were made.

## TTY readiness

The x86-64 Linux dispatcher handles syscall 270, including the argument
package in r9. The wait uses the existing Linux poll timer and producer
wakeups, with readiness and wait registration under the same scheduler lock.
A wake rechecks readiness against the original deadline. Output bitmaps are
written on completion and the result counts ready bits across all three sets.
The implementation is bounded to 1024 descriptors and uses the existing
descriptor readiness model. Regular-file readiness, broad descriptor parity,
and caught-signal interruption of an unmasked wait were not verified.

With a null mask pointer, including a valid r9 package containing a null mask,
the call can proceed. A malformed r9 package fails with EFAULT. A valid
non-null mask is explicitly rejected with EOPNOTSUPP. The existing signal
state can track blocked signals, but the user-handler arm of
`deliver_pending_signals` does not construct a signal frame or run the
handler. Atomic mask replacement, caught-signal interruption, mask
restoration and sigreturn need to fit together. The pinned btop
`src/btop_input.cpp:107` always passes `&signal_mask`, even though its mask
is empty. The guest cannot accept this as an unmasked call: an empty
replacement would unblock signals blocked in the caller.

| Probe | Linux PTY | Guest PS/2 framebuffer TTY |
|---|---|---|
| Idle 200 ms | PASS: 200 ms, return 0, read/exception sets clear | PASS: 230 ms, same sets and return |
| Stdout write set | PASS: return 1, stdout set | PASS: same |
| Closed selected fd | PASS: EBADF, input bitmap unchanged | PASS: same |
| Blocking key wait | PASS: 500 ms, read set only, byte 120 | PASS: 550 ms, same sets and byte; live trace shows park and retry returning 1 |
| Raw sixth argument | PASS: null inner mask accepted, invalid outer pointer EFAULT | PASS: same |
| Pending SIGINT, temporary empty mask | PASS: EINTR, handler ran, old mask restored | BLOCKED: EOPNOTSUPP; probe checks no handler and original mask intact |

The guest probe exited 0 and was reaped. Its stdout is captured by the TTY
server at exit and the serial command summary truncates the last fields.
The raw syscall trace retains 0, -14 and -95 for the three sixth-argument
cases. The initial harness attempt waited on a stdout marker that was only
published at exit; that run timed out without injecting a key and is retained
as `target/btop-03/probes`. The corrected run is `probes-key`.

## Interface discovery

The independent probe uses the same Alpine image, GCC 15.2.0-r2 and musl
1.2.5-r24 as the pinned btop. It initializes the result pointer to a valid
sentinel and frees the returned list only on success. Linux returned 0,
listed interfaces and cleaned up the returned list. Forcing socket allocation
failure with RLIMIT_NOFILE returned -1/EMFILE and left the sentinel unchanged.
The guest returned -1/EAFNOSUPPORT and also left the sentinel unchanged. The
probe took its ordinary failure exit 1, without a fault.

Disassembly of the matching musl probe shows `getifaddrs` calling
`__rtnetlink_enumerate`: it opens `socket(16, 0x80003, 0)`, sends a 20-byte
request and receives replies for RTM_GETLINK (18), then RTM_GETADDR (22). On
failure it does not store to the caller's output pointer. In the pinned
btop, `IfAddrsPtr` has an uninitialized pointer member, passes its address to
`getifaddrs`, and calls `freeifaddrs` unconditionally in its destructor. The
guest trace records socket result -97 and then RIP 0x60dc68 in
`freeifaddrs` (link map and disassembly). The missing discovery socket exposes
the caller's invalid cleanup; the independent failure is safe when the caller
checks the result. The guest did not reach either netlink request.

Read-only route-netlink discovery remains unimplemented. The kernel's
selected frame backend backs the existing `/proc/net/dev` counters and has
MAC/link metadata. The executing IPv4 lease is owned by
`services/net_server/src/main.rs` in `LIVE_CONFIG`, exported to existing
native consumers via `GETIP_LIVE`. A complete bounded dump needs an
authorized, lifetime-aware read path to that service or a validated
publication tied to the selected backend; a hardware-only dump must not
claim no live address. The restricted fixture actually acquired
10.0.2.15/24. The pinned btop source would use
`/sys/class/net/<if>/statistics/{rx,tx}_bytes` and fall back to
`/sys/class/net/<if>/address` if there were no IP. Those reads were not
reached in the guest. No sysfs or network mutation operations were added.

## Replay and regression results

The fixture uses BIOS/TCG, 4 vCPUs, 4096 MiB, virtio VGA, and authenticated
TTY. The pinned binary SHA-256, verified before replay, is
`bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702`.
The replay ISO SHA-256 is
`2a7823fee7c33420612f774710ccfd2fdde42360cb884a2aab8bf28457160736`.
The kernel and both static ELF hashes are in `target/btop-03/final.sha256`.

| Gate | Result | Evidence |
|---|---|---|
| btop startup | PASS | Version exits 0; initial panel chrome renders. |
| Two completed refreshes | FAIL | Worker reaches procfs, route-netlink socket fails, and the worker faults in cleanup. Dashboard captures do not establish completed refreshes. |
| Harmless key, q, exit 0 | FAIL | The replay injects h, Escape and q. Masked waits return -95; the process exits 1. |
| Reaping and TTY reuse | PASS | PID 38 is reaped; the procfs and native telemetry probes run on the same TTY and exit 0. |
| CPU | PASS in procfs, BLOCKED in btop | Four guest core rows; aggregate ticks advance 21847→22195 in the post-btop probe. |
| Memory | PASS in backing counters, BLOCKED in btop | Both Linux procfs and native telemetry report 3,558,800 KiB total. |
| Processes | PASS in procfs, BLOCKED in btop | PID 40 sees itself and does not see reaped PID 38. Fixture process appearance/disappearance inside the btop panel was NOT RUN. |
| Network | PASS in procfs, BLOCKED in btop | Restricted QEMU gateway ping completed four echo pairs. Pcap prefixes match `eth0`: RX 1244 B / 3 frames → 1700 B / 8; TX 810 B / 5 → 1392 B / 12. No guest btop interface or displayed activity was observed. |
| Runtime parity | PASS | Static musl and native file/TLS/join, including ABI routing. |
| Host regressions | PASS | 29 Helios, 31 network, and 12 telemetry unit tests. One telemetry doc test ignored. |
| Full boot gate and storage | NOT RUN | Focused boot/replay used the fixture ISO. Storage stays out of scope. |

Commands, from the repository root:

```sh
bash tools/btop-03/build-replay-image.sh
sha256sum -c target/btop-03/final.sha256
python3 tools/btop-03/host-probes.py
python3 tools/btop-03/probes.py target/btop-03/guest-ramfs-trace-config.iso target/btop-03/probes-key
python3 tools/btop-02/replay.py target/btop-03/guest-ramfs-trace-config.iso target/btop-03/btop
python3 tools/btop-02/replay.py target/btop-03/guest-ramfs-trace-config.iso target/btop-03/net --network
python3 tools/btop-02/verify-network.py target/btop-03/net
bash tools/runtime-parity/run.sh target/btop-03/parity
cargo check -p sunlight-kernel --features native_document_viewer
cargo test -p sunlight-compat-linux -p sunlight-net -p sunlight-telemetry --target x86_64-unknown-linux-gnu
objdump -d --start-address=0x60dc62 --stop-address=0x60dc7b target/btop-01/btop
for sym in getifaddrs __rtnetlink_enumerate __netlink_enumerate; do
    objdump -d --disassemble="$sym" target/btop-03/ifaddrs.elf
done
```

Use fresh output directories when repeating a QEMU run. Evidence under
`target/btop-03` includes `host.log`, `musl-netlink.dis`,
`btop-freeifaddrs.dis`, `probes-key/serial.log` (seq. 1–10),
`btop/serial.log` (masked select at seq. 18; socket failure at seq. 108 and
nearby fault), `btop/03-dashboard-*.png`, `btop/05-after.png`,
`net/{serial.log,network.pcap,counter-check.log}`, and the parity and host
test logs. Each guest directory has its own QEMU command record.
