You are working on a Rust kernel scheduler implementation. The project currently has these relevant files:

- `mod.rs`: main scheduler implementation
- `context.rs`: low-level context switch helpers
- `thread.rs`: compatibility re-export for process/thread definitions

The current scheduler is single-core oriented and must be extended to support SMP / multi-core scheduling with minimal backward-incompatible changes.

Important existing facts from the current codebase:

1. The current global scheduler is:

   `pub static SCHEDULER: spin::Mutex<Scheduler> = spin::Mutex::new(Scheduler::new());`

2. `Scheduler` currently contains:
   - `processes: Vec<Process>`
   - `ready_queue_high: VecDeque<usize>`
   - `ready_queue_medium: VecDeque<usize>`
   - `ready_queue_low: VecDeque<usize>`
   - `current: usize`
   - `current_ticks: u64`
   - `global_tick: u64`
   - `idle_context_rsp: u64`

3. The current scheduler mode is:

   `pub const SCHEDULER_MODE: SchedulerMode = SchedulerMode::RoundRobin;`

   BORE code exists but is currently not the active mode. Do not remove BORE code. Do not enable BORE unless explicitly requested.

4. Existing public APIs must remain source-compatible whenever possible:
   - `Scheduler::new()`
   - `add_process(process) -> usize`
   - `enqueue_process(idx)`
   - `enqueue_process_once(idx)`
   - `remove_from_ready_queues(idx)`
   - `seed_ready_queues_except(running_idx)`
   - `set_idle_context(rsp)`
   - `set_state(idx, new_state)`
   - `account_current_runtime()`
   - `start_charging_runtime(idx)`
   - `effective_runtime_ns(idx)`
   - `pick_next_round_robin()`
   - `current_process()`
   - `enter_first_process()`
   - `current_process_rsp()`
   - `with_scheduler(...)`
   - `request_reschedule()`
   - `check_reschedule()`

5. `context.rs` provides:
   - `unsafe fn iretq_to_context(context_rsp: u64) -> !`
   - `unsafe fn save_current_context(current_rsp: u64, process: &mut Process)`

   These should not be fundamentally changed unless absolutely necessary.

6. `thread.rs` is only a compatibility re-export:
   - `pub use crate::process::{Process, ProcessState, KERNEL_STACK_SIZE};`

7. The scheduler currently uses three tier queues:
   - high
   - medium
   - low

   Each stores process indices as `usize`.

8. `add_process()` currently only inserts the process into `processes`, and does not enqueue it. Preserve this behavior.

9. `enqueue_process()` currently:
   - ignores invalid indices
   - ignores non-Ready processes
   - removes the index from queues first to prevent duplicates
   - selects a queue via `process.get_queue_tier()`
   - pushes to high/medium/low queue

10. The goal is to add SMP support while preserving the existing RoundRobin behavior as much as possible.

Primary goals:

- Add per-CPU scheduling state.
- Keep `processes: Vec<Process>` global.
- Move runnable queues and current process tracking to per-CPU state.
- Preserve old public APIs by turning them into compatibility wrappers where necessary.
- Keep migration minimal to improve cache locality and reduce audio jitter.
- Add a simple CPU selection policy that prefers:
  1. previous CPU / last CPU
  2. idle or least-loaded CPU
  3. allowed CPU mask if implemented
  4. fallback to bootstrap/current CPU
- Avoid a single global round-robin pointer as the main SMP design.
- Avoid a single global runqueue as the final architecture.
- Design should be suitable for low-latency audio workloads, but does not need strict hard real-time guarantees.

Desired design:

