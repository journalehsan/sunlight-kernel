# Helios BTop-05: masked pselect6 and normal exit

This phase follows [BTop-04](btop-04-interface-discovery.md) and retains
the [BTop-03](btop-03-readiness-network.md) probe. The pinned executable
is unchanged. The masked syscall still explicitly returns EOPNOTSUPP.
No production kernel code was changed.

## Pinned x86-64 caller

The retained link map and disassembly in `target/btop-05/pinned-musl.dis`
identify the linked call sites:

- `pselect` at `0x60e480` calls syscall 270 through musl's cancellation
  wrapper. It copies the optional 16-byte timeout to a private stack
  slot and sends `{mask pointer, 8}` by pointer as syscall argument six
  (r9). btop asks for fd 0 read readiness, with null write/exception
  sets. Its mask is the original main-thread mask saved when blocking
  SIGUSR1; a non-null pointer is passed even when that mask is empty.
- btop uses `signal()` for INT, TSTP, CONT, WINCH, USR1 and USR2. The linked
  `signal` at `0x60e5cc` supplies an empty action mask and SA_RESTART
  (0x10000000). `__libc_sigaction` at `0x61a8a3` adds SA_RESTORER
  (0x04000000), uses the 32-byte `{handler, flags, restorer, mask}`
  layout and invokes syscall 13 with sigsetsize 8. Its one-argument
  handler returns to `0x61aaba`, which invokes syscall 15 (rt_sigreturn).
- The linked `pthread_kill` at `0x614fd6` invokes tkill (200), with the
  target TID and signal number. No tgkill behavior is needed to model
  that wrapper. btop itself uses process-directed `kill(getpid(), SIGUSR1)`
  to interrupt input and SIGUSR2 on Ctrl+R. The replay did not exercise
  every registered handler.

## Why this stops at the probe

- `sys_linux_rt_sigaction` stores the handler, flags and action mask in
  the shared process but drops the restorer, accepts raw handler
  addresses without execute validation, and cannot round-trip the full
  old action. `sys_linux_rt_sigprocmask` operates on per-task state and
  strips unmaskable bits; clone copies the parent mask. Pending state is
  per task. The tkill shim admits only self, and the cooperative send
  path applies foreground/session checks before queueing and waking.
- `deliver_pending_signals` takes only INT, TERM, HUP and CHLD in its
  ordinary scan; it clears a caught instance and logs the stub without
  entering userspace. KILL has a separate forced path. The scheduler
  timer path handles pending fatal signals using the task state but
  does not construct caught-signal contexts. These paths do not amount
  to Linux signal delivery for any of btop's handlers.
- Native C has a separate synchronous signal path (syscall 174):
  it validates executable addresses and the stack write, preserves the
  red zone, and keeps saved GPR/stack/FS/FX state in kernel memory. It
  offers useful context-management primitives but its private trampoline,
  synchronous scope, and kernel-only context are a different ABI.
  Helios has no dispatch for Linux syscall 15. The ordinary syscall
  frame uses RCX and R11 for SYSRET RIP and flags, while a Linux signal
  return context needs independent RIP/RFLAGS/RCX/R11. A safe Linux
  path needs a full user-return context, validated stack and frame
  handling, FP/SIMD preservation, and a return path that can restore
  that context. Directly invoking the native trampoline or reading raw
  user registers into the SYSRET frame would not meet that contract.
- The existing pselect fd scan and registration share SCHEDULER with
  wakeups. A parked syscall rewinds RIP and saves its deadline for
  re-entry. `wake_for_signal` clears the poll wake tick and makes a
  blocked target runnable; it does not carry a caught-signal result.
  Mask swap, selection of a pending caught signal, fd readiness and
  park must be serialized, and the independent pre-wait mask must
  survive a park or unrelated wake. The handler must observe the
  temporary mask plus the action mask and self-blocking; its return
  must restore the pre-wait mask and resume with EINTR. SA_RESTART
  does not restart this wait. A restore-on-syscall-return patch alone
  would restore too early on the parked path; leaving the temporary
  mask active through an unqualified rewind can consume or hide a wake.

A follow-up needs a Linux-specific return-to-user and rt_sigreturn
path, target-thread admission preserving existing permissions, and an
explicit interrupted-wait transition. A guest fixture must observe the
actual park before sending; this guest has no `/proc/self/task/TID/syscall`
observer used by the Linux reference. A narrow scheduler checkpoint
would provide that evidence without extending procfs. No handler flags,
signals or delivery modes were added in this phase.

## Reproduction

Run from the repository root, with the retained BTop-04 image recipe and
artifacts. Use fresh evidence directories. The image recipe traps and
reverses temporary tracing and RAMFS packaging patches. The probe copies
the host ELF before adding the Helios ELF stamp.

