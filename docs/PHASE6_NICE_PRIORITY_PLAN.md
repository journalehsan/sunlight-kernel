# SunlightOS Phase 6 — Nice/Priority Kernel Hook Plan

## Goal

Add a minimal `nice` primitive in Ring 0 so userland can bias CPU share via
timeslice length, while keeping scheduling policy simple.

## Why this bite is useful

- Keeps kernel policy lightweight (microkernel-friendly).
- Gives immediate operator tooling (`nice`, `renice`).
- Creates a stable ABI hook for a future Ring 3 `priority_manager`.

## Scope (this bite)

1. Add `nice: i8` to `Process` with range `-10..=10`.
2. Derive process timeslice from `nice` (no scheduler algorithm rewrite).
3. Add `getnice/setnice` syscalls with ownership + privilege checks.
4. Add `nice` / `renice` applets in `sunlight-utils`.
5. Verify no gate regressions.

Out of scope: external policy daemons, ananicy rules, BORE redesign.

## Important fixes vs initial draft

### 1) Syscall number conflict

Current assignments in `kernel/src/arch/x86_64/syscall.rs` include:
- `80 = PowerCtl`
- `81 = GetTimeUtc`
- `82 = SysInfo`

So `SetNice=80` / `GetNice=81` is invalid.  
Use:
- `83 = SetNice`
- `84 = GetNice`

### 2) Keep scheduler behavior stable

Kernel currently has `SCHEDULER_MODE` and uses `RoundRobin` by default.
This bite must **only** change quantum length per process; queue selection and
pick-next policy remain unchanged.

### 3) Userland API shape

Use absolute nice value in syscall args for simplicity:
- `setnice(pid, new_nice)` (kernel clamps to `[-10, 10]`)
- `getnice(pid) -> nice`

User applets can still compute deltas if desired, but ABI should be direct.

## Implementation plan

### Step 1 — Process model

Files:
- `kernel/src/process/mod.rs`
- `kernel/src/process/fork.rs`

Changes:
- Add `pub nice: i8` to `Process`.
- Initialize `nice = 0` in `Process::new`.
- Ensure fork child inherits parent `nice` in both fork paths.

### Step 2 — Scheduler quantum mapping

File:
- `kernel/src/sched/mod.rs`

Changes:
- Add helper:
  - `fn time_slice_for_nice(nice: i8) -> u64`
  - mapping: `BASE(=TIME_SLICE_TICKS) - nice`, clamp `[2, 20]`.
- In `tick()`, replace fixed `TIME_SLICE_TICKS` threshold with:
  - `let quantum = time_slice_for_nice(self.processes[self.current].nice);`
  - compare `self.current_ticks >= quantum`.
- Do not change `pick_next_*`, queue tiers, or BORE logic.

### Step 3 — Syscalls and permissions

File:
- `kernel/src/arch/x86_64/syscall.rs`

Changes:
- Enum:
  - `SetNice = 83`
  - `GetNice = 84`
- Dispatch entries:
  - `83 => sys_setnice(frame),`
  - `84 => sys_getnice(frame),`
- Implement:
  - `sys_getnice(rdi=pid_or_0_self) -> i64 encoded in u64`
  - `sys_setnice(rdi=pid_or_0_self, rsi=new_nice_i64) -> i64 encoded in u64`

Permission rules:
- Root (`uid==0`) may set any process nice up/down.
- Non-root:
  - may only target same-uid processes;
  - may not decrease numeric nice (i.e., cannot raise priority).

Encoding rules:
- Success returns signed nice value as `i64` bits in `u64`.
- Failure returns `u64::MAX` (existing ABI style).

### Step 4 — libc wrappers

Files:
- `sunlight-libc/src/sys.rs`
- `sunlight-libc/src/lib.rs`

Changes:
- Add syscall numbers:
  - `SYS_SETNICE = 83`
  - `SYS_GETNICE = 84`
- Add safe wrappers:
  - `pub fn getnice(pid: u64) -> Result<i8, Errno>`
  - `pub fn setnice(pid: u64, nice: i8) -> Result<i8, Errno>`

### Step 5 — `sunlight-utils` applets

File:
- `sunlight-utils/src/main.rs`

