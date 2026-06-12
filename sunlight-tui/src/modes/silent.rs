//! Silent mode — centered logo + progress + status list

#![allow(dead_code)]

use crate::framebuffer::Framebuffer;
use crate::layout::{palette, Layout};
use crate::{draw, font};

pub struct SilentModeState {
    pub status: &'static str,
    pub progress: u32,                      // 0..=1000 permille
    pub details: [Option<&'static str>; 5], // last 5 completed steps
    pub spinner_step: u32,                  // 0..=359
}

impl SilentModeState {
    pub const fn new() -> Self {
        Self {
            status: "",
            progress: 0,
            details: [None; 5],
            spinner_step: 0,
        }
    }

    pub fn add_detail(&mut self, msg: &'static str) {
        // Shift all details up
        for i in 0..4 {
            self.details[i] = self.details[i + 1];
        }
        self.details[4] = Some(msg);
    }
}

pub fn render_silent(fb: &mut Framebuffer, layout: &Layout, state: &SilentModeState) {
    let main = &layout.main;

    // Clear main zone
    layout.clear_main(fb);

    // Calculate centered layout
    let center_x = main.x + main.w / 2;
    let center_y = main.y + main.h / 2;

    // Sun logo at top
    let logo_y = center_y.saturating_sub(140);
    draw::draw_sun_logo(fb, center_x, logo_y, 24, palette::ACCENT);

    // "SunlightOS" title
    let title = "SunlightOS";
    let title_w = font::text_width(title, 2);
    let title_x = center_x.saturating_sub(title_w / 2);
    let title_y = logo_y + 50;
    font::draw_str(fb, title_x, title_y, title, palette::TEXT, 2);

    // Subtitle
    let subtitle = "Lightweight. Secure.";
    let subtitle_w = font::text_width(subtitle, 1);
    let subtitle_x = center_x.saturating_sub(subtitle_w / 2);
    let subtitle_y = title_y + 40;
    font::draw_str(fb, subtitle_x, subtitle_y, subtitle, palette::TEXT_DIM, 1);

    // Separator line
    let sep_y = subtitle_y + 30;
    let sep_w = 300;
    let sep_x = center_x.saturating_sub(sep_w / 2);
    fb.hline(sep_x, sep_y, sep_w, palette::SEPARATOR);

    // Status text
    let status_y = sep_y + 20;
    let status_w = font::text_width(state.status, 1);
    let status_x = center_x.saturating_sub(status_w / 2);
    font::draw_str(fb, status_x, status_y, state.status, palette::TEXT, 1);

    // Progress bar
    let progress_y = status_y + 24;
    let progress_w = 400;
    let progress_x = center_x.saturating_sub(progress_w / 2);
    draw::progress_bar(
        fb,
        progress_x,
        progress_y,
        progress_w,
        10,
        state.progress,
        palette::TEXT_DARK,
        palette::ACCENT,
    );

    // Progress percentage
    let percent = (state.progress * 100) / 1000;
    let mut pct_buf = [0u8; 20];
    let pct_str = crate::fmt::fmt_u32(&mut pct_buf, percent);
    let pct_text_w = font::text_width(pct_str, 1) + 8; // +8 for "%"
    let pct_x = center_x.saturating_sub(pct_text_w / 2);
    let pct_y = progress_y + 16;
    font::draw_str(fb, pct_x, pct_y, pct_str, palette::TEXT_DIM, 1);
    font::draw_str(
        fb,
        pct_x + font::text_width(pct_str, 1),
        pct_y,
        "%",
        palette::TEXT_DIM,
        1,
    );

    // Second separator
    let sep2_y = pct_y + 24;
    fb.hline(sep_x, sep2_y, sep_w, palette::SEPARATOR);

    // Details list (last 5 completed steps)
    let details_start_y = sep2_y + 16;
    let line_h = font::line_height(1);

    for (i, detail) in state.details.iter().enumerate() {
        if let Some(msg) = detail {
            let y = details_start_y + (i as u32 * line_h);

            // Check mark or spinner
            let is_last = i == 4 && state.progress < 1000;
            let symbol = if is_last { ">" } else { "v" }; // v for checkmark, > for spinner
            let symbol_color = if is_last {
                palette::ACCENT
            } else {
                palette::SUCCESS
            };

            let symbol_x = center_x.saturating_sub(150);
            font::draw_str(fb, symbol_x, y, symbol, symbol_color, 1);

            let msg_x = symbol_x + 16;
            font::draw_str(fb, msg_x, y, msg, palette::TEXT, 1);

            // Animated spinner on last item if in progress
            if is_last && state.progress < 1000 {
                let spinner_x = msg_x + font::text_width(msg, 1) + 8;
                draw::spinner_frame(
                    fb,
                    spinner_x + 8,
                    y + 8,
                    6,
                    2,
                    state.spinner_step,
                    108,
                    palette::ACCENT,
                );
            }
        }
    }
}
