#![no_std]
#![no_main]

// No heap allocations in the Eyes demo (we write directly into the provided SHM buffer).
// Provide a tiny stub allocator to satisfy the libcore requirements if any crate pulls alloc.
struct NoAlloc;
unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}
#[global_allocator]
static A: NoAlloc = NoAlloc;

use sunlight_ipc::{debug_log, ipc_call, nameserver_lookup, sgp::SgpMsg, IpcMsg};

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
        for _ in 0..100000 {
            core::hint::spin_loop();
        }
    };

    // 1. CREATE_WINDOW (width=400, height=300 packed)
    let req = IpcMsg::with_label(SgpMsg::CREATE_WINDOW).word(0, 400u64 | (300u64 << 32));
    let reply = ipc_call(display_ep, req);

    if reply.label != SgpMsg::REPLY {
        debug_log("[EYES] CREATE_WINDOW failed\n");
        loop {}
    }

    let shm_tok = reply.caps[0];
    let win_id = reply.words[1];
    let _size = reply.words[2] as usize;
    let _stride = reply.words[3] as usize; // bytes per row (width*4)
                                           // Client-area origin (below title bar, inside borders). Mutable so dragging
                                           // updates the origin on each EVENT_POLL, keeping pupil tracking correct.
    let mut win_x = reply.words[4] as i32;
    let mut win_y = reply.words[5] as i32;

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

    // Window / eye geometry (window is a fixed 400x300 surface).
    const WIN_W: i32 = 400;
    const WIN_H: i32 = 300;
    const EYE_Y: i32 = 150;
    const LEFT_EYE_X: i32 = 120;
    const RIGHT_EYE_X: i32 = 280;
    const EYE_RADIUS: i32 = 40;
    const PUPIL_RADIUS: i32 = 10;
    // How far the pupil center may travel from the eye center while staying
    // comfortably inside the white. Real circular constraint (not a square box).
    const MAX_OFFSET: i32 = EYE_RADIUS - PUPIL_RADIUS - 4;

    // Filled-circle rasterizer into the window-local buffer. Integer-only:
    // a pixel is inside when dx*dx + dy*dy <= r*r.
    let fill_circle = |cx: i32, cy: i32, r: i32, color: u32| {
        let r2 = r * r;
        for y in (cy - r)..=(cy + r) {
            if y < 0 || y >= WIN_H {
                continue;
            }
            let dy = y - cy;
            for x in (cx - r)..=(cx + r) {
                if x < 0 || x >= WIN_W {
                    continue;
                }
                let dx = x - cx;
                if dx * dx + dy * dy <= r2 {
                    unsafe {
                        buffer.add((y * WIN_W + x) as usize).write_volatile(color);
                    }
                }
            }
        }
    };

    // Render one eye. `mouse_gx/gy` are GLOBAL (screen) cursor coords; `cx/cy`
    // are the eye center in window-local coords. We translate the eye center to
    // screen space, take the vector to the cursor, and project the pupil along
    // that direction. This works across the ENTIRE screen — there is no
    // "inside the window" dead zone.
    // `wx`/`wy` are the current client-area screen origin — passed explicitly so
    // the closure doesn't capture win_x/win_y by reference, letting us mutate
    // them inside the loop when the window is dragged.
    let render_eye = |cx: i32, cy: i32, mouse_gx: i32, mouse_gy: i32, wx: i32, wy: i32| {
        fill_circle(cx, cy, EYE_RADIUS, eye_color);

        let dx = mouse_gx - (wx + cx);
        let dy = mouse_gy - (wy + cy);

        let (px, py) = {
            let d2 = (dx as i64 * dx as i64 + dy as i64 * dy as i64) as u32;
            if d2 == 0 {
                (cx, cy)
            } else {
                let dist = isqrt(d2).max(1) as i32;
                let off = MAX_OFFSET.min(dist);
                (cx + (dx * off) / dist, cy + (dy * off) / dist)
            }
        };

        fill_circle(px, py, PUPIL_RADIUS, pupil_color);
    };

    // Initial frame so the window is never solid black; pupils centered.
    {
        for i in 0..(WIN_W * WIN_H) as usize {
            unsafe {
                buffer.add(i).write_volatile(bg_color);
            }
        }
        render_eye(
            LEFT_EYE_X,
            EYE_Y,
            win_x + LEFT_EYE_X,
            win_y + EYE_Y,
            win_x,
            win_y,
        );
        render_eye(
            RIGHT_EYE_X,
            EYE_Y,
            win_x + RIGHT_EYE_X,
            win_y + EYE_Y,
            win_x,
            win_y,
        );

        let commit = IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id);
        let _ = ipc_call(display_ep, commit);
    }

    debug_log("[EYES] Initial frame drawn. Entering event loop...\n");

    loop {
        // Block until next mouse event (zero CPU when idle).
        let poll_req = IpcMsg::with_label(SgpMsg::EVENT_POLL).word(0, win_id);
        let event_reply = ipc_call(display_ep, poll_req);

        let packed = event_reply.words[0];
        let mouse_x = (packed & 0xFFFF) as i32;
        let mouse_y = ((packed >> 16) & 0xFFFF) as i32;

        // Update client-area origin if the window was dragged since last poll.
        let pos_packed = event_reply.words[1];
        if pos_packed != 0 {
            win_x = (pos_packed & 0xFFFF_FFFF) as i32;
            win_y = (pos_packed >> 32) as i32;
        }

        // --- Zero-copy render into the shared buffer ---
        for i in 0..(WIN_W * WIN_H) as usize {
            unsafe {
                buffer.add(i).write_volatile(bg_color);
            }
        }

        render_eye(LEFT_EYE_X, EYE_Y, mouse_x, mouse_y, win_x, win_y);
        render_eye(RIGHT_EYE_X, EYE_Y, mouse_x, mouse_y, win_x, win_y);

        let commit = IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id);
        let _ = ipc_call(display_ep, commit);

        // Small throttle so we don't spin 100% CPU on the fast immediate-reply
        // EVENT_POLL path. Still very responsive for mouse following.
        for _ in 0..20000 {
            core::hint::spin_loop();
        }
    }
}
