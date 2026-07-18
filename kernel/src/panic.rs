use core::{
    fmt::{self, Write},
    panic::PanicInfo,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

const ORANGE: Rgb = Rgb::new(0xff, 0x45, 0x00);
const WHITE: Rgb = Rgb::new(0xff, 0xff, 0xff);
const DARK_ORANGE: Rgb = Rgb::new(0x7f, 0x22, 0x00);

const GLYPH_WIDTH: usize = 8;
const GLYPH_HEIGHT: usize = 16;
const GLYPH_BYTES: usize = GLYPH_HEIGHT;
const FIRST_GLYPH: u8 = 0x20;
const LAST_GLYPH: u8 = 0x7e;
const FONT_BYTES: usize = (LAST_GLYPH - FIRST_GLYPH + 1) as usize * GLYPH_BYTES;

// A fixed bitmap is preferable to minitype here: it has no allocator,
// initialization, cache, or synchronization dependency in the panic path.
static FONT: [u8; FONT_BYTES] = *include_bytes!("../../sunlight-tui/src/font8x16.bin");

static PANIC_RENDERING: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    disable_interrupts();

    if claim_panic_renderer() {
        if let Some(mut writer) = panic_framebuffer() {
            render_background(&mut writer);
            render_header(&mut writer);

            writer.set_scale(1);
            writer.set_color(WHITE);
            let _ = writer.write_str("The kernel encountered a fatal error.\n\n");
            let _ = writer.write_str("Panic: ");
            let _ = write!(writer, "{}", info.message());
            let _ = writer.write_str("\n\n");

            if let Some(location) = info.location() {
                let _ = write!(
                    writer,
                    "Location: {}:{}:{}\n",
                    location.file(),
                    location.line(),
                    location.column()
                );
            } else {
                let _ = writer.write_str("Location: unavailable\n");
            }

            let _ = writer.write_str("\nInterrupts are disabled. The system has halted.");
        }
    }

    halt_forever()
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    disable_interrupts();

    if claim_panic_renderer() {
        if let Some(mut writer) = panic_framebuffer() {
            render_background(&mut writer);
            render_header(&mut writer);

            writer.set_scale(1);
            writer.set_color(WHITE);
            let _ = writer.write_str("The kernel ran out of memory.\n\n");
            let _ = write!(
                writer,
                "Allocation request: {} bytes, alignment {}\n\n",
                layout.size(),
                layout.align()
            );
            let _ = writer.write_str("Interrupts are disabled. The system has halted.");
        }
    }

    halt_forever()
}