Add applets:
- `nice`
  - no args: print current process nice.
  - `-n N`: set current process nice to `N` and print confirmation.
  - For now no command-exec chaining in this bite (Phase 7 item).
- `renice N PID`
  - set target pid to absolute `N`.
  - print success or permission-denied message.

Input behavior:
- Parse integers safely.
- Clamp in kernel; userland can pre-validate for clearer errors.

### Step 6 — Validation

1. `cargo check --package sunlight-kernel`
2. `cargo check --package sunlight-libc`
3. `cargo check --package sunlight-utils`
4. `./tools/test.sh` (or current phase gate wrapper)
5. Manual in QEMU:
   - `nice` -> `0`
   - `nice -n 5` -> confirmation
   - `nice` -> `5`
   - `renice -5 <other-pid>` as non-root -> denied
   - clamp checks: `nice -n 99` -> `10`, `nice -n -99` -> `-10` (root allowed)

## Suggested commit breakdown

1. `kernel: add process nice field and inherit on fork`
2. `sched: derive quantum from per-process nice`
3. `syscall: add setnice/getnice with permission checks`
4. `libc/utils: expose nice syscalls and add nice/renice applets`
5. `docs: add phase6 nice plan and notes`

## Copy-paste prompt for next session

Use this prompt in the next session:

---

# SunlightOS — Phase 6 Bite: Nice/Priority Primitive (Kernel + Utils)

Constraint: do not ask for confirmation; implement, test, report, stop.

Implement a minimal `nice` primitive in kernel/userland with these exact rules:

1) Process field:
- Add `nice: i8` to `Process` (`kernel/src/process/mod.rs`), range `-10..=10`.
- Default for new processes is `0`.
- Forked child inherits parent `nice` in `kernel/src/process/fork.rs`.

2) Scheduler quantum mapping only (no algorithm rewrite):
- In `kernel/src/sched/mod.rs`, add helper:
  - `fn time_slice_for_nice(nice: i8) -> u64`
  - Use base from existing `TIME_SLICE_TICKS` (=10), compute `base - nice`, clamp `[2, 20]`.
- In `tick()`, replace hardcoded `TIME_SLICE_TICKS` threshold with per-current-process quantum.
- Do not change queueing or pick-next logic.

3) Syscalls (avoid conflicts):
- In `kernel/src/arch/x86_64/syscall.rs` add:
  - `SetNice = 83`
  - `GetNice = 84`
- Add dispatch:
  - `83 => sys_setnice(frame)`
  - `84 => sys_getnice(frame)`
- ABI:
  - `getnice`: `rdi=pid` (`0` means current process), returns signed nice value.
  - `setnice`: `rdi=pid` (`0` means current process), `rsi=absolute new nice`.
  - Kernel clamps to `[-10, 10]`.
- Permissions:
  - root (`uid==0`) can modify any process up/down.
  - non-root can only target same-uid process and cannot decrease numeric nice.
- Error return remains `u64::MAX`.

4) libc wrappers:
- Add `SYS_SETNICE=83` and `SYS_GETNICE=84` in `sunlight-libc/src/sys.rs`.
- Add safe wrappers in `sunlight-libc/src/lib.rs`:
  - `getnice(pid: u64) -> Result<i8, Errno>`
  - `setnice(pid: u64, nice: i8) -> Result<i8, Errno>`

5) sunlight-utils applets:
- Update `sunlight-utils/src/main.rs` dispatch with:
  - `nice`
  - `renice`
- Behavior:
  - `nice` -> print current nice.
  - `nice -n N` -> set current process nice to N, print confirmation.
  - `renice N PID` -> set target process nice, print result.
- This phase does not implement `nice -n N cmd` exec chaining yet.

6) Validation:
- Run:
  - `cargo check --package sunlight-kernel`
  - `cargo check --package sunlight-libc`
  - `cargo check --package sunlight-utils`
  - `./tools/test.sh` (or current gate command in this repo)
- Manual behavior checks in QEMU:
  - `nice` => `0` by default
  - `nice -n 5` then `nice` => `5`
  - clamping to `[-10,10]`
  - non-root permission denial for raising priority or cross-uid target

Report in one paragraph:
- confirm defaults/inheritance,
- confirm clamping and permission behavior,
- confirm gate/check status.

---

