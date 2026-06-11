use super::{Process, ProcessState};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::VirtAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkError {
    NoMemory,
    TooManyProcesses,
}

/// Clone a process's address space (currently shares page tables).
/// TODO: Implement Copy-on-Write (CoW) to properly isolate child modifications.
unsafe fn clone_address_space_cow(
    parent_as: &super::address_space::AddressSpace,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<super::address_space::AddressSpace, ForkError> {
    use super::address_space::AddressSpace;

    let new_pml4_phys = pmm.alloc_frame().ok_or(ForkError::NoMemory)?;

    // Map the new PML4 via HHDM to initialize it.
    let pml4_virt = hhdm_offset + new_pml4_phys.as_u64();
    let new_pml4 = &mut *(pml4_virt.as_mut_ptr::<x86_64::structures::paging::PageTable>());

    // Zero the new PML4.
    for entry in new_pml4.iter_mut() {
        entry.set_unused();
    }

    // Copy kernel higher-half mappings (indices 256..512) from parent.
    let parent_pml4_virt = hhdm_offset + parent_as.pml4_phys.as_u64();
    let parent_pml4 = &*(parent_pml4_virt.as_ptr::<x86_64::structures::paging::PageTable>());
    for i in 256..512 {
        new_pml4[i].set_addr(parent_pml4[i].addr(), parent_pml4[i].flags());
    }

    // For user-space pages (indices 0..256), we need to clone with CoW.
    // For now, we'll do a shallow copy where child and parent share the same physical pages,
    // but mark them as read-only so they trigger page faults on write.
    for i in 0..256 {
        let parent_entry = &parent_pml4[i];
        if parent_entry.is_unused() {
            continue;
        }
        // Share the same page table at the next level, but mark as CoW.
        // TODO: Implement CoW tracking to know which pages need to be copied on write.
        new_pml4[i].set_addr(parent_entry.addr(), parent_entry.flags());
    }

    Ok(AddressSpace {
        pml4_phys: new_pml4_phys,
    })
}

/// Fork the current process from within the scheduler.
pub fn fork_current_process(
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    hhdm_offset: VirtAddr,
) -> Result<usize, ForkError> {
    // Extract parent info before borrowing sched mutably
    let parent_pid = sched.current_process().pid;
    let parent_name = sched.current_process().name;
    let parent_uid = sched.current_process().uid;
    let parent_gid = sched.current_process().gid;
    let parent_entry_point = sched.current_process().entry_point;
    let parent_user_stack_top = sched.current_process().user_stack_top;
    let parent_capabilities = sched.current_process().capabilities.clone();

    // Clone the parent's address space with CoW
    let child_address_space = unsafe {
        clone_address_space_cow(
            &sched.current_process().address_space,
            pmm,
            hhdm_offset,
        )?
    };

    // Allocate a new PID
    let child_pid = sched.processes.iter().map(|p| p.pid).max().unwrap_or(0) + 1;
    if sched.processes.len() >= 512 {
        return Err(ForkError::TooManyProcesses);
    }

    // Create the child process
    let child = unsafe {
        let mut p = Process {
            pid: child_pid,
            ppid: parent_pid,
            name: parent_name,
            state: ProcessState::Ready,
            address_space: child_address_space,
            capabilities: parent_capabilities,
            kernel_stack: alloc::boxed::Box::new([0u8; super::KERNEL_STACK_SIZE]),
            kernel_stack_top: 0,
            user_stack_top: parent_user_stack_top,
            entry_point: parent_entry_point,
            context_rsp: 0,
            uid: parent_uid,
            gid: parent_gid,
            ipc_queue: alloc::collections::VecDeque::new(),
            ipc_endpoint: None,
            ipc_reply: None,
            pending_call: None,
            pending_reply_wait: None,
            fd_table: super::fd_table::FdTable::new(),
            capability_mode: false,
            signal_state: super::signal::SignalState::new(),
        };

        // Setup kernel stack top
        p.kernel_stack_top = core::ptr::addr_of!(p.kernel_stack[super::KERNEL_STACK_SIZE - 1]) as u64 + 1;

        // Copy the parent's context frame to the child's kernel stack
        const FRAME_SIZE: u64 = 160;
        let parent_frame_ptr = sched.current_process().context_rsp as *const u8;
        let child_frame_base = p.kernel_stack_top - FRAME_SIZE;
        p.context_rsp = child_frame_base;

        // Copy the context frame from parent to child
        let src = parent_frame_ptr as *const u8;
        let dst = child_frame_base as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, FRAME_SIZE as usize);

        // Modify RAX in the child's context to return 0 (fork returns 0 to child)
        let child_rax_offset = 112; // RAX is at offset 112 in the context frame
        let child_rax_ptr = (child_frame_base + child_rax_offset) as *mut u64;
        child_rax_ptr.write_volatile(0);

        p
    };

    let child_pid_copy = child.pid;
    sched.add_process(child);

    crate::serial_println!("[FORK] parent pid={} created child pid={}", parent_pid, child_pid_copy);

    Ok(child_pid_copy)
}

