//! fetch — SunlightOS lightweight HTTP downloader

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(feature = "host-linux")]
use std::env;

use sunlight_fetch::cli;
use sunlight_fetch::downloader;
use sunlight_fetch::error::FetchError;

#[cfg(feature = "host-linux")]
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match run_host(&args) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("fetch: error: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "host-linux")]
fn run_host(args: &[String]) -> Result<(), FetchError> {
    let config = cli::parse_args(args).map_err(FetchError::InvalidArgs)?;
    if config.help {
        let mut buf = String::with_capacity(1024);
        cli::print_help(&mut buf);
        println!("{buf}");
        return Ok(());
    }
    downloader::execute_download(&config)
}

// ── SunlightOS no_std entry ──────────────────────────────────────────────────

#[cfg(feature = "sunlightos")]
use core::alloc::{GlobalAlloc, Layout};
#[cfg(feature = "sunlightos")]
use core::panic::PanicInfo;
#[cfg(feature = "sunlightos")]
use alloc::string::String;
#[cfg(feature = "sunlightos")]
use sunlight_libc as libc;
#[cfg(feature = "sunlightos")]
use libc::{Errno, STDOUT};

#[cfg(feature = "sunlightos")]
const MAX_ARGS: usize = 16;

#[cfg(feature = "sunlightos")]
struct BumpAllocator;

#[cfg(feature = "sunlightos")]
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        static mut HEAP: [u8; 2 * 1024 * 1024] = [0; 2 * 1024 * 1024];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[cfg(feature = "sunlightos")]
#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[cfg(feature = "sunlightos")]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let _ = write_stderr(b"fetch: panic\n");
    libc::exit(1);
}

#[cfg(feature = "sunlightos")]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let code = match run_os(&storage[..count]) {
        Ok(()) => 0,
        Err(e) => {
            let _ = write_stderr_fmt(&alloc::format!("fetch: error: {e}\n"));
            1
        }
    };
    libc::exit(code as u64);
}

#[cfg(feature = "sunlightos")]
unsafe fn collect_args<'a>(argc: u64, argv: *const *const u8, out: &mut [&'a str]) -> usize {
    if argv.is_null() {
        return 0;
    }
    let mut count = 0;
    for i in 0..(argc as usize).min(out.len()) {
        let ptr = *argv.add(i);
        if ptr.is_null() {
            break;
        }
        let mut len = 0;
        while len < 512 && *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(ptr, len);
        out[count] = core::str::from_utf8(slice).unwrap_or("");
        count += 1;
    }
    count
}

#[cfg(feature = "sunlightos")]
fn run_os(args: &[&str]) -> Result<(), FetchError> {
    let owned: alloc::vec::Vec<String> = args.iter().map(|s| String::from(*s)).collect();
    let config = cli::parse_args(&owned).map_err(FetchError::InvalidArgs)?;
    if config.help {
        let mut buf = String::with_capacity(1024);
        cli::print_help(&mut buf);
        let _ = write_stderr_fmt(&alloc::format!("{buf}"));
        return Ok(());
    }
    downloader::execute_download(&config)
}

#[cfg(feature = "sunlightos")]
fn write_stderr(data: &[u8]) -> Result<(), Errno> {
    let mut rest = data;
    while !rest.is_empty() {
        match libc::write(STDOUT, rest) {
            Ok(n) => rest = &rest[n.min(rest.len())..],
            Err(Errno::Again) => libc::yield_now(),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(feature = "sunlightos")]
fn write_stderr_fmt(s: &str) -> Result<(), Errno> {
    write_stderr(s.as_bytes())
}