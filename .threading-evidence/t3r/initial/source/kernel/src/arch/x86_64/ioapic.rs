//! I/O APIC routing for legacy platform interrupts.
//!
//! Modern machines normally deliver the i8042 keyboard and mouse interrupts
//! through an I/O APIC even though their source numbers remain ISA IRQ1 and
//! IRQ12. Firmware/QEMU may leave virtual-wire PIC delivery working, but an OS
//! cannot rely on that state after enabling the Local APIC.

use crate::arch::x86_64::acpi::{self, LegacyInterruptRoute};
use core::sync::atomic::{AtomicBool, Ordering};

const IOREGSEL: usize = 0x00;
const IOWIN: usize = 0x10;
const IOAPIC_VERSION: u8 = 0x01;
const IOREDTBL_BASE: u8 = 0x10;
const REDIR_ACTIVE_LOW: u32 = 1 << 13;
const REDIR_LEVEL_TRIGGERED: u32 = 1 << 15;
const REDIR_MASKED: u32 = 1 << 16;

static INPUT_IRQS_VIA_IOAPIC: AtomicBool = AtomicBool::new(false);

#[inline]
fn virtual_base(physical: u32) -> usize {
    let hhdm = crate::HHDM_REQ
        .response()
        .map_or(0, |response| response.offset);
    physical as usize + hhdm as usize
}

unsafe fn read_register(physical: u32, register: u8) -> u32 {
    let base = virtual_base(physical);
    unsafe {
        ((base + IOREGSEL) as *mut u32).write_volatile(register as u32);
        ((base + IOWIN) as *const u32).read_volatile()
    }
}

unsafe fn write_register(physical: u32, register: u8, value: u32) {
    let base = virtual_base(physical);
    unsafe {
        ((base + IOREGSEL) as *mut u32).write_volatile(register as u32);
        ((base + IOWIN) as *mut u32).write_volatile(value);
    }
}

fn redirection_index(route: LegacyInterruptRoute) -> Result<u8, &'static str> {
    let index = route
        .gsi
        .checked_sub(route.gsi_base)
        .ok_or("I/O APIC route precedes controller GSI base")?;
    let version = unsafe { read_register(route.io_apic_addr, IOAPIC_VERSION) };
    let max_index = (version >> 16) & 0xff;
    if index > max_index || index > ((u8::MAX - IOREDTBL_BASE) / 2) as u32 {
        return Err("I/O APIC does not own requested redirection entry");
    }
    Ok(index as u8)
}

unsafe fn program_route(
    route: LegacyInterruptRoute,
    index: u8,
    vector: u8,
    destination_apic_id: u8,
) {
    let low_register = IOREDTBL_BASE + index * 2;
    let high_register = low_register + 1;
    let mut low = vector as u32;
    if route.active_low {
        low |= REDIR_ACTIVE_LOW;
    }
    if route.level_triggered {
        low |= REDIR_LEVEL_TRIGGERED;
    }

    // Mask before changing the destination, then publish the final unmasked
    // low word. This prevents a partially-programmed interrupt delivery.
    unsafe {
        write_register(route.io_apic_addr, low_register, low | REDIR_MASKED);
        write_register(
            route.io_apic_addr,
            high_register,
            (destination_apic_id as u32) << 24,
        );
        write_register(route.io_apic_addr, low_register, low);
    }
}

/// Route i8042 keyboard IRQ1 and auxiliary IRQ12 to the BSP Local APIC.
///
/// Both routes are resolved and validated before either is changed, so callers
/// may safely retain PIC delivery if this function returns an error.
pub fn configure_input_irqs(destination_apic_id: u8) -> Result<(), &'static str> {
    let keyboard = acpi::legacy_interrupt_route(1)?;
    let mouse = acpi::legacy_interrupt_route(12)?;
    let keyboard_index = redirection_index(keyboard)?;
    let mouse_index = redirection_index(mouse)?;

    unsafe {
        program_route(keyboard, keyboard_index, 0x21, destination_apic_id);
        program_route(mouse, mouse_index, 0x2c, destination_apic_id);
    }
    INPUT_IRQS_VIA_IOAPIC.store(true, Ordering::Release);

    crate::serial_println!(
        "[IOAPIC] keyboard IRQ1 -> GSI{} vector=0x21 apic={}",
        keyboard.gsi,
        destination_apic_id
    );
    crate::serial_println!(
        "[IOAPIC] mouse IRQ12 -> GSI{} vector=0x2c apic={}",
        mouse.gsi,
        destination_apic_id
    );
    Ok(())
}

#[inline]
pub fn input_irqs_enabled() -> bool {
    INPUT_IRQS_VIA_IOAPIC.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirection_bits_do_not_overlap_vector() {
        assert_eq!(REDIR_ACTIVE_LOW & 0xff, 0);
        assert_eq!(REDIR_LEVEL_TRIGGERED & 0xff, 0);
        assert_eq!(REDIR_MASKED & 0xff, 0);
    }
}