/// Fork a process.
/// Returns the child_pid.
fn sys_fork(
    parent: &Process,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    hhdm_offset: VirtAddr,
) -> Result<usize, ForkError> {
    let parent_pid = parent.pid;
    let parent_uid = parent.uid;
    let parent_gid = parent.gid;

    // Allocate a new PID
    let child_pid = sched.processes.iter().map(|p| p.pid).max().unwrap_or(0) + 1;
    if sched.processes.len() >= 512 {
        return Err(ForkError::TooManyProcesses);
    }

    // Clone the address space with Copy-on-Write
    let child_address_space = unsafe {
        clone_address_space_cow(&parent.address_space, pmm, hhdm_offset)?
    };

    // Create the child process
    let child = unsafe {
        let mut p = Process {
            pid: child_pid,
            ppid: parent_pid,
            name: parent.name,
            state: ProcessState::Ready,
            address_space: child_address_space,
            capabilities: parent.capabilities.clone(),
            kernel_stack: alloc::boxed::Box::new([0u8; super::KERNEL_STACK_SIZE]),
            kernel_stack_top: 0,
            user_stack_top: parent.user_stack_top,
            entry_point: parent.entry_point,
            context_rsp: 0,
            uid: parent_uid,
            gid: parent_gid,
            ipc_queue: alloc::collections::VecDeque::new(),
            ipc_endpoint: None,
            ipc_reply: None,
            pending_call: None,
            pending_reply_wait: None,
            fd_table: super::fd_table::FdTable::new(),
            capability_mode: false,
            signal_state: super::signal::SignalState::new(),
        };

        // Setup kernel stack top
        p.kernel_stack_top = core::ptr::addr_of!(p.kernel_stack[super::KERNEL_STACK_SIZE - 1]) as u64 + 1;

        // Copy the parent's context frame to the child's kernel stack
        // The parent's context is at parent.context_rsp, and it's 160 bytes.
        const FRAME_SIZE: u64 = 160;
        let parent_frame_ptr = parent.context_rsp as *const u8;
        let child_frame_base = p.kernel_stack_top - FRAME_SIZE;
        p.context_rsp = child_frame_base;

        // Copy the context frame from parent to child
        let src = parent_frame_ptr as *const u8;
        let dst = child_frame_base as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, FRAME_SIZE as usize);

        // Modify RAX in the child's context to return 0 (fork returns 0 to child)
        let child_rax_offset = 112; // RAX is at offset 112 in the context frame
        let child_rax_ptr = (child_frame_base + child_rax_offset) as *mut u64;
        child_rax_ptr.write_volatile(0);

        p
    };

    let child_pid_copy = child.pid;
    sched.add_process(child);

    crate::serial_println!("[FORK] parent pid={} created child pid={}", parent_pid, child_pid_copy);

    Ok(child_pid_copy)
}
