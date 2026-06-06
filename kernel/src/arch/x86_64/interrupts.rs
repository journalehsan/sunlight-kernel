use crate::serial_println;
use x86_64::structures::idt::{
    InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
};
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;
use x86_64::instructions::port::Port;
use x86_64::instructions::segmentation::Segment;

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

static TSS: spin::Lazy<TaskStateSegment> = spin::Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    tss.interrupt_stack_table[0] = {
        const STACK_SIZE: usize = 4096;
        static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
        // SAFETY: STACK is static and never modified by anything else.
        let stack_start = VirtAddr::from_ptr(unsafe { &STACK });
        stack_start + STACK_SIZE as u64 // stack grows downward
    };
    tss
});

struct Selectors {
    code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

static GDT: spin::Lazy<(GlobalDescriptorTable, Selectors)> = spin::Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let code_selector = gdt.append(Descriptor::kernel_code_segment());
    let tss_selector = gdt.append(Descriptor::tss_segment(&*TSS));
    (gdt, Selectors { code_selector, tss_selector })
});

/// Initialize IDT, GDT, PIC, and PIT.
pub fn init() {
    serial_println!("[IDT] Loading interrupt descriptor table...");

    // Load GDT and segment registers
    GDT.0.load();
    // SAFETY: We just loaded a valid GDT.
    unsafe {
        x86_64::instructions::segmentation::CS::set_reg(GDT.1.code_selector);
        x86_64::instructions::segmentation::SS::set_reg(x86_64::structures::gdt::SegmentSelector(0));
        x86_64::instructions::segmentation::DS::set_reg(x86_64::structures::gdt::SegmentSelector(0));
        x86_64::instructions::segmentation::ES::set_reg(x86_64::structures::gdt::SegmentSelector(0));
        x86_64::instructions::tables::load_tss(GDT.1.tss_selector);
    }

    // SAFETY: IDT is only accessed here during initialization.
    let idt = unsafe { &mut IDT };

    idt.divide_error.set_handler_fn(divide_error_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    // SAFETY: IST index 0 is valid and points to a valid stack.
    unsafe {
        idt.double_fault.set_handler_fn(double_fault_handler).set_stack_index(0);
    }
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt[0x20].set_handler_fn(timer_handler);

    idt.load();

    // Remap PIC
    remap_pic();

    // Set PIT frequency (~100 Hz)
    init_pit();

    // Enable IRQ0 (timer)
    let mut pic1_data: Port<u8> = Port::new(0x21);
    let mut pic2_data: Port<u8> = Port::new(0xA1);
    // SAFETY: PIC ports are safe to access during initialization.
    unsafe {
        pic1_data.write(0xFE); // mask all except IRQ0
        pic2_data.write(0xFF); // mask all
    }

    serial_println!("[IDT] PIC remapped, timer IRQ0 enabled at ~100Hz");
    serial_println!("[IDT] OK");
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

    // SAFETY: PIC initialization sequence is well-defined.
    unsafe {
        cmd1.write(ICW1_INIT);
        cmd2.write(ICW1_INIT);
        data1.write(0x20); // IRQ0-7 -> 0x20-0x27
        data2.write(0x28); // IRQ8-15 -> 0x28-0x2F
        data1.write(0x04); // Tell PIC1 that PIC2 is at IRQ2
        data2.write(0x02); // Tell PIC2 its cascade identity
        data1.write(ICW4_8086);
        data2.write(ICW4_8086);
        data1.write(0xFF); // mask all
        data2.write(0xFF); // mask all
    }
}

fn init_pit() {
    const PIT_CMD: u16 = 0x43;
    const PIT_CH0: u16 = 0x40;
    const MODE_3: u8 = 0x36;
    const DIVISOR: u16 = 11932; // ~100 Hz

    let mut cmd: Port<u8> = Port::new(PIT_CMD);
    let mut ch0: Port<u8> = Port::new(PIT_CH0);

    // SAFETY: PIT ports are safe to access during initialization.
    unsafe {
        cmd.write(MODE_3);
        ch0.write((DIVISOR & 0xFF) as u8);
        ch0.write((DIVISOR >> 8) as u8);
    }
}

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    serial_println!("[INT] Divide Error: {:?}", stack_frame);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    serial_println!("[INT] Invalid Opcode: {:?}", stack_frame);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial_println!("[INT] Double Fault: {:?}", stack_frame);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn gpf_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    serial_println!("[INT] General Protection Fault: {:?} code={}", stack_frame, error_code);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let addr = x86_64::registers::control::Cr2::read_raw();
    serial_println!("[INT] Page Fault at {:#x}: {:?} code={:?}", addr, stack_frame, error_code);
    loop { x86_64::instructions::hlt(); }
}

static TICKS: spin::Mutex<u64> = spin::Mutex::new(0);

extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    let mut ticks = TICKS.lock();
    *ticks += 1;
    drop(ticks);

    // Send EOI
    let mut cmd1: Port<u8> = Port::new(0x20);
    // SAFETY: PIC command port is safe for EOI.
    unsafe { cmd1.write(0x20); }
}

#[allow(dead_code)]
pub fn ticks() -> u64 {
    *TICKS.lock()
}
