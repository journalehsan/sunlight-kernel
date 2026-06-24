#![no_std]
#![no_main]

// No heap allocations in the Eyes demo (we write directly into the provided SHM buffer).
// Provide a tiny stub allocator to satisfy the libcore requirements if any crate pulls alloc.
struct NoAlloc;
unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 { core::ptr::null_mut() }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}
#[global_allocator]
static A: NoAlloc = NoAlloc;

use sunlight_ipc::{
    nameserver_lookup, ipc_call, IpcMsg, ProcessExit, sgp::SgpMsg,
    debug_log,
};

// Simple distance helper (not strictly used but kept for future)
fn _distance(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dx = x1 - x2;
    let dy = y1 - y2;
    let abs_dx = if dx < 0 { -dx } else { dx };
    let abs_dy = if dy < 0 { -dy } else { dy };
    abs_dx + abs_dy
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[EYES] PANIC\n");
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[EYES] Starting eyes tracker client...\n");

    // Wait for display_server
    let display_ep = loop {
        if let Some(ep) = nameserver_lookup("display_server") {
            break ep;
        }
        // yield via a small sleep or process_yield if exposed; for now busy with timeout lookup in practice
        // Many services just loop with yield_now equivalent via ipc timeout or direct.
        // Use a simple spin for early demo (real code would use a timer service).
        for _ in 0..100000 { core::hint::spin_loop(); }
    };

    // 1. CREATE_WINDOW (width=400, height=300 packed)
    let req = IpcMsg::with_label(SgpMsg::CREATE_WINDOW)
        .word(0, 400u64 | (300u64 << 32));
    let reply = ipc_call(display_ep, req);

    if reply.label != SgpMsg::REPLY {
        debug_log("[EYES] CREATE_WINDOW failed\n");
        loop {}
    }

    let shm_tok = reply.caps[0];
    let win_id = reply.words[1];
    let _size = reply.words[2] as usize;
    let stride = reply.words[3] as usize; // bytes per row (width*4)

    debug_log("[EYES] Window created, mapping SHM...\n");

    // 2. Map the Shared Memory Region (multi-page now supported)
    let buffer = match sunlight_ipc::shm_map(shm_tok) {
        Ok(p) => p as *mut u32,
        Err(_) => {
            debug_log("[EYES] shm_map failed\n");
            loop {}
        }
    };

    let bg_color: u32 = 0x00222222;
    let eye_color: u32 = 0x00FFFFFF;
    let pupil_color: u32 = 0x00000000;

    debug_log("[EYES] Entering main event loop (blocking on EVENT_POLL)\n");

    loop {
        // 3. Block until next mouse event (zero CPU when idle)
        let poll_req = IpcMsg::with_label(SgpMsg::EVENT_POLL).word(0, win_id);
        let event_reply = ipc_call(display_ep, poll_req);

        let packed = event_reply.words[0];
        let mouse_x = (packed & 0xFFFF) as i32;
        let mouse_y = ((packed >> 16) & 0xFFFF) as i32;

        // --- Zero-copy render into the shared buffer ---

        // Clear (simple solid)
        for i in 0..(400 * 300) {
            unsafe { buffer.add(i).write_volatile(bg_color); }
        }

        // Eyes positions (hardcoded for 400x300)
        let eye_y = 150i32;
        let left_eye_x = 120i32;
        let right_eye_x = 280i32;
        let eye_radius = 40i32;
        let pupil_radius = 10i32;

        let render_eye = |cx: i32, cy: i32| {
            // Compute pupil pos: direction toward mouse, clamped
            let mut px = mouse_x;
            let mut py = mouse_y;

            // Clamp to eye disk (simple axis-aligned box for demo reliability)
            if px < cx - eye_radius + 8 { px = cx - eye_radius + 8; }
            if px > cx + eye_radius - 8 { px = cx + eye_radius - 8; }
            if py < cy - eye_radius + 8 { py = cy - eye_radius + 8; }
            if py > cy + eye_radius - 8 { py = cy + eye_radius - 8; }

            // Draw eye (white rect for simplicity + speed)
            for y in (cy - eye_radius)..(cy + eye_radius) {
                if y < 0 || y >= 300 { continue; }
                for x in (cx - eye_radius)..(cx + eye_radius) {
                    if x < 0 || x >= 400 { continue; }
                    let idx = (y * 400 + x) as usize;
                    unsafe { buffer.add(idx).write_volatile(eye_color); }
                }
            }

            // Draw pupil (black)
            for y in (py - pupil_radius)..(py + pupil_radius) {
                if y < 0 || y >= 300 { continue; }
                for x in (px - pupil_radius)..(px + pupil_radius) {
                    if x < 0 || x >= 400 { continue; }
                    let idx = (y * 400 + x) as usize;
                    unsafe { buffer.add(idx).write_volatile(pupil_color); }
                }
            }
        };

        render_eye(left_eye_x, eye_y);
        render_eye(right_eye_x, eye_y);

        // 4. COMMIT_FRAME
        let commit = IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id);
        let _ = ipc_call(display_ep, commit);
    }
}
