//! SplashScreen — main public API

#![allow(dead_code)]

use crate::font;
use crate::framebuffer::Framebuffer;
use crate::layout::{palette, Layout};
use crate::modes::{debug, silent};

#[derive(Clone, Copy)]
pub enum BootMode {
    /// Show only logo + progress + status (kernel cmdline: shutup-mode)
    Silent,
    /// Show scrolling log panel + progress (default)  
    Debug,
}

pub struct FooterInfo {
    pub kernel_status: &'static str,
    pub cpu_arch: &'static str,
    pub ram_mib: u32,
}

pub struct SplashScreen {
    fb: Framebuffer,
    layout: Layout,
    mode: BootMode,
    silent: silent::SilentModeState,
    debug: debug::DebugModeState,
    footer: FooterInfo,
    version: &'static str,
    phase: &'static str,
    tick: u32,
}

impl SplashScreen {
    /// Initialize from Limine framebuffer response.
    /// SAFETY: caller must ensure fb_addr points to valid Limine framebuffer memory
    pub unsafe fn init(
        fb_addr: *mut u32,
        fb_width: u32,
        fb_height: u32,
        fb_pitch: u32,
        mode: BootMode,
        ram_mib: u32,
    ) -> Self {
        let fb = Framebuffer::from_limine(fb_addr, fb_width, fb_height, fb_pitch);
        let layout = Layout::new(fb_width, fb_height);

        let mut splash = Self {
            fb,
            layout,
            mode,
            silent: silent::SilentModeState::new(),
            debug: debug::DebugModeState::new(),
            footer: FooterInfo {
                kernel_status: "Initializing",
                cpu_arch: "x86_64",
                ram_mib,
            },
            version: "v0.1.0",
            phase: "Phase 2",
            tick: 0,
        };

        splash.redraw();
        splash
    }

    /// Update progress bar (permille 0..=1000)
    pub fn set_progress(&mut self, permille: u32) {
        self.silent.progress = permille.min(1000);
        self.debug.progress = permille.min(1000);
    }

    /// Update status line (shown below progress bar)
    pub fn set_status(&mut self, msg: &'static str) {
        self.silent.status = msg;
        self.debug.status = msg;
    }

    /// Append a line to the debug log.
    /// In silent mode: adds to the detail list.
    /// In debug mode: adds to the scrolling log.
    pub fn log(&mut self, msg: &'static str) {
        // Auto-detect line kind
        let kind = if msg.contains(" OK") || msg.ends_with("OK") {
            debug::LineKind::Ok
        } else if msg.contains("PANIC") || msg.contains("ERROR") {
            debug::LineKind::Error
        } else if msg.contains("WARNING") || msg.contains("WARN") {
            debug::LineKind::Warning
        } else {
            debug::LineKind::Info
        };

        self.debug.log.push(msg, kind);

        // For silent mode, only add completed steps (lines ending with OK)
        if matches!(kind, debug::LineKind::Ok) {
            self.silent.add_detail(msg);
        }
    }

    /// Same as log() but detects LineKind from content
    pub fn log_auto(&mut self, msg: &'static str) {
        self.log(msg);
    }

    /// Advance animation by one tick.
    /// Call this from the timer IRQ or a busy-wait loop.
    pub fn tick(&mut self) {
        self.tick += 1;

        // Update spinner (rotate 6 degrees per tick for smooth animation)
        self.silent.spinner_step = (self.silent.spinner_step + 6) % 360;
        self.debug.spinner_step = (self.debug.spinner_step + 6) % 360;

        // Only redraw spinner area for performance (full redraw is expensive)
        // For now, skip partial redraw optimization
    }

    /// Show panic screen — replaces main zone with error display.
    pub fn show_panic(&mut self, msg: &'static str, addr: u64) {
        self.layout.clear_main(&mut self.fb);
        self.footer.kernel_status = "PANIC";

        let main = &self.layout.main;
        let center_x = main.x + main.w / 2;
        let center_y = main.y + main.h / 2;

        // Large X symbol
        let x_y = center_y.saturating_sub(60);
        font::draw_str(&mut self.fb, center_x - 8, x_y, "X", palette::ERROR, 3);

        // "Kernel Panic" title
        let title = "Kernel Panic";
        let title_w = font::text_width(title, 2);
        let title_x = center_x.saturating_sub(title_w / 2);
        let title_y = x_y + 50;
        font::draw_str(&mut self.fb, title_x, title_y, title, palette::ERROR, 2);

        // Message
        let msg_y = title_y + 40;
        let msg_w = font::text_width(msg, 1);
        let msg_x = center_x.saturating_sub(msg_w / 2);
        font::draw_str(&mut self.fb, msg_x, msg_y, msg, palette::TEXT, 1);

        // Address
        let mut addr_buf = [0u8; 20];
        let addr_str = crate::fmt::fmt_hex(&mut addr_buf, addr);
        let addr_y = msg_y + 24;
        let addr_w = font::text_width(addr_str, 1);
        let addr_x = center_x.saturating_sub(addr_w / 2);
        font::draw_str(&mut self.fb, addr_x, addr_y, addr_str, palette::TEXT_DIM, 1);

        // Help text
        let help = "System halted. Please reboot.";
        let help_y = addr_y + 40;
        let help_w = font::text_width(help, 1);
        let help_x = center_x.saturating_sub(help_w / 2);
        font::draw_str(&mut self.fb, help_x, help_y, help, palette::TEXT_DIM, 1);

        // Redraw footer
        self.draw_footer();
    }

