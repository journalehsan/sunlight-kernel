# Helios BTop-06: atomic masked pselect6 and interactive btop

This follows [BTop-05](btop-05-masked-pselect.md) and
[SIG-01](helios-sig01.md). Earlier evidence is intact. The pinned executable
was verified and packaged unchanged. Nothing was committed or pushed.

## Wait and return invariant

Linux x86-64 syscall 270 takes a pointer to two 64-bit words in r9: a
sigset pointer and its byte size. A null descriptor or null sigset pointer
is unmasked; otherwise size must be 8 and the full mask must be readable.
The kernel validates this before installing the mask, clears SIGKILL and
SIGSTOP from the installed mask, and does not retain a userspace pointer
to the sigset across retries.

TTY readiness producers, the fd scan and registration, and caught
thread-directed signal enqueue share the scheduler lock. A supported
signal that queues first is visible to pending selection after installing
the temporary mask. If registration wins, the sender both enqueues
and makes an eligible waiter runnable under that lock; enqueue retains a
coalesced pending instance even if the thread is already runnable. The
remote path uses the existing ready-queue and reschedule notification
mechanism. A blocked signal remains queued without waking the wait.

The parked state carries the pre-wait and temporary masks separately
from the deadline. The existing park IPI and syscall rewind are used for
replay; the userspace instruction at the rewind is SYSCALL again. The
ring-3 timer hook skips a thread with a registered masked continuation,
including the window before replay. Replay consumes the registration and
checks pending before revalidating fd arguments. If no signal wins, it
restores the original mask while processing argument errors, then applies
the same saved temporary value in the next locked check and re-registers
if needed. A send during retry validation cannot be lost: target IRQs
are masked, enqueue retains pending, and the next check precedes sleep.
Timeout, readiness and errors clear the registration and deadline and
restore the original mask; readiness and signal wake clear the poll
wake tick, and replay clears it defensively. The thread state is private,
initialized on creation and reset on exec. Finished tasks cannot resume
the continuation.

For interruption, Helios converts the actual syscall register save into
the same 15-GPR and five-word frame SIG-01 uses. It preserves SYSCALL's
architectural RCX/R11 clobbers, sets RAX to `-EINTR`, and uses the RIP
*after* SYSCALL. The validated SIG-01 frame builder sets the live handler
mask to the temporary mask plus the action mask and the delivered signal.
The saved ucontext mask is the original caller mask, which the validated
`rt_sigreturn` path restores when the handler leaves it intact. Handler
entry and sigreturn use IRETQ with IRQs still masked in kernel code.
EINTR cannot become visible to the caller before the handler has run.
Normal syscall returns keep the existing SYSRETQ path.

The admitted caught routes are same-address-space, same-group Linux
`tkill` for SIGINT and SIGUSR1, subject to the existing foreground
authorization and SIG-01 action checks. There is one frame per thread.
Masked wait with an alternate stack configured or an active signal frame
returns EOPNOTSUPP. Process-directed caught delivery, tgkill, arbitrary
signals and action flags, nested frames, alternate stacks, XSAVE,
SA_RESTART syscall restart, and raw timeout writeback on interrupted
pselect6 are outside this contract. SA_RESTART is accepted as action
metadata. This change does not implement masked ppoll or promise Linux
ordering when fd readiness and a caught signal become observable together.

The opt-in checkpoint (`SUNLIGHT_BTOP06_WAIT_TRACE=1`) lets the original
fixture observe `/proc/self/task/TID/syscall`. The node is live and
rechecks both the Linux thread-group and VM identity; it reports 270 only
while the target is blocked with a masked wait registered. This is a
test observer and does not expose registers, arguments, or general task
procfs support. It is absent from normal builds. The image recipe
temporarily pins only the two masked-select fixtures: the waiter to vCPU 0
and the sender to vCPU 1. The same image runs btop and the regressions
without this placement override for those binaries.

The first btop replay on the masked-wait image refreshed but did not
complete Help or quit. Its linked input loop sets VMIN=0 and VTIME=0 and
drains stdin with repeated read calls. Helios was parking on an empty
noncanonical read. The fix returns zero for that immediate empty read,
leaving nonblocking fd behavior and canonical reads as before. It does
not add general timed termios reads.

## Reproduction

From the repository root, with the retained BTop-04 recipe and BTop-03,
BTop-05 and SIG-01 fixture artifacts:

