//! Vortex Shell — SunlightOS desktop surface.
//!
//! Renders the wallpaper fullscreen plus two shell panel strips:
//!   • Top bar:    [☀ workspaces]  [App Title / SunlightOS]  [status cluster]
//!   • Bottom bar: [overview|sidebar|settings]  [grid|term|tasks|calc]  [Search…]

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, get_init_cap, ipc_call, nameserver_lookup, process_yield, IpcMsg, ProcessExit,
    SgpMsg, SpawnRequest,
};
use sunlight_ui::{
    image::TgaImage, App, Canvas, Color, Event, Rect, Theme, Window, WindowConfig,
};

// ---------------------------------------------------------------------------
// Wallpaper asset
// ---------------------------------------------------------------------------

static WALLPAPER_TGA: &[u8] = include_bytes!("../../../docs/images/wallpaper.tga");
const FALLBACK_BG: u32 = 0x00121214;

// ---------------------------------------------------------------------------
// Window geometry
// ---------------------------------------------------------------------------

// Fallback if GET_SCREEN_INFO fails (display server not yet ready on first poll).
const FALLBACK_W: u32 = 1280;
const FALLBACK_H: u32 = 720;

// Desktop-layer config flags (see app.rs WindowConfig docs).
// bits[1:0]=2 Desktop, bits[3:2]=3 Fullscreen, bit[4]=1 NoChrome → 0x1E
const DESKTOP_LAYER_FLAGS: u64 = 0x1E;

// ---------------------------------------------------------------------------
// Panel geometry constants
// ---------------------------------------------------------------------------

const RADIUS: u32 = 7;
const TOP_H: u32 = 36;        // top bar height
const TOP_Y: i32 = 6;         // top bar Y offset from screen top
const TOP_PAD: i32 = 8;       // horizontal margin from screen edge

const BOT_H: u32 = 44;        // bottom cluster height
const BOT_Y_OFF: i32 = 8;     // distance from screen bottom to bottom of cluster
const ICON_BTN: u32 = 36;     // square size for icon buttons in clusters
const CLUSTER_PAD: i32 = 6;   // inner horizontal padding inside clusters
const ICON_GAP: i32 = 4;      // gap between icon buttons

const SEARCH_W: u32 = 200;    // search box width
const SEARCH_H: u32 = 32;     // search box height

// ---------------------------------------------------------------------------
// Heap (bump allocator — no dynamic allocations used)
// ---------------------------------------------------------------------------

const HEAP_SIZE: usize = 64 * 1024;
#[repr(align(16))]
struct BumpHeap(core::cell::UnsafeCell<[u8; HEAP_SIZE]>);
unsafe impl Sync for BumpHeap {}
static BUMP_HEAP: BumpHeap = BumpHeap(core::cell::UnsafeCell::new([0u8; HEAP_SIZE]));

