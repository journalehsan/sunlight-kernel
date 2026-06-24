#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use sunlight_ipc::{
    endpoint_create, ipc_call, ipc_recv, ipc_reply, nameserver_register,
    CapabilityToken, IpcMsg, sgp::SgpMsg,
};
use sunlight_ipc::{debug_log, ProcessExit};

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
    shm_cap: CapabilityToken,
    buffer: *mut u32,   // Our mapping of the client's SHM (for composition)
    width: u32,
    height: u32,
    x: u32,
    y: u32,
}

struct CompositorState {
    windows: Vec<Window>,
    /// (endpoint_token_value, window_id) — we reply to these on input
    event_waiters: Vec<(u64, u64)>,
    mouse_x: u16,
    mouse_y: u16,
    fb: *mut u32,
    fb_width: u32,
    fb_height: u32,
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
    let (fb_ptr, packed_wh, pitch, _bpp) = match sunlight_ipc::map_framebuffer() {
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
        event_waiters: Vec::new(),
        mouse_x: (fb_width / 2) as u16,
        mouse_y: (fb_height / 2) as u16,
        fb: fb_ptr as *mut u32,
        fb_width,
        fb_height,
    };

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

                        state.windows.push(Window {
                            id,
                            shm_cap: shm_tok,
                            buffer: our_buf,
                            width: w,
                            height: h,
                            x: 80,
                            y: 60,
                        });

                        let mut reply = IpcMsg::with_label(SgpMsg::REPLY)
                            .word(1, id)
                            .word(2, size as u64)
                            .word(3, (w * 4) as u64); // stride
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
                if let Some(win) = state.windows.iter().find(|w| w.id == win_id) {
                    if !win.buffer.is_null() {
                        // Immediate composition: copy client SHM region into FB
                        for row in 0..win.height {
                            unsafe {
                                let src = win.buffer.add((row * win.width) as usize);
                                let dst_base = state.fb.add(((win.y + row) * state.fb_width + win.x) as usize);
                                core::ptr::copy_nonoverlapping(src, dst_base, win.width as usize);
                            }
                        }
                    }
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            SgpMsg::EVENT_POLL => {
                let win_id = msg.words[0];
                // Park the client — do not reply now. The next mouse event will wake it.
                // (The caller's ipc_call is blocked in the kernel until we reply.)
                state.event_waiters.push((msg.badge, win_id)); // badge or caller info if available; we use a simple token
                // For real directed reply we would store more context.
                // In this prototype the wake happens via the mouse path below.
            }

            SgpMsg::DESTROY_WINDOW => {
                let win_id = msg.words[0];
                state.windows.retain(|w| w.id != win_id);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            0x2 => {
                // Mouse event forwarded by sunlight-mouse
                let packed = msg.words[0];
                state.mouse_x = (packed & 0xffff) as u16;
                state.mouse_y = ((packed >> 16) & 0xffff) as u16;

                // Wake all parked clients
                let waiters = core::mem::take(&mut state.event_waiters);
                for (_ctx, _wid) in waiters {
                    let wake = IpcMsg::with_label(SgpMsg::REPLY).word(0, packed);
                    // Best effort reply. For multiple clients a more advanced reply cap or per-client endpoint would be used.
                    let _ = ipc_reply(wake);
                }
            }

            _ => {
                // Unknown: just ack
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }
        }
    }
}