```sh
out=target/btop-06
mkdir -p "$out"
bash tools/btop-06/build-image.sh > "$out/build-placement.log" 2>&1
iso=$out/guest-ramfs-trace-config.iso
sha256sum --check "$out/artifacts.sha256"
python3 tools/btop-05/host.py target/btop-05/masked-select.elf > "$out/host-original.log"
python3 tools/btop-05/host.py "$out/masked-select.elf" > "$out/host.log"
python3 tools/btop-06/guest.py "$iso" "$out/masked-placement" > "$out/masked-placement.log" 2>&1
python3 tools/btop-06/guest.py "$iso" "$out/btop-placement" --btop > "$out/btop-placement.log" 2>&1
python3 tools/helios-sig01/guest.py "$iso" "$out/sig01-placement" > "$out/sig01-placement.log" 2>&1
python3 tools/btop-05/guest.py "$iso" "$out/unmasked-placement" --unmasked > "$out/unmasked-placement.log" 2>&1
bash tools/runtime-parity/run.sh "$out/parity-final" > "$out/parity-final.log" 2>&1
cargo test -p sunlight-compat-linux --target x86_64-unknown-linux-gnu
cargo test --manifest-path sunlight-tty/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check -p sunlight-kernel --features native_document_viewer
git diff --check
```

Changed paths: `kernel/src/arch/x86_64/{linux_signal,syscall,syscall_procfs}.rs`,
`kernel/src/linux_procfs.rs`, `kernel/src/process/signal.rs`,
`tools/btop-06/{build-image.sh,guest.py,masked-select.c}`, and this report.
The packaging and startup trace patches are reversed by the image recipe.

## Artifacts

The guest used TCG, `-cpu max -m 4096M -smp 4`. Each guest directory
under `target/btop-06/` contains `command.json` and `serial.log`.
The btop replay also records its effective QEMU command with the isolated
SLIRP/VirtIO Ethernet device in `network-command.json` and its packets in
`network.pcap`. Runtime parity uses its own mixed-ABI 4 GiB image.

| Artifact | SHA-256 |
|---|---|
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Original BTop-05 Linux fixture | `9a3637283a73c9002f73704dfccc95387f8c66091385f70d3ef9636cf928c143` |
| Original BTop-05 guest fixture | `8e88072d4a982ede9f3e618b6706a8947a58c4d9e1ad52c44a72c7b7889f8552` |
| Extended Linux fixture | `2976cbc632e8f64dde198fa41b2e3e97fc5c1d50456c2cadc17a41e9b5446237` |
| Extended guest fixture | `1b5e841394c2c233ac2dae29b0d1e51dcf6ec2b5863de2e8038d9821bc635c33` |
| SIG-01 guest fixture | `9d4fb990072b670add5bbe6426fb8e425752cccf2f82ad4697f3836d7ebaf324` |
| Unmasked TTY guest fixture | `1a8ce92ddfd745424091d0a0ddd6e6e8ac2ecede5007dd91d16d72046d31929e` |
| Final guest kernel | `5e86dbae0cfa56625be3520ab4d90caf59c152b3a3c4f8ddcb5e55f636863841` |
| Final btop / signal guest ISO | `2e1fe7b5d11fe9c2c1135437f1cc20e36dedb87637885c8e1906a6a69fa0ee65` |
| Mixed-ABI parity kernel | `022c3697ecedbcbef43937b19084e951ae047fa2e33ddffdf68eefc50a3468a6` |
| Mixed-ABI parity ISO | `15356279c18c54fd72aac5a19c2e2586276dee125fa0dd67372e3d2783131988` |

## Verification

The extended fixture snapshots the handler's count and TID immediately
after each interrupted call, before querying the restored mask. The
caller blocks INT and USR1. The temporary mask blocks TERM; the action
adds USR2 and normal self-blocking adds INT. The handler therefore sees
INT, TERM and USR2 with USR1 clear, and the caller sees INT and USR1 with
TERM and USR2 clear. The sibling verifies its inherited mask separately.
One sibling sends before registration; another observes syscall 270 and
sends while the target is blocked. Linux uses its own procfs observer.

From `masked-placement/serial.log` (separate gates condensed):

