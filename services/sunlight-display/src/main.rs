#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use sunlight_ipc::debug_log;
use sunlight_ipc::{
    endpoint_create, ipc_recv, ipc_reply, nameserver_register, sgp::SgpMsg, CapabilityToken,
    IpcMsg, MouseMsg,
};

struct BumpAllocator;
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024];
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

const FP_SHIFT: i32 = 16;
const FP_ONE: i32 = 1 << FP_SHIFT;

// ---------------------------------------------------------------------------
// Pointer tuning constants (16.16 fixed-point).
//
// SENSITIVITY is the master "feel" knob. It is a flat multiplier applied to
// every raw delta from the mouse driver before acceleration:
//   * 0.5  -> slow / high-precision (good for fine work, drawing)
//   * 1.0  -> raw 1:1 mapping (driver units == pixels at low speed)
//   * 1.5  -> default, comfortable desktop feel
//   * 3.0  -> fast / "gaming" (small hand motions cross the screen)
// A future settings UI should write `CompositorState::mouse_sensitivity_fp`
// (clamped to [SENS_MIN_FP, SENS_MAX_FP]); nothing else needs to change.
// ---------------------------------------------------------------------------
const SENS_DEFAULT_FP: i32 = (FP_ONE * 3) / 2; // 1.5
#[allow(dead_code)] // valid range for a future settings UI clamp
const SENS_MIN_FP: i32 = FP_ONE / 2; //            0.5
#[allow(dead_code)]
const SENS_MAX_FP: i32 = FP_ONE * 3; //            3.0

// Acceleration curve. Below ACCEL_LOW the motion is precise (pure 1:1 so the
// cursor lands exactly where the hand says). Above it, the per-event gain ramps
// linearly with delta magnitude up to ACCEL_MAX_FP, so flicks cover ground fast
// while slow drags stay pixel-accurate.
const ACCEL_LOW: i32 = 3; //                speed (px/event) below which gain == 1.0
const ACCEL_SLOPE_FP: i32 = FP_ONE / 16; // gain added per unit of speed past ACCEL_LOW
const ACCEL_MAX_FP: i32 = (FP_ONE * 5) / 2; // 2.5x hard cap

// Exponential position smoothing. We accumulate the "true" target position and
// move the displayed cursor a fraction of the remaining distance each event:
//   displayed += alpha * (target - displayed)
// Slow motions (<= SMOOTH_SNAP_SPEED) snap instantly (alpha = 1.0) so precise
// pointing has zero lag; faster motions blend (SMOOTH_ALPHA_FP) to iron out the
// jitter of raw PS/2 deltas. Trade-off: a hair of lag during fast flicks in
// exchange for buttery, jitter-free travel.
const SMOOTH_SNAP_SPEED: i32 = 3;
const SMOOTH_ALPHA_FP: i32 = (FP_ONE * 45) / 100; // 0.45

// Keep the cursor hot-spot a couple pixels inside the panel so it can never
// "escape" off an edge.
const EDGE_MARGIN: i32 = 2;

struct PointerState {
    // Smoothed, on-screen position (what we actually draw).
    x_fp: i32,
    y_fp: i32,
    // Raw accumulated target the smoothing chases toward.
    target_x_fp: i32,
    target_y_fp: i32,
    buttons: u8,
    fb_width: u32,
    fb_height: u32,
}

impl PointerState {
    fn new(fb_w: u32, fb_h: u32) -> Self {
        let cx = ((fb_w as i32 / 2).max(0)) << FP_SHIFT;
        let cy = ((fb_h as i32 / 2).max(0)) << FP_SHIFT;
        Self {
            x_fp: cx,
            y_fp: cy,
            target_x_fp: cx,
            target_y_fp: cy,
            buttons: 0,
            fb_width: fb_w,
            fb_height: fb_h,
        }
    }

    fn apply_motion(&mut self, dx: i32, dy: i32, buttons: u8, sensitivity_fp: i32) {
        let speed = dx.abs().max(dy.abs());

        // 1) Acceleration: 1.0x for slow motion, ramping to ACCEL_MAX_FP for fast.
        let accel_fp = if speed <= ACCEL_LOW {
            FP_ONE
        } else {
            (FP_ONE + (speed - ACCEL_LOW) * ACCEL_SLOPE_FP).min(ACCEL_MAX_FP)
        };

        // 2) Effective gain = sensitivity * acceleration (fixed-point multiply).
        let gain_fp = ((sensitivity_fp as i64 * accel_fp as i64) >> FP_SHIFT) as i64;

        // 3) Advance the raw target (delta is in whole pixels -> shift to FP).
        self.target_x_fp = (self.target_x_fp as i64 + (dx as i64 * gain_fp)) as i32;
        self.target_y_fp = (self.target_y_fp as i64 + (dy as i64 * gain_fp)) as i32;
        self.clamp_target();

        // 4) Exponential smoothing toward the target. Slow motions snap so
        //    precise pointing never lags; fast motions blend out jitter.
        let alpha_fp = if speed <= SMOOTH_SNAP_SPEED {
            FP_ONE
        } else {
            SMOOTH_ALPHA_FP
        };
        self.x_fp += (((self.target_x_fp - self.x_fp) as i64 * alpha_fp as i64) >> FP_SHIFT) as i32;
        self.y_fp += (((self.target_y_fp - self.y_fp) as i64 * alpha_fp as i64) >> FP_SHIFT) as i32;

        self.buttons = buttons;
    }

