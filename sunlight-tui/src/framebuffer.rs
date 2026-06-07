//! Raw framebuffer operations — no heap, no floats

#![allow(dead_code)]

pub struct Framebuffer {
    addr:   *mut u32,
    width:  u32,
    height: u32,
    pitch:  u32,   // bytes per row — NOT pixels per row
}

impl Framebuffer {
    /// SAFETY: caller must ensure addr is valid Limine framebuffer memory
    #[inline]
    pub unsafe fn from_limine(addr: *mut u32, width: u32, height: u32, pitch: u32) -> Self {
        Self { addr, width, height, pitch }
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

    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        let x_end = (x + w).min(self.width);
        let y_end = (y + h).min(self.height);
        
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
        let x_end = (x + len).min(self.width);
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
