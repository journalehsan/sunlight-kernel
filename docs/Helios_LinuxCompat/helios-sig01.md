# Helios SIG-01: x86-64 caught SIGUSR1 and rt_sigreturn

This phase follows [BTop-05](btop-05-masked-pselect.md). It does not change
syscall 270 or its EOPNOTSUPP result for a non-null mask. It uses the
unchanged pinned btop as a packaging input; the signal fixture is a
separately linked static musl executable.

## Return paths and scope

- `syscall_entry` uses FMASK and a two-SWAPGS pivot, saves 15 GPRs on
  the current task's supervisor stack, and saves user RSP at byte 120.
  SYSCALL has already replaced RCX with the return RIP and R11 with user
  RFLAGS. The ordinary return stores the dispatch result in saved RAX,
  pops the GPRs, sets IF in R11 and executes SYSRETQ from the saved user
  RSP. Park/retry and native C synchronous signals use that syscall frame;
  it cannot supply RCX/R11 from before the SYSCALL. Its register layout
  and ordinary return instructions are unchanged.
- `timer_entry` and `reschedule_entry` have full GPR saves followed by
  the hardware RIP/CS/RFLAGS/RSP/SS frame. Their paths restore a live or
  scheduler-selected context with IRETQ. SIG-01 only checks the current
  running task's ring-3 **LAPIC timer** frame, under SCHEDULER, before
  switching the FP owner or address space. Both raw IRQ stubs clear
  live DF before calling Rust without changing the saved flags. No
  delivery is added on the
  reschedule IPI, syscall return, exec/startup, ordinary IRQ/exception
  return, fault, or parked syscall path.
- Scheduler bootstrap and exec already use `iretq_to_context` with that
  same 20-word layout. SIG-01 reuses it for a validated supervisor-stack
  frame. Unlike the no-GPR bootstrap helper `jump_to_userspace`, this
  restores all 15 GPRs. IRQs remain masked until the IRETQ restores IF.
  Neither native C's private trampoline nor ordinary SYSRET is used for
  sigreturn.

The admitted case is caught SIGUSR1 sent by tkill to self or to a Linux
sibling with the same group and address-space identity, after existing
foreground authorization. Pending state is per thread and coalesces.
The disposition and restorer are shared. The caught path accepts
SA_RESTORER and optional SA_SIGINFO/SA_RESTART; other action modes do
not enter this delivery path. At the timer boundary, an
unblocked pending signal enters the normal user stack once; there is at
most one active frame per thread. The handler runs with the interrupted
mask plus the action mask and SIGUSR1. There is no signal-induced wake,
syscall interruption/restart, process-directed caught delivery, arbitrary
boundary delivery, nested frame support, alternate stack support, or
XSAVE support. SA_RESTART is accepted as action metadata and has no
wait-restart semantics in this scope. TLS/FS base is not a field of the
supported frame.

## Frame and validation

The fixture retrieves the action with raw rt_sigaction after musl registers
it: four 64-bit words (handler, flags, restorer, 8-byte mask).
`__restore_rt` invokes syscall 15 after the handler's RET has popped the
restorer address (fixture address `0x4023bb`). The Linux wire prefix contains the restorer address,
ucontext (including the 256-byte sigcontext and 8-byte signal mask), and
128-byte siginfo. The separate 64-byte-aligned FP image uses legacy 512-byte
FXSAVE; the handler's stack is 8 mod 16 and leaves the interrupted 128-byte
red zone untouched. The frame records the actual SunlightOS user CS/SS;
the Linux host uses its own selectors and exposed an extended XSAVE image.
The handler starts with reset FP state and with DF and RF cleared.
Sigreturn may restore RF from the interrupted timer frame; its transient
value need not remain set after the first resumed instruction.

The entry address and restorer must be executable user mappings. Delivery
checks the interrupted selectors/flags and validates the complete write
range before publishing an active frame, changing the mask, or rewriting
the timer return context. Sigreturn requires the current thread's active
frame and the expected post-RET RSP. It copies the frame into kernel
storage, checks the supported ucontext/stack mode, fixed ring-3 selectors,
allowed flags with IF set, executable user RIP and writable user stack,
and checks the FP pointer and all 512 image bytes against the saved
snapshot. FP edits are unsupported. FXRSTOR consumes only the trusted
FXSAVE image. Mask, FP, and GPR restoration occur only after validation;
invalid state exits with status 139. The existing mask setter clears
SIGKILL and SIGSTOP. Neither a user-supplied selector nor a user-supplied
FP image is fed to privileged restore instructions.

## Reproduction

Run from the repository root with the retained BTop-04 recipe and verified
artifacts. The build script packages a stamped copy of the same ELF it
runs on Linux; it reverses the temporary RAMFS and trace patches on exit.
SIG-01 serial traces are opt-in with `SUNLIGHT_HELIOS_SIG01_TRACE=1`.

```sh
out=target/helios-sig01
mkdir -p "$out"
bash tools/helios-sig01/build-image.sh > "$out/build.log" 2>&1
iso=$out/guest-ramfs-trace-config.iso
python3 tools/helios-sig01/guest.py "$iso" "$out/guest"
python3 tools/btop-05/guest.py "$iso" "$out/unmasked-verified" --unmasked
bash tools/runtime-parity/run.sh "$out/parity-verified"
cargo test -p sunlight-compat-linux --target x86_64-unknown-linux-gnu
cargo test --manifest-path sunlight-tty/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check -p sunlight-kernel --features native_document_viewer
git diff --check
```