    /// Full redraw of all zones.
    pub fn redraw(&mut self) {
        // Draw chrome
        self.layout.draw_chrome(&mut self.fb);

        // Draw header
        self.draw_header();

        // Draw mode-specific content
        match self.mode {
            BootMode::Silent => {
                silent::render_silent(&mut self.fb, &self.layout, &self.silent);
            }
            BootMode::Debug => {
                debug::render_debug(&mut self.fb, &self.layout, &self.debug);
            }
        }

        // Draw footer
        self.draw_footer();
    }

    /// Update footer RAM info (call after PMM init when you know RAM)
    pub fn set_ram(&mut self, ram_mib: u32) {
        self.footer.ram_mib = ram_mib;
    }

    /// Update footer status
    pub fn set_kernel_status(&mut self, status: &'static str) {
        self.footer.kernel_status = status;
    }

    /// Update the phase string shown in the header
    pub fn set_phase(&mut self, phase: &'static str) {
        self.phase = phase;
    }

    /// Clear the main content zone before handing display ownership to another renderer.
    pub fn clear_main(&mut self) {
        self.layout.clear_main(&mut self.fb);
    }

    fn draw_header(&mut self) {
        let header = &self.layout.header;

        // Left: sun symbol + "SunlightOS"
        let left_x = 16;
        let left_y = (header.h - 16) / 2; // vertically center 16px text

        // Sun symbol (simplified as asterisk for now)
        font::draw_str(&mut self.fb, left_x, left_y, "*", palette::ACCENT, 1);
        font::draw_str(
            &mut self.fb,
            left_x + 16,
            left_y,
            "SunlightOS",
            palette::TEXT,
            1,
        );

        // Right: version | phase | mode
        let mode_str = match self.mode {
            BootMode::Silent => "SILENT",
            BootMode::Debug => "DEBUG",
        };

        let mut right_text = [0u8; 64];
        let mut pos = 0;

        // Build right text: "v0.1.0 | Phase 2 | DEBUG"
        for &b in self.version.as_bytes() {
            right_text[pos] = b;
            pos += 1;
        }
        right_text[pos] = b' ';
        right_text[pos + 1] = b'|';
        right_text[pos + 2] = b' ';
        pos += 3;

        for &b in self.phase.as_bytes() {
            right_text[pos] = b;
            pos += 1;
        }
        right_text[pos] = b' ';
        right_text[pos + 1] = b'|';
        right_text[pos + 2] = b' ';
        pos += 3;

        for &b in mode_str.as_bytes() {
            right_text[pos] = b;
            pos += 1;
        }

        let right_str = unsafe { core::str::from_utf8_unchecked(&right_text[..pos]) };
        let right_w = font::text_width(right_str, 1);
        let right_x = header.w.saturating_sub(right_w + 16);

        font::draw_str(
            &mut self.fb,
            right_x,
            left_y,
            right_str,
            palette::TEXT_DIM,
            1,
        );
    }

    fn draw_footer(&mut self) {
        let footer = &self.layout.footer;

        // Clear footer first
        self.fb
            .fill_rect(footer.x, footer.y, footer.w, footer.h, palette::SURFACE);

        let left_x = 16;
        let left_y = footer.y + (footer.h - 16) / 2;

        // Left: "Status: OK" / "Status: PANIC"
        let status_color = if self.footer.kernel_status == "OK" {
            palette::SUCCESS
        } else if self.footer.kernel_status.starts_with("PANIC") {
            palette::ERROR
        } else if self.footer.kernel_status.starts_with("WARN") {
            palette::WARNING
        } else {
            palette::TEXT
        };

        font::draw_str(
            &mut self.fb,
            left_x,
            left_y,
            "Status: ",
            palette::TEXT_DIM,
            1,
        );
        font::draw_str(
            &mut self.fb,
            left_x + 64,
            left_y,
            self.footer.kernel_status,
            status_color,
            1,
        );

        // Right: "CPU: x86_64  RAM: 251 MiB"
        let mut right_text = [0u8; 64];
        let mut pos = 0;

        // "CPU: "
        right_text[pos..pos + 5].copy_from_slice(b"CPU: ");
        pos += 5;

        // arch
        for &b in self.footer.cpu_arch.as_bytes() {
            right_text[pos] = b;
            pos += 1;
        }

        // "  RAM: "
        right_text[pos..pos + 7].copy_from_slice(b"  RAM: ");
        pos += 7;

        // RAM value
        let mut ram_buf = [0u8; 20];
        let ram_str = crate::fmt::fmt_u32(&mut ram_buf, self.footer.ram_mib);
        for &b in ram_str.as_bytes() {
            right_text[pos] = b;
            pos += 1;
        }

        // " MiB"
        right_text[pos..pos + 4].copy_from_slice(b" MiB");
        pos += 4;

        let right_str = unsafe { core::str::from_utf8_unchecked(&right_text[..pos]) };
        let right_w = font::text_width(right_str, 1);
        let right_x = footer.w.saturating_sub(right_w + 16);

        font::draw_str(
            &mut self.fb,
            right_x,
            left_y,
            right_str,
            palette::TEXT_DIM,
            1,
        );
    }
}
