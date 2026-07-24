//! Panel, ProgressBar, Histogram, StatusBadge widgets.

use crate::font::VecText;
use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

// ── Panel ─────────────────────────────────────────────────────────────────────

/// A plain filled panel with an optional title bar.
pub struct Panel<'a> {
    pub rect: Rect,
    /// If Some, a title bar is drawn at the top.
    pub title: Option<&'a str>,
    pub title_h: u32,
    /// Vector font for the title. Falls back to the 5×7 bitmap font when `None`.
    font: Option<&'a dyn VecText>,
}

impl<'a> Panel<'a> {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            title: None,
            title_h: 20,
            font: None,
        }
    }

    pub fn with_title(rect: Rect, title: &'a str) -> Self {
        Self {
            rect,
            title: Some(title),
            title_h: 20,
            font: None,
        }
    }

    /// Render the title with a Sunlight vector font (MiniType).
    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    /// Returns the inner content rect (below any title bar).
    pub fn content_rect(&self) -> Rect {
        if self.title.is_some() {
            Rect::new(
                self.rect.x,
                self.rect.y + self.title_h as i32,
                self.rect.w,
                self.rect.h.saturating_sub(self.title_h),
            )
        } else {
            self.rect
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.panel);

        if let Some(title) = self.title {
            let title_rect = Rect::new(self.rect.x, self.rect.y, self.rect.w, self.title_h);
            canvas.fill_rect(title_rect, theme.panel_alt);
            if let Some(font) = self.font {
                font.draw_vcenter(
                    canvas,
                    title,
                    self.rect.x + 8,
                    self.rect.y,
                    self.title_h,
                    theme.accent,
                );
            } else {
                canvas.draw_text(
                    self.rect.x + 8,
                    self.rect.y + (self.title_h as i32 - 10) / 2,
                    title,
                    theme.accent,
                );
            }
            canvas.hbar(
                self.rect.x,
                self.rect.y + self.title_h as i32,
                self.rect.w,
                1,
                theme.border,
            );
        }

        canvas.draw_rect(self.rect, theme.border);
    }
}

// ── ProgressBar ───────────────────────────────────────────────────────────────

/// Horizontal progress bar, value in 0.0..=1.0.
pub struct ProgressBar {
    pub rect: Rect,
    pub value: f32,
    /// Override fill color. If None, uses `theme.accent`.
    pub color: Option<Color>,
    pub show_pct: bool,
}

impl ProgressBar {
    pub fn new(rect: Rect, value: f32) -> Self {
        Self {
            rect,
            value: value.clamp(0.0, 1.0),
            color: None,
            show_pct: false,
        }
    }

    pub fn with_color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
    pub fn with_pct(mut self) -> Self {
        self.show_pct = true;
        self
    }

    /// Choose accent / warn / danger based on value thresholds.
    pub fn auto_color(mut self, theme: &Theme) -> Self {
        self.color = Some(if self.value > 0.85 {
            theme.danger
        } else if self.value > 0.65 {
            theme.warn
        } else {
            theme.accent
        });
        self
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        // Background track
        canvas.fill_rect(self.rect, theme.panel_alt);
        canvas.draw_rect(self.rect, theme.border);

        // Fill
        let fill_w = ((self.rect.w as f32) * self.value) as u32;
        if fill_w > 0 {
            let fill_rect = Rect::new(self.rect.x, self.rect.y, fill_w, self.rect.h);
            let color = self.color.unwrap_or(theme.accent);
            canvas.fill_rect(fill_rect, color);
        }

        // Percentage label
        if self.show_pct {
            let pct = (self.value * 100.0) as u32;
            let mut buf = [0u8; 8];
            let label = fmt_u32(pct, &mut buf);
            canvas.draw_text_centered(self.rect, label, theme.text);
        }
    }
}

// ── Histogram ────────────────────────────────────────────────────────────────

/// Rolling bar-chart histogram (e.g. CPU usage over time).
pub struct Histogram<'a> {
    pub rect: Rect,
    /// Samples in chronological order (oldest first), values 0.0..=1.0.
    pub samples: &'a [f32],
    pub color: Option<Color>,
    /// Draw a horizontal line at this value (0.0..=1.0).
    pub baseline: Option<f32>,
}

impl<'a> Histogram<'a> {
    pub fn new(rect: Rect, samples: &'a [f32]) -> Self {
        Self {
            rect,
            samples,
            color: None,
            baseline: None,
        }
    }

    pub fn with_color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
    pub fn with_baseline(mut self, v: f32) -> Self {
        self.baseline = Some(v);
        self
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.panel_alt);
        canvas.draw_rect(self.rect, theme.border);

        let color = self.color.unwrap_or(theme.accent);
        let n = self.samples.len();
        if n == 0 {
            return;
        }

        let bar_w = (self.rect.w as usize / n).max(1) as u32;
        let h = self.rect.h as f32;

        for (i, &v) in self.samples.iter().enumerate() {
            let bar_h = ((v.clamp(0.0, 1.0)) * h) as u32;
            if bar_h == 0 {
                continue;
            }
            let bx = self.rect.x + (i as u32 * bar_w) as i32;
            let by = self.rect.bottom() - bar_h as i32;
            canvas.fill_rect(Rect::new(bx, by, bar_w.saturating_sub(1), bar_h), color);
        }

        // Baseline
        if let Some(bl) = self.baseline {
            let by = self.rect.bottom() - ((bl.clamp(0.0, 1.0)) * h) as i32;
            canvas.hbar(self.rect.x, by, self.rect.w, 1, theme.text_dim);
        }
    }
}

// ── StatusBadge ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Ok,
    Warn,
    Danger,
    Dim,
    Accent,
}

/// Small colored dot + optional label (e.g. "ready", "blocked").
pub struct StatusBadge<'a> {
    pub x: i32,
    pub y: i32,
    pub kind: BadgeKind,
    pub label: Option<&'a str>,
    pub font: Option<&'a dyn VecText>,
}

impl<'a> StatusBadge<'a> {
    pub fn new(x: i32, y: i32, kind: BadgeKind) -> Self {
        Self {
            x,
            y,
            kind,
            label: None,
            font: None,
        }
    }

    pub fn with_label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Render the optional label with a Sunlight vector font.
    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    /// Forward an optional inherited font without forcing callers to branch.
    pub fn with_font_if(mut self, font: Option<&'a dyn VecText>) -> Self {
        self.font = font;
        self
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let dot_color = match self.kind {
            BadgeKind::Ok => theme.ok,
            BadgeKind::Warn => theme.warn,
            BadgeKind::Danger => theme.danger,
            BadgeKind::Dim => theme.text_dim,
            BadgeKind::Accent => theme.accent,
        };

        // 6×6 filled dot
        canvas.fill_rect(Rect::new(self.x, self.y + 2, 6, 6), dot_color);

        if let Some(label) = self.label {
            if let Some(font) = self.font {
                font.draw_vcenter(canvas, label, self.x + 9, self.y, 12, theme.text_dim);
            } else {
                canvas.draw_text(self.x + 9, self.y, label, theme.text_dim);
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Format a u32 into a scratch buffer, returns the filled slice as &str.
fn fmt_u32<'b>(mut n: u32, buf: &'b mut [u8; 8]) -> &'b str {
    if n == 0 {
        buf[7] = b'0';
        return core::str::from_utf8(&buf[7..]).unwrap();
    }
    let mut i = 8usize;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    core::str::from_utf8(&buf[i..]).unwrap()
}
