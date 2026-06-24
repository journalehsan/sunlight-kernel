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
    nameserver_lookup, ipc_call, IpcMsg, sgp::SgpMsg,
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

// Integer square root (for direction normalization without floats)
fn isqrt(n: u32) -> u32 {
    if n < 2 {
        return n;
    }
    let mut x0 = n / 2;
    let mut x1 = (x0 + n / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + n / x0) / 2;
    }
    x0
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
    let _stride = reply.words[3] as usize; // bytes per row (width*4)
    let win_x = reply.words[4] as i32;
    let win_y = reply.words[5] as i32;

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

    // Initial frame so the window is never solid black. Pupils centered.
    // We do this before the first EVENT_POLL so something is visible immediately.
    {
        for i in 0..(400 * 300) {
            unsafe { buffer.add(i).write_volatile(bg_color); }
        }
        let eye_y = 150i32;
        let left_eye_x = 120i32;
        let right_eye_x = 280i32;
        let eye_radius = 40i32;
        let pupil_radius = 10i32;

        let draw_centered = |cx: i32, cy: i32| {
            // white eye square
            for y in (cy - eye_radius)..(cy + eye_radius) {
                if y < 0 || y >= 300 { continue; }
                for x in (cx - eye_radius)..(cx + eye_radius) {
                    if x < 0 || x >= 400 { continue; }
                    unsafe { buffer.add((y * 400 + x) as usize).write_volatile(eye_color); }
                }
            }
            // pupil centered
            for y in (cy - pupil_radius)..(cy + pupil_radius) {
                if y < 0 || y >= 300 { continue; }
                for x in (cx - pupil_radius)..(cx + pupil_radius) {
                    if x < 0 || x >= 400 { continue; }
                    unsafe { buffer.add((y * 400 + x) as usize).write_volatile(pupil_color); }
                }
            }
        };

        draw_centered(left_eye_x, eye_y);
        draw_centered(right_eye_x, eye_y);

        let commit = IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id);
        let _ = ipc_call(display_ep, commit);
    }

    debug_log("[EYES] Initial frame drawn. Entering event loop...\n");

    loop {
        // 3. Block until next mouse event (zero CPU when idle)
        let poll_req = IpcMsg::with_label(SgpMsg::EVENT_POLL).word(0, win_id);
        let event_reply = ipc_call(display_ep, poll_req);

        let packed = event_reply.words[0];
        let mouse_x = (packed & 0xFFFF) as i32;
        let mouse_y = ((packed >> 16) & 0xFFFF) as i32;

        // Convert to window-local coords and test if cursor is inside our window
        let rel_x = mouse_x - win_x;
        let rel_y = mouse_y - win_y;
        const WIN_W: i32 = 400;
        const WIN_H: i32 = 300;
        let inside = rel_x >= 0 && rel_x < WIN_W && rel_y >= 0 && rel_y < WIN_H;

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
            // Default: pupils centered (looking "forward"). When cursor inside our window,
            // compute a limited offset in the exact direction toward the cursor.
            let (base_px, base_py) = if inside {
                let dx = rel_x - cx;
                let dy = rel_y - cy;
                let max_off = eye_radius - pupil_radius - 5;
                let d2 = (dx as i64 * dx as i64 + dy as i64 * dy as i64) as u32;
                if d2 == 0 {
                    (cx, cy)
                } else {
                    let dist = isqrt(d2).max(1);
                    let offx = (dx * max_off) / (dist as i32);
                    let offy = (dy * max_off) / (dist as i32);
                    (cx + offx, cy + offy)
                }
            } else {
                (cx, cy)
            };

            // Keep pupil fully inside the eye square
            let min_p = eye_radius - pupil_radius;
            let px = base_px.clamp(cx - min_p, cx + min_p);
            let py = base_py.clamp(cy - min_p, cy + min_p);

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

        // Small throttle so we don't spin 100% CPU on the fast immediate-reply EVENT_POLL path.
        // 20k iterations is still very responsive for mouse following.
        for _ in 0..20000 {
            core::hint::spin_loop();
        }
    }
}