    fn min_fp(&self) -> i32 {
        EDGE_MARGIN << FP_SHIFT
    }
    fn max_x_fp(&self) -> i32 {
        ((self.fb_width as i32 - 1 - EDGE_MARGIN).max(EDGE_MARGIN)) << FP_SHIFT
    }
    fn max_y_fp(&self) -> i32 {
        ((self.fb_height as i32 - 1 - EDGE_MARGIN).max(EDGE_MARGIN)) << FP_SHIFT
    }

    fn clamp_target(&mut self) {
        self.target_x_fp = self.target_x_fp.clamp(self.min_fp(), self.max_x_fp());
        self.target_y_fp = self.target_y_fp.clamp(self.min_fp(), self.max_y_fp());
    }

    // Belt-and-suspenders clamp on the displayed position so the drawn hot-spot
    // is always on screen even mid-smoothing.
    fn sync_clamp(&mut self) {
        self.x_fp = self.x_fp.clamp(self.min_fp(), self.max_x_fp());
        self.y_fp = self.y_fp.clamp(self.min_fp(), self.max_y_fp());
    }

    fn x(&self) -> i32 {
        (self.x_fp >> FP_SHIFT).max(0).min((self.fb_width - 1) as i32)
    }

    fn y(&self) -> i32 {
        (self.y_fp >> FP_SHIFT).max(0).min((self.fb_height - 1) as i32)
    }
}

struct Window {
    id: u64,
    _shm_cap: CapabilityToken,
    buffer: *mut u32,
    width: u32,
    height: u32,
    x: u32,
    y: u32,
}

struct CompositorState {
    windows: Vec<Window>,
    mouse_x: u16,
    mouse_y: u16,
    pointer: PointerState,
    // Pointer sensitivity (16.16 fixed-point). Adjust this (within
    // [SENS_MIN_FP, SENS_MAX_FP]) from a future settings UI to change mouse
    // speed; see the SENSITIVITY docs near the constants above.
    mouse_sensitivity_fp: i32,
    fb: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
}

fn fb_stride(state: &CompositorState) -> usize {
    (state.fb_pitch / 4) as usize
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
    let stride = fb_stride(state);
    for y in 0..state.fb_height as usize {
        let row = y * stride;
        for x in 0..state.fb_width as usize {
            unsafe { state.fb.add(row + x).write(DESKTOP_COLOR); }
        }
    }
}

fn composite_window(state: &CompositorState, win: &Window) {
    if win.buffer.is_null() || win.x >= state.fb_width || win.y >= state.fb_height {
        return;
    }

    let copy_width = win.width.min(state.fb_width - win.x) as usize;
    let copy_height = win.height.min(state.fb_height - win.y) as usize;
    let stride = fb_stride(state);

    for row in 0..copy_height {
        unsafe {
            let src = win.buffer.add(row * win.width as usize);
            let dst = state
                .fb
                .add((win.y as usize + row) * stride + win.x as usize);
            core::ptr::copy_nonoverlapping(src, dst, copy_width);
        }
    }
}

fn draw_cursor(state: &CompositorState) {
    let base_x = state.mouse_x as i32;
    let base_y = state.mouse_y as i32;
    let stride = fb_stride(state);

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
                    .add(y as usize * stride + x as usize)
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

fn debug_hex(val: u32) {
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    let hex = b"0123456789ABCDEF";
    for i in 0..8 {
        buf[2 + i] = hex[((val >> (28 - i * 4)) & 0xF) as usize];
    }
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 99u64 => _,
            in("rdi") buf.as_ptr() as u64,
            in("rsi") 10u64,
            lateout("rcx") _, lateout("r11") _,
            options(nostack)
        );
    }
}

fn debug_dec(val: u32) {
    let mut buf = [0u8; 11];
    let mut n = val;
    let mut i = 11;
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let len = 11 - i;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 99u64 => _,
            in("rdi") buf.as_ptr().add(i) as u64,
            in("rsi") len as u64,
            lateout("rcx") _, lateout("r11") _,
            options(nostack)
        );
    }
}

