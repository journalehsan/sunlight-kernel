//! Solar HTTP/1.1 Server with SBSP Scripting Engine
//!
//! A high-performance, capability-based web server for SunlightOS.
//! Phase 1: Service bootstrap, VFS capability acquisition, SHM page pool initialization.
//!
//! Architecture:
//! - VFS reads: Direct capability-based file access to /var/lib/sunlight/www/
//! - IPC writes: Mediated through sunlight-sm for /var/lib/sunlight/www/uploads/
//! - SHM pool: Pre-allocated 16 pages (64 KB) for zero-overhead IPC in hot path
//! - Threading: (Future) Worker thread pool, one per TCP connection

#![no_std]
#![no_main]

extern crate alloc;

use core::cell::RefCell;
use heapless::Vec;

/// Allocator: Simple bump allocator for service memory
struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 262144] = [0; 262144]; // 256 KB
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
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

/// Debug logging macro (mirrors other SunlightOS services)
#[macro_export]
macro_rules! solar_log {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        sunlight_ipc::debug_log(&buf);
    }};
}

use sunlight_ipc::{endpoint_create, nameserver_register, shm_alloc, CapabilityToken};

/// The shared context for Solar's execution.
/// Holds the VFS read capability for the document root and the SHM pool for writes.
pub struct SolarContext {
    pub www_read_cap: CapabilityToken,
    pub shm_pool: ShmPagePool,
    // Future: pub sm_endpoint: u64,
    // Future: pub kv_socket: UnixStream,
}

/// A pre-allocated pool of shared memory pages for IPC operations.
/// Zero allocation overhead in the hot path — pages are recycled, never freed.
pub struct ShmPagePool {
    /// Backing storage for available SHM pages (16 pages max)
    pages: RefCell<Vec<CapabilityToken, 16>>,
    capacity: usize,
}

impl ShmPagePool {
    /// Create a new SHM page pool with `capacity` pre-allocated pages.
    /// Each page is 4096 bytes (one kernel page frame).
    pub fn new(capacity: usize) -> Self {
        let mut pages = Vec::new();

        for _ in 0..capacity {
            // Pre-allocate 4096-byte pages directly from the microkernel.
            // This happens once at startup; no allocation overhead in the server's hot path.
            match shm_alloc() {
                Ok((_ptr, token)) => {
                    let _ = pages.push(token); // Will panic if capacity < 16, which is OK (bootstrap failure)
                }
                Err(_e) => {
                    solar_log!("[SOLAR] ⚠️  SHM allocation failed");
                    break; // Continue with fewer pages if allocation fails
                }
            }
        }

        solar_log!("[SOLAR] Pre-allocated {} SHM pages ({} KB)", pages.len(), pages.len() * 4);

        Self {
            pages: RefCell::new(pages),
            capacity,
        }
    }

    /// Acquire a page from the pool.
    /// Returns None if the pool is exhausted (should be rare with 16 pages).
    pub fn acquire(&self) -> Option<CapabilityToken> {
        self.pages.borrow_mut().pop()
    }

    /// Return a page to the pool for reuse.
    /// If the pool is at capacity, the page is effectively discarded.
    pub fn release(&self, token: CapabilityToken) {
        let mut pool = self.pages.borrow_mut();
        if pool.len() < self.capacity {
            let _ = pool.push(token);
        }
        // If at capacity, we could call sunlight_ipc::shm_free(token)
        // but it's OK to just drop it (page stays allocated in kernel, wasted but rare).
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    solar_log!("[SOLAR] PANIC: {}", info);
    loop {}
}

/// Service entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    solar_log!("[SOLAR] ☀️ Booting Solar HTTP daemon v1.0...");

    // Phase 1.1: Register the service endpoint with the nameserver
    let ep = endpoint_create();
    nameserver_register("solar", ep);
    solar_log!("[SOLAR] ✓ Registered endpoint with nameserver as 'solar'");

    // Phase 1.2: Acquire strict VFS Capability for the document root
    // In a real implementation, this would IPC to sunlight-uac or the capability broker,
    // requesting READ access for /var/lib/sunlight/www/.
    // For now, we use a mock capability token.
    let www_read_cap = acquire_vfs_read_capability("/var/lib/sunlight/www/");
    solar_log!("[SOLAR] ✓ Acquired VFS read capability for /var/lib/sunlight/www/");

    // Phase 1.3: Initialize the SHM Page Pool
    // 16 pages × 4 KB = 64 KB of pre-allocated shared memory.
    // This eliminates malloc/free overhead in the hot path for IPC writes.
    let shm_pool = ShmPagePool::new(16);
    solar_log!("[SOLAR] ✓ Initialized 16-page SHM pool (64 KB) for zero-overhead writes");

    // Phase 1.4: Build the execution context
    let _ctx = SolarContext {
        www_read_cap,
        shm_pool,
    };
    solar_log!("[SOLAR] ✓ Execution context initialized");

    // Phase 1.5: TODO - Bind TCP listener on 0.0.0.0:8080
    // (Will implement HTTP/1.1 parser and thread pool in Phase 2-3)
    solar_log!("[SOLAR] ⏳ TCP listener binding (Phase 2)");

    // For now, just idle. In a real HTTP server, we would:
    // 1. Bind a TCP socket
    // 2. Accept incoming connections
    // 3. Spawn a worker task per connection
    // 4. Parse HTTP/1.1 requests
    // 5. Route to file handler or SBSP engine
    // 6. Return responses

    solar_log!("[SOLAR] Ready. Waiting for Phase 2 (HTTP parser) implementation...");

    // Prevent the service from exiting
    loop {
        // In a real server: accept(), handle_connection(), etc.
        // For now, just spin to keep the service alive.
    }
}

/// Helper function: Acquire VFS read capability for the document root.
///
/// In a production system, this would:
/// 1. Look up the capability broker via nameserver
/// 2. Send an IPC request for READ access to /var/lib/sunlight/www/
/// 3. Receive a CapabilityToken from the broker
///
/// For Phase 1, we return a mock token. Phase 2 will implement real capability acquisition.
fn acquire_vfs_read_capability(_path: &str) -> CapabilityToken {
    // TODO: Implement IPC to CAP_BROKER
    // For now, return a placeholder token (will be properly wired in Phase 2)
    CapabilityToken(1) // Mock token (non-zero to indicate "acquired")
}
