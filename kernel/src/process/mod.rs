pub mod address_space;
pub mod elf_loader;
pub mod env;
pub mod fd_table;
pub mod fork;
pub mod layout;
pub mod mmap;
pub mod pipe;
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
pub struct Process {
    pub pid: usize,
    pub ppid: usize, // parent pid
    pub name: [u8; 32],
    pub state: ProcessState,
    pub address_space: AddressSpace,
    pub capabilities: Vec<Capability>,
    pub kernel_stack: alloc::boxed::Box<[u8; KERNEL_STACK_SIZE]>,
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
    pub ipc_reply: Option<IpcMsg>,
    pub pending_call: Option<(u64, IpcMsg)>,
    pub pending_reply_wait: Option<(u32, IpcMsg)>,
    pub ipc_reply_target: Option<(u32, usize)>,
    pub fd_table: alloc::boxed::Box<fd_table::FdTable>,
    pub capability_mode: bool,
    pub signal_state: signal::SignalState,
    pub is_linux_compat: bool, // Phase 4.5: true if running Linux ELF binary
    /// Linux compatibility heap base for `brk(2)`.
    pub brk_base: u64,
    /// Current Linux compatibility heap break.
    pub brk_current: u64,
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

    /// Saved Linux terminal settings for ioctl(TCGETS/TCSETS) emulation.
    pub linux_termios: crate::arch::x86_64::syscall::LinuxTermios,

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
        let address_space = AddressSpace::new(pmm, hhdm_offset);

        let kernel_stack = new_kernel_stack();
        let kernel_stack_top = core::ptr::addr_of!(kernel_stack[KERNEL_STACK_SIZE - 1]) as u64 + 1;
        let user_stack_top = USER_STACK_TOP;

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
            capabilities: Vec::new(),
            kernel_stack,
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
            pending_reply_wait: None,
            ipc_reply_target: None,
            fd_table: fd_table::FdTable::new_boxed(),
            capability_mode: false,
            signal_state: signal::SignalState::new(),
            is_linux_compat: false, // default to native SunlightOS
            brk_base: 0,
            brk_current: 0,
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
            linux_termios: crate::arch::x86_64::syscall::LinuxTermios::default_cooked(),
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
        }
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

    /// Create a new thread that shares `parent_pml4`'s address space.
    ///
    /// Unlike `Process::new`, this does not allocate a new page table —
    /// `AddressSpace::from_pml4` wraps the existing one.  The caller is
    /// responsible for calling `init_context` + `set_initial_args` afterwards.
    ///
    /// # Phase 1 note
    /// The address space is shared by raw pointer identity (same pml4_phys).
    /// There is no reference count on AddressSpace; if the owner process exits
    /// first the PML4 page is not freed (currently leaked), which keeps the
    /// thread's page tables accessible.  Arc-based ownership is Phase 2.
    pub fn new_thread(
        pid: usize,
        ppid: usize,
        name: &str,
        parent_pml4: x86_64::PhysAddr,
        fd_table: alloc::boxed::Box<fd_table::FdTable>,
        env: env::EnvMap,
        uid: u32,
        gid: u32,
        nice: i8,
        capabilities: Vec<Capability>,
        tty_tab: Option<u8>,
    ) -> Self {
        let address_space = address_space::AddressSpace::from_pml4(parent_pml4);
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
            capabilities,
            kernel_stack,
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
            pending_reply_wait: None,
            ipc_reply_target: None,
            fd_table,
            capability_mode: false,
            signal_state: signal::SignalState::new(),
            is_linux_compat: false,
            brk_base: 0,
            brk_current: 0,
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
            linux_termios: crate::arch::x86_64::syscall::LinuxTermios::default_cooked(),
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
