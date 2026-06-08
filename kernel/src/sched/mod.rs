use crate::process::{Process, ProcessState};
use crate::serial_println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

pub const TIME_SLICE_TICKS: u64 = 10;

/// Flag set by timer IRQ when a reschedule is needed.
static NEEDS_RESCHEDULE: AtomicBool = AtomicBool::new(false);

pub struct Scheduler {
    pub processes: Vec<Process>,
    pub current: usize,
    pub current_ticks: u64,
    pub idle_context_rsp: u64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            processes: Vec::new(),
            current: 0,
            current_ticks: 0,
            idle_context_rsp: 0,
        }
    }

    /// Add a process to the scheduler.
    pub fn add_process(&mut self, process: Process) -> usize {
        let id = self.processes.len();
        serial_println!("[SCHED] add_process '{}' id={} current len={}", process.name, id, self.processes.len());
        self.processes.push(process);
        serial_println!("[SCHED] add_process done, new len={}", self.processes.len());
        id
    }

    /// Set the idle thread's context RSP.
    pub fn set_idle_context(&mut self, rsp: u64) {
        self.idle_context_rsp = rsp;
    }

    /// Called from timer IRQ — may set the reschedule flag.
    pub fn tick(&mut self) {
        self.current_ticks += 1;
        if self.current_ticks >= TIME_SLICE_TICKS {
            self.current_ticks = 0;
            NEEDS_RESCHEDULE.store(true, Ordering::SeqCst);
        }
    }

    /// Pick the next Ready process (round-robin).
    pub fn pick_next(&self) -> Option<usize> {
        let len = self.processes.len();
        if len == 0 {
            return None;
        }
        let start = (self.current + 1) % len;
        let mut idx = start;
        loop {
            if matches!(self.processes[idx].state, ProcessState::Ready) {
                return Some(idx);
            }
            idx = (idx + 1) % len;
            if idx == start {
                break;
            }
        }
        None
    }

    /// Get the currently running process.
    pub fn current_process(&self) -> &Process {
        &self.processes[self.current]
    }

    pub fn current_process_mut(&mut self) -> &mut Process {
        &mut self.processes[self.current]
    }

    pub fn is_blocked_on_recv(&self, pid: usize) -> bool {
        self.processes
            .iter()
            .any(|p| p.pid == pid && p.state == ProcessState::BlockedOnIpc)
    }

    pub fn wake_pid(&mut self, pid: usize) {
        if let Some(process) = self.processes.iter_mut().find(|p| p.pid == pid) {
            if process.state == ProcessState::BlockedOnIpc {
                process.state = ProcessState::Ready;
            }
        }
    }

    pub fn process_mut_by_pid(&mut self, pid: usize) -> Option<&mut Process> {
        self.processes.iter_mut().find(|p| p.pid == pid)
    }

    /// Run the scheduler — enter the first process and never return.
    pub fn run_forever(&mut self) -> ! {
        // Find first Ready process.
        let mut first = None;
        for (i, p) in self.processes.iter().enumerate() {
            serial_println!("[SCHED] process {} '{}' state={:?}", i, p.name, p.state);
            if matches!(p.state, ProcessState::Ready) {
                first = Some(i);
                break;
            }
        }

        if let Some(idx) = first {
            self.current = idx;
            self.processes[idx].state = ProcessState::Running;
            let rsp = self.processes[idx].context_rsp;
            serial_println!(
                "[SCHED] Entering process {} '{}' at rsp={:#x}",
                idx, self.processes[idx].name, rsp
            );
            // Switch to the process's address space before entering user space.
            unsafe {
                self.processes[idx].address_space.activate();
            }
            // SAFETY: rsp points to a valid context frame on the process's kernel stack.
            unsafe {
                context::iretq_to_context(rsp);
            }
        }

        // No user processes — enter idle loop directly.
        serial_println!("[SCHED] No user processes, entering idle");
        idle_loop();
    }
}

/// Idle loop — runs when no user process is Ready.
fn idle_loop() -> ! {
    loop {
        x86_64::instructions::interrupts::enable();
        x86_64::instructions::hlt();
    }
}

/// Check if a reschedule is needed and clear the flag.
pub fn check_reschedule() -> bool {
    NEEDS_RESCHEDULE.swap(false, Ordering::SeqCst)
}

/// Set the reschedule flag.
pub fn request_reschedule() {
    NEEDS_RESCHEDULE.store(true, Ordering::SeqCst);
}

/// Global scheduler instance.
pub static SCHEDULER: spin::Mutex<Scheduler> = spin::Mutex::new(Scheduler::new());

/// Access the global scheduler.
pub fn with_scheduler<F, R>(f: F) -> R
where
    F: FnOnce(&mut Scheduler) -> R,
{
    f(&mut SCHEDULER.lock())
}

/// Enter the first ready user process without holding the scheduler lock across
/// the privilege transition.
pub fn enter_first_process() -> ! {
    let (rsp, pml4_phys) = {
        let mut sched = SCHEDULER.lock();
        let mut first = None;
        for (i, p) in sched.processes.iter().enumerate() {
            serial_println!("[SCHED] process {} '{}' state={:?}", i, p.name, p.state);
            if matches!(p.state, ProcessState::Ready) {
                first = Some(i);
                break;
            }
        }

        if let Some(idx) = first {
            sched.current = idx;
            sched.processes[idx].state = ProcessState::Running;
            let rsp = sched.processes[idx].context_rsp;
            let pml4_phys = sched.processes[idx].address_space.pml4_phys;
            serial_println!(
                "[SCHED] Entering process {} '{}' at rsp={:#x}",
                idx, sched.processes[idx].name, rsp
            );
            (rsp, pml4_phys)
        } else {
            serial_println!("[SCHED] No user processes, entering idle");
            drop(sched);
            idle_loop();
        }
    };

    unsafe {
        x86_64::registers::control::Cr3::write(
            x86_64::structures::paging::PhysFrame::from_start_address_unchecked(pml4_phys),
            x86_64::registers::control::Cr3Flags::empty(),
        );
        context::iretq_to_context(rsp);
    }
}

/// Access the global scheduler and return the current process's context_rsp.
pub fn current_process_rsp() -> u64 {
    let sched = SCHEDULER.lock();
    sched.processes[sched.current].context_rsp
}

pub mod context;
