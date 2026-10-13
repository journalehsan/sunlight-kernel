//! Link-only native parser smoke executable. Not installed or boot validated.
#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let status = match sunlight_hayro_native_probe::open(include_bytes!(
        "../../../assets/documents/SunlightOS_PDF_Reader_Test.pdf"
    )) {
        Ok(pdf) if pdf.pages().len() == 4 => 0,
        _ => 1,
    };
    sunlight_libc::exit(status)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // Includes engine infallible allocation failure. No unwinding or recovery.
    sunlight_libc::exit(2)
}
