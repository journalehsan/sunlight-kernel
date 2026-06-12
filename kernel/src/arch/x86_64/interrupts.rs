use crate::arch::x86_64::keyboard;
use crate::serial_println;
use x86_64::instructions::port::Port;
use x86_64::instructions::segmentation::Segment;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

static TSS: spin::Lazy<TaskStateSegment> = spin::Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    // RSP0: kernel stack used when entering ring 0 from ring 3.
    tss.privilege_stack_table[0] = {
        const STACK_SIZE: usize = 256 * 1024;
        static mut STACK0: [u8; STACK_SIZE] = [0; STACK_SIZE];
        let stack_start = VirtAddr::from_ptr(unsafe { &STACK0 });
        stack_start + STACK_SIZE as u64
    };
    // IST[0]: dedicated stack for double fault handler.
    tss.interrupt_stack_table[0] = {
        const STACK_SIZE: usize = 256 * 1024;
        static mut STACK1: [u8; STACK_SIZE] = [0; STACK_SIZE];
        let stack_start = VirtAddr::from_ptr(unsafe { &STACK1 });
        stack_start + STACK_SIZE as u64
    };
    tss
});

struct Selectors {
    code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

fn user_code_segment() -> Descriptor {
    // 64-bit ring 3 code: 0x00AFFA000000FFFF
    Descriptor::UserSegment(0x00AFFA000000FFFF)
}

fn user_data_segment() -> Descriptor {
    // 64-bit ring 3 data: 0x00AFF2000000FFFF
    Descriptor::UserSegment(0x00AFF2000000FFFF)
}

static GDT: spin::Lazy<(GlobalDescriptorTable, Selectors)> = spin::Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let code_selector = gdt.append(Descriptor::kernel_code_segment());
    let _data_selector = gdt.append(Descriptor::kernel_data_segment());
    let _user_code_compat = gdt.append(user_code_segment()); // index 3, selector 0x1B
    let _user_data = gdt.append(user_data_segment()); // index 4, selector 0x23
    let _user_code_64 = gdt.append(user_code_segment()); // index 5, selector 0x2B
    let tss_selector = gdt.append(Descriptor::tss_segment(&*TSS));
    (
        gdt,
        Selectors {
            code_selector,
            tss_selector,
        },
    )
});

/// Initialize IDT, GDT, PIC, and PIT.
pub fn init() {
    serial_println!("[IDT] Loading interrupt descriptor table...");

    GDT.0.load();
    unsafe {
        x86_64::instructions::segmentation::CS::set_reg(GDT.1.code_selector);
        x86_64::instructions::segmentation::SS::set_reg(x86_64::structures::gdt::SegmentSelector(
            0,
        ));
        x86_64::instructions::segmentation::DS::set_reg(x86_64::structures::gdt::SegmentSelector(
            0,
        ));
        x86_64::instructions::segmentation::ES::set_reg(x86_64::structures::gdt::SegmentSelector(
            0,
        ));
        x86_64::instructions::tables::load_tss(GDT.1.tss_selector);
    }

    let idt = unsafe { &mut IDT };

    idt.divide_error.set_handler_fn(divide_error_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(0);
    }
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);

    // Use naked timer handler to enable manual context switching.
    unsafe {
        idt[0x20].set_handler_addr(x86_64::VirtAddr::new(
            timer_entry as *const () as usize as u64,
        ));
    }

    // Keyboard IRQ1 handler (vector 0x21)
    idt[0x21].set_handler_fn(keyboard_entry);

    idt.load();

    remap_pic();
    init_pit();

    let mut pic1_data: Port<u8> = Port::new(0x21);
    let mut pic2_data: Port<u8> = Port::new(0xA1);
    unsafe {
        pic1_data.write(0xFC); // enable IRQ0 (timer) and IRQ1 (keyboard)
        pic2_data.write(0xFF);
    }

    serial_println!("[IDT] PIC remapped, timer IRQ0 enabled at ~100Hz");
    serial_println!("[IDT] OK");
}

fn io_wait() {
    unsafe {
        let mut port: Port<u8> = Port::new(0x80);
        port.write(0);
    }
}

