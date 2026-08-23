pub mod address_space;
pub mod elf_loader;
pub mod env;
pub mod epoll;
pub mod fd_table;
pub mod fork;
pub mod layout;
pub(crate) mod mm2a_plan;
pub(crate) mod mm2b_state;
pub mod mmap;
pub mod pipe;
pub mod region;
pub mod signal;
pub mod spawn;
pub mod tty_io;

use crate::ipc::IpcMsg;
use crate::memory::pmm::PhysicalMemoryManager;
use address_space::AddressSpace;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use layout::USER_STACK_TOP;
use x86_64::VirtAddr;

pub const KERNEL_STACK_SIZE: usize = 32 * 1024;

pub fn new_kernel_stack() -> alloc::boxed::Box<[u8; KERNEL_STACK_SIZE]> {
    let mut stack = alloc::boxed::Box::<[u8; KERNEL_STACK_SIZE]>::new_uninit();
    // Avoid `Box::new([0; N])`: it builds a 32 KiB temporary on the current
    // kernel stack, which can overflow during spawn/fork syscalls.
    unsafe {
        core::ptr::write_bytes(stack.as_mut_ptr() as *mut u8, 0, KERNEL_STACK_SIZE);
        stack.assume_init()
    }
}

/// A schedulable process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcCallId {
    pub pid: usize,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingIpcCall {
    pub target_cap: u64,
    pub endpoint_id: u32,
    pub msg: IpcMsg,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcCallOutcome {
    ReplyDelivered(u64),
    DeadlineExpired(u64),
    ExplicitlyCancelled(u64),
    PeerClosed(u64),
}

impl IpcCallOutcome {
    pub const fn generation(self) -> u64 {
        match self {
            Self::ReplyDelivered(generation)
            | Self::DeadlineExpired(generation)
            | Self::ExplicitlyCancelled(generation)
            | Self::PeerClosed(generation) => generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcReplyTarget {
    pub endpoint_id: u32,
    pub call: IpcCallId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferredIpcReply {
    pub token: u64,
    pub target: IpcReplyTarget,
}

/// Linux-only process state. Keeping these fields together prevents the
/// compatibility ABI from leaking into native process semantics.
#[derive(Debug, Clone, Copy)]
pub struct LinuxProcessState {
    pub brk_base: u64,
    pub brk_current: u64,
    pub poll_wake_tick: Option<u64>,
    pub termios: crate::arch::x86_64::syscall::LinuxTermios,
    pub altstack: [u64; 3],
    pub tid_address: u64,
    pub robust_list_head: u64,
    pub robust_list_len: u64,
    pub note_ready_logged: bool,
}

impl LinuxProcessState {
    pub const fn new() -> Self {
        Self {
            brk_base: 0,
            brk_current: 0,
            poll_wake_tick: None,
            termios: crate::arch::x86_64::syscall::LinuxTermios::default_cooked(),
            altstack: [0, 2, 0],
            tid_address: 0,
            robust_list_head: 0,
            robust_list_len: 0,
            note_ready_logged: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ProcessPersonality {
    Native,
    Linux(LinuxProcessState),
}

pub struct Process {
    pub pid: usize,
    pub ppid: usize, // parent pid
    pub name: [u8; 32],
    pub state: ProcessState,
    pub address_space: AddressSpace,
    pub owns_address_space: bool,
    /// True only for a borrower created through native syscall 22. Synthetic
    /// MM-0 test borrowers leave this false so runtime diagnostics stay exact.
    pub native_thread: bool,
    pub capabilities: Vec<Capability>,
    /// Present while the task may execute. Reaping drops the allocation before
    /// the task slot becomes reusable.
    pub kernel_stack: Option<alloc::boxed::Box<[u8; KERNEL_STACK_SIZE]>>,
    pub kernel_stack_top: u64,
    pub user_stack_top: u64,
    pub entry_point: u64,
    pub context_rsp: u64,
    pub fs_base: u64,
    pub uid: u32,
    pub gid: u32,
    pub nice: i8,
    /// Exit code set by ProcessExit; read by waitpid once state is Finished.
    pub exit_code: i32,
    /// Environment variable registry (Phase 6.5 Step 2).
    /// Populated with defaults at spawn or inherited from the parent.
    pub env: env::EnvMap,
    pub ipc_queue: VecDeque<IpcMsg>,
    pub ipc_endpoint: Option<u32>,
    /// A synchronous call has exactly one terminal outcome. The scheduler lock
    /// serializes reply, deadline, cancel, and peer-close transitions; the call
    /// generation rejects stale timer entries and late replies.
    pub ipc_reply: Option<(u64, IpcMsg)>,
    pub pending_call: Option<PendingIpcCall>,
    pub ipc_call_outcome: Option<IpcCallOutcome>,
    pub ipc_call_generation: u64,
    pub ipc_next_deadline_tick: Option<u64>,
    pub ipc_deadline: Option<(u64, u64)>,
    /// Generation, absolute tick, and endpoint for a blocked timed receive.
    pub ipc_recv_deadline: Option<(u64, u64, u32)>,
    /// Completed receive deadline retained until that receive syscall retries.
    pub ipc_recv_timeout: Option<(u64, u32)>,
    pub ipc_recv_generation: u64,
    pub personality: ProcessPersonality,
    pub pending_reply_wait: Option<(u32, IpcMsg)>,
    pub ipc_reply_target: Option<IpcReplyTarget>,
    pub deferred_reply_targets: VecDeque<DeferredIpcReply>,
    pub next_deferred_reply_token: u64,
    pub fd_table: alloc::boxed::Box<fd_table::FdTable>,
    pub capability_mode: bool,
    pub signal_state: signal::SignalState,
    pub trusted_display_service: bool,
    /// Set only by the kernel's embedded-path resolver for sunlight-swapd.
    pub trusted_swap_admin_service: bool,
    /// Set only for the exact embedded freezram diagnostic applet path.
    pub trusted_zram_diagnostic: bool,
    /// Set only for the embedded PTY broker. It may resolve IPC caller
    /// credentials through the narrow PTY credential syscall.
    pub trusted_pty_service: bool,
    /// Set only for the embedded Mezzo session-lock policy service.
    pub trusted_lock_service: bool,
    /// Set only for the kernel-loaded TTY/login session service.
    pub trusted_tty_session_service: bool,
    /// Set only for the native desktop session manager service.
    pub trusted_session_service: bool,
    /// Kernel-installed identities derived from exact embedded executable
    /// paths. These are provenance markers, not process-name checks.
    pub trusted_wiseowl_braind: bool,
    pub trusted_wiseowl_console: bool,
    pub trusted_control_panel: bool,
    /// Trust-chain markers used only for the authenticated-session broker.
    pub trusted_service_manager: bool,
    pub trusted_auth_broker: bool,
    /// When set, this process may only resolve nameserver entries that map to
    /// the declared service capability profile.
    pub service_lookup_restrictions: Option<u64>,
    /// Next free virtual address for anonymous `mmap(addr=0)` allocations.
    /// 0 means "uninitialized"; the mmap handler lazily seeds it to the
    /// mmap region base on first use and bumps it per mapping so successive
    /// anonymous mappings don't alias the same VA range.
    pub mmap_next: u64,
    pub sched_type: u8, // SCHED_NORMAL=0, SCHED_FIFO=1 for real-time bypass
    pub weight: u32,    // CFS weight (default 1024)
    pub cpu_mask: u64,  // CPU affinity mask

    // === SMP scheduler fast-path fields ===
    /// Which logical CPU currently has this process as its current_task.
    /// u8::MAX means not currently running on any core.
    pub owning_core: u8,
    /// Which logical CPU's run-queue this process is enqueued in.
    /// u8::MAX means not in any ready queue.
    pub queued_on_core: u8,

    // === BORE Scheduling Metrics (Phase 3.0) ===
    /// Burst score: 0-1024 (0=interactive, 1024=CPU-bound)
    /// Lower scores → moved to HIGH priority queue
    pub burst_score: u32,
    /// Ticks consumed in current timeslice (0-10)
    pub timeslice_used: u32,
    /// Global tick counter when this process last ran
    pub last_run_tick: u64,
    /// Ticks spent blocked on IPC/IO (for interactivity detection)
    pub io_wait_time: u32,
    /// Latency bonus ticks for interactive processes (-50..+50)
    pub interactive_bonus: i32,
    /// Global tick when this process entered BlockedOnIpc state
    pub block_start_tick: u64,
    /// Counter for aging mechanism (prevent starvation)
    pub aging_counter: u32,
    /// Child pid this process is blocked in `waitpid` on, if any. Used to wake
    /// the parent from `BlockedOnIpc` when that child exits, instead of having
    /// the parent busy-spin in a yield loop.
    pub wait_child: Option<usize>,

    /// TTY tab this process is attached to, if any. Set when the shell is
    /// spawned for a tab and inherited by children, so a spawned app's fd0/fd1
    /// route to that tab's kernel stdin/stdout rings (see process::tty_io).
    pub tty_tab: Option<u8>,

    /// Shared memory regions this process owns (via shm_alloc / shm_create).
    pub owned_shared: alloc::vec::Vec<crate::memory::shared::SharedRegion>,
    /// Shared memory regions this process currently has mapped via tokens (owner + receivers).
    /// (token, base_virt, size_in_bytes)
    pub mapped_shared:
        alloc::vec::Vec<(crate::capability::CapabilityToken, x86_64::VirtAddr, usize)>,

    // === Scheduler bug-fix / feature fields ===
    /// Watchdog: maximum runtime per quantum, in ticks. None = disabled. [FEAT-1]
    pub wd_period_ticks: Option<u64>,
    /// Nice-weighted counter for RoundRobin promotion/demotion. [FEAT-3]
    pub counter: i32,
    /// Guards against double starvation boost within a single pick(). [FEAT-3]
    pub aging_boosted_this_pick: bool,
    /// Quantum override set on promotion, consumed by tick(). [FEAT-3]
    pub quantum_override: Option<u32>,

    // === CPU Accounting (for sunlight-top and scheduler) ===
    /// Total CPU runtime consumed by this process, in nanoseconds (monotonic).
    pub cpu_runtime_ns: u64,
    /// TSC-derived monotonic timestamp when this process last started running on CPU.
    /// 0 when not currently accruing (descheduled or never started).
    pub last_start_ns: u64,

    /// Current working directory, used to resolve relative paths in sys_open/chdir/getcwd.
    pub cwd: alloc::string::String,
    /// True while a finished task still has kernel-side resources pending
    /// final reclamation after its address space is no longer active.
    pub exit_cleanup_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Suspended,
    Finished,
    /// Fully reaped: kernel resources cleaned, slot is reusable by add_process.
    Reaped,
    BlockedOnIpc,
    /// Blocked waiting for a timer/sleep to expire. [FEAT-2]
    BlockedOnTimer,
    /// Blocked waiting for I/O completion. [FEAT-2]
    BlockedOnIo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueTier {
    High,
    Medium,
    Low,
}

/// A capability held by a process.
#[derive(Debug, Clone, Copy)]
pub struct Capability {
    pub token: u64,
    pub endpoint_id: u32,
    pub can_send: bool,
    pub can_recv: bool,
    pub can_grant: bool,
}

impl Process {
    pub fn is_linux_compat(&self) -> bool {
        matches!(self.personality, ProcessPersonality::Linux(_))
    }

    pub fn linux_state(&self) -> Option<&LinuxProcessState> {
        match &self.personality {
            ProcessPersonality::Linux(state) => Some(state),
            ProcessPersonality::Native => None,
        }
    }

    pub fn linux_state_mut(&mut self) -> Option<&mut LinuxProcessState> {
        match &mut self.personality {
            ProcessPersonality::Linux(state) => Some(state),
            ProcessPersonality::Native => None,
        }
    }

    /// Returns the process name as a `&str`, interpreting the fixed byte array
    /// up to the first NUL byte.
    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..len]).unwrap_or("?")
    }

    /// Create a new user process with its own address space.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn new(
        pid: usize,
        ppid: usize,
        name: &str,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Self {
        Self::try_new(pid, ppid, name, pmm, hhdm_offset)
            .expect("boot process address-space allocation failed")
    }

    /// Fallible process construction for syscall-reachable spawn/exec paths.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn try_new(
        pid: usize,
        ppid: usize,
        name: &str,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<Self, address_space::MappingError> {
        let address_space = AddressSpace::try_new(pmm, hhdm_offset)?;

        let kernel_stack = new_kernel_stack();
        let kernel_stack_top = core::ptr::addr_of!(kernel_stack[KERNEL_STACK_SIZE - 1]) as u64 + 1;
        let user_stack_top = USER_STACK_TOP;

        let mut name_arr = [0u8; 32];
        let nb = name.as_bytes();
        let nlen = nb.len().min(31);
        name_arr[..nlen].copy_from_slice(&nb[..nlen]);

        Ok(Self {
            pid,
            ppid,
            name: name_arr,
            state: ProcessState::Ready,
            address_space,
            owns_address_space: true,
            native_thread: false,
            capabilities: Vec::new(),
            kernel_stack: Some(kernel_stack),
            kernel_stack_top,
            user_stack_top,
            entry_point: 0,
            context_rsp: 0,
            fs_base: 0,
            uid: 0,
            gid: 0,
            nice: 0,
            exit_code: 0,
            env: env::EnvMap::new(),
            ipc_queue: VecDeque::new(),
            ipc_endpoint: None,
            ipc_reply: None,
            pending_call: None,
            ipc_call_outcome: None,
            ipc_call_generation: 0,
            ipc_next_deadline_tick: None,
            ipc_deadline: None,
            ipc_recv_deadline: None,
            ipc_recv_timeout: None,
            ipc_recv_generation: 0,
            personality: ProcessPersonality::Native,
            pending_reply_wait: None,
            ipc_reply_target: None,
            deferred_reply_targets: VecDeque::new(),
            next_deferred_reply_token: 0,
            fd_table: fd_table::FdTable::new_boxed(),
            capability_mode: false,
            signal_state: signal::SignalState::new(),
            trusted_display_service: false,
            trusted_swap_admin_service: false,
            trusted_zram_diagnostic: false,
            trusted_pty_service: false,
            trusted_lock_service: false,
            trusted_tty_session_service: false,
            trusted_session_service: false,
            trusted_wiseowl_braind: false,
            trusted_wiseowl_console: false,
            trusted_control_panel: false,
            trusted_service_manager: false,
            trusted_auth_broker: false,
            service_lookup_restrictions: None,
            mmap_next: 0,
            sched_type: 0,           // SCHED_NORMAL
            weight: 1024,            // default CFS weight
            cpu_mask: u64::MAX,      // all CPUs
            owning_core: u8::MAX,    // not running on any core
            queued_on_core: u8::MAX, // not in any ready queue
            burst_score: 256,        // Start at MEDIUM tier (interactive bias)
            timeslice_used: 0,       // Fresh quantum
            last_run_tick: 0,        // Will be set on first run
            io_wait_time: 0,         // No wait yet
            interactive_bonus: 20,   // Assume interactive initially
            block_start_tick: 0,     // Not blocked yet
            aging_counter: 0,        // No aging yet
            wait_child: None,        // Not waiting on a child
            tty_tab: None,           // Attached to a TTY tab only when spawned for one
            owned_shared: alloc::vec::Vec::new(),
            mapped_shared: alloc::vec::Vec::new(),
            wd_period_ticks: None,
            counter: 0,
            aging_boosted_this_pick: false,
            quantum_override: None,
            cpu_runtime_ns: 0,
            last_start_ns: 0,
            cwd: alloc::string::String::from("/"),
            exit_cleanup_pending: false,
        })
    }

    /// Build the initial context frame on the kernel stack for first entry.
    /// Layout matches the pop order used by `iretq_to_context` and the timer handler.
    pub fn init_context(&mut self, entry_point: u64, user_stack_top: u64) {
        self.entry_point = entry_point;
        self.user_stack_top = user_stack_top;

        // Frame layout (from context_rsp upward):
        // [+0]   r15
        // [+8]   r14
        // [+16]  r13
        // [+24]  r12
        // [+32]  rbp
        // [+40]  rbx
        // [+48]  r11
        // [+56]  r10
        // [+64]  r9
        // [+72]  r8
        // [+80]  rdi
        // [+88]  rsi
        // [+96]  rdx
        // [+104] rcx
        // [+112] rax
        // [+120] RIP
        // [+128] CS
        // [+136] RFLAGS
        // [+144] RSP
        // [+152] SS
        const FRAME_SIZE: u64 = 160;

        let frame_base = self.kernel_stack_top - FRAME_SIZE;
        self.context_rsp = frame_base;

        // SAFETY: frame_base is within the allocated kernel stack.
        unsafe {
            let base = frame_base as *mut u64;
            // 15 GPRs (all zero)
            for i in 0..15 {
                base.add(i).write_volatile(0);
            }
            // RIP
            base.add(15).write_volatile(entry_point);
            // CS (Ring 3 code)
            base.add(16).write_volatile(0x2B);
            // RFLAGS (IF set)
            base.add(17).write_volatile(0x202);
            // RSP (user stack top)
            base.add(18).write_volatile(user_stack_top);
            // SS (Ring 3 data)
            base.add(19).write_volatile(0x23);
        }
    }

    /// Create a new thread that shares the parent's address space and ledger.
    ///
    /// Unlike `Process::new`, this does not allocate a new page table. The
    /// checked shared handle wraps the existing root and MM-2C ledger. The caller is
    /// responsible for calling `init_context` + `set_initial_args` afterwards.
    ///
    pub fn new_thread(
        pid: usize,
        ppid: usize,
        name: &str,
        shared_address_space: address_space::SharedAddressSpaceHandle,
        fd_table: alloc::boxed::Box<fd_table::FdTable>,
        env: env::EnvMap,
        uid: u32,
        gid: u32,
        nice: i8,
        capabilities: Vec<Capability>,
        tty_tab: Option<u8>,
    ) -> Self {
        let address_space = address_space::AddressSpace::from_shared(shared_address_space);
        let kernel_stack = new_kernel_stack();
        let kernel_stack_top = core::ptr::addr_of!(kernel_stack[KERNEL_STACK_SIZE - 1]) as u64 + 1;

        let mut name_arr = [0u8; 32];
        let nb = name.as_bytes();
        let nlen = nb.len().min(31);
        name_arr[..nlen].copy_from_slice(&nb[..nlen]);

        Self {
            pid,
            ppid,
            name: name_arr,
            state: ProcessState::Ready,
            address_space,
            owns_address_space: false,
            native_thread: false,
            capabilities,
            kernel_stack: Some(kernel_stack),
            kernel_stack_top,
            user_stack_top: 0,
            entry_point: 0,
            context_rsp: 0,
            fs_base: 0,
            uid,
            gid,
            nice,
            exit_code: 0,
            env,
            ipc_queue: VecDeque::new(),
            ipc_endpoint: None,
            ipc_reply: None,
            pending_call: None,
            ipc_call_outcome: None,
            ipc_call_generation: 0,
            ipc_next_deadline_tick: None,
            ipc_deadline: None,
            ipc_recv_deadline: None,
            ipc_recv_timeout: None,
            ipc_recv_generation: 0,
            personality: ProcessPersonality::Native,
            pending_reply_wait: None,
            ipc_reply_target: None,
            deferred_reply_targets: VecDeque::new(),
            next_deferred_reply_token: 0,
            fd_table,
            capability_mode: false,
            signal_state: signal::SignalState::new(),
            trusted_display_service: false,
            trusted_swap_admin_service: false,
            trusted_zram_diagnostic: false,
            trusted_pty_service: false,
            trusted_lock_service: false,
            trusted_tty_session_service: false,
            trusted_session_service: false,
            trusted_wiseowl_braind: false,
            trusted_wiseowl_console: false,
            trusted_control_panel: false,
            trusted_service_manager: false,
            trusted_auth_broker: false,
            service_lookup_restrictions: None,
            mmap_next: 0,
            sched_type: 0,
            weight: 1024,
            cpu_mask: u64::MAX,
            owning_core: u8::MAX,
            queued_on_core: u8::MAX,
            burst_score: 256,
            timeslice_used: 0,
            last_run_tick: 0,
            io_wait_time: 0,
            interactive_bonus: 20,
            block_start_tick: 0,
            aging_counter: 0,
            wait_child: None,
            tty_tab,
            owned_shared: Vec::new(),
            mapped_shared: Vec::new(),
            wd_period_ticks: None,
            counter: 0,
            aging_boosted_this_pick: false,
            quantum_override: None,
            cpu_runtime_ns: 0,
            last_start_ns: 0,
            cwd: alloc::string::String::from("/"),
            exit_cleanup_pending: false,
        }
    }

    /// Set initial userspace argument registers for a freshly initialized context.
    pub fn set_initial_args(&mut self, rdi: u64, rsi: u64, rdx: u64, rcx: u64) {
        unsafe {
            let base = self.context_rsp as *mut u64;
            base.add(10).write_volatile(rdi);
            base.add(11).write_volatile(rsi);
            base.add(12).write_volatile(rdx);
            base.add(13).write_volatile(rcx);
        }
    }

    /// Determine which priority queue this process belongs to based on burst_score
    pub fn get_queue_tier(&self) -> QueueTier {
        match self.burst_score {
            0..=256 => QueueTier::High,     // Interactive
            257..=768 => QueueTier::Medium, // Mixed
            769..=1024 => QueueTier::Low,   // CPU-bound
            _ => QueueTier::Low,            // Clamp to Low for out-of-range values
        }
    }
}
