use crate::arch::x86_64::keyboard;
use crate::serial_println;
use core::sync::atomic::{AtomicU64, Ordering};
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

pub fn set_tss_rsp0(stack_top: u64) {
    unsafe {
        let tss_ptr = &*TSS as *const TaskStateSegment as *mut TaskStateSegment;
        (*tss_ptr).privilege_stack_table[0] = VirtAddr::new(stack_top);
    }
}

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

    // Mouse IRQ12 handler (vector 0x2C = 32 + 12)
    idt[0x2C].set_handler_fn(mouse_entry);

    idt.load();

    remap_pic();
    init_pit();

    let mut pic1_data: Port<u8> = Port::new(0x21);
    let mut pic2_data: Port<u8> = Port::new(0xA1);
    unsafe {
        pic1_data.write(0xFC); // enable IRQ0 (timer) and IRQ1 (keyboard) on PIC1
        pic2_data.write(0xEF); // enable IRQ12 (mouse) on PIC2 (bit 4 = IRQ12)
    }

    // Calibrate TSC against PIT for accurate per-process CPU accounting.
    // This uses direct PIT counter reads and completes in ~1-2 ms.
    calibrate_tsc_from_pit();
    let hz = TSC_HZ_APPROX.load(Ordering::Relaxed);
    if hz != 0 {
        serial_println!("[TIME] TSC calibrated ~{} Hz for monotonic ns clock", hz);
    } else {
        serial_println!("[TIME] TSC calibration unavailable; using tick-based ns fallback");
    }

    serial_println!("[IDT] PIC remapped, timer IRQ0 enabled at ~100Hz");
    serial_println!("[IDT] OK");
}

fn is_user_frame(stack_frame: &InterruptStackFrame) -> bool {
    stack_frame.code_segment.0 & 0x3 == 0x3
}