Introduce a new per-CPU scheduling structure, for example:
```rust
pub struct CpuScheduler {
pub ready_queue_high: VecDeque<usize>,
pub ready_queue_medium: VecDeque<usize>,
pub ready_queue_low: VecDeque<usize>,

pub current: Option<usize>,
pub current_ticks: u64,
pub idle_context_rsp: u64,

pub local_tick: u64,
pub need_resched: bool,
}

Then modify Scheduler approximately like this:

                                                                    rust
pub struct Scheduler {
pub processes: Vec<Process>,

pub cpus: Vec<CpuScheduler>,

pub global_tick: u64,

// Optional compatibility fields, if needed temporarily:
// pub current: usize,
// pub current_ticks: u64,
// pub idle_context_rsp: u64,
}

If the kernel currently cannot allocate Vec<CpuScheduler> in const fn new(), use a fixed maximum CPU count or keep a bootstrap single-CPU state until runtime initialization.

Possible constants:

                                                                    rust
pub const MAX_CPUS: usize = 64;
pub const BOOT_CPU_ID: usize = 0;

If Vec allocation is not allowed in const fn, consider:

                                                                    rust
pub struct Scheduler {
pub processes: Vec<Process>,
pub cpus: [CpuScheduler; MAX_CPUS],
pub num_cpus: usize,
pub global_tick: u64,
}

But only do this if CpuScheduler can be const-initialized. Otherwise, implement an initialization function such as:

                                                                    rust
pub fn init_smp(&mut self, num_cpus: usize)

Requirements for per-CPU queue operations:

Implement helper methods on CpuScheduler:

                                                                    rust
impl CpuScheduler {
pub const fn new() -> Self;

pub fn ready_len(&self) -> usize;

pub fn contains(&self, idx: usize) -> bool;

pub fn remove(&mut self, idx: usize);

pub fn enqueue_by_tier(&mut self, idx: usize, tier: QueueTier);

pub fn pop_next_by_tier(&mut self) -> Option<usize>;
}

If QueueTier already exists, use the existing type. If not, reuse whatever get_queue_tier() returns.

Important duplicate-prevention rule:

A process index must not appear in more than one ready queue globally. When enqueueing a process:

    first remove it from all CPU queues
    then enqueue it into exactly one CPU queue

Implement:

                                                                    rust
pub fn remove_from_all_cpu_queues(&mut self, idx: usize)

or modify existing:

                                                                    rust
pub fn remove_from_ready_queues(&mut self, idx: usize)

so that it removes the process from every CPU’s queues.

Backward compatibility behavior:

Keep this existing public method signature:

                                                                    rust
pub fn enqueue_process(&mut self, idx: usize)

But internally change it to:

                                                                    rust
pub fn enqueue_process(&mut self, idx: usize) {
if idx >= self.processes.len() {
return;
}

if self.processes[idx].state != ProcessState::Ready {
return;
}

self.remove_from_ready_queues(idx);

let cpu = self.select_target_cpu(idx);
let tier = self.processes[idx].get_queue_tier();

self.cpus[cpu].enqueue_by_tier(idx, tier);
}

CPU selection policy:

Implement:

                                                                    rust
pub fn select_target_cpu(&self, idx: usize) -> usize

Initial simple policy:

    If process has a last_cpu field and that CPU is valid, online, and not overloaded, return it.
    Otherwise choose the CPU with the smallest ready_len().
    Prefer online CPUs only.
    Fallback to BOOT_CPU_ID.

If the existing Process struct does not have these fields, add them if possible:

                                                                    rust
pub last_cpu: Option<usize>,
pub preferred_cpu: Option<usize>,
pub allowed_cpus: CpuMask,
pub latency_sensitive: bool,

If adding a full CpuMask is too invasive, initially use:

                                                                    rust
pub last_cpu: Option<usize>,
pub preferred_cpu: Option<usize>,
pub latency_sensitive: bool,

or even only:

                                                                    rust
pub last_cpu: Option<usize>

Do not over-engineer CPU affinity in the first patch.

Audio-friendly scheduling guidance:

The design should reduce audio jitter. Implement simple policy hooks:

    Prefer keeping a process on its previous CPU.
    Avoid migration unless the previous CPU is significantly more loaded.
    Optionally add latency_sensitive: bool to Process.
    For latency-sensitive processes:
        prefer the previous CPU strongly
        avoid moving them between CPUs
        choose an idle CPU if no previous CPU exists
        avoid queueing them behind long low-priority queues where possible

Possible scoring:

                                                                    rust
fn cpu_score_for_process(&self, cpu: usize, idx: usize) -> usize {
let load = self.cpus[cpu].ready_len();

let mut score = load * 10;

if self.processes[idx].last_cpu == Some(cpu) {
score = score.saturating_sub(8);
}

if self.processes[idx].latency_sensitive {
score = score.saturating_sub(4);
}

score
}

Keep the scoring simple and safe.

RoundRobin changes:

Current method:

                                                                    rust
pub fn pick_next_round_robin(&mut self) -> Option<usize>

must remain for compatibility.

Internally, introduce:

                                                                    rust
pub fn pick_next_round_robin_for_cpu(&mut self, cpu_id: usize) -> Option<usize>

The old method may call:

                                                                    rust
self.pick_next_round_robin_for_cpu(current_cpu_id_or_boot_cpu())

If there is no working current_cpu_id() yet, use BOOT_CPU_ID temporarily.

The per-CPU picker should:

    pop from high queue first
    then medium
    then low
    skip invalid indices
    skip non-Ready processes
    set selected process state to Running
    update CPU current
    update last_cpu
    start runtime charging
    return selected process index

Pseudo-code:

                                                                    rust
pub fn pick_next_round_robin_for_cpu(&mut self, cpu_id: usize) -> Option<usize> {
let cpu = &mut self.cpus[cpu_id];

while let Some(idx) = cpu.pop_next_by_tier() {
if idx >= self.processes.len() {
continue;
}

if self.processes[idx].state != ProcessState::Ready {
continue;
}

self.processes[idx].state = ProcessState::Running;
self.processes[idx].last_run_tick = self.global_tick;
self.processes[idx].last_cpu = Some(cpu_id);

cpu.current = Some(idx);
cpu.current_ticks = 0;

self.start_charging_runtime(idx);

return Some(idx);
}

None
}

Be careful with Rust borrow checker:

    Do not hold a mutable borrow of self.cpus[cpu_id] while also mutably borrowing self.processes[idx] if it causes borrow conflicts.
    Use scopes or helper methods.
    Pop the index first, release queue borrow, then update process fields.

State transition behavior:

Modify set_state(idx, new_state) while preserving its external behavior.

Rules:

    If idx invalid: return.
    Save old state.
    Set new state.
    If new state is Ready:
        remove idx from all CPU queues
        enqueue_process(idx)
    If new state is not Ready:
        remove idx from all CPU queues
    If new state is Running:
        do not keep it in any ready queue
    If Finished:
        remove it from all queues
        ensure no CPU current points to it if possible

Be careful to avoid recursive borrow/method problems if set_state() calls enqueue_process().

Context switch integration:

The current context functions are:

                                                                    rust
unsafe fn save_current_context(current_rsp: u64, process: &mut Process)
unsafe fn iretq_to_context(context_rsp: u64) -> !

Do not change their ABI.

For SMP:

    current process must become per-CPU
    saving context should save into the current process of the current CPU
    current_process_rsp() should return the context of the current process on the current CPU
    if there is no current process for the CPU, return the CPU’s idle context rsp

Introduce helper:

                                                                    rust
pub fn current_cpu_id() -> usize

Initially this can return 0 if APIC/CPU-local storage is not implemented yet. Add TODO comments.

Later this should read from CPU-local storage, LAPIC ID mapping, or per-CPU data.

Compatibility wrapper:

                                                                    rust
pub fn current_process_rsp() -> u64 {
let sched = SCHEDULER.lock();
let cpu_id = current_cpu_id();
sched.current_process_rsp_for_cpu(cpu_id)
}

And:

                                                                    rust
impl Scheduler {
pub fn current_process_rsp_for_cpu(&self, cpu_id: usize) -> u64 {
if let Some(idx) = self.cpus[cpu_id].current {
if idx < self.processes.len() {
return self.processes[idx].context_rsp;
}
}

self.cpus[cpu_id].idle_context_rsp
}
}

Runtime accounting:

Current method:

                                                                    rust
pub fn account_current_runtime(&mut self) -> u64

must remain.

Add:

                                                                    rust
pub fn account_current_runtime_for_cpu(&mut self, cpu_id: usize) -> u64

Compatibility wrapper:

                                                                    rust
pub fn account_current_runtime(&mut self) -> u64 {
let cpu_id = current_cpu_id();
self.account_current_runtime_for_cpu(cpu_id)
}

Implementation:

    find current process from self.cpus[cpu_id].current
    if no current process, return 0
    otherwise preserve existing accounting logic:
        use now_ns()
        if last_start_ns != 0, delta = now - last_start_ns
        add delta to cpu_runtime_ns
        set last_start_ns = 0
        return delta

Idle context:

Current method:

                                                                    rust
pub fn set_idle_context(&mut self, rsp: u64)

must remain.

Add:

                                                                    rust
pub fn set_idle_context_for_cpu(&mut self, cpu_id: usize, rsp: u64)

Compatibility wrapper:

                                                                    rust
pub fn set_idle_context(&mut self, rsp: u64) {
let cpu_id = current_cpu_id();
self.set_idle_context_for_cpu(cpu_id, rsp);
}

If current_cpu_id() is not implemented, this sets CPU 0 idle context for now.

Load balancing:

Implement only a simple, conservative load balancer initially.

Goal:

    Do not migrate constantly.
    Avoid audio jitter.
    Only migrate when imbalance is clear.

Add:

                                                                    rust
pub fn balance_load(&mut self)

Initial policy:

    Compute ready lengths per CPU.
    If max_load > min_load + 1 or +2, move one non-latency-sensitive Ready process from busiest CPU to least-loaded CPU.
    Do not move currently Running processes.
    Avoid moving latency-sensitive processes unless absolutely necessary.
    Preserve queue tier when moving.

Pseudo-code:

                                                                    rust
pub fn balance_load(&mut self) {
let busiest = self.find_busiest_cpu();
let idlest = self.find_idlest_cpu();

if busiest == idlest {
return;
}

let busy_load = self.cpus[busiest].ready_len();
let idle_load = self.cpus[idlest].ready_len();

if busy_load <= idle_load + 1 {
return;
}

if let Some(idx) = self.cpus[busiest].pop_migratable_task(&self.processes) {
let tier = self.processes[idx].get_queue_tier();
self.cpus[idlest].enqueue_by_tier(idx, tier);
}
}

Because Rust borrowing may be tricky, implement pop_migratable_task carefully. It can scan high/medium/low queues and skip latency-sensitive tasks.

Do not implement aggressive work stealing unless necessary.

Reschedule/IPI:

Existing functions:

                                                                    rust
request_reschedule()
check_reschedule()

probably use global flags. Preserve them.

For SMP, add per-CPU reschedule flags if possible:

                                                                    rust
pub fn request_reschedule_for_cpu(cpu_id: usize)
pub fn check_reschedule_for_cpu(cpu_id: usize) -> bool

Compatibility wrappers:

    old request_reschedule() targets current CPU
    old check_reschedule() checks current CPU

If IPI support is not present yet:

    add TODO comments
    setting need_resched for target CPU is enough for now

If IPI support exists:

    when enqueueing a high-priority or latency-sensitive task on another CPU, send a reschedule IPI to that CPU

Audio-friendly enhancement:

    if a latency-sensitive task becomes Ready and is queued to an idle CPU, request reschedule immediately
    if queued to a remote CPU, mark that CPU need_resched

enter_first_process:

Current enter_first_process() starts the first ready process globally.

For compatibility:

    keep the public function
    make it operate on BOOT_CPU_ID
    later introduce:

                                                                    rust
pub fn enter_first_process_for_cpu(cpu_id: usize) -> !

Bootstrap behavior:

    CPU 0 uses existing behavior
    APs should start idle loop until scheduler state and idle context are initialized
    do not try to run the same Ready process on multiple CPUs

Safety:

    Never hold the scheduler lock across iretq_to_context
    Keep current behavior of collecting rsp and pml4_phys, dropping the lock, then switching CR3 and jumping

Process duplication safety:

Ensure these invariants:

    A process can be Running on at most one CPU.
    A process can appear in at most one ready queue across all CPUs.
    A process in Ready state may be queued.
    A process in Running state must not be queued.
    A Blocked/Sleeping/Finished process must not be queued.
    cpu.current must not point to a Finished process.
    Context switch must save the context into the process currently assigned to that CPU.

Locking guidance:

For the first SMP-compatible version, it is acceptable to keep one global spin::Mutex<Scheduler> for simplicity and compatibility.

Do not attempt fine-grained per-CPU locks in the first patch unless the codebase already has a safe pattern for that.

Add comments noting that future optimization can split:

    global process table lock
    per-CPU runqueue locks

But do not prematurely add lock complexity.

Potential future structure:

                                                                    rust
struct Scheduler {
processes: Vec<Process>,
cpus: [CpuScheduler; MAX_CPUS],
}

protected by one global lock initially.

Testing / validation expectations:

After implementation, verify:

    Single-core behavior still works.
    Existing RoundRobin behavior still works on CPU 0.
    add_process() still does not enqueue automatically.
    enqueue_process() prevents duplicates.
    set_state(idx, Ready) queues the task once.
    set_state(idx, Finished) removes it from all CPU queues.
    current_process_rsp() returns the current CPU’s current process context.
    If no current process exists, it returns that CPU’s idle context.
    Load balancing does not move Running tasks.
    Latency-sensitive tasks are migrated less often.
    No scheduler lock is held across iretq.
    No task can run on two CPUs at once.

Implementation style:

    Use small, incremental changes.
    Avoid rewriting unrelated BORE code.
    Preserve existing comments when possible.
    Add comments for SMP assumptions and TODOs.
    Prefer compatibility wrappers over signature-breaking changes.
    If a required feature such as current_cpu_id() is not yet implemented, provide a safe stub returning BOOT_CPU_ID and mark it clearly as TODO.

Deliverables:

    Updated scheduler structs:
        CpuScheduler
        modified Scheduler

    CPU-aware queue helpers:
        remove_from_ready_queues
        remove_from_all_cpu_queues
        enqueue_process
        enqueue_process_once

    CPU-aware current helpers:
        current_process_rsp_for_cpu
        compatibility current_process_rsp

    CPU-aware runtime accounting:
        account_current_runtime_for_cpu
        compatibility account_current_runtime

    CPU-aware RoundRobin:
        pick_next_round_robin_for_cpu
        compatibility pick_next_round_robin

    CPU selection:
        select_target_cpu
        simple load/last_cpu scoring

    Optional conservative load balancing:
        balance_load

    Per-CPU idle context:
        set_idle_context_for_cpu
        compatibility set_idle_context

    Preserve single-core behavior by default.

Important: prioritize correctness and compatibility over perfect SMP scalability in the first implementation.