```sh
bash tools/btop-05/build-image.sh >target/btop-05/build.log 2>&1
out=target/btop-05
iso=$out/guest-ramfs-trace-config.iso
python3 tools/btop-05/host.py "$out/masked-select.elf"
python3 tools/btop-05/guest.py "$iso" "$out/final-masked"
python3 tools/btop-05/guest.py "$iso" "$out/final-unmasked" --unmasked
python3 tools/btop-04/replay.py "$iso" "$out/final-btop" --mode btop
bash tools/runtime-parity/run.sh "$out/parity"
cargo test -p sunlight-compat-linux --target x86_64-unknown-linux-gnu
cargo test --manifest-path sunlight-tty/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check -p sunlight-kernel --features native_document_viewer
git diff --check
```

The btop wrapper may exit zero after verifying interface reads despite
a nested TTY replay timeout. Evaluate the interactive gate from the
nested replay, exit/reap trace and scanouts.

| Artifact | SHA-256 |
|---|---|
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Static musl reference | `9a3637283a73c9002f73704dfccc95387f8c66091385f70d3ef9636cf928c143` |
| Stamped guest probe | `8e88072d4a982ede9f3e618b6706a8947a58c4d9e1ad52c44a72c7b7889f8552` |
| Guest kernel | `5db67f7545b0d5d2f62d83fa86ffa2855985e783ef8d3a7d18875338bfd360cf` |
| 4 GiB replay ISO | `a0f95b444965b458b35b15876ddcfcc3256e9e4f5b5ca081567d1b27ca338d2a` |

The 4096 MiB, four-vCPU QEMU command lines, serial traces, and
screenshots are under `target/btop-05/`. Runtime parity uses a separate
mixed-ABI image with hashes in `parity/guest/*-sha256.log`.

## Results

| Gate | Result | Evidence |
|---|---|---|
| Linux reference: pending signal, delivery during wait, handler mask and restoration | **PASS** | `host.log`: both calls returned -1/EINTR, both ran the handler with its expected mask, and both restored the caller mask. The sender observed syscall 270 before delivery and checked that its own mask was unchanged. |
| Linux reference: masked timeout, masked TTY readiness and EBADF | **PASS** | `host.log`: timeout returned 0; TTY returned a ready fd and read `x`; EBADF left the fd set unchanged. Each restored the caller mask. |
| Guest pending signal | **FAIL** | `final-masked/serial.log`: syscall 270 returned -95, `caught=0 handler_mask=0 restored=1`. Probe exited 1 normally and was reaped. The observed original mask was never replaced. |
| Guest delivery while waiting, masked timeout, masked readiness, masked EBADF and cross-thread isolation | **NOT RUN** | The strict guest probe stopped at its first failed gate. The implementation required to run these cases remains **BLOCKED**. |
| Guest unmasked TTY readiness and EBADF | **PASS** | `final-unmasked/serial.log`: 200 ms idle wait, stdout writable, EBADF with unchanged set, then coordinated input after traced wait registration. Read one byte `x`; exit 0 and reap. |
| btop panel refresh and guest data | **PASS** | `final-btop/tty/03-dashboard-{0,1,2}.png`: changing CPU values across four CPUs, 3.39 GiB total memory, PID 38 btop, and eth0 at guest DHCP address 10.0.2.42 with 1.21 KiB RX and 958 B TX totals. Worker PID 39 repeatedly read the corresponding procfs and sysfs inputs and completed both rtnetlink dumps. Some text and borders render incompletely in these captures. |
| btop keyboard interaction | **FAIL** | `h` was injected through QMP; `04-help.png` still shows the dashboard with no Help view. Main PID 38 repeatedly returned -95 from syscall 270. |
| btop q exit and reaping | **FAIL** | `q` was injected; the nested replay timed out after 20 seconds and saved `05-no-exit.png`. No exit status or reap was observed for interactive PID 38. The earlier `--version` PID 37 exit and reap are separate. |
| Same TTY after normal btop exit | **NOT RUN** | The replay had no normal interactive exit after which to run a shell command. |
| Runtime parity and focused tests | **PASS** | Separate mixed-ABI guest passed file I/O, TLS, join and syscall routing for musl and native. The kernel feature check, 29 Helios tests, 42 TTY tests and diff check passed. |
| Full boot/storage gate | **NOT RUN** | Outside the focused probe and replay. |

The first packaging attempt linked the older BTop-04 embedded VFS
server. The guest shell reported “command not found” for the new
probes even though the kernel ELF contained their entries. This was
a fixture failure. The corrected recipe embeds the rebuilt VFS server;
the earlier image is retained under `packaging-first/` and the earlier
replays remain separate.

Changed paths: this report and `tools/btop-05/{build-image.sh,guest.py,host.py,masked-select.c}`.
No commit or push was made.
