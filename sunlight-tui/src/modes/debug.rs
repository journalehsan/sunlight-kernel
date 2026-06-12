//! Debug mode — scrolling log panel + progress

#![allow(dead_code)]

use crate::framebuffer::Framebuffer;
use crate::layout::{palette, Layout};
use crate::{draw, font};

pub const LOG_CAPACITY: usize = 64;

#[derive(Clone, Copy)]
pub enum LineKind {
    Info,    // TEXT color
    Ok,      // SUCCESS color
    Error,   // ERROR color
    Warning, // WARNING color
}

#[derive(Clone, Copy)]
pub struct LogLine {
    pub text: [u8; 128],
    pub len: u8,
    pub kind: LineKind,
}

impl LogLine {
    pub const fn empty() -> Self {
        Self {
            text: [0; 128],
            len: 0,
            kind: LineKind::Info,
        }
    }

    pub fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.text[0..self.len as usize]) }
    }
}

pub struct LogBuffer {
    lines: [LogLine; LOG_CAPACITY],
    head: usize,
    count: usize,
}

impl LogBuffer {
    pub const fn new() -> Self {
        Self {
            lines: [LogLine::empty(); LOG_CAPACITY],
            head: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, line: &str, kind: LineKind) {
        let bytes = line.as_bytes();
        let len = bytes.len().min(127);

        let mut log_line = LogLine::empty();
        log_line.text[..len].copy_from_slice(&bytes[..len]);
        log_line.len = len as u8;
        log_line.kind = kind;

        self.lines[self.head] = log_line;
        self.head = (self.head + 1) % LOG_CAPACITY;
        if self.count < LOG_CAPACITY {
            self.count += 1;
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8], kind: LineKind) {
        if let Ok(s) = core::str::from_utf8(bytes) {
            self.push(s, kind);
        }
    }

    /// Get line at visual index (0 = oldest visible line)
    pub fn get(&self, idx: usize) -> Option<&LogLine> {
        if idx >= self.count {
            return None;
        }
        let actual_idx = if self.count < LOG_CAPACITY {
            idx
        } else {
            (self.head + idx) % LOG_CAPACITY
        };
        Some(&self.lines[actual_idx])
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

pub struct DebugModeState {
    pub log: LogBuffer,
    pub status: &'static str,
    pub progress: u32,     // 0..=1000
    pub spinner_step: u32, // 0..=359
}

impl DebugModeState {
    pub const fn new() -> Self {
        Self {
            log: LogBuffer::new(),
            status: "",
            progress: 0,
            spinner_step: 0,
        }
    }
}

pub fn render_debug(fb: &mut Framebuffer, layout: &Layout, state: &DebugModeState) {
    let main = &layout.main;

    // Clear main zone
    layout.clear_main(fb);

    // Log panel dimensions (leave space for status + progress at bottom)
    let panel_margin = 16;
    let bottom_section_height = 60;
    let panel_x = main.x + panel_margin;
    let panel_y = main.y + panel_margin;
    let panel_w = main.w.saturating_sub(panel_margin * 2);
    let panel_h = main
        .h
        .saturating_sub(panel_margin * 2 + bottom_section_height);

    // Draw log panel border
    draw::rect_outline(
        fb,
        panel_x,
        panel_y,
        panel_w,
        panel_h,
        1,
        palette::SEPARATOR,
    );

    // Title
    let title = "Boot Log";
    font::draw_str(fb, panel_x + 8, panel_y - 8, title, palette::TEXT_DIM, 1);

    // Render log lines (newest at bottom)
    let line_h = font::line_height(1);
    let max_lines = (panel_h.saturating_sub(16)) / line_h;
    let log_count = state.log.count();
    let start_idx = if log_count > max_lines as usize {
        log_count - max_lines as usize
    } else {
        0
    };

    for i in 0..max_lines as usize {
        let log_idx = start_idx + i;
        if let Some(line) = state.log.get(log_idx) {
            let color = match line.kind {
                LineKind::Info => palette::TEXT,
                LineKind::Ok => palette::SUCCESS,
                LineKind::Error => palette::ERROR,
                LineKind::Warning => palette::WARNING,
            };

            let y = panel_y + 8 + (i as u32 * line_h);
            let text = line.as_str();

            // Truncate if too long
            let max_chars = (panel_w.saturating_sub(16)) / 8;
            let display_text = if text.len() > max_chars as usize {
                &text[..(max_chars as usize).saturating_sub(1)]
            } else {
                text
            };

            font::draw_str(fb, panel_x + 8, y, display_text, color, 1);
        }
    }

    // Status line and progress bar at bottom
    let status_y = main.y + main.h.saturating_sub(bottom_section_height) + 16;
    let progress_y = status_y + 24;

    // Status text
    let mut status_line = [0u8; 128];
    let status_bytes = state.status.as_bytes();
    let status_len = status_bytes.len().min(100);
    status_line[..status_len].copy_from_slice(&status_bytes[..status_len]);
    status_line[status_len] = b'.';
    status_line[status_len + 1] = b'.';
    status_line[status_len + 2] = b'.';
    let status_str = unsafe { core::str::from_utf8_unchecked(&status_line[..status_len + 3]) };

    font::draw_str(fb, panel_x, status_y, "Status: ", palette::TEXT_DIM, 1);
    font::draw_str(fb, panel_x + 64, status_y, status_str, palette::TEXT, 1);

    // Spinner
    let spinner_x = main.x + main.w - 32;
    draw::spinner_frame(
        fb,
        spinner_x,
        status_y + 8,
        10,
        3,
        state.spinner_step,
        108,
        palette::ACCENT,
    );

    // Progress bar
    let progress_w = panel_w;
    draw::progress_bar(
        fb,
        panel_x,
        progress_y,
        progress_w,
        8,
        state.progress,
        palette::TEXT_DARK,
        palette::ACCENT,
    );

    // Progress percentage
    let percent = (state.progress * 100) / 1000;
    let mut pct_buf = [0u8; 20];
    let pct_str = crate::fmt::fmt_u32(&mut pct_buf, percent);
    let pct_x = panel_x + progress_w + 8;
    font::draw_str(fb, pct_x, progress_y, pct_str, palette::TEXT_DIM, 1);
    font::draw_str(
        fb,
        pct_x + font::text_width(pct_str, 1),
        progress_y,
        "%",
        palette::TEXT_DIM,
        1,
    );
}