The build script records the musl link map/disassembly and runs
`timeout 8 target/helios-sig01/sig01.elf`. The guest invocation records
its full QEMU command in `guest/command.json` (TCG, `-cpu max`,
`-m 4096M -smp 4`) and raw serial in `guest/serial.log`. The ELF stamp
changes only seven header bytes (OSABI plus six padding bytes); the
instructions are identical.

| Artifact | SHA-256 |
|---|---|
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Static musl fixture | `fc40262ed326a38a0c66713b48b70175aa153d48f78a145cb102c50c0404d396` |
| Stamped guest fixture | `9d4fb990072b670add5bbe6426fb8e425752cccf2f82ad4697f3836d7ebaf324` |
| Guest kernel | `6b5aded3eb45ac3c34dc97fee823a90e17ed7bbf77008fdd26fc3f6474561914` |
| 4 GiB signal/TTY image | `fe4973a13e33279f2bd4a4b647f70af5f80fa86d974e4c3de1a9c082168b3f52` |

## Verification

In the positive run the sender waits for a handshake from the assembly
loop, then sends SIGUSR1 to the main TID. The loop has no syscall and
sets RCX and R11 to distinct 64-bit values, keeps CF and DF set, and
holds an XMM15 value and a stack red-zone canary. Its indirect poll branch
leaves flags unchanged. The real musl handler observes the saved machine
context and mask, deliberately clobbers XMM15, and returns through the
linked restorer. The resumed fixture checks the flags (excluding the
transient RF), both GPRs, stack pointer, XMM15 and the canary. It also
joins the sender and checks that the sender's mask stayed unchanged.

Final guest serial, shortened to the relevant lines:

```text
[SIG01] timer-delivery tid=37 rip=0x401c09 flags=0x10647 rsp=0x7fffffffeb88 rcx=0x123456789abcdef0 r11=0xfedcba9876543210
[SIG01] sigreturn tid=37 rip=0x401c09 flags=0x10647 rsp=0x7fffffffeb88 rcx=0x123456789abcdef0 r11=0xfedcba9876543210
SIG01 PASS count=1 target=37 handler=37 sender=38 masks=800/a02/800/800 action=14000004 restorer=4023bb uc=6 selectors=2300000000002b fp=0
[TTY]  exit: /bin/sig01 -> 0
```

| Gate | Result | Evidence under `target/helios-sig01/` |
|---|---|---|
| Static musl reference on Linux | **PASS** | `linux.log`: exactly one handler on the target TID; masks `800/a02/800/800`; independent RCX/R11 restored; interrupted and resumed flags both `0x647`; XMM15, RSP and red zone restored. The host advertised `uc_flags=7` and FP XSAVE magic `0x46505853`. |
| Same fixture, final 4 GiB guest | **PASS** | `guest/serial.log`, `guest.log`, and guest capture: TID 37 handled one signal sent by TID 38; same four masks as Linux. Matched timer and IRETQ inputs include RIP, RFLAGS, RCX, R11 and RSP. Fixture exit 0 also requires resumed flags, stack, XMM15 and red zone to match. |
| Guest frame validation | **PASS** | Eight mutated frames (selector, IOPL, unmapped RIP, noncanonical RIP, invalid RSP, FP pointer, XSAVE marker in legacy FP image, altstack metadata) and a sigreturn with no active frame all exited 139. Each emitted its armed marker; no second successful sigreturn, fault trace, or panic was observed. |
| Native/musl runtime parity | **PASS** | `parity-verified.log`: separate 4 GiB mixed-ABI guest, file I/O, TLS, joins and per-thread ABI routing. ISO: `0f373f0b473293cd7479b37ccb278098ceeaea55a395cb39fd8128e21f09247b`; kernel: `9c86f19e47f84b70b882116bccc2164b9938b9755a5ab0521f66e7cdeb26eb1f`. |
| Focused Helios and TTY regression | **PASS** | `compat.log` (29 tests), `tty.log` (42 tests), kernel feature check and diff check. `unmasked-verified/serial.log` on the final ISO: timeout, stdout readiness, EBADF, QMP TTY input `x`, exit 0 and reap. The fixture's masked request still returned -95. |
| Early guest iterations | **FAIL**, superseded | `first/` stopped before the handler; `rf-rejected/` reproduced it with a saved timer RFLAGS of `0x10247`. RF was then admitted to the bounded IRETQ policy and cleared for handler entry. `pre-df/` contains the first complete round trip before the raw IRQ DF clearing and the DF fixture check. |
| Masked wait semantics | **BLOCKED** | No temporary mask, wait interruption or restart handling was added. |
| btop Help, normal btop exit, and shell reuse after btop | **NOT RUN** | Prior results remain in the BTop-05 report. |
| Linux malformed-frame cases and full boot/storage gate | **NOT RUN** | Outside this focused guest validation. |

Changed paths for SIG-01:

- `compat-linux/src/{abi,lib}.rs`
- `kernel/src/arch/x86_64/{cpu,interrupts,linux_signal,mod,syscall}.rs`
- `kernel/src/memory/user.rs` and `kernel/src/process/{mod,signal}.rs`
- `tools/helios-sig01/{build-image.sh,guest.py,registers.S,roundtrip.c}`
- `docs/Helios_LinuxCompat/helios-sig01.md`

The BTop-05 files and pinned executable were preserved. Nothing was
committed or pushed.
