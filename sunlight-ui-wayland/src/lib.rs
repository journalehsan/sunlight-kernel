//! Canvas renderer for the bounded Helios libwayland client.

#![no_std]

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::panic::PanicInfo;
use sunlight_ui::{Canvas, Color, Event, Point, Rect};

extern "C" {
    fn posix_memalign(ptr: *mut *mut c_void, alignment: usize, size: usize) -> i32;
    fn free(ptr: *mut c_void);
    fn abort() -> !;
}

struct MuslAllocator;

unsafe impl GlobalAlloc for MuslAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut ptr = core::ptr::null_mut();
        let alignment = layout.align().max(core::mem::size_of::<*mut c_void>());
        if unsafe { posix_memalign(&mut ptr, alignment, layout.size()) } == 0 {
            ptr.cast()
        } else {
            core::ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _: Layout) {
        unsafe { free(ptr.cast()) }
    }
}

#[global_allocator]
static ALLOCATOR: MuslAllocator = MuslAllocator;

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    unsafe { abort() }
}

const WIDTH: u32 = 256;
const HEIGHT: u32 = 160;
const BUTTON: Rect = Rect::new(24, 96, 104, 32);

/// Translate an already focused, serial-validated Wayland button event.
#[no_mangle]
pub extern "C" fn sunlight_wayland_button(
    x_fixed: i32,
    y_fixed: i32,
    code: u32,
    state: u32,
    counter: &mut u32,
) -> bool {
    if code != 0x110
        || state != 1
        || !(0..((WIDTH as i32) << 8)).contains(&x_fixed)
        || !(0..((HEIGHT as i32) << 8)).contains(&y_fixed)
    {
        return false;
    }
    update(
        Event::MouseDown {
            x: x_fixed >> 8,
            y: y_fixed >> 8,
            button: 0,
        },
        counter,
    )
}

fn update(event: Event, counter: &mut u32) -> bool {
    match event {
        Event::MouseDown { x, y, button: 0 }
            if BUTTON.contains(Point::new(x, y)) && *counter == 0 =>
        {
            *counter += 1;
            true
        }
        _ => false,
    }
}

/// Paint an opaque frame and return the bridge's checksum of its pixel words.
///
/// # Safety
///
/// The client must supply an aligned, writable mapping of exactly
/// `stride * height` u32 pixels, with exclusive access until this call returns.
#[no_mangle]
pub unsafe extern "C" fn sunlight_wayland_paint(
    pixels: *mut u32,
    width: u32,
    height: u32,
    stride: u32,
    frame: u32,
    checksum: &mut u32,
) -> bool {
    if (width, height, stride) != (WIDTH, HEIGHT, WIDTH)
        || frame > 3
        || pixels.is_null()
        || !pixels.is_aligned()
    {
        return false;
    }
    let pixels = unsafe { core::slice::from_raw_parts_mut(pixels, (stride * height) as usize) };
    let mut canvas = Canvas::new(pixels, stride, width, height);
    let (background, accent, caption, progress) = if frame != 1 {
        (
            Color::rgb(24, 45, 75),
            Color::rgb(40, 220, 255),
            "FRAME 1 / PAINTING",
            72,
        )
    } else {
        (
            Color::rgb(68, 35, 24),
            Color::rgb(255, 203, 50),
            "FRAME 2 / COMPLETE",
            192,
        )
    };
    canvas.fill_rect(Rect::new(0, 0, width, height), background);
    canvas.fill_rect(Rect::new(24, 32, 208, 4), accent);
    canvas.draw_text(24, 48, "SUNLIGHT UI / WAYLAND", Color::rgb(255, 255, 255));
    if frame >= 2 {
        let pressed = frame == 3;
        let button = if pressed {
            Color::rgb(255, 203, 50)
        } else {
            accent
        };
        canvas.fill_rect(BUTTON, button);
        canvas.draw_text(36, 104, "CLICK", Color::rgb(16, 20, 28));
        canvas.draw_text(
            160,
            104,
            if pressed { "COUNT 1" } else { "COUNT 0" },
            button,
        );
    } else {
        canvas.draw_text(24, 76, caption, accent);
        canvas.fill_rect(Rect::new(24, 112, 208, 12), Color::rgb(16, 20, 28));
        canvas.fill_rect(Rect::new(24, 112, progress, 12), accent);
    }

    let expected = if frame != 1 {
        [255, 220, 40, 255]
    } else {
        [50, 203, 255, 255]
    };
    if canvas.pixels[(32 * stride + 24) as usize].to_ne_bytes() != expected
        || canvas.pixels.iter().any(|pixel| pixel >> 24 != 255)
    {
        return false;
    }
    *checksum = canvas.pixels.iter().fold(2166136261u32, |hash, pixel| {
        (hash ^ pixel).wrapping_mul(16777619)
    });
    true
}