#[allow(dead_code)]
fn debug_i32(val: i32) {
    if val < 0 {
        debug_log("-");
        debug_dec((-val) as u32);
    } else {
        debug_dec(val as u32);
    }
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

    let (fb_ptr, packed_wh, pitch, bpp) = match sunlight_ipc::map_framebuffer() {
        Some(v) => v,
        None => {
            debug_log("[DISPLAY] Failed to map framebuffer. Exiting.\n");
            loop {}
        }
    };

    let fb_width = (packed_wh & 0xffffffff) as u32;
    let fb_height = (packed_wh >> 32) as u32;

    debug_log("[DISPLAY] fb size: ");
    debug_dec(fb_width);
    debug_log("x");
    debug_dec(fb_height);
    debug_log(" pitch=");
    debug_hex(pitch as u32);
    debug_log(" bpp=");
    debug_dec(bpp as u32);
    debug_log("\n");

    let mut state = CompositorState {
        windows: Vec::new(),
        mouse_x: (fb_width / 2) as u16,
        mouse_y: (fb_height / 2) as u16,
        pointer: PointerState::new(fb_width, fb_height),
        mouse_sensitivity_fp: SENS_DEFAULT_FP, // 1.5 — see SENSITIVITY docs above
        fb: fb_ptr as *mut u32,
        fb_width,
        fb_height,
        fb_pitch: pitch as u32,
    };
    redraw_scene(&state);

    let mut next_win_id: u64 = 1;

    loop {
        let msg = ipc_recv(my_ep);

        match msg.label {
            SgpMsg::CREATE_WINDOW => {
                let w = (msg.words[0] & 0xffffffff) as u32;
                let h = (msg.words[0] >> 32) as u32;
                let size = (w as usize * h as usize * 4).max(4096);

                match sunlight_ipc::shm_create(size, 0) {
                    Ok((_, shm_tok)) => {
                        let our_buf = match sunlight_ipc::shm_map(shm_tok) {
                            Ok(p) => p as *mut u32,
                            Err(_) => core::ptr::null_mut(),
                        };

                        let id = next_win_id;
                        next_win_id += 1;

                        // Place the window prominently: horizontally centered and
                        // in the upper-middle of the screen, rather than auto-tiled
                        // into a corner. Subsequent windows get a small cascade
                        // offset so they don't perfectly overlap.
                        let cascade = ((id.saturating_sub(1)) % 5) as u32 * 24;
                        let win_x: u32 = state
                            .fb_width
                            .saturating_sub(w)
                            .saturating_div(2)
                            .saturating_add(cascade);
                        let win_y: u32 = (state.fb_height / 4)
                            .saturating_sub(h / 2)
                            .saturating_add(cascade);

                        debug_log("[DISPLAY] create_window id=");
                        debug_dec(id as u32);
                        debug_log(" pos=");
                        debug_dec(win_x);
                        debug_log("x");
                        debug_dec(win_y);
                        debug_log(" size=");
                        debug_dec(w);
                        debug_log("x");
                        debug_dec(h);
                        debug_log(" stride=");
                        debug_dec(w * 4);
                        debug_log("\n");

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
                            .word(3, (w * 4) as u64)
                            .word(4, win_x as u64)
                            .word(5, win_y as u64);
                        reply.caps[0] = shm_tok;
                        reply.cap_count = 1;

                        let _ = ipc_reply(reply);
                    }
                    Err(_) => {
                        let err = IpcMsg::with_label(0xA1FE);
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
                let packed = (state.mouse_x as u64) | ((state.mouse_y as u64) << 16);
                let wake = IpcMsg::with_label(SgpMsg::REPLY).word(0, packed);
                let _ = ipc_reply(wake);
            }

            SgpMsg::DESTROY_WINDOW => {
                let win_id = msg.words[0];
                state.windows.retain(|w| w.id != win_id);
                redraw_scene(&state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            MouseMsg::RAW_MOTION => {
                let raw = msg.words[0];
                let dx = ((raw & 0xFFFF) as i16) as i32;
                let dy = (((raw >> 16) & 0xFFFF) as i16) as i32;
                let buttons = ((raw >> 32) & 0xFF) as u8;

                // NOTE: no per-event serial logging here — the UART is slow and
                // blocking, so printing on every mouse packet visibly stutters
                // the cursor. Keep this path lean.
                state
                    .pointer
                    .apply_motion(dx, dy, buttons, state.mouse_sensitivity_fp);
                state.pointer.sync_clamp();

                state.mouse_x = state.pointer.x() as u16;
                state.mouse_y = state.pointer.y() as u16;

                redraw_scene(&state);

                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            _ => {
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }
        }
    }
}
