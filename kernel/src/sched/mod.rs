use alloc::vec::Vec;
use thread::{CpuContext, ThreadState};
// use crate::serial_println;

pub struct Scheduler {
    threads: Vec<thread::Thread>,
    current: usize,
    bootstrap_context: CpuContext,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            threads: Vec::new(),
            current: 0,
            bootstrap_context: CpuContext::empty(),
        }
    }

    /// Spawn a new kernel thread with entry point `f`.
    pub fn spawn(&mut self, name: &'static str, f: fn() -> !) -> usize {
        let id = self.threads.len();
        let mut thread = thread::Thread::new(id, name, f);
        thread.state = ThreadState::Ready;
        self.threads.push(thread);
        id
    }

    /// Yield current thread, switch to next Ready thread (round-robin).
    /// SAFETY: Must only be called from the currently running thread.
    pub fn yield_now(&mut self) {
        if self.threads.len() <= 1 {
            return;
        }

        let current = self.current;
        self.threads[current].state = ThreadState::Ready;

        // Find next ready thread.
        let mut next = (current + 1) % self.threads.len();
        let start = next;
        loop {
            if matches!(self.threads[next].state, ThreadState::Ready) {
                break;
            }
            next = (next + 1) % self.threads.len();
            if next == start {
                // No other ready threads; just return.
                self.threads[current].state = ThreadState::Running;
                return;
            }
        }

        self.threads[next].state = ThreadState::Running;
        self.current = next;

        // SAFETY: current != next, so references don't overlap.
        let threads = self.threads.as_mut_ptr();
        unsafe {
            context::switch_to(
                &mut (*threads.add(current)).context,
                &(*threads.add(next)).context,
            );
        }
    }

    /// Mark current thread as finished, switch away.
    /// SAFETY: Must only be called from the currently running thread. Never returns.
    pub fn exit(&mut self) -> ! {
        let current = self.current;
        self.threads[current].state = ThreadState::Finished;

        // Find next ready thread.
        let mut next = (current + 1) % self.threads.len();
        let start = next;
        let found = loop {
            if matches!(self.threads[next].state, ThreadState::Ready) {
                break true;
            }
            next = (next + 1) % self.threads.len();
            if next == start {
                break false;
            }
        };

        if !found {
            // Switch back to bootstrap context.
            unsafe {
                context::switch_to_exit(&self.bootstrap_context);
            }
        }

        self.threads[next].state = ThreadState::Running;
        self.current = next;

        // SAFETY: next context is valid.
        unsafe {
            context::switch_to_exit(
                &self.threads[next].context,
            );
        }
    }

    /// Run the scheduler from the initial bootstrap context.
    pub fn run(&mut self) {
        if self.threads.is_empty() {
            return;
        }
        self.current = 0;
        self.threads[0].state = ThreadState::Running;

        // SAFETY: saves bootstrap context and switches to thread 0.
        unsafe {
            context::switch_to(
                &mut self.bootstrap_context,
                &self.threads[0].context,
            );
        }
    }
}

// Global scheduler access for cooperative scheduling.
// SAFETY: only the currently running thread accesses this.
static mut SCHEDULER: *mut Scheduler = core::ptr::null_mut();

/// Set the global scheduler pointer.
/// SAFETY: must be called exactly once before yield_now/exit.
pub unsafe fn set_scheduler(s: &mut Scheduler) {
    SCHEDULER = s;
}

/// Yield the current thread.
/// SAFETY: must be called from the currently running thread.
pub fn yield_now() {
    // SAFETY: cooperative scheduler, only current thread accesses SCHEDULER.
    unsafe { (*SCHEDULER).yield_now() }
}

/// Exit the current thread. Never returns.
/// SAFETY: must be called from the currently running thread.
pub fn exit() -> ! {
    // SAFETY: cooperative scheduler, only current thread accesses SCHEDULER.
    unsafe { (*SCHEDULER).exit() }
}

pub mod context;
pub mod thread;
