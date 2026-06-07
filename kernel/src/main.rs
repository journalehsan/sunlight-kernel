#![no_std]
#![no_main]
#![deny(warnings)]
#![allow(dead_code, unused_imports)]
#![allow(static_mut_refs)]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod arch;
mod capability;
mod ipc;
mod memory;
mod panic;
mod process;
mod sched;

use arch::x86_64::{interrupts, serial, syscall};
use memory::{heap, pmm::PhysicalMemoryManager, vmm::VirtualMemoryManager};
use process::{layout, Process};
use x86_64::VirtAddr;

static PMM: spin::Mutex<PhysicalMemoryManager> = spin::Mutex::new(PhysicalMemoryManager::new());

// Limine requests
static MEMMAP_REQ: limine::request::MemmapRequest = limine::request::MemmapRequest::new();
static HHDM_REQ: limine::request::HhdmRequest = limine::request::HhdmRequest::new();

// Embedded service binaries (must be built before kernel)
static INIT_ELF_BYTES: &[u8] =
    include_bytes!("../../target/x86_64-unknown-none/release/sunlight-init");
static TIMER_SERVER_ELF_BYTES: &[u8] =
    include_bytes!("../../target/x86_64-unknown-none/release/sunlight-timer-server");

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Enable interrupts early; the timer will fire and the handler will
    // only save context when a user process is actually running.
    unsafe { core::arch::asm!("sti"); }

    serial::init();

    serial_println!("══════════════════════════════════════");
    serial_println!("  SunlightOS — Phase 2 Boot Sequence  ");
    serial_println!("══════════════════════════════════════");

    // 1. PMM
    serial_println!("[PMM] Initializing...");
    let memmap_response = MEMMAP_REQ.response().expect("no memmap from bootloader");
    let entries = memmap_response.entries();
    let hhdm_response = HHDM_REQ.response().expect("no hhdm from bootloader");
    let hhdm_offset = VirtAddr::new(hhdm_response.offset);
    {
        let mut pmm = PMM.lock();
        unsafe { pmm.init(entries); }
        let (total, free) = pmm.stats();
        serial_println!("[PMM] {}/{} MiB free", free * 4 / 1024, total * 4 / 1024);
    }
    serial_println!("[PMM] OK");

    // 2. VMM
    serial_println!("[VMM] Initializing...");
    let mut vmm = unsafe { VirtualMemoryManager::init(hhdm_offset) };
    serial_println!("[VMM] OK");

    // 3. IDT + PIC + PIT
    interrupts::init();
    serial_println!("[IDT] OK");

    // 4. Heap
    serial_println!("[HEAP] Initializing 1 MiB kernel heap at {:#x}...", heap::HEAP_START.as_u64());
    {
        let mut pmm = PMM.lock();
        heap::init_heap(&mut vmm, &mut pmm);
    }
    {
        let v: alloc::vec::Vec<u32> = (0..16).collect();
        serial_println!("[HEAP] Test alloc OK: Vec of {} items", v.len());
    }
    serial_println!("[HEAP] OK");

    // 5. Syscall MSRs
    serial_println!("[SYSCALL] Setting up MSRs...");
    unsafe {
        syscall::setup_syscall_msrs(VirtAddr::new(syscall::syscall_entry as *const () as u64));
    }
    serial_println!("[SYSCALL] OK");

    // 6. Capability broker
    serial_println!("[CAP]  Capability broker initialized");
    capability::init_token_seed();

    // 7. IPC bus
    serial_println!("[IPC]  IPC bus initialized");

    // 8. Spawn init (pid=1)
    serial_println!("[PROC] Spawning init (pid=1)...");
    {
        let mut pmm = PMM.lock();
        let mut init = unsafe {
            Process::new(1, "init", &mut pmm, hhdm_offset)
        };
        serial_println!("[PROC] Loading init ELF ({} bytes)...", INIT_ELF_BYTES.len());
        let entry = process::elf_loader::load_elf(INIT_ELF_BYTES, &mut init, &mut pmm, hhdm_offset);
        if let Some(entry) = entry {
            // Map user stack
            let stack_pages = (layout::USER_STACK_SIZE + 4095) / 4096;
            for i in 0..stack_pages {
                let page_addr = VirtAddr::new(layout::USER_STACK_TOP - (i + 1) * 4096);
                let page = x86_64::structures::paging::Page::from_start_address(page_addr).unwrap();
                let frame_addr = pmm.alloc_frame().expect("stack alloc");
                let phys = unsafe { x86_64::structures::paging::PhysFrame::from_start_address_unchecked(frame_addr) };
                let flags = x86_64::structures::paging::PageTableFlags::PRESENT
                    | x86_64::structures::paging::PageTableFlags::WRITABLE
                    | x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE;
                unsafe {
                    init.address_space.map_page(page, phys, flags, &mut pmm, hhdm_offset);
                }
            }
            init.init_context(entry, layout::USER_STACK_TOP);
            sched::with_scheduler(|s| { s.add_process(init); });
        } else {
            serial_println!("[PROC] Failed to load init ELF");
        }
    }

    // 9. Spawn timer_server (pid=2)
    serial_println!("[PROC] Spawning timer_server (pid=2)...");
    {
        let mut pmm = PMM.lock();
        let mut timer = unsafe {
            Process::new(2, "timer_server", &mut pmm, hhdm_offset)
        };
        let entry = process::elf_loader::load_elf(TIMER_SERVER_ELF_BYTES, &mut timer, &mut pmm, hhdm_offset);
        if let Some(entry) = entry {
            let stack_pages = (layout::USER_STACK_SIZE + 4095) / 4096;
            for i in 0..stack_pages {
                let page_addr = VirtAddr::new(layout::USER_STACK_TOP - (i + 1) * 4096);
                let page = x86_64::structures::paging::Page::from_start_address(page_addr).unwrap();
                let frame_addr = pmm.alloc_frame().expect("stack alloc");
                let phys = unsafe { x86_64::structures::paging::PhysFrame::from_start_address_unchecked(frame_addr) };
                let flags = x86_64::structures::paging::PageTableFlags::PRESENT
                    | x86_64::structures::paging::PageTableFlags::WRITABLE
                    | x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE;
                unsafe {
                    timer.address_space.map_page(page, phys, flags, &mut pmm, hhdm_offset);
                }
            }
            timer.init_context(entry, layout::USER_STACK_TOP);
            sched::with_scheduler(|s| { s.add_process(timer); });
        } else {
            serial_println!("[PROC] Failed to load timer_server ELF");
        }
    }

    serial_println!("[PROC] Entering scheduler — dropping to Ring 3");
    serial_println!("══════════════════════════════════════");

    // Start scheduler — first process runs, kernel becomes interrupt-only
    // run_forever sets the first Ready process to Running and iretq's into it.
    // The saved RFLAGS in init_context has IF=1, so interrupts are enabled
    // when the process starts.
    unsafe { core::arch::asm!("sti"); }
    sched::enter_first_process()
}
