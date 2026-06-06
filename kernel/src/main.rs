#![no_std]
#![no_main]
#![deny(warnings)]
#![allow(static_mut_refs)]
#![feature(abi_x86_interrupt)]

extern crate alloc;

use alloc::boxed::Box;

mod arch;
mod memory;
mod panic;
mod sched;

use arch::x86_64::{interrupts, serial};
use memory::{heap, pmm::PhysicalMemoryManager, vmm::VirtualMemoryManager};
use sched::Scheduler;
use x86_64::VirtAddr;

// Limine requests
static MEMMAP_REQ: limine::request::MemmapRequest = limine::request::MemmapRequest::new();
static HHDM_REQ: limine::request::HhdmRequest = limine::request::HhdmRequest::new();

// Global PMM
static PMM: spin::Mutex<PhysicalMemoryManager> = spin::Mutex::new(PhysicalMemoryManager::new());

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial::init();

    serial_println!("══════════════════════════════════════");
    serial_println!("  SunlightOS — Phase 1 Boot Sequence  ");
    serial_println!("══════════════════════════════════════");

    // 1. PMM
    serial_println!("[PMM] Initializing...");
    let memmap_response = MEMMAP_REQ.response().expect("no memmap from bootloader");
    let entries = memmap_response.entries();
    {
        let mut pmm = PMM.lock();
        // SAFETY: called exactly once before any alloc/free.
        unsafe { pmm.init(entries); }
        let (total, free) = pmm.stats();
        serial_println!("[PMM] {}/{} MiB free", free * 4 / 1024, total * 4 / 1024);
    }
    serial_println!("[PMM] OK");

    // 2. VMM
    serial_println!("[VMM] Initializing...");
    let hhdm_response = HHDM_REQ.response().expect("no hhdm from bootloader");
    let hhdm_offset = VirtAddr::new(hhdm_response.offset);
    let mut vmm = {
        // SAFETY: hhdm_offset is correct, page tables are valid.
        let vmm = unsafe { VirtualMemoryManager::init(hhdm_offset) };
        vmm
    };
    serial_println!("[VMM] OK");

    // 3. IDT + PIC
    interrupts::init();
    x86_64::instructions::interrupts::enable();
    serial_println!("[INT] Interrupts enabled");

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

    // 5. Scheduler
    serial_println!("[SCHED] Starting scheduler...");
    let sched = alloc::boxed::Box::leak(Box::new(Scheduler::new()));

    sched.spawn("thread-alpha", || {
        for i in 0..5 {
            serial_println!("[thread-alpha] tick {}", i);
            sched::yield_now();
        }
        serial_println!("[thread-alpha] done");
        sched::exit();
    });

    sched.spawn("thread-beta", || {
        for i in 0..5 {
            serial_println!("[thread-beta]  tick {}", i);
            sched::yield_now();
        }
        serial_println!("[thread-beta]  done");
        sched::exit();
    });

    // SAFETY: scheduler pointer set before run, never modified after.
    unsafe { sched::set_scheduler(sched); }
    sched.run();

    serial_println!("══════════════════════════════════════");
    serial_println!("[SunlightOS] Phase 1 OK");
    serial_println!("══════════════════════════════════════");

    loop {
        x86_64::instructions::hlt();
    }
}
