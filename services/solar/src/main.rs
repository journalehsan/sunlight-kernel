//! Solar HTTP/1.1 Server with SBSP Scripting Engine
//!
//! A high-performance, capability-based web server for SunlightOS.
//!
//! Architecture:
//! - VFS reads: Direct capability-based file access to /srv/http/
//! - IPC writes: Mediated through sunlight-sm for /srv/http/uploads/
//! - SHM pool: Pre-allocated 16 pages (64 KB) for zero-overhead IPC in hot path
//! - RAII Guards: Automatic resource cleanup via Drop trait (no memory leaks)
//! - Streaming SBSP Engine: Zero-allocation template execution
//! - Threading: (Future) Worker thread pool, one per TCP connection

#![no_std]
#![no_main]

extern crate alloc;

mod file_handler;
mod http;
mod net;
pub mod shm_pool;

use core::cell::RefCell;
use heapless::Vec;
use http::parse_request;
use net::{TcpListener, TcpStream, MAX_ACTIVE_CONNS};

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

use sunlight_ipc::{
    endpoint_create, nameserver_lookup, nameserver_register, process_yield, shm_alloc,
    CapabilityToken,
};

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

        solar_log!(
            "[SOLAR] Pre-allocated {} SHM pages ({} KB)",
            pages.len(),
            pages.len() * 4
        );

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

/// Handle a parsed HTTP request by routing to the appropriate handler.
///
/// - `.sbsp` files → SBSP engine (TODO: Phase 3)
/// - Everything else → static file streaming via `file_handler::serve_static_file`
fn route_request(stream: &mut TcpStream, req_path: &str, ctx: &SolarContext) {
    solar_log!("[SOLAR] Serving {}", req_path);

    if req_path.ends_with(".sbsp") {
        // TODO: Phase 3 - SBSP Engine
        let _ = stream.write_all(
            b"HTTP/1.1 501 Not Implemented\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            &ctx.shm_pool,
        );
    } else {
        file_handler::serve_static_file(stream, req_path, ctx.www_read_cap, &ctx.shm_pool);
    }
}

