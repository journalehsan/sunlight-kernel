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
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{Page, PageTableFlags, PhysFrame},
};

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
static VFS_SERVER_ELF_BYTES: &[u8] =
    include_bytes!("../../target/x86_64-unknown-none/release/sunlight-vfs-server");

/// Virtual address in each user process at which the FAT32 share page is mapped.
const FAT_SHARE_VADDR: u64 = sunlight_fat::FAT_SHARE_VADDR;

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

    // 5. virtio-blk + FAT32 bootstrap
    // Initialize the block device, read FAT32 test files, and write them into a
    // shared physical page that will be mapped into the vfs_server's address space.
    let fat_share_phys = init_block_and_fat(hhdm_offset);

    // 6. Syscall MSRs
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

    // 7. Capability broker
    splash.set_status("Initializing capability broker");
    serial_println!("[CAP]  Capability broker initialized");
    splash.log("[CAP] Capability broker initialized");
    splash.set_progress(600);  // 60%
    splash.redraw();
    capability::init_token_seed();

    // 8. IPC bus
    splash.set_status("Initializing IPC bus");
    serial_println!("[IPC]  IPC bus initialized");
    serial_println!("[IPC]  IpcMsg format: fixed 80-byte struct");
    serial_println!("[IPC]  Syscalls: IpcCall IpcReplyWait IpcRecv NotifySend NotifyWait");
    serial_println!("[IPC]  Fastpath check: enabled (stub)");
    splash.log("[IPC] IPC bus initialized");
    splash.set_progress(700);  // 70%
    splash.redraw();

    // 9. Spawn init (pid=1)
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

    // 10. Spawn vfs_server (pid=3) with the FAT32 share page mapped
    serial_println!("[PROC] Spawning vfs_server (pid=3)...");
    splash.set_status("Loading vfs_server");
    splash.log("[PROC] Spawning vfs_server (pid=3)...");
    splash.redraw();
    {
        let mut pmm = PMM.lock();
        // SAFETY: hhdm_offset was provided by Limine and initialized before user process creation.
        let mut vfs = unsafe {
            Process::new(3, "vfs_server", &mut pmm, hhdm_offset)
        };
        let entry = process::elf_loader::load_elf(VFS_SERVER_ELF_BYTES, &mut vfs, &mut pmm, hhdm_offset);
        if let Some(entry) = entry {
            let stack_pages = (layout::USER_STACK_SIZE + 4095) / 4096;
            for i in 0..stack_pages {
                let page_addr = VirtAddr::new(layout::USER_STACK_TOP - (i + 1) * 4096);
                let page = x86_64::structures::paging::Page::from_start_address(page_addr).unwrap();
                let frame_addr = pmm.alloc_frame().expect("stack alloc");
                // SAFETY: pmm.alloc_frame returns a page-aligned physical frame start.
                let phys = unsafe { x86_64::structures::paging::PhysFrame::from_start_address_unchecked(frame_addr) };
                let flags = x86_64::structures::paging::PageTableFlags::PRESENT
                    | x86_64::structures::paging::PageTableFlags::WRITABLE
                    | x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE;
                // SAFETY: page and frame are valid user-stack mappings for this process address space.
                unsafe {
                    vfs.address_space.map_page(page, phys, flags, &mut pmm, hhdm_offset);
                }
            }

            // Map the FAT32 share page (read-only) at FAT_SHARE_VADDR in the vfs_server.
            // Always mapped: zeroed page when no block device, populated when disk present.
            // SAFETY: fat_share_phys is a page-aligned physical frame allocated by PMM.
            {
                let share_page = Page::from_start_address(VirtAddr::new(FAT_SHARE_VADDR))
                    .expect("FAT_SHARE_VADDR is not page-aligned");
                let share_frame = unsafe {
                    PhysFrame::from_start_address_unchecked(fat_share_phys)
                };
                let share_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
                unsafe {
                    vfs.address_space.map_page(share_page, share_frame, share_flags, &mut pmm, hhdm_offset);
                }
            }

            vfs.init_context(entry, layout::USER_STACK_TOP);
            sched::with_scheduler(|s| { s.add_process(vfs); });
            splash.log("[PROC] vfs_server pid=3");
        } else {
            serial_println!("[PROC] Failed to load vfs_server ELF");
            splash.log("[PROC] Failed to load vfs_server ELF");
        }
    }

    splash.set_progress(900);  // 90%
    splash.redraw();

    // 11. Spawn timer_server (pid=2)
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

    splash.set_progress(1000);  // 100%
    splash.set_status("SunlightOS ready");
    splash.set_kernel_status("OK");
    splash.log("[SunlightOS] Phase 2 OK");
    splash.redraw();

    serial_println!("[PROC] Entering scheduler — dropping to Ring 3");
    serial_println!("══════════════════════════════════════");

    // Start scheduler — first process runs, kernel becomes interrupt-only.
    // Interrupts are still disabled here; iretq_to_context will restore the
    // first process's RFLAGS (IF=1 from init_context), enabling them in user mode.
    // Do NOT call sti here — it creates a window where the timer interrupt fires
    // while enter_first_process holds the scheduler lock, causing a deadlock.
    sched::enter_first_process()
}