fn remap_pic() {
    const PIC1_CMD: u16 = 0x20;
    const PIC1_DATA: u16 = 0x21;
    const PIC2_CMD: u16 = 0xA0;
    const PIC2_DATA: u16 = 0xA1;
    const ICW1_INIT: u8 = 0x11;
    const ICW4_8086: u8 = 0x01;

    let mut cmd1: Port<u8> = Port::new(PIC1_CMD);
    let mut data1: Port<u8> = Port::new(PIC1_DATA);
    let mut cmd2: Port<u8> = Port::new(PIC2_CMD);
    let mut data2: Port<u8> = Port::new(PIC2_DATA);

    unsafe {
        cmd1.write(ICW1_INIT);
        io_wait();
        cmd2.write(ICW1_INIT);
        io_wait();
        data1.write(0x20);
        io_wait();
        data2.write(0x28);
        io_wait();
        data1.write(0x04);
        io_wait();
        data2.write(0x02);
        io_wait();
        data1.write(ICW4_8086);
        io_wait();
        data2.write(ICW4_8086);
        io_wait();
        data1.write(0xFF);
        io_wait();
        data2.write(0xFF);
        io_wait();
    }
}

fn init_pit() {
    const PIT_CMD: u16 = 0x43;
    const PIT_CH0: u16 = 0x40;
    const MODE_3: u8 = 0x36;
    const DIVISOR: u16 = 11932;

    let mut cmd: Port<u8> = Port::new(PIT_CMD);
    let mut ch0: Port<u8> = Port::new(PIT_CH0);

    unsafe {
        cmd.write(MODE_3);
        ch0.write((DIVISOR & 0xFF) as u8);
        ch0.write((DIVISOR >> 8) as u8);
    }
}

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    serial_println!("[INT] Divide Error: {:?}", stack_frame);
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    serial_println!("[INT] Invalid Opcode: {:?}", stack_frame);
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial_println!("[INT] Double Fault: {:?}", stack_frame);
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn gpf_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    serial_println!(
        "[INT] General Protection Fault: {:?} code={}",
        stack_frame,
        error_code
    );
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn page_fault_handler(
    _stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let vaddr = x86_64::registers::control::Cr2::read_raw();

    // Check if this is a write fault (CoW candidate)
    if error_code.contains(PageFaultErrorCode::CAUSED_BY_WRITE) {
        // Try to handle as CoW page fault
        if handle_cow_page_fault(vaddr) {
            return; // CoW fault handled successfully
        }
    }

    // Not a CoW fault — unrecoverable
    serial_println!("[FAULT] Page Fault at {:#x}: code={:?}", vaddr, error_code);
    loop {
        x86_64::instructions::hlt();
    }
}

/// Handle Copy-on-Write page fault
/// Returns true if handled, false if unrecoverable
fn handle_cow_page_fault(vaddr: u64) -> bool {
    // Only handle user-space addresses
    if vaddr >= 0x0000_8000_0000_0000 {
        return false;
    }

    let page_addr = vaddr & !0xFFF; // Page-align
    let page = match x86_64::structures::paging::Page::from_start_address(x86_64::VirtAddr::new(
        page_addr,
    )) {
        Ok(p) => p,
        Err(_) => return false,
    };

    // Get current page mappings
    let hhdm = match crate::HHDM_REQ.response() {
        Some(resp) => x86_64::VirtAddr::new(resp.offset),
        None => return false,
    };

    let mut pmm = crate::PMM.lock();
    let mut sched = crate::sched::SCHEDULER.lock();

    let process = sched.current_process_mut();

    // Look up the current physical frame
    let old_phys = match unsafe { process.address_space.lookup_phys(page, hhdm) } {
        Some(phys) => phys,
        None => return false,
    };

    // Allocate a new frame for the copy
    let new_phys = match pmm.alloc_frame() {
        Some(phys) => phys,
        None => return false,
    };

    // Copy the page content
    let old_vaddr = hhdm + old_phys.as_u64();
    let new_vaddr = hhdm + new_phys.as_u64();

    unsafe {
        core::ptr::copy_nonoverlapping(
            old_vaddr.as_ptr::<u8>(),
            new_vaddr.as_mut_ptr::<u8>(),
            4096,
        );
    }

    // Remap the page as writable
    let new_frame = match x86_64::structures::paging::PhysFrame::from_start_address(new_phys) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let flags = x86_64::structures::paging::PageTableFlags::PRESENT
        | x86_64::structures::paging::PageTableFlags::WRITABLE
        | x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE;

    unsafe {
        process
            .address_space
            .map_page(page, new_frame, flags, &mut *pmm, hhdm);
    }

    crate::serial_println!(
        "[COW] CoW page fault at {:#x}: allocated new frame",
        page_addr
    );
    true
}

static TICKS: spin::Mutex<u64> = spin::Mutex::new(0);