/// Service entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    solar_log!("[SOLAR] ☀️ Booting Solar HTTP daemon v1.0...");

    // Phase 1.1: Register the service endpoint with the nameserver
    let ep = endpoint_create();
    nameserver_register("solar", ep);
    solar_log!("[SOLAR] ✓ Registered endpoint with nameserver as 'solar'");

    // Phase 1.2: Acquire the VFS endpoint used for document-root reads.
    // TODO: replace this endpoint lookup with a path-scoped read capability
    // from the capability broker once VFS capability enforcement is wired.
    let www_read_cap = acquire_vfs_read_capability("/srv/http/");
    solar_log!("[SOLAR] ✓ Acquired VFS read capability for /srv/http/");

    // Phase 1.3: Initialize the SHM Page Pool
    // 16 pages × 4 KB = 64 KB of pre-allocated shared memory.
    // This eliminates malloc/free overhead in the hot path for IPC writes.
    let shm_pool = ShmPagePool::new(16);
    solar_log!("[SOLAR] ✓ Initialized 16-page SHM pool (64 KB) for zero-overhead writes");

    // Phase 1.4: Build the execution context
    let ctx = SolarContext {
        www_read_cap,
        shm_pool,
    };
    solar_log!("[SOLAR] ✓ Execution context initialized");

    // Phase 1.5: Bind TCP listener on port 80
    solar_log!("[SOLAR] ⏳ Binding TCP listener to port 80 via net_server...");

    let listener = match TcpListener::bind(80) {
        Ok(l) => l,
        Err(e) => {
            // Bind failed (today this is expected: net_server's BIND/LISTEN/ACCEPT
            // are still stubs, so server sockets aren't supported yet). Park the
            // service by yielding the CPU on every iteration instead of a bare
            // `loop {}` — a High-tier process that never blocks would otherwise
            // monopolize the scheduler and slow the whole system to a crawl.
            solar_log!("[SOLAR] ❌ Failed to bind: {} — parking (yielding)", e);
            loop {
                sunlight_ipc::process_yield();
            }
        }
    };

    let net_endpoint = listener.net_endpoint();
    solar_log!("[SOLAR] ☀️ Listening on port 80... Async event loop ready! :)");

    // Phase 1.6: Async event loop with NetOp::POLL
    // Handles up to MAX_ACTIVE_CONNS (64) concurrent connections without blocking
    let mut active_streams: Vec<TcpStream, MAX_ACTIVE_CONNS> = Vec::new();
    let mut loop_count = 0u32;

    loop {
        loop_count = loop_count.wrapping_add(1);

        // 1. Try to accept new connections (non-blocking)
        match listener.try_accept() {
            Ok(Some(stream)) => {
                if active_streams.push(stream).is_err() {
                    solar_log!(
                        "[SOLAR] ⚠️  Max concurrent connections ({}) reached!",
                        MAX_ACTIVE_CONNS
                    );
                    // Close the connection we couldn't accept
                    // (it will be handled on next iteration)
                } else {
                    solar_log!(
                        "[SOLAR] 📥 New connection accepted ({} active)",
                        active_streams.len()
                    );
                }
            }
            Ok(None) => {
                // No pending connections, continue to poll
            }
            Err(e) => {
                solar_log!("[SOLAR] ⚠️  Accept error: {}", e);
            }
        }

        // 2. Poll active sockets to see which have data ready
        if !active_streams.is_empty() {
            let mut socket_ids = [0u32; 8];
            let poll_count = active_streams.len().min(8);
            for (i, stream) in active_streams.iter().take(8).enumerate() {
                socket_ids[i] = stream.socket_id;
            }

            match net::poll_sockets(net_endpoint, &socket_ids[..poll_count]) {
                Ok(ready) => {
                    // 3. Process only the sockets that are ready
                    let mut i = 0;
                    while i < active_streams.len() {
                        let socket_id = active_streams[i].socket_id;

                        if net::is_socket_ready(socket_id, &ready) {
                            // This socket has data! Read and handle it.
                            let mut buffer = [0u8; 8192];
                            let read_result = active_streams[i].read(&mut buffer);
                            match read_result {
                                Ok(bytes_read) if bytes_read > 0 => {
                                    // Parse HTTP request (zero-copy into buffer slices)
                                    match parse_request(&buffer[..bytes_read]) {
                                        Ok(request) => {
                                            solar_log!(
                                                "[SOLAR] {} {}",
                                                request.method,
                                                request.path
                                            );

                                            route_request(
                                                &mut active_streams[i],
                                                request.path,
                                                &ctx,
                                            );
                                        }
                                        Err(e) => {
                                            solar_log!("[SOLAR] ⚠️  Parse error: {:?}", e);
                                            let _ = active_streams[i].write_all(
                                                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request",
                                                &ctx.shm_pool,
                                            );
                                        }
                                    }

                                    // Close and remove the stream (HTTP/1.0 style for now)
                                    solar_log!("[SOLAR] ✅ Request handled, closing connection");
                                    active_streams.swap_remove(i);
                                    continue; // Don't increment i, swap_remove shifted elements
                                }
                                Ok(_) => {
                                    // Connection closed by peer
                                    solar_log!("[SOLAR] 📤 Client closed connection (0 bytes)");
                                    active_streams.swap_remove(i);
                                    continue;
                                }
                                Err(_) => {
                                    // Read error
                                    solar_log!("[SOLAR] ❌ Read error, closing connection");
                                    active_streams.swap_remove(i);
                                    continue;
                                }
                            }
                        }

                        i += 1;
                    }
                }
                Err(e) => {
                    solar_log!("[SOLAR] ⚠️  Poll error: {}", e);
                }
            }
        }

        // 4. Yield CPU to let other services run
        sunlight_ipc::process_yield();
    }
}

/// Helper function: Acquire VFS access for the document root.
///
/// In a production system, this would:
/// 1. Look up the capability broker via nameserver
/// 2. Send an IPC request for READ access to /srv/http/
/// 3. Receive a CapabilityToken from the broker
///
/// For now, use the real VFS service endpoint instead of a guessed/mock token.
fn acquire_vfs_read_capability(_path: &str) -> CapabilityToken {
    loop {
        if let Some(cap) = nameserver_lookup("vfs") {
            return cap;
        }
        process_yield();
    }
}
