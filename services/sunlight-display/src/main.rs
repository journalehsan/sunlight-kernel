#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use sunlight_ipc::{
    endpoint_create, ipc_recv, ipc_reply, nameserver_register,
    CapabilityToken, IpcMsg, sgp::SgpMsg,
};
use sunlight_ipc::debug_log;

/// Very small bump allocator so we can use Vec/alloc in the compositor.
struct BumpAllocator;
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024]; // 8 MiB
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

struct Window {
    id: u64,
    _shm_cap: CapabilityToken,
    buffer: *mut u32,   // Our mapping of the client's SHM (for composition)
    width: u32,
    height: u32,
    x: u32,
    y: u32,
}

struct CompositorState {
    windows: Vec<Window>,
    mouse_x: u16,
    mouse_y: u16,
    fb: *mut u32,
    fb_width: u32,
    fb_height: u32,
}

const DESKTOP_COLOR: u32 = 0x00181818;
const CURSOR_COLOR: u32 = 0x00F5F5F5;
const CURSOR_SHADOW_COLOR: u32 = 0x00000000;
const CURSOR_W: usize = 8;
const CURSOR_H: usize = 12;
const CURSOR_BITMAP: [u8; CURSOR_H] = [
    0b1000_0000,
    0b1100_0000,
    0b1110_0000,
    0b1111_0000,
    0b1111_1000,
    0b1111_1100,
    0b1111_1110,
    0b1111_1000,
    0b1101_0000,
    0b1001_0000,
    0b0001_0000,
    0b0000_0000,
];

fn clear_framebuffer(state: &CompositorState) {
    let pixels = (state.fb_width as usize).saturating_mul(state.fb_height as usize);
    if pixels == 0 {
        return;
    }
    unsafe {
        core::ptr::write_bytes(state.fb, 0, pixels);
        for i in 0..pixels {
            state.fb.add(i).write(DESKTOP_COLOR);
        }
    }
}

fn composite_window(state: &CompositorState, win: &Window) {
    if win.buffer.is_null() || win.x >= state.fb_width || win.y >= state.fb_height {
        return;
    }

    let copy_width = win.width.min(state.fb_width - win.x) as usize;
    let copy_height = win.height.min(state.fb_height - win.y) as usize;

    for row in 0..copy_height {
        unsafe {
            let src = win.buffer.add(row * win.width as usize);
            let dst = state
                .fb
                .add((win.y as usize + row) * state.fb_width as usize + win.x as usize);
            core::ptr::copy_nonoverlapping(src, dst, copy_width);
        }
    }
}

fn draw_cursor(state: &CompositorState) {
    let base_x = state.mouse_x as i32;
    let base_y = state.mouse_y as i32;

    for (row, mask) in CURSOR_BITMAP.iter().copied().enumerate() {
        for col in 0..CURSOR_W {
            if (mask & (1 << (7 - col))) == 0 {
                continue;
            }

            let x = base_x + col as i32;
            let y = base_y + row as i32;
            if x < 0 || y < 0 || x >= state.fb_width as i32 || y >= state.fb_height as i32 {
                continue;
            }

            let color = if col == CURSOR_W - 1 || row == CURSOR_H - 1 {
                CURSOR_SHADOW_COLOR
            } else {
                CURSOR_COLOR
            };
            unsafe {
                state
                    .fb
                    .add(y as usize * state.fb_width as usize + x as usize)
                    .write(color);
            }
        }
    }
}

