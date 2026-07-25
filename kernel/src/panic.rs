use core::{
    fmt::{self, Write},
    panic::PanicInfo,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

const ORANGE: Rgb = Rgb::new(0xcd, 0x4c, 0x13);
const WHITE: Rgb = Rgb::new(0xff, 0xff, 0xff);
const BLACK: Rgb = Rgb::new(0x00, 0x00, 0x00);
const DARK_ORANGE: Rgb = Rgb::new(0x99, 0x38, 0x0e);

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
    // Never take blocking locks, never allocate, never enable interrupts.
    // Paint the framebuffer first so a stuck serial mutex cannot blank the
    // orange screen of death.
    disable_interrupts();

    if claim_panic_renderer() {
        if let Some(mut writer) = panic_framebuffer() {
            // Fill orange immediately so a later re-panic still leaves a
            // visible fatal screen even if message/QR formatting fails.
            render_background(&mut writer);
            render_sad_face(&mut writer);

            writer.set_scale(2);
            writer.set_color(WHITE);
            let _ = writer.write_str(
                "Your SunlightOS system ran into a problem and needs to restart.\n",
            );
            let _ = writer.write_str(
                "We're just collecting error info, and then system will halt.\n\n",
            );

            writer.set_scale(1);
            let _ = writer.write_str("Panic: ");
            // Ignore format errors; FramebufferWriter always returns Ok.
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

            render_qr_and_support_info(&mut writer);
        }
    }

    // Serial is best-effort and must never block: if SERIAL is held (including
    // by this CPU while formatting a previous serial_println!), drop the log
    // rather than hang before/after the screen is shown.
    crate::serial_println_try!("\n[KERNEL PANIC] {}", info);

    halt_forever()
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    disable_interrupts();

    if claim_panic_renderer() {
        if let Some(mut writer) = panic_framebuffer() {
            render_background(&mut writer);
            render_sad_face(&mut writer);

            writer.set_scale(2);
            writer.set_color(WHITE);
            let _ = writer.write_str("Your SunlightOS system ran out of memory.\n");
            let _ = writer.write_str("System will halt to prevent data corruption.\n\n");

            writer.set_scale(1);
            let _ = write!(
                writer,
                "Allocation request: {} bytes, alignment {}\n\n",
                layout.size(),
                layout.align()
            );

            render_qr_and_support_info(&mut writer);
        }
    }

    crate::serial_println_try!(
        "\n[KERNEL PANIC] Out of memory (layout size={} align={})",
        layout.size(),
        layout.align()
    );

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

fn render_sad_face(writer: &mut FramebufferWriter) {
    writer.set_color(WHITE);
    writer.set_scale(8);
    let _ = writer.write_str(":(\n\n");
}

fn render_qr_and_support_info(writer: &mut FramebufferWriter) {
    // Stack-only QR; never allocates. If the frame is too short, skip the QR
    // rather than painting over the panic message at the top of the screen.
    let module_size = 4;
    let quiet_modules = 2;
    let qr_size = (25 + quiet_modules * 2) * module_size;
    let bottom = writer.margin_y.max(20);
    let needed = bottom.saturating_add(qr_size);
    if writer.height < needed.saturating_add(writer.cursor_y.saturating_add(8)) {
        // Still show support URL/text so the user has something actionable.
        writer.set_scale(1);
        writer.set_color(WHITE);
        let _ = writer.write_str("\nFor more info: https://sunlightos.org/stopcode\n");
        let _ = writer.write_str("Stop code: KERNEL_PANIC\n");
        return;
    }

    let qr = QrCode25::new("https://sunlightos.org/stopcode");
    let qr_x = writer.margin_x;
    let qr_y = writer.height.saturating_sub(needed);

    // Render crisp white background block for QR code
    writer.fill_rect(qr_x, qr_y, qr_size, qr_size, WHITE);

    // Draw dark modules inside white block
    for row in 0..25 {
        for col in 0..25 {
            if qr.is_dark(row, col) {
                let mx = qr_x + (col + quiet_modules) * module_size;
                let my = qr_y + (row + quiet_modules) * module_size;
                writer.fill_rect(mx, my, module_size, module_size, BLACK);
            }
        }
    }

    // Support info text next to QR code (clip if the frame is too narrow).
    let text_x = qr_x.saturating_add(qr_size).saturating_add(24);
    let text_y = qr_y.saturating_add(4);
    if text_x >= writer.width.saturating_sub(writer.margin_x) {
        return;
    }

    writer.set_cursor(text_x, text_y);
    writer.set_scale(1);
    writer.set_color(WHITE);
    let _ = writer.write_str("For more information about this issue and possible fixes, visit:\n");

    writer.set_cursor(text_x, writer.cursor_y);
    let _ = writer.write_str("https://sunlightos.org/stopcode\n\n");

    writer.set_cursor(text_x, writer.cursor_y);
    let _ = writer.write_str("If you call a support person, give them this info:\n");

    writer.set_cursor(text_x, writer.cursor_y);
    let _ = writer.write_str("Stop code: KERNEL_PANIC\n");
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
        let margin_x = width.min(48);
        let margin_y = height.min(48);
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

    fn set_cursor(&mut self, x: usize, y: usize) {
        self.cursor_x = x;
        self.cursor_y = y;
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

// -----------------------------------------------------------------------------
// QR Code (Version 2, 25x25) Generator for no_std
// -----------------------------------------------------------------------------

struct BitWriter<'a> {
    buf: &'a mut [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    fn write(&mut self, val: u32, bits: u8) {
        for i in (0..bits).rev() {
            let bit = ((val >> i) & 1) as u8;
            if self.byte_pos < self.buf.len() {
                self.buf[self.byte_pos] |= bit << (7 - self.bit_pos);
                self.bit_pos += 1;
                if self.bit_pos == 8 {
                    self.bit_pos = 0;
                    self.byte_pos += 1;
                }
            }
        }
    }

    fn write_byte(&mut self, byte: u8) {
        if self.byte_pos < self.buf.len() {
            self.buf[self.byte_pos] = byte;
            self.byte_pos += 1;
            self.bit_pos = 0;
        }
    }
}

const fn gf_tables() -> ([u8; 256], [u8; 256]) {
    let mut exp = [0u8; 256];
    let mut log = [0u8; 256];
    let mut x = 1u16;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11d;
        }
        i += 1;
    }
    exp[255] = exp[0];
    (exp, log)
}

