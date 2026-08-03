//! Ring-3 USB HID boot-mouse service.

#![no_std]
#![no_main]

mod usb_mouse;

use sunlight_ipc::{
    getpid, ipc_call_timeout, nameserver_lookup, process_yield, DevicedMsg, DriverCaps, DriverKind,
    DriverState, IpcMsg, MouseMsg, PointerReport, ProcessExit,
};

const FORWARD_TIMEOUT_MS: u64 = 50;

fn debug_log(message: &str) {
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 99u64 => _,
            in("rdi") message.as_ptr() as u64,
            in("rsi") message.len() as u64,
            lateout("rcx") _, lateout("r11") _,
            options(nostack)
        );
    }
}

#[cfg(feature = "usb_mouse_debug")]
fn debug_u64(value: u64) {
    let mut buffer = [0u8; 20];
    let mut value = value;
    let mut start = buffer.len();
    loop {
        start -= 1;
        buffer[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    debug_log(unsafe { core::str::from_utf8_unchecked(&buffer[start..]) });
}

#[cfg(feature = "usb_mouse_debug")]
fn debug_i16(value: i16) {
    if value < 0 {
        debug_log("-");
        debug_u64((-(value as i32)) as u64);
    } else {
        debug_u64(value as u64);
    }
}

#[cfg(feature = "usb_mouse_debug")]
fn debug_byte(value: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = [HEX[(value >> 4) as usize], HEX[(value & 0x0f) as usize]];
    debug_log(unsafe { core::str::from_utf8_unchecked(&bytes) });
}

fn pack_short_name(name: &str) -> u64 {
    let mut word = 0;
    let bytes = name.as_bytes();
    let mut index = 0;
    while index < bytes.len().min(8) {
        word |= (bytes[index] as u64) << (index * 8);
        index += 1;
    }
    word
}

fn register_with_deviced(state: DriverState) {
    let Some(deviced) = sunlight_ipc::nameserver_lookup_timeout("deviced", 5) else {
        return;
    };
    let metadata = (DriverKind::Mouse as u64) | ((state as u64) << 16);
    let message = IpcMsg::with_label(DevicedMsg::REGISTER_DRIVER)
        .word(0, pack_short_name("usbmouse"))
        .word(1, getpid())
        .word(2, metadata)
        .word(
            3,
            DriverCaps::INPUT | DriverCaps::POINTER | DriverCaps::RELATIVE_MOTION,
        );
    let _ = ipc_call_timeout(deviced, message, 20);
}

fn dispatch(event: usb_mouse::MouseEvent, tty: sunlight_ipc::CapabilityToken) {
    let report = PointerReport::new(event.dx, event.dy, event.buttons);
    let message = IpcMsg::with_label(MouseMsg::RAW_MOTION)
        .word(0, report.pack())
        .word(1, 1); // one HID report in this batch
    let _ = ipc_call_timeout(tty, message, FORWARD_TIMEOUT_MS);
    if event.wheel != 0 {
        // USB HID positive wheel values mean scrolling up; the UI event ABI
        // uses positive values for scrolling down.
        let delta = event.wheel.saturating_neg();
        let message = IpcMsg::with_label(MouseMsg::RAW_WHEEL).word(0, delta as u16 as u64);
        let _ = ipc_call_timeout(tty, message, FORWARD_TIMEOUT_MS);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[USB-MOUSE] PANIC\n");
    ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[USB-MOUSE] ring-3 xHCI HID driver starting\n");
    if usb_mouse::init().is_err() {
        debug_log("[USB-MOUSE] no usable xHCI boot mouse\n");
        register_with_deviced(DriverState::Failed);
        ProcessExit::exit(1);
    }
    register_with_deviced(DriverState::Ready);
    debug_log("[USB-MOUSE] boot mouse ready\n");

    let tty = loop {
        if let Some(cap) = nameserver_lookup("tty") {
            break cap;
        }
        process_yield();
    };
    debug_log("[USB-MOUSE] tty input router ready\n");

    loop {
        if let Some(event) = usb_mouse::poll() {
            dispatch(event, tty);
        } else {
            process_yield();
        }
    }
}
