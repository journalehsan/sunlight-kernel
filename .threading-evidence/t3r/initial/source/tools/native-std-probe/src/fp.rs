//! Native syscall probe for task-local x87, XMM and MXCSR preservation.
#![no_std]
#![no_main]
use core::sync::atomic::{AtomicUsize, Ordering};

static DONE: AtomicUsize = AtomicUsize::new(0);
static FAILED: AtomicUsize = AtomicUsize::new(0);

core::arch::global_asm!(include_str!("fp.S"));
extern "C" {
    fn fp_sentinel(seed: u64) -> u64;
}

fn log(message: &str) {
    unsafe { sunlight_libc::sys::syscall2(99, message.as_ptr() as u64, message.len() as u64) };
}

extern "C" fn worker(arg: *mut u8) -> *mut u8 {
    if unsafe { fp_sentinel(arg as u64 + 7) } != 0 {
        FAILED.fetch_add(1, Ordering::Relaxed);
    }
    DONE.fetch_add(1, Ordering::Release);
    core::ptr::null_mut()
}

#[no_mangle]
extern "C" fn native_probe_main() -> ! {
    log("[NATIVE-STD] FP probe started (native ABI, 8 borrowers)\n");
    for index in 0..8 {
        if unsafe { sunlight_libc::thread::spawn(worker, index as *mut u8) }.is_err() {
            log("[NATIVE-STD] FP spawn FAILED\n");
            sunlight_libc::exit(1);
        }
    }
    let main_failed = unsafe { fp_sentinel(31) } != 0;
    while DONE.load(Ordering::Acquire) != 8 {
        sunlight_libc::yield_now();
    }
    if main_failed || FAILED.load(Ordering::Acquire) != 0 {
        log("[NATIVE-STD] FP isolation FAILED\n");
        sunlight_libc::exit(1);
    }
    log("[NATIVE-STD] x87/XMM/MXCSR preemption isolation OK\n");
    let pdf = sunlight_hayro_native_probe::open(include_bytes!(
        "../../../assets/documents/SunlightOS_PDF_Reader_Test.pdf"
    ));
    let image = sunlight_hayro_native_probe::vector_fixture();
    if !pdf.as_ref().is_ok_and(|pdf| pdf.pages().len() == 4)
        || !image.as_ref().is_ok_and(|image| {
            image.width == 64
                && image.height == 48
                && image.pixels[0] == 0xffff_ffff
                && image.pixels[16 * 64 + 16] == 0xffff_0000
        })
    {
        log("[NATIVE-STD] parser/raster FAILED\n");
        sunlight_libc::exit(1);
    }
    drop(pdf);
    drop(image);
    log("[NATIVE-STD] native PDF parser and vector raster OK\n");
    sunlight_libc::exit(0)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    log("[NATIVE-STD] FP probe panicked\n");
    sunlight_libc::exit(2)
}
