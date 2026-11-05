# T1 — trusted syscall stack and thread completion

Status: **T1 COMPLETE for the supported dense-APIC x86-64 configuration and
required QEMU TCG matrix**. Date: 2026-11-04. T2–T7 remain separate findings.
Base HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`. This is a dirty-tree
milestone; HEAD alone does not reproduce it. No commit or push is requested.

## Source identity and evidence

Durable evidence is in `.threading-evidence/t1/`. `before/source.json` records
SHA-256 for every tracked and visible untracked source file, HEAD, and a canonical
digest of the sorted filename/hash mapping. `before/status.txt` and
`before/dirty.patch` preserve the original status and tracked binary diff.
The before canonical SHA-256 is
`3ffa083e99af823607903e2a4b9d4f2f0a277cb047aacf89125f3c23e106d203`.
Existing IPC, scheduler, interrupt, syscall, process, manifest and audit changes
were present before T1 and are retained. Original audit evidence is untouched.

`tools/threading_t1_validate.py` creates a new timestamped directory for each
attempt, with `source.json`, exact argv, output, elapsed time, exit status and
verdict in `result.json`. Guest attempts also preserve serial output and ISO
SHA-256. Synthetic parser/ABI controls are labelled as host controls, never as
native execution. The `images/` directory retains executable images separately
from disposable Cargo target storage. Read the final validation ledger below
before treating any intermediate build as final-source evidence.
`final/source.json` records the delivered source identity; `final/t1.patch`
isolates this milestone from the original dirty tree. `validation-summary.json`
indexes every retained command attempt, selected matrix result, failure and
image identity. All 14 modified existing files are within the T1 architecture,
lifecycle and fixture scope; every other initially fingerprinted file retains
its initial hash. No commit or push was made.

## Root cause and boundary audit

Previously LSTAR selected `syscall_entry`; STAR selected kernel CS 0x08 and
SYSRET user CS 0x2b / SS 0x23; EFER.SCE enabled SYSCALL. FMASK cleared only IF.
SYSCALL does **not** load TSS.RSP0. Entry immediately pushed 15 GPRs to the
unchanged caller RSP and called Rust. Disabling local IRQs did not protect those
frames against a sibling sharing the VM on another CPU.

| Boundary | Before T1 | T1 |
| --- | --- | --- |
| SYSCALL first instruction | User RSP; IF masked | User RSP; IF/TF/DF/NT/AC masked; no stack access |
| Before first push | User-writable stack | Local trusted slot selects the dispatched thread's kernel stack |
| Register frame / Rust dispatch | User stack | Supervisor heap allocation, private to thread |
| IPC/futex returning WouldBlock | Rust on user stack, then SYSRET and pending reschedule | Rust on thread kernel stack, then original user context and existing reschedule |
| Explicit CPL0 software reschedule | Special fixture repivoted to kernel stack | Existing thread kernel continuation, no repivot over live frames |
| IRQ/exception from CPL3 | Hardware TSS.RSP0 | Same per-thread TSS.RSP0 |
| IRQ/exception from CPL0 | Interrupted current stack | Trusted current kernel stack; NMI always uses dedicated IST[1] |
| Exec preparation | Special pivot/copy to kernel stack | Ordinary trusted syscall stack below reserved context region |
| Successful exec | IRET to installed context | Same; no return through retired VM |
| Self-exit | Child TID could clear while privileged frames used user stack | Reaper publishes completion after physical handoff |
| Return | Pop GPRs on user stack, SYSRET | Pop GPRs on trusted stack, restore original user RSP immediately before SYSRET |

## Entry, GS and ABI contract

`syscall_stack.rs` provides 64 fixed supervisor slots, containing stack top at
offset 0 and temporary user RSP at offset 8. Each CPU installs its slot in
IA32_KERNEL_GS_BASE before running userspace. CR4.FSGSBASE remains disabled.
There is no user API for changing KERNEL_GS_BASE; ARCH_SET_FS remains the only
supported arch_prctl write. Loading a userspace GS selector does not select the
privileged slot. This patch does not assume the user's active GS base is zero.

With IRQs and single-step traps already masked by hardware, entry does:

1. SWAPGS; store RSP in GS:8 without touching the user stack.
2. Reject an unarmed slot; load RSP from GS:0.
3. Reserve 256 bytes at the top, align down to 16, push original RSP.
4. SWAPGS back, before any call or scheduling.
5. Push the existing 15-register, 120-byte SyscallRegs frame and call dispatch
   with SysV alignment. RBX retains the frame across Rust calls.

The frame address is exactly `align_down(kernel_stack_top - 256, 16) - 128`.
Dispatch checks it against the locally selected TSS top without taking a new
global lock. The scheduler validates the actual Box allocation's end against
`kernel_stack_top` under its existing ownership lock before dispatch, then
installs the same top in TSS and the syscall slot with local IRQs disabled.
Kernel heap mappings have PRESENT|WRITABLE, without USER_ACCESSIBLE. Kernel
mapping inheritance keeps these allocations present across CR3 replacement.
An unarmed slot stops in a CLI/HLT loop without raising an exception or writing
a frame on the still-untrusted RSP. Such a slot is a kernel invariant failure,
unreachable after a valid scheduler installation; it is not a user-input error.

The register ABI is unchanged: RAX returns the result; frame mutations remain
visible in the other registers, including IPC's extended register payloads.
RCX and R11 retain architectural SYSCALL return RIP/flags. IF is set on return
as before. Noncanonical/high-half RCX is rejected on the trusted stack before
SYSRET can fault at CPL0. The user RSP is opaque: entry never dereferences it,
and an otherwise valid syscall can return with its exact original bits even
when that RSP is unmapped, zero or noncanonical. Userspace must repair it before
performing its own stack access. FS base and x87/XMM/MXCSR save/restore paths
are unchanged; kernel code remains built without SIMD/TLS use.

## Nesting and continuation ownership

The GS scratch lifetime ends at the second SWAPGS. Ordinary maskable IRQs
cannot intervene there. NMI uses a separate 16 KiB supervisor IST stack per CPU,
including when CPL0 RSP still equals the user RSP or has just been restored for
SYSRET. The NMI shootdown handler does not use GS, invoke syscalls or schedule.
NMI delivery blocks further NMI entry until IRET. Double fault retains IST[0].
The same-privilege 64-bit IRET return restores SS/RSP from its trusted IST frame;
it does not dereference the restored RSP before resuming the interrupted bridge.
The stack-address exception checks apply to reading the IRET frame itself.
See Intel SDM Volume 2A, IRET, printed pages 3-478–3-482, for the return
pseudocode and exception conditions. This is the architectural basis for
retaining even a noncanonical incoming RSP across NMI entry and return.
[Intel's instruction reference](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2a-manual.pdf).
After the pivot, ordinary exceptions use the current trusted stack. The bridge
accesses only preinstalled kernel mappings; invalid user RSP cannot cause a
page fault there. SYSRET's potentially faulting RIP is checked before restoring
RSP. This does not claim containment of arbitrary hardware failure or corrupted
kernel metadata. Sparse/nonzero APIC identity remains the separate T3 finding.

Blocking IPC and native/Linux futex paths retain their existing retry/park
contracts. Ordinary syscall IRQ masking is unchanged. The Doorbell fixture's
explicit software interrupt can suspend a CPL0 syscall continuation; its frame
and all Rust calls now remain on the private thread allocation across migration.
Its former top-of-stack repivot is removed. No continuation uses a per-CPU
execution stack. Per-CPU storage is only the bounded entry bridge and NMI IST.

Selection, remote-wake requests, fairness, quantum policy, locks and physical
handoff pins are unchanged. A resumed task gets the destination CPU's own slot
and the same private stack. The outgoing pin still prevents selection, slot
reuse and stack reclamation until the assembly epilogue has pivoted away.
Self-exit pivots to the existing stable idle stack before the pin's XCHG release.

Exec's previous special pivot would overwrite the now-live dispatcher. It now
calls preparation directly. Ordinary entry reserves the top 256 bytes for
`Process::init_context`'s 160-byte IRET frame. Failed exec returns on the original
trusted frame. Successful exec drops its owned temporaries, resets image FP/FS
state through the existing path and enters the replacement context with IRET;
no SYSRET continuation through the old VM is adopted. Existing borrower rejection,
authority retirement, shared-memory retirement and active-VM checks remain.

## Linux completion and reuse

`finish_current_process` no longer clears child TID. Both self-exit and external
termination use the existing `reap_process_resources` path. It first checks the
physical outgoing-stack pins and task ownership/queue state. Only after those
checks does it consume `tid_address`, clear the user word and wake its futex.
The old execution path is then physically off its stack and will never resume.
It cannot access user stack/TLS after publication. The final child-TID store
itself is the last required user-memory access to that completion word.

The retained Process, its live address-space handle/generation, and its existing
pin gate are the completion record. There is no new queue, allocation or delayed
lookup by reusable PID/slot. The slot cannot be reused until Reaped. Cleanup and
failed user stores remain deterministic and one-shot. Native std join continues
to wait for Reaped with syscall 156.

Child-TID publication now uses a validated aligned `AtomicU32::store(Release)`
through retained HHDM backing. The one-page, single-word store is its final
memory access: observing zero cannot precede trailing byte copies into reused
TLS. Invalid, unaligned, unmapped, supervisor or read-only destinations are
rejected without a store; the consumed registration is cleaned once. Supported
Linux thread runtimes supply aligned 32-bit futex words. **T4 remains open for
Linux FUTEX_WAIT's non-atomic comparison**; its child-TID-store portion is
addressed as a necessary part of T1's publication boundary.

## Adversarial fixtures

`syscall_stack_test` extends the isolated scheduler lifecycle image. Ordinary
fixtures are unaffected unless the opt-in feature is selected.

- E1/E2: raw assembly enters with unmapped 0x70000000, zero at the inaccessible
  lower boundary, noncanonical 0x0000800000000000, and exactly the first mapped
  user-stack page at 0x7fffffdff000. Fixture setup asserts that this page is
  mapped and the preceding page is absent. The probe verifies exact RSP
  preservation and continues execution. No call/push occurs on hostile RSP.
- E3: native siblings share one VM and distinct kernel/user stacks. The victim
  enters 32 yielding syscalls with RSP in a scratch user stack; the attacker
  overwrites the 128 bytes immediately below it and acknowledges execution.
  Every ordinary dispatch checks its trusted frame address.
- E5: a Linux assembly process clones 32 children using the real TLS/futex/
  clear-child-TID paths. Children perform their last FS access, then exit with
  RSP zero. On completion the parent immediately destroys all 64 KiB of child
  user stack and TLS, then clones again at the same addresses. The kernel
  fixture requires 32 successful clears, with no physical pin or CPU/queue owner.
  Before publication it also tests aligned words at page start/end and rejection
  of unaligned, unmapped, read-only and supervisor destinations.
- E7: existing lifecycle assertions retain forced exit races, delayed physical
  handoff, safe reaping, slot reuse and integer/XMM cookies.

`syscall_stack_auth_test` extends the isolated real capability/exec fixture.
An additional native sibling yields while the reader is blocked in authenticated
IPC. An observation under the existing Yield scheduler guard checks exact VM
identity and BlockedOnIpc before acknowledging on the fixture's existing SHM
backing. The reply waits for that acknowledgment. The sibling is reaped before
authorized exec. This hook is debug-only and has no production syscall selector.
The same image retains real authority consumption, reply/cancel, SHM retirement,
failed exec, live-borrower rejection, eight image replacements and final exit.

## Validation ledger

Corrected 1-CPU entry/exit and 1/2-CPU authenticated/blocking/exec pilots passed
before expanding to the final SMP matrix. Every row below was rerun with the
new ordinary entry; historical scheduler passes were not substituted for it.

| Native suite | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
| --- | --- | --- | --- | --- |
| T1 hostile RSP, mapped guard boundary, sibling writes, 32 Linux completion/reuse cycles, scheduler lifecycle | PASS | PASS | PASS | PASS |
| Real authenticated IPC, blocked-caller sibling progress, reply/cancel, authority/SHM/VM retirement, failed exec and eight replacements | PASS | PASS | PASS | PASS |
| R2D remote wake / initial and delayed physical pin handoff / reaping | PASS | PASS | PASS | PASS |
| Native Doorbell explicit CPL0 continuation / blocking / stale wake / reaping | PASS | PASS | PASS | PASS |
| Original Linux clone/futex/clear-child-TID, shared FD/eventfd, FS/TLS and signal-state probe | PASS | PASS | PASS | PASS |
| Actual native FP probe: nine execution contexts, x87/XMM0/XMM15/MXCSR and Hayro parser / Vello raster | PASS | PASS | PASS | PASS |
| Actual native Hayro service: std spawn/join, TLS destructors, futex synchronization, shared descriptors, OOM/panic/group exit and rendering RPC | PASS | PASS | PASS | PASS |

The single-CPU remote-wake controls do not establish a cross-CPU interleaving;
the 2/4/12-CPU validators establish those cases. Doorbell validators retain
their individual FORCED/NOT_FORCED classifications rather than promoting
every single-CPU case. All final suite classifications pass.

Final control logs contain:

```text
[T1] PASS invalid_rsp=unmapped,zero,noncanonical,guard-boundary sibling_iterations=32 completion_reuses=32 trusted_frames=checked
[T1-AUTH] PASS sibling_progress=blocked-authenticated-ipc private_kernel_stack=checked
[NATIVE-STD] x87/XMM/MXCSR preemption isolation OK
[NATIVE-STD] native PDF parser and vector raster OK
[NATIVE-STD] collections/clocks/locks/TLS/join OK
[DOCSERV] native Hayro IPC open/render/close OK
```

Host/architecture results:

- Kernel host library: 169 tests pass, including exec authority, participant
  exit, PID reuse, deferred reply identity and VM/SHM lifecycle controls.
- Production scheduler state/pin/request/quantum/lifecycle host gates pass;
  their raw output is retained as `scheduler-host-details.log`.
- Linux compatibility: 29 tests pass. Strict T1 parser controls, nine Account
  exec parser tests and five IPC boundary parser tests pass.
- Bare-metal kernel type check and final debug/release builds pass.
- Emitted syscall entry/return ABI and negative controls pass for final control,
  ordinary debug and stripped ordinary release ELFs.
- Formatting passes on T1's changed ranges and newly edited complete files.
  Whole-workspace formatting still fails in pre-existing source. `git diff
  --check` reports the pre-existing sunlight-uac manifest's trailing blank line.
- Strict Cargo Clippy has baseline dependency/build-script failures. A no-deps
  attempt hits the nested native-std SVG builder's clippy-driver/sysroot issue.
  To examine the actual kernel, `direct-clippy-driver.py` retains Cargo's real
  compiler argv/environment and invokes clippy-driver directly after a normal
  build, changing only compiler and emission/output destination. It reports
  **156 diagnostics on unchanged source spans, zero on new/changed T1 spans**.
  `atomic-direct-clippy-detail/classification.json` maps them against the initial
  dirty-tree source, not just HEAD. No lint suppression was added to the repo.

The ordinary no-feature release kernel boots successfully with two CPUs through
Foundation Complete, login input readiness, sunlightd and timezone readiness.
The ordinary no-feature debug build also passes. Neither binary contains the
new T1 fixture markers; hook scan results are retained. The supervisor artifact
was rebuilt with ordinary features after the document-service fixture build.

Source compatibility: all final kernel implementation files match the final
snapshot. The auth image predates only subsequent changes to the inactive
`scheduler_b_test` fixture/assembly; those inputs are excluded by its feature
selection. The final atomic-controls image includes the guard-boundary and one-time
verdict fixes, aligned release completion, and negative completion-store address
controls. The final Linux completion/store changes are isolated to nonzero Linux
child-TID registrations. Those paths were rerun on 1/2/4/12 CPUs, including the
original raw Linux clone/futex/FD/TLS probe. Native auth, pin, Doorbell, FP and
std/Hayro fixtures have no Linux child-TID registrations, so their retained full
matrices remain source-compatible; ordinary entry is byte-identical. Production
release/debug builds and release boot were repeated after the atomic change. Host library and handoff evidence is reused only where those
compiled sources are byte-identical. Every affected native entry path was run
fresh. These are functional TCG results, not physical-hardware certification or
performance measurements, and do not discharge the separate T3 topology finding.

Known intermediate attempts retained:

- Initial ABI checker failed to select lowercase local text symbols; the next
  checker attempt caught its own negative-control whitespace bug. Both host
  harness defects were corrected; emitted entry ABI and negative controls pass.
- First 1-CPU pilot completed all 32 Linux reuse cycles but reached the existing
  12000-tick lifecycle deadline with the 256-entry sibling workload unfinished.
  The workload was reduced to 32 entries with a writer acknowledgment. No SMP
  acceptance is inferred from that attempt.
- An intermediate rebuild overlapped addition of the new auth feature and
  rejected an unexpected cfg value. It is not a usable image.
- Workspace rustfmt has pre-existing failures. Strict Clippy stops on existing
  dependency/build-script diagnostics. A no-deps follow-up also fails in the
  existing nested native-std SVG build under clippy-driver (unsupported std PAL
  selection); it does not establish a clean Clippy result for the kernel.
- A later stack-control attempt emitted a duplicate partial verdict during
  shutdown; the strict host gate rejected it and reached its deadline. The
  fixture now publishes its terminal verdict once. The rejected record remains
  failed, even though its assertions had reached PASS.
- The first native service build selected legacy `native-smoke`, which enables
  MuPDF and failed in its unsupported C/Clang target bindings. The correct
  `native-hayro-smoke` build and all four native service guests pass.
- A combined userspace build omitted sunshell's `sunlight` feature and failed;
  the explicit native-feature rebuild passes.
- Final review identified that the old child-TID byte-copy could split across
  pages and expose zero before a trailing write. It was replaced with a
  validated aligned release store; affected completion tests and production
  checks were repeated. `final-pre-atomic/` and
  `validation-summary-pre-atomic.json` preserve the superseded snapshot.
- The first release ABI probe expected symbols, but ordinary release has
  `strip=true`. The checker now requires a unique emitted entry signature when
  symbols are absent; the stripped release check passes.
- Building the native FP fixture automatically refreshed six local package
  version entries in its lockfile. The generated lockfile is retained in
  evidence, and the repository lockfile was restored to its exact initial hash.

No failed attempt is used as acceptance evidence. The first manual formatting
probe omitted rustfmt's required unstable-options flag; its command and error
are recorded in `manual-attempts.json`. The corrected formatting checks pass.

## Unsafe and security review

The new unsafe boundaries are MSR installation of a static local slot, naked
entry/return, hardware NMI writes to static supervisor stack storage, the
validated HHDM AtomicU32 completion store, and the debug-only SHM progress
acknowledgment. Offset assertions fix the slot ABI;
allocation/TSS/frame checks establish stack identity; IRQ masking and the NMI
IST establish the scratch lifetime. Completion stores require a writable user
mapping, retained backing under the scheduler guard (or exclusive unpublished
VM ownership), natural alignment, and a single-page word. The emitted helper
has one `mov DWORD PTR [rsi],ebp` store followed only by its trusted epilogue;
`atomic-store-disassembly.txt` retains the complete checked build output. The SHM observation uses the existing
fixture-owned backing while its same-VM caller is blocked. It introduces no
production selector or pointer API. Exec and Doorbell remove their former
repivot/copy assembly rather than layering another stack over live frames.

No capability checks, user-copy validation, syscall authorization, scheduler
locks, quantum or work-stealing policy were relaxed. Existing handoff pins,
active-VM checks and native Reaped join remain. The fixed NMI storage costs
1 MiB for 64 CPUs and is allocated statically; ordinary per-thread stacks stay
32 KiB. An emitted-frame scan found syscall cursor storage at 0x4088 bytes and
spawn preparation at 0x3198 bytes in the debug image. This is a capacity audit,
not a formal whole-call-graph stack bound or closure of T7's missing guards.

## Reproduction

Run from the repository root. Use new evidence directories for each attempt:

```sh
python3 tools/threading_t1_validate.py command kernel-host -- cargo test --package sunlight-kernel --lib --target x86_64-unknown-linux-gnu --offline
python3 tools/threading_t1_validate.py command scheduler-host -- python3 tools/test_scheduler_host.py
python3 tools/threading_t1_validate.py command linux-host -- cargo test --package sunlight-compat-linux --lib --target x86_64-unknown-linux-gnu --offline
python3 tools/threading_t1_validate.py command parser -- python3 -m unittest discover -s tools -p test_threading_t1.py
python3 tools/threading_t1_validate.py command build -- cargo build --package sunlight-kernel --bin sunlight-kernel --features syscall_stack_test --offline
objcopy --strip-debug target/x86_64-unknown-none/debug/sunlight-kernel /tmp/t1-kernel.elf
bash tools/make_hybrid_iso.sh /tmp/t1-kernel.elf /tmp/t1.iso target/limine "$PWD"
python3 tools/threading_t1_validate.py command abi -- python3 tools/check_syscall_stack_abi.py /tmp/t1-kernel.elf
python3 tools/threading_t1_validate.py guest pilot --iso /tmp/t1.iso --cpus 1 --marker '[T1] PASS' --marker '[SCHED-B] PASS cpus=1 '
python3 tools/threading_t1_validate.py command auth-build -- env SUNLIGHT_CAP_EXEC_PROBE=1 cargo build --package sunlight-kernel --bin sunlight-kernel --features syscall_stack_auth_test --offline
```

Copy/hash the auth ELF and construct a fresh ISO similarly. Require `[T1-AUTH]
PASS`, `[CAP-R41] COMPLETE`, `[CAP-P] native PASS` and `[EXEC-R2] eight replacements
PASS; entering original /bin/true`. Run corrected 1/2-CPU pilots before the
1/2/4/12-CPU matrix. The runner records the exact QEMU TCG command. The complete exact invocations for remote-wake/pin, native-std/Hayro, Linux
thread and production gates are retained in `validation-summary.json`. Build
steps for the actual native fixtures are:

```sh
bash tools/build_helios_probes.sh
env SUNLIGHT_INJECT_PHASE=helios-thread-probe cargo build -p sunlight-kernel --bin sunlight-kernel --features key_inject --offline
env RUSTFLAGS="-C link-arg=-T$PWD/services/user-space.ld" cargo build --manifest-path tools/native-std-probe/Cargo.toml --target tools/targets/x86_64-sunlightos.json --bin native-fp-probe --release -Z build-std=core,alloc -Z json-target-spec --offline
bash tools/build_native_std.sh build -p sunlight-docserv --no-default-features --features native-hayro-smoke --release --offline
python3 tools/test_remote_wake_smp.py --pin-r2d --cpus 1 2 4 12 --timeout 180 --evidence-dir /tmp/t1-pin-new
python3 tools/test_native_doorbell.py --cpus 1 2 4 12 --timeout 180 --evidence-dir /tmp/t1-doorbell-new
cargo build --package sunlight-kernel --bin sunlight-kernel --release --no-default-features --offline
```

Each feature/injection image must be copied immediately after its build and
before building a different kernel feature set. See the indexed `fp-users`,
`fp-shell`, `fp-image`, `std-supervisor`, `std-image` and `restore-supervisor`
commands for the exact native service flags and build order. Test-image guests
use the frozen retained ISO, not a mutable Cargo output path. Restore ordinary
supervisor artifacts before building the production image.

## Remaining audit findings

T2 fallible bounded thread admission/allocation, T3 APIC-to-logical CPU mapping,
T4 atomic Linux futex comparison (child-TID publication is addressed here), T5 robust owner-death cleanup, T6 retained
waitpid receipts, and T7 guard mappings remain open. See
[the November 4 audit](threading-audit-2026-11-04.md). This milestone changes no
scheduler policy, capability authorization, unrelated FastLane routing, desktop
or audio application source.