fn terminate_current_user_process(reason: &str, code: i32) -> ! {
    let kstack_top = crate::sched::finish_current_process(code, reason);
    crate::sched::request_reschedule();

    unsafe {
        if kstack_top != 0 {
            core::arch::asm!("mov rsp, {}", in(reg) kstack_top);
        }
        core::arch::asm!("sti", "2:", "hlt", "jmp 2b", options(noreturn),);
    }
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

/// Latch and read current PIT channel 0 countdown value.
/// Used for early TSC calibration against real hardware timebase.
fn read_pit_count() -> u16 {
    unsafe {
        let mut cmd: Port<u8> = Port::new(0x43);
        let mut ch0: Port<u8> = Port::new(0x40);
        // Latch count: channel 0, low+high, no mode change
        cmd.write(0x00);
        let lo = ch0.read();
        let hi = ch0.read();
        ((hi as u16) << 8) | (lo as u16)
    }
}

/// Calibrate TSC frequency using PIT hardware counter (no dependency on IRQs).
/// Computes fixed-point multiplier for cheap now_ns() in hot paths.
/// Safe to call with interrupts disabled; spins briefly (~1-2 ms).
pub fn calibrate_tsc_from_pit() {
    const PIT_INPUT_HZ: u64 = 1_193_182;
    // Measure over a modest delta to get decent precision without long spin.
    let start_count = read_pit_count() as u64;
    let tsc0 = unsafe { core::arch::x86_64::_rdtsc() };
    // Target ~2000 PIT counts drop (~1.6 ms at 1.19MHz). Enough for ~GHz class CPU.
    let target_drop: u64 = 2000;
    let mut spins: u32 = 0;
    loop {
        let cur = read_pit_count() as u64;
        let dropped = if start_count >= cur {
            start_count - cur
        } else {
            // handle wrap of 16-bit counter
            start_count + ((0xFFFFu64 - cur) + 1)
        };
        if dropped >= target_drop {
            break;
        }
        spins += 1;
        if spins > 10_000_000 {
            // Safety timeout: leave uncalibrated, fallbacks will be used.
            return;
        }
        core::hint::spin_loop();
    }
    let tsc1 = unsafe { core::arch::x86_64::_rdtsc() };
    let tsc_delta = tsc1.saturating_sub(tsc0);
    let dropped = {
        let cur = read_pit_count() as u64;
        if start_count >= cur {
            start_count - cur
        } else {
            start_count + (0xFFFF - cur) + 1
        }
    };
    if tsc_delta < 1000 || dropped == 0 {
        return;
    }
    // freq_hz = (tsc_delta / time_seconds)
    // time_s = dropped / PIT_INPUT_HZ
    let hz = tsc_delta.saturating_mul(PIT_INPUT_HZ) / dropped;
    if hz < 100_000_000 {
        // Unrealistically low (or calibration on very slow emu); keep fallback.
        return;
    }
    TSC_HZ_APPROX.store(hz, Ordering::SeqCst);
    // Fixed point: ns = (dt_tsc * mul) >> 32
    // mul = (1_000_000_000 << 32) / hz
    let mul = ((1_000_000_000u128 << 32) / (hz as u128)) as u64;
    TSC_TO_NS_MUL.store(mul, Ordering::SeqCst);
    TSC_ORIGIN.store(tsc0, Ordering::SeqCst);
}

/// Return current monotonic time in nanoseconds since an arbitrary boot origin.
/// Uses calibrated TSC when available; otherwise coarse tick-based fallback.
/// Cost: one rdtsc + mul/shift (very cheap) when calibrated.
pub fn now_ns() -> u64 {
    let mul = TSC_TO_NS_MUL.load(Ordering::Relaxed);
    if mul != 0 {
        let origin = TSC_ORIGIN.load(Ordering::Relaxed);
        let tsc = unsafe { core::arch::x86_64::_rdtsc() };
        let dt = tsc.saturating_sub(origin);
        return ((dt as u128 * (mul as u128)) >> 32) as u64;
    }
    // Fallback: 100 Hz ticks * 10_000_000 ns
    ticks().saturating_mul(10_000_000)
}

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    // Diagnostic 1d: log pid/rip/rsp for every fault (before existing log)
    let pid =
        crate::sched::with_scheduler(|s| s.processes.get(s.current).map(|p| p.pid).unwrap_or(0));
    serial_println!(
        "[FAULT] #0 pid={} rip=0x{:x} rsp=0x{:x} err=0x{:x}",
        pid,
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        0u64
    );
    serial_println!("[INT] Divide Error: {:?}", stack_frame);
    if is_user_frame(&stack_frame) {
        terminate_current_user_process("divide-error", 128 + 8);
    }
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    // Diagnostic 1d: log pid/rip/rsp for #UD
    let pid =
        crate::sched::with_scheduler(|s| s.processes.get(s.current).map(|p| p.pid).unwrap_or(0));
    serial_println!(
        "[FAULT] #6 pid={} rip=0x{:x} rsp=0x{:x} err=0x{:x}",
        pid,
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        0u64
    );
    serial_println!("[INT] Invalid Opcode: {:?}", stack_frame);
    if is_user_frame(&stack_frame) {
        terminate_current_user_process("invalid-opcode", 128 + 4);
    }
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    let pid =
        crate::sched::with_scheduler(|s| s.processes.get(s.current).map(|p| p.pid).unwrap_or(0));
    serial_println!(
        "[FAULT] #8 pid={} rip=0x{:x} rsp=0x{:x} err=0x{:x}",
        pid,
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        _error_code
    );
    serial_println!("[INT] Double Fault: {:?}", stack_frame);
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn gpf_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    // Diagnostic 1d: log pid/rip/rsp for #GP
    let pid =
        crate::sched::with_scheduler(|s| s.processes.get(s.current).map(|p| p.pid).unwrap_or(0));
    serial_println!(
        "[FAULT] #13 pid={} rip=0x{:x} rsp=0x{:x} err=0x{:x}",
        pid,
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        error_code
    );
    serial_println!(
        "[INT] General Protection Fault: {:?} code={}",
        stack_frame,
        error_code
    );
    if is_user_frame(&stack_frame) {
        terminate_current_user_process("general-protection", 128 + 11);
    }
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn page_fault_handler(
    _stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    // Diagnostic 1d: log pid/rip/rsp for #PF (before the vaddr read)
    let pid =
        crate::sched::with_scheduler(|s| s.processes.get(s.current).map(|p| p.pid).unwrap_or(0));
    let rip = _stack_frame.instruction_pointer.as_u64();
    let rsp = _stack_frame.stack_pointer.as_u64();
    serial_println!(
        "[FAULT] #14 pid={} rip=0x{:x} rsp=0x{:x} err=0x{:x}",
        pid,
        rip,
        rsp,
        0u64
    );
    let vaddr = x86_64::registers::control::Cr2::read_raw();

    // Not-present fault: check whether this page was swapped out to ZRAM.
    if !error_code.contains(PageFaultErrorCode::PROTECTION_VIOLATION)
        && handle_swap_page_fault(vaddr)
    {
        return;
    }

    // Check if this is a write fault (CoW candidate)
    if error_code.contains(PageFaultErrorCode::CAUSED_BY_WRITE) {
        // Try to handle as CoW page fault
        if handle_cow_page_fault(vaddr) {
            return; // CoW fault handled successfully
        }
    }

    // Not a CoW fault — unrecoverable
    serial_println!("[FAULT] Page Fault at {:#x}: code={:?}", vaddr, error_code);
    if is_user_frame(&_stack_frame) {
        terminate_current_user_process("page-fault", 128 + 11);
    }
    loop {
        x86_64::instructions::hlt();
    }
}

/// Handle a fault on a page that was swapped out to ZRAM.
/// Returns true if `vaddr`'s page held a swapped marker and was faulted back in.
fn handle_swap_page_fault(vaddr: u64) -> bool {
    if vaddr >= 0x0000_8000_0000_0000 {
        return false;
    }

    let page_addr = vaddr & !0xFFF;
    let page = match x86_64::structures::paging::Page::from_start_address(x86_64::VirtAddr::new(
        page_addr,
    )) {
        Ok(p) => p,
        Err(_) => return false,
    };

    let hhdm = match crate::HHDM_REQ.response() {
        Some(resp) => x86_64::VirtAddr::new(resp.offset),
        None => return false,
    };

    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();
    let process = sched.current_process_mut();
    let pid = process.pid;

    if unsafe { process.address_space.swapped_block_id(page, hhdm) }.is_none() {
        return false;
    }

    let flags = x86_64::structures::paging::PageTableFlags::PRESENT
        | x86_64::structures::paging::PageTableFlags::WRITABLE
        | x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE
        | x86_64::structures::paging::PageTableFlags::NO_EXECUTE;

    match unsafe {
        crate::memory::swap::swap_in_page(
            &mut process.address_space,
            page,
            pid,
            flags,
            hhdm,
            &mut pmm,
        )
    } {
        Ok(_) => {
            crate::serial_println!("[SWAP] page-in at {:#x}", page_addr);
            true
        }
        Err(e) => {
            crate::serial_println!("[SWAP] page-in failed at {:#x}: {:?}", page_addr, e);
            false
        }
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

    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();

    let process = sched.current_process_mut();

    // Look up the current physical frame
    let old_phys = match unsafe { process.address_space.lookup_phys(page, hhdm) } {
        Some(phys) => phys,
        None => return false,
    };

    // Allocate a new frame for the copy
    let new_phys = match pmm.alloc_frame_owned(process.pid as u32) {
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
static TELEMETRY_TICK_COUNT: AtomicU64 = AtomicU64::new(0);

// === Monotonic kernel clock (TSC calibrated) ===
// These are initialized during calibrate_tsc_from_pit().
static TSC_ORIGIN: AtomicU64 = AtomicU64::new(0);
static TSC_TO_NS_MUL: AtomicU64 = AtomicU64::new(0); // (dt * mul) >> 32 == nanoseconds
static TSC_HZ_APPROX: AtomicU64 = AtomicU64::new(0);

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

/// Wake tty_server every N timer ticks for the foreground render cadence.
/// Timer is ~100 Hz, so 3 ticks ≈ 30 ms ≈ ~33 FPS. Larger = less CPU but
/// choppier; smaller = smoother but starves the foreground app of CPU.
const TTY_WAKE_INTERVAL_TICKS: u64 = 3;

/// Rust side of the timer handler.
/// `saved_rsp` points to the pushed registers on the kernel stack.
/// Returns 0 to resume the interrupted context, or a new RSP to switch.
#[no_mangle]
pub extern "C" fn timer_rust(saved_rsp: u64) -> u64 {
    // Send EOI immediately to reduce interrupt latency.
    unsafe {
        let mut cmd1: Port<u8> = Port::new(0x20);
        cmd1.write(0x20);
        // noisy per-tick debug log intentionally disabled
        // crate::serial_println!("[IRQ0] Timer interrupt - EOI sent to PIC");
    }

    let mut ticks = TICKS.lock();
    *ticks += 1;
    let _t = *ticks;
    drop(ticks);
    let ticks_total = TELEMETRY_TICK_COUNT
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);

    // Poll key injection buffer for test automation (no IRQ1 needed)
    keyboard::poll_inject_buffer();

    // Disable interrupts while holding scheduler lock to prevent deadlock
    // if keyboard IRQ fires while we hold the lock
    x86_64::instructions::interrupts::disable();
    let mut sched = crate::sched::SCHEDULER.lock();
    sched.tick();

    if ticks_total % 100 == 0 {
        let pmm = crate::PMM.lock();
        // SAFETY: timer handler has interrupts disabled during scheduler mutation and telemetry update.
        unsafe {
            crate::telemetry::update_telemetry(&mut sched, &pmm, ticks_total);
        }
    }

    // Every tick, enqueue a timer notification for timer_server (if it exists).
    let timer_endpoint = sched
        .processes
        .iter()
        .find(|p| p.name_str() == "timer_server")
        .and_then(|p| p.ipc_endpoint.map(|endpoint| (endpoint, p.pid)));

    if let Some((endpoint_id, timer_pid)) = timer_endpoint {
        let mut bus = crate::ipc::IPC_BUS.lock();
        bus.send_timer_tick(endpoint_id, &mut sched, timer_pid);
    }

    // Wake tty_server on a render cadence. Its idle loop parks in BlockedOnIpc
    // between events (an empty ipc_reply_wait blocks via block_on_recv), so
    // without this it only ran — and only repainted a live foreground app like
    // `top` — when a keyboard IRQ happened to wake it. Waking every few ticks
    // gives a steady ~30 FPS refresh; spacing it (not every tick) leaves CPU
    // for the foreground app to actually produce frames instead of starving it.
    if ticks_total % TTY_WAKE_INTERVAL_TICKS == 0 {
        if let Some(tty_pid) = sched
            .processes
            .iter()
            .find(|p| p.name_str() == "tty_server")
            .map(|p| p.pid)
        {
            sched.wake_pid(tty_pid);
        }
    }

    let result = if crate::sched::check_reschedule() {
        let current = sched.current;
        // Guard: processes may be empty during early boot (interrupts enabled before
        // any process is loaded). Skip reschedule until the scheduler has entries.
        if current >= sched.processes.len() {
            0
        } else {
            // === CPU Accounting (Phase 1) + Phase 4 churn detection ===
            // Charge exact runtime. If this was an extremely short burst, penalize
            // burst_score temporarily to avoid scheduler/idle churn from thrashing tasks.
            sched.account_and_apply_churn_penalty();

            // Save current context.
            sched.processes[current].context_rsp = saved_rsp;
            sched.processes[current].fs_base =
                unsafe { x86_64::registers::model_specific::Msr::new(0xC0000100).read() };

            // Phase 2: maintain runnable-only invariant in tier queues.
            let was_runnable = matches!(
                sched.processes[current].state,
                crate::process::ProcessState::Ready | crate::process::ProcessState::Running
            );
            if sched.processes[current].state == crate::process::ProcessState::Running {
                sched.processes[current].state = crate::process::ProcessState::Ready;
            }
            // If it is still (or became) Ready, ensure it is enqueued for BORE.
            // If it became Blocked*/Suspended/Finished, ensure it is NOT queued.
            if matches!(
                sched.processes[current].state,
                crate::process::ProcessState::Ready
            ) {
                if crate::sched::SCHEDULER_MODE == crate::sched::SchedulerMode::Bore {
                    sched.enqueue_process_once(current);
                }
            } else if !was_runnable
                || !matches!(
                    sched.processes[current].state,
                    crate::process::ProcessState::Ready
                )
            {
                sched.remove_from_ready_queues(current);
            }

            if let Some(next) = sched.pick_next() {
                let next_rsp = sched.processes[next].context_rsp;
                let next_stack_top = sched.processes[next].kernel_stack_top;
                let next_fs_base = sched.processes[next].fs_base;
                let prev = current;
                sched.current = next;
                sched.processes[next].state = crate::process::ProcessState::Running;
                sched.processes[next].last_run_tick = sched.global_tick;

                // === CPU Accounting: start charging the newly scheduled process ===
                sched.start_charging_runtime(next);

                // Switch address space.
                unsafe {
                    sched.processes[next].address_space.activate();
                    x86_64::registers::model_specific::Msr::new(0xC0000100).write(next_fs_base);
                }

                // Update TSS RSP0 for next interrupt.
                // SAFETY: timer handler runs with interrupts disabled; no concurrent TSS access.
                // Note: ltr is NOT needed here — the CPU reads RSP0 from the TSS memory
                // on each privilege-level stack switch. We only need to update the TSS contents.
                set_tss_rsp0(next_stack_top);

                if sched.processes[prev].state == crate::process::ProcessState::Finished {
                    sched.reap_process_resources(prev);
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

    // IMPORTANT: do NOT re-enable interrupts here.
    //
    // The naked `timer_entry` still has to (a) optionally switch the stack
    // pointer to `result` (the next process's saved kernel stack), (b) pop 15
    // GPRs, and (c) `iretq`. If interrupts were enabled now, a nested timer or
    // keyboard IRQ could fire *after* `mov rsp, rax` — i.e. while RSP points at
    // a half-restored context — and the nested handler could context-switch
    // again, clobbering the frame this handler is mid-way through restoring.
    // The result is a garbage IRET frame and a #GP on `iretq` (observed as an
    // intermittent GPF at the `iretq` instruction after long uptime).
    //
    // Interrupts are re-enabled atomically by `iretq` itself: every runnable
    // context's saved RFLAGS has IF=1 (init_context writes 0x202; preempted
    // frames are saved with IF set and the asm OR's 0x200 into the outgoing
    // frame's RFLAGS as well). So leaving IF=0 here is correct and safe.

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
        crate::serial_println!("[IRQ1] Keyboard interrupt - EOI sent to PIC");
    }
}

// Mouse IRQ12 handler
extern "x86-interrupt" fn mouse_entry(_stack_frame: InterruptStackFrame) {
    use crate::arch::x86_64::mouse;
    mouse::handle_irq12();
}
