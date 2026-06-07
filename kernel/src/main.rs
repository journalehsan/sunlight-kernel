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
static FB_REQ: limine::request::FramebufferRequest = limine::request::FramebufferRequest::new();

// Embedded service binaries (must be built before kernel)
static INIT_ELF_BYTES: &[u8] =
    include_bytes!("../../target/x86_64-unknown-none/release/sunlight-init");
static TIMER_SERVER_ELF_BYTES: &[u8] =
    include_bytes!("../../target/x86_64-unknown-none/release/sunlight-timer-server");

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Keep interrupts disabled during boot. The PIT is programmed when the IDT
    // is initialized, but timer IRQs must not preempt early boot while kernel
    // locks and scheduler state are still being initialized.
    x86_64::instructions::interrupts::disable();

    serial::init();

    // Initialize TUI from framebuffer (before PMM, no heap needed)
    let fb_resp = FB_REQ.response().expect("no framebuffer");
    let fb = fb_resp.framebuffers().first().expect("no framebuffer available");
    let mut splash = unsafe {
        sunlight_tui::SplashScreen::init(
            fb.address() as *mut u32,
            fb.width as u32,
            fb.height as u32,
            fb.pitch as u32,
            sunlight_tui::BootMode::Debug,
            0,  // RAM unknown yet, updated after PMM
        )
    };

    serial_println!("══════════════════════════════════════");
    serial_println!("  SunlightOS — Phase 2 Boot Sequence  ");
    serial_println!("══════════════════════════════════════");

    // 1. PMM
    serial_println!("[PMM] Initializing...");
    splash.set_status("Initializing physical memory");
    splash.set_progress(0);
    splash.log("[PMM] Initializing...");
    splash.redraw();
    let memmap_response = MEMMAP_REQ.response().expect("no memmap from bootloader");
    let entries = memmap_response.entries();
    let hhdm_response = HHDM_REQ.response().expect("no hhdm from bootloader");
    let hhdm_offset = VirtAddr::new(hhdm_response.offset);
    {
        let mut pmm = PMM.lock();
        unsafe { pmm.init(entries); }
        let (total, free) = pmm.stats();
        serial_println!("[PMM] {}/{} MiB free", free * 4 / 1024, total * 4 / 1024);
        splash.set_ram((total * 4 / 1024) as u32);
    }
    serial_println!("[PMM] OK");
    splash.log("[PMM] OK");
    splash.set_progress(100);  // 10%
    splash.redraw();

    // 2. VMM
    serial_println!("[VMM] Initializing...");
    splash.set_status("Setting up virtual memory");
    splash.log("[VMM] Initializing...");
    splash.redraw();
    let mut vmm = unsafe { VirtualMemoryManager::init(hhdm_offset) };
    serial_println!("[VMM] OK");
    splash.log("[VMM] OK");
    splash.set_progress(200);  // 20%
    splash.redraw();

    // 3. IDT + PIC + PIT
    splash.set_status("Loading interrupt handlers");
    splash.log("[IDT] Loading...");
    splash.redraw();
    interrupts::init();
    serial_println!("[IDT] OK");
    splash.log("[IDT] OK");
    splash.set_progress(300);  // 30%
    splash.redraw();

    // 4. Heap
    serial_println!("[HEAP] Initializing 1 MiB kernel heap at {:#x}...", heap::HEAP_START.as_u64());
    splash.set_status("Initializing kernel heap");
    splash.log("[HEAP] Initializing...");
    splash.redraw();
    {
        let mut pmm = PMM.lock();
        heap::init_heap(&mut vmm, &mut pmm);
    }
    {
        let v: alloc::vec::Vec<u32> = (0..16).collect();
        serial_println!("[HEAP] Test alloc OK: Vec of {} items", v.len());
    }
    serial_println!("[HEAP] OK");
    splash.log("[HEAP] OK");
    splash.set_progress(400);  // 40%
    splash.redraw();

    // 5. Syscall MSRs
    serial_println!("[SYSCALL] Setting up MSRs...");
    splash.set_status("Setting up system calls");
    splash.log("[SYSCALL] Setting up MSRs...");
    splash.redraw();
    unsafe {
        syscall::setup_syscall_msrs(VirtAddr::new(syscall::syscall_entry as *const () as u64));
    }
    serial_println!("[SYSCALL] OK");
    splash.log("[SYSCALL] OK");
    splash.set_progress(500);  // 50%
    splash.redraw();

    // 6. Capability broker
        splash.set_status("Initializing capability broker");
serial_println!("[CAP]  Capability broker initialized");
    splash.log("[CAP] Capability broker initialized");
    splash.set_progress(600);  // 60%
    splash.redraw();
    capability::init_token_seed();

    // 7. IPC bus
        splash.set_status("Initializing IPC bus");
serial_println!("[IPC]  IPC bus initialized");
    splash.log("[IPC] IPC bus initialized");
    splash.set_progress(700);  // 70%
    splash.redraw();

    // 8. Spawn init (pid=1)
        splash.set_status("Loading init process");
serial_println!("[PROC] Spawning init (pid=1)...");
    splash.log("[PROC] Spawning init (pid=1)...");
    splash.redraw();
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
            splash.log("[PROC] init pid=1");
        } else {
            serial_println!("[PROC] Failed to load init ELF");
            splash.log("[PROC] Failed to load init ELF");
        }
    }

    splash.set_progress(800);  // 80%
    splash.redraw();

    // 9. Spawn timer_server (pid=2)
    serial_println!("[PROC] Spawning timer_server (pid=2)...");
    splash.set_status("Loading timer_server");
    splash.log("[PROC] Spawning timer_server (pid=2)...");
    splash.redraw();
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
            splash.log("[PROC] timer_server pid=2");
        } else {
            serial_println!("[PROC] Failed to load timer_server ELF");
            splash.log("[PROC] Failed to load timer_server ELF");
        }
    }

    splash.set_progress(900);  // 90%
    splash.redraw();

    splash.set_progress(1000);  // 100%
    splash.set_status("SunlightOS ready");
    splash.set_kernel_status("OK");
    splash.log("[SunlightOS] Phase 2 OK");
    splash.redraw();

    serial_println!("[PROC] Entering scheduler — dropping to Ring 3");
    serial_println!("══════════════════════════════════════");

    // Start scheduler — first process runs, kernel becomes interrupt-only
    // run_forever sets the first Ready process to Running and iretq's into it.
    // The saved RFLAGS in init_context has IF=1, so interrupts are enabled
    // when the process starts.
    unsafe { core::arch::asm!("sti"); }
    sched::enter_first_process()
}