struct BumpAlloc;
unsafe impl core::alloc::GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        use core::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let heap_ptr = BUMP_HEAP.0.get() as *mut u8;
        let cur = NEXT.load(Ordering::Relaxed);
        let aligned = (cur + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        NEXT.store(end, Ordering::Relaxed);
        unsafe { heap_ptr.add(aligned) }
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: BumpAlloc = BumpAlloc;

// ---------------------------------------------------------------------------
// Pixel-art icon bitmaps (1 bit per pixel, u16 rows, MSB = leftmost pixel)
// Width is the number of significant bits; stored in the MSBs of each u16.
// All icons are 16×16 pixel fields scaled to fit an ICON_BTN×ICON_BTN cell.
// ---------------------------------------------------------------------------

/// Sun icon — filled circle + 8 short rays.
const SUN_ROWS: [u16; 16] = [
    0b0000001000000000,
    0b0001001010010000,
    0b0000100111001000,
    0b0001000111000100,
    0b0010011111110010,
    0b0000011111100000,
    0b1100011111100011,
    0b0000011111100000,
    0b0000011111100000,
    0b1100011111100011,
    0b0000011111100000,
    0b0010011111110010,
    0b0001000111000100,
    0b0000100111001000,
    0b0001001010010000,
    0b0000001000000000,
];

/// Overview icon — 2×2 grid of rounded squares.
const OVERVIEW_ROWS: [u16; 16] = [
    0b0111101111000000,
    0b0100101001000000,
    0b0100101001000000,
    0b0111101111000000,
    0b0000000000000000,
    0b0111101111000000,
    0b0100101001000000,
    0b0100101001000000,
    0b0111101111000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Sidebar icon — vertical bar on left + content area.
const SIDEBAR_ROWS: [u16; 16] = [
    0b1111111111111100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1111111111111100,
];

/// Settings icon — gear / cogwheel approximation.
const SETTINGS_ROWS: [u16; 16] = [
    0b0000011000000000,
    0b0001011010000000,
    0b0001111110000000,
    0b0011100011100000,
    0b0110100010110000,
    0b1101111111011000,
    0b1100111110011000,
    0b1100111110011000,
    0b1101111111011000,
    0b0110100010110000,
    0b0011100011100000,
    0b0001111110000000,
    0b0001011010000000,
    0b0000011000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Launcher grid icon — 3×3 dots.
const GRID_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0110001100011000,
    0b0110001100011000,
    0b0000000000000000,
    0b0000000000000000,
    0b0110001100011000,
    0b0110001100011000,
    0b0000000000000000,
    0b0000000000000000,
    0b0110001100011000,
    0b0110001100011000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Terminal icon — ">_" prompt shape.
const TERMINAL_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0000000000000000,
    0b1100000000000000,
    0b0110000000000000,
    0b0011000000000000,
    0b0110000000000000,
    0b1100000000000000,
    0b0000000000000000,
    0b0000111111100000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Tasks monitor icon — 3 horizontal bars (activity / list).
const TASKS_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0000000000000000,
    0b1111111111110000,
    0b1111111111110000,
    0b0000000000000000,
    0b1111111000000000,
    0b1111111000000000,
    0b0000000000000000,
    0b1111111111000000,
    0b1111111111000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Calculator icon — display + keypad grid.
const CALC_ROWS: [u16; 16] = [
    0b0111111111100000,
    0b0100000000100000,
    0b0100000000100000,
    0b0111111111100000,
    0b0000000000000000,
    0b0110011001100000,
    0b0110011001100000,
    0b0000000000000000,
    0b0110011001100000,
    0b0110011001100000,
    0b0000000000000000,
    0b0110011001100000,
    0b0110011001100000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

// Width of the 16-pixel icon bitmap (used in draw_icon16).
const ICON16_W: u32 = 16;

// ---------------------------------------------------------------------------
// Click zone bookkeeping
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum DockAction {
    None,
    LaunchCalc,
}

// ---------------------------------------------------------------------------
// Shell application state
// ---------------------------------------------------------------------------

struct VortexShell {
    wallpaper: Option<TgaImage>,
    /// Bounds of each clickable dock button (local coords), plus the action.
    dock_zones: [(Rect, DockAction); 3],
    /// Tracks whether mouse is hovering over a dock icon (index 0..3).
    hover: Option<usize>,
}

impl VortexShell {
    fn new() -> Self {
        let wallpaper = TgaImage::parse(WALLPAPER_TGA).ok();
        if wallpaper.is_some() {
            debug_log("[VORTEX] wallpaper loaded\n");
        } else {
            debug_log("[VORTEX] wallpaper unavailable — using fallback\n");
        }
        Self {
            wallpaper,
            dock_zones: [(Rect::new(0, 0, 0, 0), DockAction::None); 3],
            hover: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

/// Draw a 16×16 pixel-art icon scaled 1:1 centred inside `cell`.
fn draw_icon16(canvas: &mut Canvas, cell: Rect, rows: &[u16; 16], color: Color) {
    let ox = cell.x + (cell.w as i32 - ICON16_W as i32) / 2;
    let oy = cell.y + (cell.h as i32 - 16i32) / 2;
    for (row_idx, &row_bits) in rows.iter().enumerate() {
        for col in 0..ICON16_W as usize {
            let bit = (row_bits >> (ICON16_W as usize - 1 - col)) & 1;
            if bit != 0 {
                canvas.put_pixel(ox + col as i32, oy + row_idx as i32, color);
            }
        }
    }
}

/// Draw a panel pill: filled rounded rect with a 1-px border.
fn draw_panel(canvas: &mut Canvas, rect: Rect, fill: Color, border: Color) {
    canvas.fill_rounded_rect(rect, RADIUS, fill);
    canvas.stroke_rounded_rect(rect, RADIUS, 1, border);
}

/// Draw an icon button cell. `highlight` draws it with the accent tint.
fn draw_icon_btn(
    canvas: &mut Canvas,
    cell: Rect,
    rows: &[u16; 16],
    theme: &Theme,
    highlight: bool,
    hover: bool,
) {
    if hover {
        canvas.fill_rounded_rect(cell, 5, theme.panel_alt);
    }
    let icon_color = if highlight {
        theme.accent
    } else if hover {
        theme.text
    } else {
        theme.text_dim
    };
    draw_icon16(canvas, cell, rows, icon_color);
}

// ---------------------------------------------------------------------------
// Top bar layout
// ---------------------------------------------------------------------------

fn draw_top_bar(canvas: &mut Canvas, theme: &Theme, screen_w: u32) {
    let bar = Rect::new(TOP_PAD, TOP_Y, screen_w - (TOP_PAD * 2) as u32, TOP_H);
    draw_panel(canvas, bar, theme.panel, theme.border);

    // ── Left zone: sun + workspace dots ──────────────────────────────────────
    let left_x = bar.x + 10;

    // Sun icon (orange)
    let sun_cell = Rect::new(left_x, bar.y, 28, TOP_H);
    draw_icon16(canvas, sun_cell, &SUN_ROWS, theme.accent);

    // Three workspace indicator dots
    let dot_start_x = left_x + 34;
    let dot_cy = bar.y + bar.h as i32 / 2;
    for i in 0..3i32 {
        let dot_x = dot_start_x + i * 14;
        let dot_color = if i == 0 { theme.accent } else { theme.text_dim };
        // 6×6 filled rounded rect as a dot
        canvas.fill_rounded_rect(
            Rect::new(dot_x, dot_cy - 3, 6, 6),
            3,
            dot_color,
        );
    }

    // ── Center zone: current app title ───────────────────────────────────────
    let title = "SunlightOS";
    let title_w = Canvas::measure_text(title);
    let title_x = bar.x + (bar.w as i32 - title_w as i32) / 2;
    let title_y = bar.y + (bar.h as i32 - 7) / 2; // font height = 7
    canvas.draw_text(title_x, title_y, title, theme.text);

    // ── Right zone: status cluster placeholder ───────────────────────────────
    // Show static placeholders; real data wired in a later patch.
    let status = "NET  BAT  12:00";
    let status_w = Canvas::measure_text(status);
    let status_x = bar.right() - status_w as i32 - 14;
    let status_y = bar.y + (bar.h as i32 - 7) / 2;
    canvas.draw_text(status_x, status_y, status, theme.text_dim);

    // Small indicator dots for each status icon
    let dot_colors = [theme.ok, theme.warn, theme.text_dim];
    for (i, &dc) in dot_colors.iter().enumerate() {
        let dx = status_x - 12 - (dot_colors.len() as i32 - 1 - i as i32) * 10;
        canvas.fill_rounded_rect(Rect::new(dx, dot_cy - 3, 6, 6), 3, dc);
    }


}

// ---------------------------------------------------------------------------
// Bottom bar layout
// ---------------------------------------------------------------------------

/// Compute y coordinate of the top of the bottom clusters.
fn bot_y(screen_h: u32) -> i32 {
    screen_h as i32 - BOT_Y_OFF - BOT_H as i32
}

/// Draw the bottom-left cluster: overview | sidebar | settings.
fn draw_bot_left(canvas: &mut Canvas, theme: &Theme, by: i32) {
    let icons: &[(&[u16; 16], bool)] = &[
        (&OVERVIEW_ROWS, false),
        (&SIDEBAR_ROWS, false),
        (&SETTINGS_ROWS, false),
    ];
    let n = icons.len() as u32;
    let cluster_w = CLUSTER_PAD as u32 * 2 + n * ICON_BTN + (n - 1) * ICON_GAP as u32;
    let cluster = Rect::new(TOP_PAD, by, cluster_w, BOT_H);
    draw_panel(canvas, cluster, theme.panel, theme.border);

    let mut cx = cluster.x + CLUSTER_PAD;
    for (rows, _accent) in icons {
        let cell = Rect::new(cx, cluster.y + (BOT_H as i32 - ICON_BTN as i32) / 2, ICON_BTN, ICON_BTN);
        draw_icon_btn(canvas, cell, rows, theme, false, false);
        cx += ICON_BTN as i32 + ICON_GAP;
    }
}

/// Draw the bottom-center dock and return the three clickable zone rects
/// (terminal, tasks, calc).
fn draw_bot_center(canvas: &mut Canvas, theme: &Theme, by: i32, screen_w: u32, hover: Option<usize>) -> [Rect; 3] {
    let icons: &[&[u16; 16]; 4] = &[
        &GRID_ROWS,
        &TERMINAL_ROWS,
        &TASKS_ROWS,
        &CALC_ROWS,
    ];
    let n = icons.len() as u32;
    let cluster_w = CLUSTER_PAD as u32 * 2 + n * ICON_BTN + (n - 1) * ICON_GAP as u32;
    let cx_start = (screen_w as i32 - cluster_w as i32) / 2;
    let cluster = Rect::new(cx_start, by, cluster_w, BOT_H);
    draw_panel(canvas, cluster, theme.panel, theme.border);

    let mut x = cluster.x + CLUSTER_PAD;
    let mut clickable = [Rect::new(0, 0, 0, 0); 3];
    for (i, rows) in icons.iter().enumerate() {
        let cell = Rect::new(x, cluster.y + (BOT_H as i32 - ICON_BTN as i32) / 2, ICON_BTN, ICON_BTN);
        let is_hover = hover.map(|h| h == i.saturating_sub(1) && i > 0).unwrap_or(false);
        draw_icon_btn(canvas, cell, rows, theme, false, is_hover);
        // Icons 1,2,3 are clickable (terminal, tasks, calc); icon 0 (grid) is placeholder
        if i >= 1 {
            clickable[i - 1] = cell;
        }
        x += ICON_BTN as i32 + ICON_GAP;
    }
    clickable
}

/// Draw the bottom-right search box.
fn draw_bot_right(canvas: &mut Canvas, theme: &Theme, by: i32, screen_w: u32) {
    let sx = screen_w as i32 - TOP_PAD - SEARCH_W as i32;
    let sy = by + (BOT_H as i32 - SEARCH_H as i32) / 2;
    let search_rect = Rect::new(sx, sy, SEARCH_W, SEARCH_H);
    draw_panel(canvas, search_rect, theme.panel_alt, theme.border);

    // Placeholder text
    let ph = "Search...";
    let ph_w = Canvas::measure_text(ph);
    let ph_x = search_rect.x + (search_rect.w as i32 - ph_w as i32) / 2;
    let ph_y = search_rect.y + (search_rect.h as i32 - 7) / 2;
    canvas.draw_text(ph_x, ph_y, ph, theme.text_dim);
}

// ---------------------------------------------------------------------------
// App impl
// ---------------------------------------------------------------------------

impl App for VortexShell {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let cw = canvas.width;
        let ch = canvas.height;

        // ── Wallpaper ────────────────────────────────────────────────────────
        if let Some(ref wp) = self.wallpaper {
            canvas.draw_image_cover(wp);
        } else {
            canvas.fill_rect(Rect::new(0, 0, cw, ch), Color(FALLBACK_BG));
        }

        // ── Top bar ──────────────────────────────────────────────────────────
        draw_top_bar(canvas, theme, cw);

        // ── Bottom panels ────────────────────────────────────────────────────
        let by = bot_y(ch);
        draw_bot_left(canvas, theme, by);
        let dock_cells = draw_bot_center(canvas, theme, by, cw, self.hover);
        draw_bot_right(canvas, theme, by, cw);

        // Record clickable zones (terminal, tasks, calc).
        self.dock_zones = [
            (dock_cells[0], DockAction::None),       // terminal — TODO(phase2-launch)
            (dock_cells[1], DockAction::None),       // tasks    — TODO(phase2-launch)
            (dock_cells[2], DockAction::LaunchCalc),
        ];
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                for (rect, action) in &self.dock_zones {
                    if rect.contains(sunlight_ui::geom::Point::new(x, y)) {
                        spawn_app(*action);
                        return false;
                    }
                }
                false
            }
            Event::MouseMove { x, y } => {
                let prev = self.hover;
                self.hover = None;
                for (i, (rect, _)) in self.dock_zones.iter().enumerate() {
                    if rect.contains(sunlight_ui::geom::Point::new(x, y)) {
                        self.hover = Some(i);
                        break;
                    }
                }
                self.hover != prev
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// App launch
// ---------------------------------------------------------------------------

fn spawn_app(action: DockAction) {
    let path = match action {
        DockAction::LaunchCalc => "/bin/calc",
        DockAction::None => return,
    };
    let init_cap = get_init_cap();
    let req = SpawnRequest::new(path, "");
    let mut msg = IpcMsg::with_label(0);
    req.pack_into(&mut msg);
    let _ = ipc_call(init_cap, msg);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[VORTEX] starting\n");

    let mut shell = VortexShell::new();

    // Resolve display_server endpoint (spin until ready).
    let display_ep = loop {
        if let Some(ep) = nameserver_lookup("display_server") {
            break ep;
        }
        process_yield();
    };

    // Query physical framebuffer dimensions before allocating the SHM window.
    // This ensures the shell canvas matches the actual screen, not the image size.
    let packed = ipc_call(display_ep, IpcMsg::with_label(SgpMsg::GET_SCREEN_INFO));
    let (screen_w, screen_h) = if packed.label == SgpMsg::REPLY && packed.words[0] != 0 {
        let w = (packed.words[0] & 0xFFFF_FFFF) as u32;
        let h = (packed.words[0] >> 32) as u32;
        (w.max(320), h.max(240)) // clamp to sanity minimums
    } else {
        debug_log("[VORTEX] GET_SCREEN_INFO failed, using fallback resolution\n");
        (FALLBACK_W, FALLBACK_H)
    };

    debug_log("[VORTEX] screen ");
    debug_log_u32(screen_w);
    debug_log("x");
    debug_log_u32(screen_h);
    debug_log("\n");

    // Create window at the exact physical screen size.
    // The SHM buffer will match canvas.width/height so panel positions are correct.
    let mut window = loop {
        match Window::connect(WindowConfig {
            width: screen_w,
            height: screen_h,
            title: "Vortex Shell",
        }) {
            Some(w) => break w,
            None => process_yield(),
        }
    };

    window.configure_flags(DESKTOP_LAYER_FLAGS);
    debug_log("[VORTEX] desktop layer registered, entering event loop\n");

    window.run(&mut shell);
    ProcessExit::exit(0);
}

/// Minimal decimal logger for u32 (avoids pulling in format!/alloc).
fn debug_log_u32(mut n: u32) {
    let mut buf = [0u8; 10];
    let mut len = 0usize;
    if n == 0 {
        debug_log("0");
        return;
    }
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    // buf is reversed — print digits in correct order
    let mut s = [0u8; 11];
    for i in 0..len {
        s[i] = buf[len - 1 - i];
    }
    if let Ok(text) = core::str::from_utf8(&s[..len]) {
        debug_log(text);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[VORTEX] panic\n");
    loop {
        process_yield();
    }
}
