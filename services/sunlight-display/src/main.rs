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

// Acceleration curve — three tiers, all TUNING-annotated for easy adjustment:
//
//  Tier 0 (speed <= ACCEL_LOW)  : pure 1:1 — pixel-perfect for fine work / drawing
//  Tier 1 (ACCEL_LOW < speed <= ACCEL_HIGH): gentle linear ramp — natural desktop feel
//  Tier 2 (speed > ACCEL_HIGH)  : capped at ACCEL_MAX_FP — fast flicks stay controlled
//
// The 0.05/unit slope (ACCEL_SLOPE_FP) means at speed=20 gain reaches ~1.75x,
// leaving headroom below the 2.0x cap and preventing the "cursor teleports"
// feeling that plagued the old 4x cap.
const ACCEL_LOW: i32 = 5; // TUNING: slow-tier ceiling (px/event); below = 1:1
const ACCEL_HIGH: i32 = 20; // TUNING: fast-tier floor; above = capped at ACCEL_MAX_FP
const ACCEL_SLOPE_FP: i32 = FP_ONE / 20; // TUNING: gain added per unit past ACCEL_LOW (0.05/px)
const ACCEL_MAX_FP: i32 = FP_ONE * 2; // TUNING: 2.0x hard cap (was 4x — felt too aggressive)

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
    // Master speed knob in 16.16 fixed-point (see SENSITIVITY constants above).
    // A future settings UI should write this field directly on the PointerState
    // (clamped to [SENS_MIN_FP, SENS_MAX_FP]) — nothing else needs to change.
    sensitivity_fp: i32,
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
            sensitivity_fp: SENS_DEFAULT_FP, // 1.5 — comfortable for ~80 % of users
        }
    }

    fn apply_motion(&mut self, dx: i32, dy: i32, buttons: u8) {
        let speed = dx.abs().max(dy.abs());

        // 1) Three-tier acceleration:
        //    Tier 0 (≤ ACCEL_LOW)  : 1:1 — no acceleration; precise pixel control.
        //    Tier 1 (≤ ACCEL_HIGH) : linear ramp from 1.0x up toward ACCEL_MAX_FP.
        //    Tier 2 (> ACCEL_HIGH) : hard-capped at ACCEL_MAX_FP; fast flicks stay sane.
        let accel_fp = if speed <= ACCEL_LOW {
            FP_ONE
        } else if speed <= ACCEL_HIGH {
            (FP_ONE + (speed - ACCEL_LOW) * ACCEL_SLOPE_FP).min(ACCEL_MAX_FP)
        } else {
            ACCEL_MAX_FP
        };

        // 2) Effective gain = sensitivity × acceleration (fixed-point multiply).
        let gain_fp = ((self.sensitivity_fp as i64 * accel_fp as i64) >> FP_SHIFT) as i64;

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
    mouse_sensitivity_fp: i32,
    dragged_window_id: Option<u64>,
    prev_buttons: u8,
    fb: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
}

fn fb_stride(state: &CompositorState) -> usize {
    (state.fb_pitch / 4) as usize
}

const DESKTOP_COLOR: u32 = 0x00181818;
const TITLEBAR_H: u32 = 24;
const TITLEBAR_COLOR: u32 = 0x00333333;
const BORDER_W: u32 = 2;
const BORDER_COLOR: u32 = 0x00555555;
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

