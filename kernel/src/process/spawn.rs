use super::{Process, ProcessState};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
use crate::capability::{CapabilityBroker, CapabilityRights};
use x86_64::{
    VirtAddr,
    structures::paging::{Page, PageTableFlags, PhysFrame},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    NotFound,
    PermissionDenied,
    ElfLoadFailed,
    NoMemory,
    InvalidPath,
}

/// Spawn a new process from a static ELF binary on the filesystem.
/// For the kernel, we embed the sunshell binary and look it up by path.
pub fn spawn_from_path(
    path: &str,
    _argv: &[&str],
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    _caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
) -> Result<usize, SpawnError> {
    // Find the embedded binary for the requested path.
    // The kernel embeds service binaries via include_bytes!.
    let bytes = match path {
        "/bin/sshl" | "/bin/sh" | "/bin/ssh" => {
            // SAFETY: These statics are embedded at kernel build time.
            crate::SUNSHELL_ELF_BYTES
        }
        _ => {
            crate::serial_println!("[SPAWN] Unknown path: {}", path);
            return Err(SpawnError::NotFound);
        }
    };

    crate::serial_println!("[SPAWN] Loading {} ({} bytes)", path, bytes.len());

    let pid = sched.processes.len() + 1;
    let mut process = unsafe {
        Process::new(pid, "sshl", pmm, hhdm_offset)
    };

    let entry = super::elf_loader::load_elf(bytes, &mut process, pmm, hhdm_offset);
    let entry = entry.ok_or(SpawnError::ElfLoadFailed)?;

    // Allocate user stack
    let stack_pages = (super::layout::USER_STACK_SIZE + 4095) / 4096;
    for i in 0..stack_pages {
        let page_addr = VirtAddr::new(super::layout::USER_STACK_TOP - (i + 1) * 4096);
        let page = Page::from_start_address(page_addr).unwrap();
        let frame_addr = pmm.alloc_frame().ok_or(SpawnError::NoMemory)?;
        let phys = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE;
        unsafe {
            process.address_space.map_page(page, phys, flags, pmm, hhdm_offset);
        }
    }

    process.init_context(entry, super::layout::USER_STACK_TOP);
    let actual_pid = process.pid;
    let _id = sched.add_process(process);

    crate::serial_println!("[SPAWN] {} spawned pid={}", path, actual_pid);
    Ok(actual_pid)
}
