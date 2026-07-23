//! Raw framebuffer operations — no heap, no floats

#![allow(dead_code)]

pub struct Framebuffer {
    addr: *mut u32,
    width: u32,
    height: u32,
    pitch: u32, // bytes per row — NOT pixels per row
}

pub const BYTES_PER_PIXEL: u32 = core::mem::size_of::<u32>() as u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramebufferLayout {
    pub width: u32,
    pub height: u32,
    pub pitch_bytes: u32,
    pub pixels_per_scan_line: u32,
    pub row_bytes: u32,
    pub framebuffer_bytes: u64,
}

/// Validate the XRGB8888 geometry used by the direct TTY/login renderer.
///
/// Limine reports pitch in bytes, and firmware framebuffers may pad each
/// scanline.  Keep that padding in the total mapping size while proving that
/// every visible row fits before any volatile framebuffer write occurs.
pub fn validate_layout(width: u32, height: u32, pitch_bytes: u32) -> Option<FramebufferLayout> {
    if width == 0 || height == 0 || pitch_bytes == 0 || pitch_bytes % BYTES_PER_PIXEL != 0 {
        return None;
    }
    let row_bytes = width.checked_mul(BYTES_PER_PIXEL)?;
    if pitch_bytes < row_bytes {
        return None;
    }
    let framebuffer_bytes = u64::from(pitch_bytes).checked_mul(u64::from(height))?;
    Some(FramebufferLayout {
        width,
        height,
        pitch_bytes,
        pixels_per_scan_line: pitch_bytes / BYTES_PER_PIXEL,
        row_bytes,
        framebuffer_bytes,
    })
}

impl Framebuffer {
    /// SAFETY: caller must ensure addr is valid Limine framebuffer memory
    #[inline]
    pub unsafe fn from_limine(addr: *mut u32, width: u32, height: u32, pitch: u32) -> Self {
        Self {
            addr,
            width,
            height,
            pitch,
        }
    }

    #[inline(always)]
    pub fn put_pixel(&mut self, x: u32, y: u32, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        // CRITICAL: pitch is bytes per row, divide by 4 for u32 pixels
        let offset = (y as usize * (self.pitch as usize / 4)) + x as usize;
        // SAFETY: bounds checked above, caller guaranteed valid framebuffer
        unsafe {
            self.addr.add(offset).write_volatile(color);
        }
    }

    #[inline(always)]
    pub fn get_pixel(&self, x: u32, y: u32) -> u32 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let offset = (y as usize * (self.pitch as usize / 4)) + x as usize;
        // SAFETY: bounds checked above, caller guaranteed valid framebuffer
        unsafe { self.addr.add(offset).read_volatile() }
    }

    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        let x_end = x.saturating_add(w).min(self.width);
        let y_end = y.saturating_add(h).min(self.height);

        for row in y..y_end {
            for col in x..x_end {
                self.put_pixel(col, row, color);
            }
        }
    }

    /// Horizontal line — faster than fill_rect for h=1
    pub fn hline(&mut self, x: u32, y: u32, len: u32, color: u32) {
        if y >= self.height {
            return;
        }
        let x_end = x.saturating_add(len).min(self.width);
        let offset = y as usize * (self.pitch as usize / 4);

        for col in x..x_end {
            // SAFETY: bounds checked, valid framebuffer
            unsafe {
                self.addr.add(offset + col as usize).write_volatile(color);
            }
        }
    }

    /// Vertical line
    pub fn vline(&mut self, x: u32, y: u32, len: u32, color: u32) {
        if x >= self.width {
            return;
        }
        let y_end = (y + len).min(self.height);

        for row in y..y_end {
            self.put_pixel(x, row, color);
        }
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_1080p_and_1200p_without_a_fixed_height_limit() {
        let hd = validate_layout(1920, 1080, 1920 * 4).unwrap();
        assert_eq!(hd.pixels_per_scan_line, 1920);
        assert_eq!(hd.framebuffer_bytes, 8_294_400);

        let wuxga = validate_layout(1920, 1200, 1920 * 4).unwrap();
        assert_eq!(wuxga.pixels_per_scan_line, 1920);
        assert_eq!(wuxga.framebuffer_bytes, 9_216_000);
        assert!(wuxga.framebuffer_bytes > 8 * 1024 * 1024);
    }

    #[test]
    fn preserves_padded_scanlines_and_rejects_short_or_misaligned_pitch() {
        let padded = validate_layout(1920, 1200, 8192).unwrap();
        assert_eq!(padded.pixels_per_scan_line, 2048);
        assert_eq!(padded.row_bytes, 7680);
        assert_eq!(padded.framebuffer_bytes, 9_830_400);

        assert_eq!(validate_layout(1920, 1200, 7676), None);
        assert_eq!(validate_layout(1920, 1200, 7682), None);
    }
}