/// Naked timer interrupt entry. Manually saves all GPRs to match the
/// `iretq_to_context` / `init_context` layout, calls the Rust handler,
/// and optionally switches context.
#[unsafe(naked)]
pub unsafe extern "C" fn timer_entry() {
    core::arch::naked_asm!(
        // Push all 15 GPRs in reverse pop order (rax first, r15 last)
        // so that after pushes rsp points to r15 and the layout matches
        // init_context / iretq_to_context.
        "push rax",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov rdi, rsp",
        "call timer_rust",
        // Save return value (new RSP) in r12 (callee-saved, preserved by timer_rust).
        "mov r12, rax",
        // Set IF=1 in the CPU-pushed RFLAGS on the current stack.
        // After 15 pushes, CPU RFLAGS is at rsp + 120 + 16 = rsp + 136.
        "mov rbx, [rsp + 136]",
        "or rbx, 0x200",
        "mov [rsp + 136], rbx",
        // Restore return value.
        "mov rax, r12",
        "test rax, rax",
        "jz 1f",
        "mov rsp, rax",
        "1:",
        // Pop in the same order as iretq_to_context (r15 first, rax last).
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rax",
        "iretq",
    );
}

/// Rust side of the timer handler.
/// `saved_rsp` points to the pushed registers on the kernel stack.
/// Returns 0 to resume the interrupted context, or a new RSP to switch.
#[no_mangle]
pub extern "C" fn timer_rust(saved_rsp: u64) -> u64 {
    // Send EOI immediately to reduce interrupt latency.
    unsafe {
        let mut cmd1: Port<u8> = Port::new(0x20);
        cmd1.write(0x20);
    }

    let mut ticks = TICKS.lock();
    *ticks += 1;
    let _t = *ticks;
    drop(ticks);

    // Poll key injection buffer for test automation (no IRQ1 needed)
    keyboard::poll_inject_buffer();

    // Disable interrupts while holding scheduler lock to prevent deadlock
    // if keyboard IRQ fires while we hold the lock
    x86_64::instructions::interrupts::disable();
    let mut sched = crate::sched::SCHEDULER.lock();
    sched.tick();

    // Every tick, enqueue a timer notification for timer_server (if it exists).
    let timer_endpoint = sched
        .processes
        .iter()
        .find(|p| p.name == "timer_server")
        .and_then(|p| p.ipc_endpoint.map(|endpoint| (endpoint, p.pid)));
    if let Some((endpoint_id, timer_pid)) = timer_endpoint {
        let mut bus = crate::ipc::IPC_BUS.lock();
        bus.send_timer_tick(endpoint_id, &mut sched, timer_pid);
    }

    let result = if crate::sched::check_reschedule() {
        let current = sched.current;
        // Guard: processes may be empty during early boot (interrupts enabled before
        // any process is loaded). Skip reschedule until the scheduler has entries.
        if current >= sched.processes.len() {
            0
        } else {
            // Save current context.
            sched.processes[current].context_rsp = saved_rsp;
            if sched.processes[current].state == crate::process::ProcessState::Running {
                sched.processes[current].state = crate::process::ProcessState::Ready;
                if crate::sched::SCHEDULER_MODE == crate::sched::SchedulerMode::Bore {
                    sched.enqueue_process(current);
                }
            }

            if let Some(next) = sched.pick_next() {
                let next_rsp = sched.processes[next].context_rsp;
                let next_stack_top = sched.processes[next].kernel_stack_top;
                sched.current = next;
                sched.processes[next].state = crate::process::ProcessState::Running;
                sched.processes[next].last_run_tick = sched.global_tick;

                // Switch address space.
                unsafe {
                    sched.processes[next].address_space.activate();
                }

                // Update TSS RSP0 for next interrupt.
                // SAFETY: timer handler runs with interrupts disabled; no concurrent TSS access.
                // Note: ltr is NOT needed here — the CPU reads RSP0 from the TSS memory
                // on each privilege-level stack switch. We only need to update the TSS contents.
                unsafe {
                    let tss_ptr = &*TSS as *const TaskStateSegment as *mut TaskStateSegment;
                    (*tss_ptr).privilege_stack_table[0] = VirtAddr::new(next_stack_top);
                }

                next_rsp
            } else {
                0
            }
        }
    } else {
        0
    };

    // Release scheduler lock by dropping it
    drop(sched);

    // Re-enable interrupts
    x86_64::instructions::interrupts::enable();

    result
}

#[allow(dead_code)]
pub fn ticks() -> u64 {
    *TICKS.lock()
}

// ---------------------------------------------------------------------------
// Keyboard IRQ1 handler
// ---------------------------------------------------------------------------

extern "x86-interrupt" fn keyboard_entry(_stack_frame: InterruptStackFrame) {
    keyboard::handle_irq1();

    // Send EOI to PIC
    unsafe {
        let mut cmd1: Port<u8> = Port::new(0x20);
        cmd1.write(0x20);
    }
}