/// Initialize virtio-blk, read the FAT32 test files, and return the physical
/// address of the share page.
///
/// Always returns a valid physical address (the share page is always allocated
/// and mapped into vfs_server). The page is zeroed (magic=0) when no device
/// was found; vfs_server checks the magic and skips the boot mount gracefully.
///
/// Logs [BLK] and [FAT] gate lines to the serial port.
fn init_block_and_fat(hhdm_offset: VirtAddr) -> PhysAddr {
    serial_println!("[BLK]  Scanning PCI...");

    // Always allocate the share page; virtio queue and request buffer are only
    // used when a device is present.
    let share_phys = PMM.lock().alloc_frame().expect("fat share alloc");
    let share_virt = hhdm_offset.as_u64() + share_phys.as_u64();

    // Zero the share page so vfs_server gets a safe sentinel when no device exists.
    // SAFETY: share_virt is a valid HHDM-mapped kernel frame of 4096 bytes.
    unsafe { (share_virt as *mut u8).write_bytes(0, 4096) };

    // SAFETY: PCI port I/O requires ring-0, which we have during kernel boot.
    let blk_info = unsafe { sunlight_virtio::find_virtio_blk() };
    let (_, _, _, io_base) = match blk_info {
        Some(info) => info,
        None => {
            serial_println!("[BLK]  No virtio-blk found — /boot will be unavailable");
            return share_phys;
        }
    };

    // Allocate virtio queue and request buffer only when device is present.
    let (queue_phys, req_phys) = {
        let mut pmm = PMM.lock();
        let q = pmm.alloc_frames(sunlight_virtio::QUEUE_PAGES)
            .expect("virtio queue alloc");
        let r = pmm.alloc_frame().expect("virtio req alloc");
        (q, r)
    };

    let hhdm = hhdm_offset.as_u64();
    let queue_virt = hhdm + queue_phys.as_u64();
    let req_virt = hhdm + req_phys.as_u64();
    serial_println!("[BLK]  Found virtio-blk");

    // SAFETY: All physical/virtual addresses are valid kernel-allocated frames;
    // we hold ring-0 privilege for port I/O.
    let mut blk = match unsafe {
        sunlight_virtio::VirtioBlk::init(
            io_base,
            queue_phys.as_u64(),
            queue_virt,
            req_phys.as_u64(),
            req_virt,
        )
    } {
        Some(b) => b,
        None => {
            serial_println!("[BLK]  virtio-blk init failed");
            return share_phys;
        }
    };

    serial_println!("[BLK]  Negotiated features");
    serial_println!("[BLK]  Queue initialized");

    // Test: read LBA 0 (BPB sector)
    let mut sector0 = [0u8; 512];
    // SAFETY: blk was initialized with valid queue/req buffers above.
    if !unsafe { blk.read_block(0, &mut sector0) } {
        serial_println!("[BLK]  Read LBA 0 FAILED");
        return share_phys;
    }
    serial_println!("[BLK]  Read LBA 0 OK");

    // Initialize FAT32 using a closure that calls blk.read_block
    let mut blk_reader = |lba: u64, buf: &mut [u8; 512]| -> bool {
        // SAFETY: blk is valid and we are in single-threaded kernel boot.
        unsafe { blk.read_block(lba, buf) }
    };

    let mut fat = match sunlight_fat::Fat32::mount(&mut blk_reader) {
        Some(f) => f,
        None => {
            serial_println!("[FAT]  FAT32 detection failed");
            return share_phys;
        }
    };
    serial_println!("[FAT]  FAT32 detected");

    // Populate the share page with pre-read file contents
    // SAFETY: share_virt points to a valid writable physical frame (one page).
    let share = unsafe { &mut *(share_virt as *mut sunlight_fat::FatSharePage) };
    *share = sunlight_fat::FatSharePage::zeroed();

    let mut count = 0u32;

    // Read /HELLO.TXT from FAT32 root
    if count < sunlight_fat::share::MAX_SHARE_FILES as u32 {
        let entry = &mut share.files[count as usize];
        let src_path = b"/HELLO.TXT";
        let path_len = src_path.len().min(48);
        entry.path[..path_len].copy_from_slice(&src_path[..path_len]);
        entry.path_len = path_len as u32;

        if let Some(n) = fat.read_file(b"/HELLO.TXT", &mut entry.data) {
            entry.data_len = n as u32;
            count += 1;
        }
    }

    // Read /BOOT/PHASE35.TXT from FAT32
    if count < sunlight_fat::share::MAX_SHARE_FILES as u32 {
        let entry = &mut share.files[count as usize];
        let src_path = b"/BOOT/PHASE35.TXT";
        let path_len = src_path.len().min(48);
        entry.path[..path_len].copy_from_slice(&src_path[..path_len]);
        entry.path_len = path_len as u32;

        if let Some(n) = fat.read_file(b"/BOOT/PHASE35.TXT", &mut entry.data) {
            entry.data_len = n as u32;
            count += 1;
        }
    }

    share.count = count;
    share.magic = sunlight_fat::SHARE_MAGIC;

    share_phys
}
