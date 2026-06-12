//! Layout engine and color palette

#![allow(dead_code)]

use crate::framebuffer::Framebuffer;

pub const HEADER_HEIGHT: u32 = 48;
pub const FOOTER_HEIGHT: u32 = 32;
pub const SEPARATOR_THICKNESS: u32 = 1;

pub mod palette {
    pub const BG: u32 = 0x000000; // true black
    pub const SURFACE: u32 = 0x111111; // header/footer bg
    pub const SEPARATOR: u32 = 0x2A2A2A; // zone dividers
    pub const ACCENT: u32 = 0xE8820C; // SunlightOS orange
    pub const ACCENT_DIM: u32 = 0x7A4406; // dimmed orange
    pub const TEXT: u32 = 0xEEEEEE; // primary text
    pub const TEXT_DIM: u32 = 0x888888; // secondary/detail text
    pub const TEXT_DARK: u32 = 0x444444; // very dim (progress track)
    pub const SUCCESS: u32 = 0x44CC44; // ✓ green
    pub const ERROR: u32 = 0xFF4444; // ✗ red
    pub const WARNING: u32 = 0xFFAA00; // ⚠ yellow
}

// Standard 16-color ANSI palette (matching luxOS)
pub const ANSI_COLORS: [u32; 16] = [
    0x1F1F1F, // 0: black
    0x990000, // 1: red
    0x00A600, // 2: green
    0x999900, // 3: yellow
    0x0000B2, // 4: blue
    0xB200B2, // 5: magenta
    0x00A6B2, // 6: cyan
    0xBFBFBF, // 7: white
    0x666666, // 8: bright black (gray)
    0xE60000, // 9: bright red
    0x00D900, // 10: bright green
    0xE6E600, // 11: bright yellow
    0x0000FF, // 12: bright blue
    0xE600E6, // 13: bright magenta
    0x00E6E6, // 14: bright cyan
    0xE6E6E6, // 15: bright white
];

#[derive(Clone, Copy)]
pub struct Zone {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub struct Layout {
    pub fb_width: u32,
    pub fb_height: u32,
    pub header: Zone,
    pub main: Zone,
    pub footer: Zone,
}

impl Layout {
    pub fn new(fb_width: u32, fb_height: u32) -> Self {
        let header = Zone {
            x: 0,
            y: 0,
            w: fb_width,
            h: HEADER_HEIGHT,
        };

        let main_y = HEADER_HEIGHT + SEPARATOR_THICKNESS;
        let footer_y = fb_height.saturating_sub(FOOTER_HEIGHT);
        let main_h = footer_y
            .saturating_sub(main_y)
            .saturating_sub(SEPARATOR_THICKNESS);

        let main = Zone {
            x: 0,
            y: main_y,
            w: fb_width,
            h: main_h,
        };

        let footer = Zone {
            x: 0,
            y: footer_y,
            w: fb_width,
            h: FOOTER_HEIGHT,
        };

        Self {
            fb_width,
            fb_height,
            header,
            main,
            footer,
        }
    }

    /// Draw the permanent chrome: header bg, footer bg, separator lines
    pub fn draw_chrome(&self, fb: &mut Framebuffer) {
        // Fill background
        fb.fill_rect(0, 0, self.fb_width, self.fb_height, palette::BG);

        // Header background
        fb.fill_rect(
            self.header.x,
            self.header.y,
            self.header.w,
            self.header.h,
            palette::SURFACE,
        );

        // Footer background
        fb.fill_rect(
            self.footer.x,
            self.footer.y,
            self.footer.w,
            self.footer.h,
            palette::SURFACE,
        );

        // Separator below header
        fb.hline(0, self.header.h, self.fb_width, palette::SEPARATOR);

        // Separator above footer
        fb.hline(
            0,
            self.footer.y - SEPARATOR_THICKNESS,
            self.fb_width,
            palette::SEPARATOR,
        );
    }

    /// Clear the main zone to background color
    pub fn clear_main(&self, fb: &mut Framebuffer) {
        fb.fill_rect(
            self.main.x,
            self.main.y,
            self.main.w,
            self.main.h,
            palette::BG,
        );
    }
}