```text
[PSELECT] enqueue tid=39 sender=40 cpu=1 state=Running registered=false wake=false
[SIG01] pselect-delivery tid=39 rip=0x40692c flags=0x212 rsp=0x7fffffffe9e0 rcx=0x40692c r11=0x212
[SIG01] sigreturn tid=39 rip=0x40692c flags=0x212 rsp=0x7fffffffe9e0 rcx=0x40692c r11=0x212
[PSELECT] park tid=39 cpu=0 nfds=0
[PSELECT] enqueue tid=39 sender=41 cpu=1 state=BlockedOnTimer registered=true wake=true
[SIG01] pselect-delivery tid=39 rip=0x40692c flags=0x212 rsp=0x7fffffffe9e0 rcx=0x40692c r11=0x212
[SIG01] sigreturn tid=39 rip=0x40692c flags=0x212 rsp=0x7fffffffe9e0 rcx=0x40692c r11=0x212
[TTY]  exit: /bin/masked-select -> 0
```

| Gate | Result | Evidence under `target/btop-06/` |
|---|---|---|
| Original and extended static musl fixtures on Linux | **PASS** | `host-original.log`, `host.log`: pending and blocked sends yield EINTR; the extended fixture also checks exactly one handler on the intended TID, independent masks, sibling enqueue before registration, and empty noncanonical read returning zero. |
| Original BTop-05 fixture on final guest | **PASS** | `masked-placement/serial.log`: target 37, sender 38; two pselect deliveries and two sigreturns; normal exit 0 and reap. |
| Extended fixture and both sides of wait registration | **PASS** | `masked-placement/serial.log`: target 39; one self-pending delivery, one pre-registration sibling send (TID 40), one blocked sibling send (TID 41). All three take the pselect IRET boundary before EINTR, with handler/count/TID and restored-mask checks required for exit 0. |
| Cross-vCPU wake | **PASS** | `masked-placement.log`: in both fixtures, the waiter parked on vCPU 0 and the sender enqueued from vCPU 1 while the target was `BlockedOnTimer` and registered. This establishes cross-vCPU publication and completion, without asserting that an already descheduled target requires an IPI to its former core. |
| Masked timeout, real TTY, EBADF, malformed descriptor/mask | **PASS** | `masked-placement/serial.log`: timeout returns 0; QMP key `x` after registration makes pselect return 1; fixture requires read byte 120, empty noncanonical read returning 0, EBADF with unchanged fd set, and EFAULT/EINVAL/EFAULT/success for unreadable descriptor/wrong size/unreadable set/null set. Original mask checked on each path. |
| SIG-01 and unmasked readiness | **PASS** | `sig01-placement.log`: timer round trip and nine rejection cases; `unmasked-placement/serial.log`: idle timeout, writable stdout, EBADF, QMP key and exit 0/reap. |
| Mixed native/musl parity | **PASS** | `parity-final.log`: file I/O, TLS, joins, and trace verification of per-thread ABI routing. |
| Focused host and build checks | **PASS** | `compat-final.log` (29 tests), `tty-final.log` (42 tests), `check-final.log`, `diff-final.log`, and final artifact hash verification. |
| btop panel refresh on final guest | **PASS** | `btop-placement/dashboard-{0,1,2}.png` and OCR show CPU readings 70%, 69% and 71%, 3.39 GiB RAM, PID 37 btop and Ethernet at 10.0.2.42 with 1.21 KiB RX and 958 bytes TX. Worker TID 38 repeatedly opened guest CPU, memory, process and Ethernet counters and received both rtnetlink dumps. Some borders and characters are incomplete. |
| btop Help, return, q, reap and same TTY | **PASS** | QMP `h` opened Help (`help.png`, OCR contains “Description:” and “Quits program.”); a second `h` closed it (`dashboard-return-0.png`). QMP `q` then produced exit 0. The worker and leader were reaped, and `/bin/proc-native` exited 0 on the same TTY (`after.png`). |
| First interactive replay | **FAIL**, superseded | `btop/` and `btop.log`: refreshed but Help could not open and q timed out. The next replay, `btop-final/`, opened Help after the noncanonical read fix, but the Esc sequence did not produce a verified return to the dashboard before q. The final replay uses `h`, which btop binds to close Help, and checks the dashboard before sending q. |
| Unsupported signal semantics | **BLOCKED** | The existing limits listed above remain. |
| Full boot/storage gate, alternate stack or active-handler masked-wait rejection probes | **NOT RUN** | Outside the focused QEMU and host gates. |


The final replay recorded these lines in `btop-placement/serial.log`:

```text
[SCHED] process_reaped pid=38 name='btop' user_frames=0 page_tables=0 swap_blocks=0 ipc_cleared=0
[SCHED] process_reaped pid=37 name='btop' user_frames=1361 page_tables=14 swap_blocks=0 ipc_cleared=0
[TTY]  exit: /bin/btop -> 0
[TTY]  exit: /bin/proc-native -> 0
```