fn draw_rect(state: &CompositorState, x: u32, y: u32, w: u32, h: u32, color: u32) {
    if x >= state.fb_width || y >= state.fb_height || w == 0 || h == 0 {
        return;
    }
    let stride = fb_stride(state);
    let x_end = (x + w).min(state.fb_width) as usize;
    let y_end = (y + h).min(state.fb_height) as usize;
    for row in y as usize..y_end {
        for col in x as usize..x_end {
            unsafe { state.fb.add(row * stride + col).write(color); }
        }
    }
}

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

    let chrome_w = win.width + BORDER_W * 2;
    let chrome_h = TITLEBAR_H + win.height + BORDER_W;

    // Left border
    draw_rect(state, win.x, win.y, BORDER_W, chrome_h, BORDER_COLOR);
    // Right border
    draw_rect(state, win.x + BORDER_W + win.width, win.y, BORDER_W, chrome_h, BORDER_COLOR);
    // Bottom border
    draw_rect(state, win.x, win.y + TITLEBAR_H + win.height, chrome_w, BORDER_W, BORDER_COLOR);
    // Title bar
    draw_rect(state, win.x, win.y, chrome_w, TITLEBAR_H, TITLEBAR_COLOR);

    // Client content — starts at (win.x + BORDER_W, win.y + TITLEBAR_H)
    let client_x = win.x + BORDER_W;
    let client_y = win.y + TITLEBAR_H;
    if client_x >= state.fb_width || client_y >= state.fb_height {
        return;
    }

    let copy_width = win.width.min(state.fb_width - client_x) as usize;
    let copy_height = win.height.min(state.fb_height - client_y) as usize;
    let stride = fb_stride(state);

    for row in 0..copy_height {
        unsafe {
            let src = win.buffer.add(row * win.width as usize);
            let dst = state.fb.add((client_y as usize + row) * stride + client_x as usize);
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
        mouse_sensitivity_fp: SENS_DEFAULT_FP,
        dragged_window_id: None,
        prev_buttons: 0,
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

                        // words[4]/[5] = client-area origin (below title bar / inside borders)
                        let mut reply = IpcMsg::with_label(SgpMsg::REPLY)
                            .word(1, id)
                            .word(2, size as u64)
                            .word(3, (w * 4) as u64)
                            .word(4, (win_x + BORDER_W) as u64)
                            .word(5, (win_y + TITLEBAR_H) as u64);
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
                let win_id = msg.words[0];
                let packed = (state.mouse_x as u64) | ((state.mouse_y as u64) << 16);
                let mut wake = IpcMsg::with_label(SgpMsg::REPLY).word(0, packed);
                // words[1]: current client-area origin so clients re-anchor after drag
                if let Some(win) = state.windows.iter().find(|w| w.id == win_id) {
                    let cx = (win.x + BORDER_W) as u64;
                    let cy = (win.y + TITLEBAR_H) as u64;
                    wake = wake.word(1, cx | (cy << 32));
                }
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

                // Snapshot cursor position before applying motion so we can
                // compute the actual pixel delta for window dragging.
                let prev_cx = state.pointer.x() as u32;
                let prev_cy = state.pointer.y() as u32;
                let prev_buttons = state.prev_buttons;

                state.pointer.sensitivity_fp = state.mouse_sensitivity_fp;
                state.pointer.apply_motion(dx, dy, buttons);
                state.pointer.sync_clamp();

                state.mouse_x = state.pointer.x() as u16;
                state.mouse_y = state.pointer.y() as u16;

                let cx = state.pointer.x() as u32;
                let cy = state.pointer.y() as u32;
                let left_down = (buttons & 1) != 0;
                let was_left_down = (prev_buttons & 1) != 0;

                if left_down && !was_left_down {
                    // Left button just pressed — hit-test title bars front-to-back.
                    let mut clicked_id: Option<u64> = None;
                    for win in state.windows.iter().rev() {
                        let tb_x = win.x;
                        let tb_y = win.y;
                        let tb_w = win.width + BORDER_W * 2;
                        if cx >= tb_x && cx < tb_x + tb_w && cy >= tb_y && cy < tb_y + TITLEBAR_H {
                            clicked_id = Some(win.id);
                            break;
                        }
                    }
                    if let Some(id) = clicked_id {
                        state.dragged_window_id = Some(id);
                        // Z-order: move clicked window to the end so it renders on top.
                        if let Some(pos) = state.windows.iter().position(|w| w.id == id) {
                            let win = state.windows.remove(pos);
                            state.windows.push(win);
                        }
                    }
                } else if !left_down {
                    state.dragged_window_id = None;
                }

                // Apply drag delta to the window being moved.
                if left_down {
                    let dcx = cx as i32 - prev_cx as i32;
                    let dcy = cy as i32 - prev_cy as i32;
                    if dcx != 0 || dcy != 0 {
                        if let Some(drag_id) = state.dragged_window_id {
                            if let Some(win) = state.windows.iter_mut().find(|w| w.id == drag_id) {
                                win.x = (win.x as i32 + dcx).max(0) as u32;
                                win.y = (win.y as i32 + dcy).max(0) as u32;
                            }
                        }
                    }
                }

                state.prev_buttons = buttons;

                redraw_scene(&state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            _ => {
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }
        }
    }
}