#[inline]
fn claim_panic_renderer() -> bool {
    PANIC_RENDERING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn render_background(writer: &mut FramebufferWriter) {
    writer.clear(ORANGE);

    let border = writer.scale_dimension(6);
    writer.fill_rect(0, 0, writer.width, border, DARK_ORANGE);
    writer.fill_rect(
        0,
        writer.height.saturating_sub(border),
        writer.width,
        border,
        DARK_ORANGE,
    );
}

fn render_header(writer: &mut FramebufferWriter) {
    writer.set_color(WHITE);
    writer.set_scale(2);
    let _ = writer.write_str("SunlightOS\n");
    let _ = writer.write_str("Orange Screen of Death\n\n");
}

fn panic_framebuffer() -> Option<FramebufferWriter> {
    // This deliberately bypasses every console/framebuffer lock. Limine's
    // response and framebuffer mapping are immutable after boot; the panic
    // renderer is the sole writer after PANIC_RENDERING is claimed.
    let response = crate::FB_REQ.response()?;
    let framebuffer = response.framebuffers().first()?;

    let address = framebuffer.address().cast::<u8>();
    let width = usize::try_from(framebuffer.width).ok()?;
    let height = usize::try_from(framebuffer.height).ok()?;
    let pitch = usize::try_from(framebuffer.pitch).ok()?;
    let bytes_per_pixel = usize::from(framebuffer.bpp.div_ceil(8));
    let row_bytes = width.checked_mul(bytes_per_pixel)?;
    let framebuffer_bytes = height.checked_mul(pitch)?;

    if address.is_null()
        || width == 0
        || height == 0
        || !(2..=4).contains(&bytes_per_pixel)
        || pitch < row_bytes
        || framebuffer_bytes > isize::MAX as usize
    {
        return None;
    }

    let format = PixelFormat {
        bytes_per_pixel,
        red_size: framebuffer.red_mask_size,
        red_shift: framebuffer.red_mask_shift,
        green_size: framebuffer.green_mask_size,
        green_shift: framebuffer.green_mask_shift,
        blue_size: framebuffer.blue_mask_size,
        blue_shift: framebuffer.blue_mask_shift,
    };

    if framebuffer.memory_model != limine::framebuffer::FRAMEBUFFER_RGB || !format.is_valid() {
        return None;
    }

    // SAFETY: the address and geometry come from Limine's immutable response.
    // Pixel accesses remain within height * pitch and use volatile writes.
    Some(unsafe { FramebufferWriter::new(address, width, height, pitch, format) })
}

#[inline]
fn disable_interrupts() {
    x86_64::instructions::interrupts::disable();
}

fn halt_forever() -> ! {
    disable_interrupts();
    loop {
        x86_64::instructions::hlt();
    }
}

#[derive(Clone, Copy)]
struct Rgb {
    red: u8,
    green: u8,
    blue: u8,
}

impl Rgb {
    const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

#[derive(Clone, Copy)]
struct PixelFormat {
    bytes_per_pixel: usize,
    red_size: u8,
    red_shift: u8,
    green_size: u8,
    green_shift: u8,
    blue_size: u8,
    blue_shift: u8,
}

impl PixelFormat {
    fn is_valid(self) -> bool {
        let bits = self.bytes_per_pixel * 8;
        (1..=8).contains(&self.red_size)
            && (1..=8).contains(&self.green_size)
            && (1..=8).contains(&self.blue_size)
            && usize::from(self.red_size) + usize::from(self.red_shift) <= bits
            && usize::from(self.green_size) + usize::from(self.green_shift) <= bits
            && usize::from(self.blue_size) + usize::from(self.blue_shift) <= bits
            && self.channel_masks_do_not_overlap()
    }

    fn channel_masks_do_not_overlap(self) -> bool {
        let red = channel_mask(self.red_size, self.red_shift);
        let green = channel_mask(self.green_size, self.green_shift);
        let blue = channel_mask(self.blue_size, self.blue_shift);
        red & green == 0 && red & blue == 0 && green & blue == 0
    }

    fn encode(self, color: Rgb) -> u32 {
        scale_channel(color.red, self.red_size) << self.red_shift
            | scale_channel(color.green, self.green_size) << self.green_shift
            | scale_channel(color.blue, self.blue_size) << self.blue_shift
    }
}

#[inline]
fn channel_mask(size: u8, shift: u8) -> u32 {
    ((1u32 << size) - 1) << shift
}

#[inline]
fn scale_channel(channel: u8, size: u8) -> u32 {
    let max = (1u32 << size) - 1;
    (u32::from(channel) * max + 127) / 255
}

struct FramebufferWriter {
    address: *mut u8,
    width: usize,
    height: usize,
    pitch: usize,
    format: PixelFormat,
    cursor_x: usize,
    cursor_y: usize,
    margin_x: usize,
    margin_y: usize,
    scale: usize,
    color: Rgb,
}

impl FramebufferWriter {
    unsafe fn new(
        address: *mut u8,
        width: usize,
        height: usize,
        pitch: usize,
        format: PixelFormat,
    ) -> Self {
        let margin_x = width.min(32);
        let margin_y = height.min(32);
        Self {
            address,
            width,
            height,
            pitch,
            format,
            cursor_x: margin_x,
            cursor_y: margin_y,
            margin_x,
            margin_y,
            scale: 1,
            color: WHITE,
        }
    }

    fn set_color(&mut self, color: Rgb) {
        self.color = color;
    }

    fn set_scale(&mut self, scale: usize) {
        self.scale = scale.max(1);
    }

    fn scale_dimension(&self, value: usize) -> usize {
        value.saturating_mul(self.scale)
    }

    fn clear(&mut self, color: Rgb) {
        self.fill_rect(0, 0, self.width, self.height, color);
        self.cursor_x = self.margin_x;
        self.cursor_y = self.margin_y;
    }

    fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: Rgb) {
        let x_end = x.saturating_add(width).min(self.width);
        let y_end = y.saturating_add(height).min(self.height);
        let pixel = self.format.encode(color);

        for row in y..y_end {
            for column in x..x_end {
                self.write_encoded_pixel(column, row, pixel);
            }
        }
    }

    fn plot_pixel(&mut self, x: usize, y: usize, color: Rgb) {
        if x < self.width && y < self.height {
            self.write_encoded_pixel(x, y, self.format.encode(color));
        }
    }

    fn write_encoded_pixel(&mut self, x: usize, y: usize, pixel: u32) {
        let offset = y * self.pitch + x * self.format.bytes_per_pixel;

        // SAFETY: construction validates framebuffer geometry, and callers
        // clip x/y to its bounds. Volatile byte writes support 16/24/32-bpp
        // packed RGB layouts without alignment assumptions.
        unsafe {
            for byte in 0..self.format.bytes_per_pixel {
                ptr::write_volatile(self.address.add(offset + byte), (pixel >> (byte * 8)) as u8);
            }
        }
    }

    fn draw_char(&mut self, character: char) {
        let ascii = if character.is_ascii() {
            character as u8
        } else {
            b'?'
        };
        let ascii = if (FIRST_GLYPH..=LAST_GLYPH).contains(&ascii) {
            ascii
        } else {
            b'?'
        };
        let glyph_start = usize::from(ascii - FIRST_GLYPH) * GLYPH_BYTES;

        for row in 0..GLYPH_HEIGHT {
            let bits = FONT[glyph_start + row];
            for column in 0..GLYPH_WIDTH {
                if bits & (0x80 >> column) == 0 {
                    continue;
                }
                let pixel_x = self.cursor_x + column * self.scale;
                let pixel_y = self.cursor_y + row * self.scale;
                for scale_y in 0..self.scale {
                    for scale_x in 0..self.scale {
                        self.plot_pixel(pixel_x + scale_x, pixel_y + scale_y, self.color);
                    }
                }
            }
        }
    }

    fn newline(&mut self) {
        self.cursor_x = self.margin_x;
        self.cursor_y = self
            .cursor_y
            .saturating_add(GLYPH_HEIGHT.saturating_mul(self.scale) + self.scale);
    }

    fn write_display_char(&mut self, character: char) {
        match character {
            '\n' => {
                self.newline();
                return;
            }
            '\r' => {
                self.cursor_x = self.margin_x;
                return;
            }
            '\t' => {
                for _ in 0..4 {
                    self.write_display_char(' ');
                }
                return;
            }
            _ => {}
        }

        let glyph_width = GLYPH_WIDTH.saturating_mul(self.scale);
        if self.cursor_x.saturating_add(glyph_width) > self.width.saturating_sub(self.margin_x) {
            self.newline();
        }
        if self
            .cursor_y
            .saturating_add(GLYPH_HEIGHT.saturating_mul(self.scale))
            > self.height.saturating_sub(self.margin_y)
        {
            return;
        }

        self.draw_char(character);
        self.cursor_x = self
            .cursor_x
            .saturating_add(glyph_width.saturating_add(self.scale));
    }
}

impl Write for FramebufferWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for character in text.chars() {
            self.write_display_char(character);
        }
        Ok(())
    }
}
