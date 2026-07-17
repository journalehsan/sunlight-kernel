use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
use x86_64::VirtAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkError {
    Unsupported,
}

/// Fork remains disabled until the child can receive independently owned page
/// tables and user frames. This function must fail before touching any state.
pub fn fork_current_process(
    _pmm: &mut PhysicalMemoryManager,
    _sched: &mut Scheduler,
    _hhdm_offset: VirtAddr,
) -> Result<usize, ForkError> {
    Err(ForkError::Unsupported)
}