fn redraw_scene(state: &CompositorState) {
    clear_framebuffer(state);
    for win in &state.windows {
        composite_window(state, win);
    }
    draw_cursor(state);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[DISPLAY] PANIC\n");
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[DISPLAY] sunlight-display (compositor) starting\n");

    let my_ep = endpoint_create();
    nameserver_register("display_server", my_ep);
    debug_log("[DISPLAY] registered as display_server\n");

    // Map the physical framebuffer (new syscall)
    let (fb_ptr, packed_wh, _pitch, _bpp) = match sunlight_ipc::map_framebuffer() {
        Some(v) => v,
        None => {
            debug_log("[DISPLAY] Failed to map framebuffer. Exiting.\n");
            loop {}
        }
    };

    let fb_width = (packed_wh & 0xffffffff) as u32;
    let fb_height = (packed_wh >> 32) as u32;

    debug_log("[DISPLAY] Framebuffer mapped\n");

    let mut state = CompositorState {
        windows: Vec::new(),
        mouse_x: (fb_width / 2) as u16,
        mouse_y: (fb_height / 2) as u16,
        fb: fb_ptr as *mut u32,
        fb_width,
        fb_height,
    };
    redraw_scene(&state);

    let mut next_win_id: u64 = 1;

    loop {
        // Block here for SGP requests from clients or mouse events (label 0x2) from mouse driver
        let msg = ipc_recv(my_ep);

        match msg.label {
            SgpMsg::CREATE_WINDOW => {
                let w = (msg.words[0] & 0xffffffff) as u32;
                let h = (msg.words[0] >> 32) as u32;
                let size = (w as usize * h as usize * 4).max(4096);

                match sunlight_ipc::shm_create(size, 0) {
                    Ok((_, shm_tok)) => {
                        // Map into our address space so we can composite
                        let our_buf = match sunlight_ipc::shm_map(shm_tok) {
                            Ok(p) => p as *mut u32,
                            Err(_) => core::ptr::null_mut(),
                        };

                        let id = next_win_id;
                        next_win_id += 1;

                        // Fixed placement for the demo (overlapping if many windows created).
                        let win_x: u32 = 80;
                        let win_y: u32 = 60;

                        state.windows.push(Window {
                            id,
                            _shm_cap: shm_tok,
                            buffer: our_buf,
                            width: w,
                            height: h,
                            x: win_x,
                            y: win_y,
                        });
                        redraw_scene(&state);

                        let mut reply = IpcMsg::with_label(SgpMsg::REPLY)
                            .word(1, id)
                            .word(2, size as u64)
                            .word(3, (w * 4) as u64) // stride
                            .word(4, win_x as u64)
                            .word(5, win_y as u64);
                        reply.caps[0] = shm_tok;
                        reply.cap_count = 1;

                        let _ = ipc_reply(reply);
                    }
                    Err(_) => {
                        let err = IpcMsg::with_label(0xA1FE); // simple error
                        let _ = ipc_reply(err);
                    }
                }
            }

            SgpMsg::COMMIT_FRAME => {
                let win_id = msg.words[0];
                if state.windows.iter().any(|w| w.id == win_id) {
                    redraw_scene(&state);
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            SgpMsg::EVENT_POLL => {
                let _win_id = msg.words[0];
                // Reply immediately with the current mouse position.
                // This unblocks the client (EVENT_POLL was designed to block until
                // "next input", but with the single reply_target model we reply with
                // latest-known here so eyes (and similar) can render and follow the cursor.
                let packed = (state.mouse_x as u64) | ((state.mouse_y as u64) << 16);
                let wake = IpcMsg::with_label(SgpMsg::REPLY).word(0, packed);
                let _ = ipc_reply(wake);
                // (We no longer park in event_waiters for this path.)
            }

            SgpMsg::DESTROY_WINDOW => {
                let win_id = msg.words[0];
                state.windows.retain(|w| w.id != win_id);
                redraw_scene(&state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            0x2 => {
                // Mouse event forwarded by sunlight-mouse
                let packed = msg.words[0];
                state.mouse_x = (packed & 0xffff) as u16;
                state.mouse_y = ((packed >> 16) & 0xffff) as u16;
                redraw_scene(&state);

                // Unblock the mouse driver (it used ipc_call). We use the current
                // reply target which was set when we received this 0x2 via ipc_recv.
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));

                // (Any old parked EVENT_POLL clients are now served via immediate reply
                // in the EVENT_POLL arm above.)
            }

            _ => {
                // Unknown: just ack
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }
        }
    }
}
