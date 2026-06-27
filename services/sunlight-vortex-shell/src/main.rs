//! Vortex Shell — SunlightOS desktop surface (Phase 1 skeleton).
//!
//! Renders the system wallpaper as a fullscreen Desktop-layer window that sits
//! permanently behind all normal application windows.  No panels, icons, or
//! transparency yet — those are Phase 2 tasks (see TODOs below).

#![no_std]
#![no_main]

use sunlight_ipc::{debug_log, process_yield, ProcessExit};
use sunlight_ui::{
    image::TgaImage, App, Canvas, Color, Event, Rect, Theme, Window, WindowConfig,
};

// ---------------------------------------------------------------------------
// Wallpaper asset
// ---------------------------------------------------------------------------

// Embedded at compile time from the same asset the compositor uses.
// TODO(phase2-wallpaper): load /var/sunlightos/wallpapers/wallpaper.tga via
// VFS at runtime so the user can swap wallpapers without recompiling.
static WALLPAPER_TGA: &[u8] = include_bytes!("../../../docs/images/wallpaper.tga");

// Dark SunlightOS fallback used when the TGA is absent or fails to parse.
const FALLBACK_BG: u32 = 0x00121214;

// ---------------------------------------------------------------------------
// Window geometry
// ---------------------------------------------------------------------------

// Phase 1 uses a fixed request size.  The compositor clips the blit to the
// actual framebuffer width, so this safely covers displays up to 1920×1080.
//
// TODO(phase2-resolution): add a GET_SCREEN_INFO SGP call so the shell can
// allocate its SHM buffer at the exact framebuffer dimensions and avoid any
// partial-row artefacts on >1920-wide screens.
const SHELL_W: u32 = 1920;
const SHELL_H: u32 = 1080;

// ---------------------------------------------------------------------------
// Desktop-layer config flags (sent via CONFIGURE_WINDOW after creation)
// ---------------------------------------------------------------------------
//
// Bit layout (see ipc/src/lib.rs SgpMsg config_flags):
//   [1:0] = 0b10  → WindowType::Desktop   (never raised on click)
//   [3:2] = 0b11  → WindowState::Fullscreen (compositor maps to fb_w × fb_h)
//   [4]   = 1     → BorderStyle::None     (no title bar / chrome)
//   [5]   = 0     → ZIndexType::Normal    (behind OnTop windows)
//
// Result: 0b00011110 = 0x1E = 30
const DESKTOP_LAYER_FLAGS: u64 = 0x1E;

// ---------------------------------------------------------------------------
// Minimal bump allocator (required by no_std + no_main linker setup).
// The shell itself makes no heap allocations today.
// ---------------------------------------------------------------------------

const HEAP_SIZE: usize = 32 * 1024;
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
// Shell application state
// ---------------------------------------------------------------------------

struct VortexShell {
    wallpaper: Option<TgaImage>,
}

impl VortexShell {
    fn new() -> Self {
        let wallpaper = TgaImage::parse(WALLPAPER_TGA).ok();
        if wallpaper.is_some() {
            debug_log("[VORTEX] wallpaper loaded\n");
        } else {
            debug_log("[VORTEX] wallpaper unavailable — using fallback\n");
        }
        Self { wallpaper }
    }
}

impl App for VortexShell {
    fn view(&mut self, canvas: &mut Canvas, _theme: &Theme) {
        let cw = canvas.width;
        let ch = canvas.height;

        if let Some(wp) = self.wallpaper {
            // Bilinear-style nearest-neighbour stretch to cover the canvas.
            // Safe: TgaImage::pixel_xrgb clamps out-of-bounds coords internally.
            let iw = wp.width;
            let ih = wp.height;
            for y in 0..ch {
                let src_y = y * ih / ch;
                for x in 0..cw {
                    let src_x = x * iw / cw;
                    canvas.put_pixel(x as i32, y as i32, Color(wp.pixel_xrgb(src_x, src_y)));
                }
            }
        } else {
            // Solid dark fallback matching the compositor's DESKTOP_COLOR.
            canvas.fill_rect(Rect::new(0, 0, cw, ch), Color(FALLBACK_BG));
        }

        // TODO(phase2-panel):   draw a top status bar (clock, power, network).
        // TODO(phase2-dock):    draw a bottom application dock.
        // TODO(phase2-icons):   draw desktop icon grid over the wallpaper.
    }

    fn update(&mut self, _event: Event) -> bool {
        // The desktop surface is static for now — no redraws on input events.
        //
        // TODO(phase2-panel):   return true on Tick events to update the clock.
        // TODO(phase2-context): return true on right-click to show context menu.
        // TODO(phase2-wallpaper): return true on a wallpaper-change IPC signal.
        false
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[VORTEX] starting\n");

    let mut shell = VortexShell::new();

    // Retry until the display server is up.
    let mut window = loop {
        match Window::connect(WindowConfig {
            width: SHELL_W,
            height: SHELL_H,
            title: "Vortex Shell",
        }) {
            Some(w) => break w,
            None => process_yield(),
        }
    };

    // Promote to the desktop layer: fullscreen + no chrome + Desktop type.
    // This call sends CONFIGURE_WINDOW after CREATE_WINDOW so the compositor
    // repositions the window to (0, 0) covering the full framebuffer.
    window.configure_flags(DESKTOP_LAYER_FLAGS);

    debug_log("[VORTEX] desktop layer registered, entering event loop\n");

    // Run the event loop.  update() always returns false (static wallpaper),
    // so the loop idles at the 200 ms IPC poll timeout between redraws.
    window.run(&mut shell);

    ProcessExit::exit(0);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[VORTEX] panic\n");
    loop {
        process_yield();
    }
}