static GF_EXP_LOG: ([u8; 256], [u8; 256]) = gf_tables();

struct QrCode25 {
    modules: [[bool; 25]; 25],
}

impl QrCode25 {
    fn new(url: &str) -> Self {
        let mut modules = [[false; 25]; 25];
        let mut is_func = [[false; 25]; 25];

        // Finder patterns
        Self::place_finder(&mut modules, &mut is_func, 0, 0);
        Self::place_finder(&mut modules, &mut is_func, 0, 18);
        Self::place_finder(&mut modules, &mut is_func, 18, 0);

        // Alignment pattern
        Self::place_alignment(&mut modules, &mut is_func, 16, 16);

        // Timing patterns
        for i in 8..17 {
            modules[6][i] = i % 2 == 0;
            is_func[6][i] = true;
            modules[i][6] = i % 2 == 0;
            is_func[i][6] = true;
        }

        // Dark module
        modules[17][8] = true;
        is_func[17][8] = true;

        // Reserve format info modules
        for i in 0..9 {
            if i != 6 {
                is_func[8][i] = true;
                is_func[i][8] = true;
            }
        }
        for i in 17..25 {
            is_func[8][i] = true;
            is_func[i][8] = true;
        }

        // Construct bitstream
        let mut codewords = [0u8; 44];
        let bytes = url.as_bytes();
        let len = bytes.len().min(31);

        let mut bit_buf = BitWriter::new(&mut codewords[..34]);
        bit_buf.write(0b0100, 4); // Byte mode
        bit_buf.write(len as u32, 8); // Length
        for &b in &bytes[..len] {
            bit_buf.write(b as u32, 8);
        }
        bit_buf.write(0, 4); // Terminator

        let mut pad = 0xEC;
        while bit_buf.byte_pos < 34 {
            bit_buf.write_byte(pad);
            pad = if pad == 0xEC { 0x11 } else { 0xEC };
        }

        // Compute 10 RS EC codewords
        let (exp, log) = &GF_EXP_LOG;
        let g = [45, 32, 139, 89, 78, 125, 119, 237, 215, 80];
        let mut ec = [0u8; 10];
        for i in 0..34 {
            let feedback = codewords[i] ^ ec[0];
            for j in 0..9 {
                let term = if feedback == 0 || g[j] == 0 {
                    0
                } else {
                    exp[(log[g[j] as usize] as usize + log[feedback as usize] as usize) % 255]
                };
                ec[j] = ec[j + 1] ^ term;
            }
            let term = if feedback == 0 || g[9] == 0 {
                0
            } else {
                exp[(log[g[9] as usize] as usize + log[feedback as usize] as usize) % 255]
            };
            ec[9] = term;
        }
        for i in 0..10 {
            codewords[34 + i] = ec[i];
        }

        // Place bits in zig-zag order with Mask 0
        let mut bit_idx = 0;
        let total_bits = 44 * 8;
        let mut col = 24i32;
        let mut up = true;

        while col > 0 {
            if col == 6 {
                col -= 1;
            }
            let rows: [usize; 25] = if up {
                [24, 23, 22, 21, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
            } else {
                [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24]
            };

            for &r in &rows {
                for &c in &[col as usize, (col - 1) as usize] {
                    if !is_func[r][c] {
                        let bit = if bit_idx < total_bits {
                            let byte_i = bit_idx / 8;
                            let bit_i = 7 - (bit_idx % 8);
                            ((codewords[byte_i] >> bit_i) & 1) != 0
                        } else {
                            false
                        };
                        bit_idx += 1;
                        let mask = (r + c) % 2 == 0;
                        modules[r][c] = bit ^ mask;
                    }
                }
            }
            col -= 2;
            up = !up;
        }

        // Format Info (L level, Mask 0 = 0x77C4)
        Self::place_format_info(&mut modules, 0x77C4);

        Self { modules }
    }

    fn place_finder(modules: &mut [[bool; 25]; 25], is_func: &mut [[bool; 25]; 25], r0: usize, c0: usize) {
        for r in 0..7 {
            for c in 0..7 {
                let is_black = r == 0 || r == 6 || c == 0 || c == 6 || (r >= 2 && r <= 4 && c >= 2 && c <= 4);
                if r0 + r < 25 && c0 + c < 25 {
                    modules[r0 + r][c0 + c] = is_black;
                    is_func[r0 + r][c0 + c] = true;
                }
            }
        }
        for r in 0..8 {
            for c in 0..8 {
                if r == 7 || c == 7 {
                    if r0 + r < 25 && c0 + c < 25 {
                        modules[r0 + r][c0 + c] = false;
                        is_func[r0 + r][c0 + c] = true;
                    }
                }
            }
        }
    }

    fn place_alignment(modules: &mut [[bool; 25]; 25], is_func: &mut [[bool; 25]; 25], r0: usize, c0: usize) {
        for r in 0..5 {
            for c in 0..5 {
                let is_black = r == 0 || r == 4 || c == 0 || c == 4 || (r == 2 && c == 2);
                modules[r0 + r][c0 + c] = is_black;
                is_func[r0 + r][c0 + c] = true;
            }
        }
    }

    fn place_format_info(modules: &mut [[bool; 25]; 25], format_bits: u16) {
        let coords1 = [
            (8, 0), (8, 1), (8, 2), (8, 3), (8, 4), (8, 5), (8, 7), (8, 8),
            (7, 8), (5, 8), (4, 8), (3, 8), (2, 8), (1, 8), (0, 8),
        ];
        let coords2 = [
            (24, 8), (23, 8), (22, 8), (21, 8), (20, 8), (19, 8), (18, 8), (17, 8),
            (8, 17), (8, 18), (8, 19), (8, 20), (8, 21), (8, 22), (8, 23), (8, 24),
        ];
        for i in 0..15 {
            let bit = ((format_bits >> i) & 1) != 0;
            let (r1, c1) = coords1[i];
            modules[r1][c1] = bit;
            let (r2, c2) = coords2[if i < 8 { i } else { i + 1 }];
            modules[r2][c2] = bit;
        }
    }

    fn is_dark(&self, row: usize, col: usize) -> bool {
        self.modules[row][col]
    }
}
