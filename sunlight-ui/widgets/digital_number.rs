//! Reusable seven-segment style numeric display.
//!
//! Supported characters: digits `0`–`9`, colon `:`, decimal point `.`, and
//! minus sign `-`. All other characters are rejected by the validation API
//! and replaced with a blank (unlit) digit when formatting via
//! [`DigitalNumberWidget::set_value_str`].
//!
//! Measurement is independent of the currently displayed characters: width
//! is always based on the configured maximum character count and digit
//! geometry so layout does not shift when the value changes.

use crate::geom::{Point, Rect, Size};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

/// Horizontal alignment of the digit strip inside the widget rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DigitalAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Characters accepted by [`DigitalNumberWidget`].
///
/// | Char | Meaning        |
/// |------|----------------|
/// | 0–9  | Digits         |
/// | `:`  | Colon          |
/// | `.`  | Decimal point  |
/// | `-`  | Minus sign     |
pub const SUPPORTED_CHARS: &[char] = &[
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', '.', '-',
];

/// Maximum characters stored in the widget (includes separators).
pub const DIGITAL_VALUE_CAP: usize = 16;

/// One seven-segment pattern as a bitmask of segments A..G (bit 0 = A … bit 6 = G).
///
/// ```text
///  AAA
/// F   B
///  GGG
/// E   C
///  DDD
/// ```
const SEG_A: u8 = 1 << 0;
const SEG_B: u8 = 1 << 1;
const SEG_C: u8 = 1 << 2;
const SEG_D: u8 = 1 << 3;
const SEG_E: u8 = 1 << 4;
const SEG_F: u8 = 1 << 5;
const SEG_G: u8 = 1 << 6;

/// Digit segment tables for 0–9.
const DIGIT_SEGS: [u8; 10] = [
    SEG_A | SEG_B | SEG_C | SEG_D | SEG_E | SEG_F, // 0
    SEG_B | SEG_C,                                 // 1
    SEG_A | SEG_B | SEG_D | SEG_E | SEG_G,         // 2
    SEG_A | SEG_B | SEG_C | SEG_D | SEG_G,         // 3
    SEG_B | SEG_C | SEG_F | SEG_G,                 // 4
    SEG_A | SEG_C | SEG_D | SEG_F | SEG_G,         // 5
    SEG_A | SEG_C | SEG_D | SEG_E | SEG_F | SEG_G, // 6
    SEG_A | SEG_B | SEG_C,                         // 7
    SEG_A | SEG_B | SEG_C | SEG_D | SEG_E | SEG_F | SEG_G, // 8
    SEG_A | SEG_B | SEG_C | SEG_D | SEG_F | SEG_G, // 9
];

/// Cached segment layout for a single digit cell (relative to cell origin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentGeom {
    /// Horizontal segments: A, G, D as (x, y, w, h).
    a: Rect,
    g: Rect,
    d: Rect,
    /// Vertical segments: B, C, E, F.
    b: Rect,
    c: Rect,
    e: Rect,
    f: Rect,
    /// Cell size used to produce this geometry.
    cell: Size,
    thickness: u32,
}

impl SegmentGeom {
    /// Build crisp axis-aligned segment rects for `digit_w` × `digit_h`.
    fn compute(digit_w: u32, digit_h: u32) -> Self {
        let digit_w = digit_w.max(4);
        let digit_h = digit_h.max(6);
        // Thickness scales with size; keep odd-ish for centered middle bar.
        let t = ((digit_w.min(digit_h) + 4) / 8).clamp(1, 6);
        let inset = t;
        let inner_w = digit_w.saturating_sub(inset * 2).max(1);
        let mid_y = digit_h / 2;

        let a = Rect::new(inset as i32, 0, inner_w, t);
        let g = Rect::new(inset as i32, mid_y as i32 - (t as i32) / 2, inner_w, t);
        let d = Rect::new(inset as i32, digit_h as i32 - t as i32, inner_w, t);

        // Verticals sit between the horizontal bars.
        let top_h = mid_y.saturating_sub(t).max(1);
        let bot_h = digit_h.saturating_sub(mid_y).saturating_sub(t).max(1);
        let top_y = t as i32;
        let bot_y = (mid_y + t / 2) as i32;

        let b = Rect::new(digit_w as i32 - t as i32, top_y, t, top_h);
        let c = Rect::new(digit_w as i32 - t as i32, bot_y, t, bot_h);
        let e = Rect::new(0, bot_y, t, bot_h);
        let f = Rect::new(0, top_y, t, top_h);

        Self {
            a,
            g,
            d,
            b,
            c,
            e,
            f,
            cell: Size::new(digit_w, digit_h),
            thickness: t,
        }
    }

    fn ensure(self, digit_w: u32, digit_h: u32) -> Self {
        if self.cell.w == digit_w.max(4) && self.cell.h == digit_h.max(6) {
            self
        } else {
            Self::compute(digit_w, digit_h)
        }
    }
}

/// Configuration / view model for a digital number strip.
#[derive(Debug, Clone)]
pub struct DigitalNumberWidget {
    pub rect: Rect,
    /// Pixel width of one digit cell.
    pub digit_w: u32,
    /// Pixel height of one digit cell.
    pub digit_h: u32,
    /// Gap between adjacent cells (pixels).
    pub spacing: u32,
    /// How many character cells to reserve (stable measure).
    pub max_chars: u8,
    /// Pad numeric values with leading zeros up to `max_chars` digit slots.
    /// Separators still consume cells; only pure digit runs are zero-padded
    /// when using [`Self::set_u32`] / [`Self::set_i32`].
    pub leading_zeros: bool,
    pub align: DigitalAlign,
    /// Lit segment / glyph color. `None` → `theme.accent` (or `theme.text` for dimless).
    pub foreground: Option<Color>,
    /// Unlit segment color. `None` → very dim panel tint.
    pub inactive: Option<Color>,
    /// Stored display characters (fixed capacity, no heap on set when in place).
    value: [u8; DIGITAL_VALUE_CAP],
    value_len: u8,
    /// Cached segment geometry (rebuilt when digit size changes).
    seg: SegmentGeom,
}

impl DigitalNumberWidget {
    pub fn new(rect: Rect) -> Self {
        let digit_h = rect.h.max(10);
        let digit_w = (digit_h * 3 / 5).max(6);
        Self {
            rect,
            digit_w,
            digit_h,
            spacing: (digit_w / 5).max(1),
            max_chars: 5,
            leading_zeros: false,
            align: DigitalAlign::Left,
            foreground: None,
            inactive: None,
            value: [b' '; DIGITAL_VALUE_CAP],
            value_len: 0,
            seg: SegmentGeom::compute(digit_w, digit_h),
        }
    }

    pub fn with_digit_size(mut self, w: u32, h: u32) -> Self {
        self.digit_w = w.max(4);
        self.digit_h = h.max(6);
        self.seg = SegmentGeom::compute(self.digit_w, self.digit_h);
        self
    }

    pub fn with_spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn with_max_chars(mut self, n: u8) -> Self {
        self.max_chars = n.clamp(1, DIGITAL_VALUE_CAP as u8);
        self
    }

    pub fn with_leading_zeros(mut self, on: bool) -> Self {
        self.leading_zeros = on;
        self
    }

    pub fn with_align(mut self, align: DigitalAlign) -> Self {
        self.align = align;
        self
    }

    pub fn with_colors(mut self, foreground: Color, inactive: Color) -> Self {
        self.foreground = Some(foreground);
        self.inactive = Some(inactive);
        self
    }

    pub fn with_value(mut self, s: &str) -> Self {
        let _ = self.set_value_str(s);
        self
    }

    /// True when every character is in the supported set (or the string is empty).
    pub fn is_supported_str(s: &str) -> bool {
        s.chars().all(is_supported_char)
    }

    /// Reject unsupported characters. Returns `Ok(sanitized length)` or `Err` with
    /// the first bad character.
    pub fn validate_str(s: &str) -> Result<usize, char> {
        let mut n = 0usize;
        for ch in s.chars() {
            if !is_supported_char(ch) {
                return Err(ch);
            }
            n += 1;
            if n > DIGITAL_VALUE_CAP {
                break;
            }
        }
        Ok(n.min(DIGITAL_VALUE_CAP))
    }

    /// Set display from a string. Unsupported characters become blank cells
    /// (unlit digit placeholder). Truncates to [`DIGITAL_VALUE_CAP`].
    /// Returns `false` if any character was unsupported or truncated.
    pub fn set_value_str(&mut self, s: &str) -> bool {
        let mut ok = true;
        let mut len = 0u8;
        for ch in s.chars() {
            if len as usize >= DIGITAL_VALUE_CAP {
                ok = false;
                break;
            }
            if is_supported_char(ch) {
                self.value[len as usize] = ch as u8;
            } else {
                self.value[len as usize] = b' ';
                ok = false;
            }
            len += 1;
        }
        self.value_len = len;
        ok
    }

    /// Format an unsigned integer. With `leading_zeros`, pads with `0` up to
    /// `max_chars` (capped at the digit capacity).
    pub fn set_u32(&mut self, value: u32) {
        let mut buf = [0u8; DIGITAL_VALUE_CAP];
        let mut n = value;
        let mut i = 0usize;
        if n == 0 {
            buf[0] = b'0';
            i = 1;
        } else {
            while n > 0 && i < DIGITAL_VALUE_CAP {
                buf[i] = b'0' + (n % 10) as u8;
                n /= 10;
                i += 1;
            }
            // reverse
            buf[..i].reverse();
        }
        let width = self.max_chars as usize;
        if self.leading_zeros && i < width {
            let pad = width - i;
            // shift right
            for k in (0..i).rev() {
                if k + pad < DIGITAL_VALUE_CAP {
                    buf[k + pad] = buf[k];
                }
            }
            for k in 0..pad {
                buf[k] = b'0';
            }
            i = width.min(DIGITAL_VALUE_CAP);
        }
        self.value[..i].copy_from_slice(&buf[..i]);
        self.value_len = i as u8;
    }

    /// Format a signed integer (leading `-` when negative).
    pub fn set_i32(&mut self, value: i32) {
        if value < 0 {
            let mut tmp = Self::new(self.rect)
                .with_digit_size(self.digit_w, self.digit_h)
                .with_max_chars(self.max_chars.saturating_sub(1).max(1))
                .with_leading_zeros(self.leading_zeros);
            // Avoid recursive ownership: format abs into local buffer.
            let abs = value.unsigned_abs();
            tmp.set_u32(abs);
            let mut out = [0u8; DIGITAL_VALUE_CAP];
            out[0] = b'-';
            let digs = tmp.value_len as usize;
            let copy = digs.min(DIGITAL_VALUE_CAP - 1);
            out[1..1 + copy].copy_from_slice(&tmp.value[..copy]);
            self.value[..1 + copy].copy_from_slice(&out[..1 + copy]);
            self.value_len = (1 + copy) as u8;
        } else {
            self.set_u32(value as u32);
        }
    }

    /// Current stored value as a byte slice (ASCII supported chars or space).
    pub fn value_bytes(&self) -> &[u8] {
        &self.value[..self.value_len as usize]
    }

    /// Stable content size for `max_chars` cells (independent of current value).
    pub fn measure(&self) -> Size {
        measure_digital(
            self.max_chars as u32,
            self.digit_w,
            self.digit_h,
            self.spacing,
        )
    }

    /// Measure an explicit character count at the current digit size.
    pub fn measure_chars(&self, char_count: u32) -> Size {
        measure_digital(char_count, self.digit_w, self.digit_h, self.spacing)
    }

    fn refresh_geom(&mut self) {
        self.seg = self.seg.ensure(self.digit_w, self.digit_h);
    }

    /// Origin of the digit strip inside `rect` given alignment.
    fn content_origin(&self) -> Point {
        let size = self.measure();
        let dx = match self.align {
            DigitalAlign::Left => 0,
            DigitalAlign::Center => (self.rect.w as i32 - size.w as i32) / 2,
            DigitalAlign::Right => self.rect.w as i32 - size.w as i32,
        };
        let dy = (self.rect.h as i32 - size.h as i32) / 2;
        Point::new(self.rect.x + dx.max(0), self.rect.y + dy.max(0))
    }

    /// Draw into `canvas`. Uses theme accent / dim colors when overrides are unset.
    pub fn draw(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.refresh_geom();
        let fg = self.foreground.unwrap_or(theme.accent);
        let dim = self.inactive.unwrap_or_else(|| theme.panel_alt.lighten(18));

        let origin = self.content_origin();
        let cell_step = self.digit_w + self.spacing;
        let n = self.max_chars as usize;
        let shown = self.value_len as usize;

        for i in 0..n {
            let cx = origin.x + (i as i32) * cell_step as i32;
            let cy = origin.y;
            let ch = if i < shown {
                self.value[i] as char
            } else {
                ' '
            };
            draw_char_cell(canvas, cx, cy, ch, &self.seg, fg, dim);
        }
    }

    /// Immutable draw when geometry is already current (no cache update).
    pub fn draw_immutable(&self, canvas: &mut Canvas, theme: &Theme) {
        let fg = self.foreground.unwrap_or(theme.accent);
        let dim = self.inactive.unwrap_or_else(|| theme.panel_alt.lighten(18));
        let origin = self.content_origin();
        let cell_step = self.digit_w + self.spacing;
        let n = self.max_chars as usize;
        let shown = self.value_len as usize;
        for i in 0..n {
            let cx = origin.x + (i as i32) * cell_step as i32;
            let cy = origin.y;
            let ch = if i < shown {
                self.value[i] as char
            } else {
                ' '
            };
            draw_char_cell(canvas, cx, cy, ch, &self.seg, fg, dim);
        }
    }
}

/// Stable measurement independent of digit values.
pub fn measure_digital(char_count: u32, digit_w: u32, digit_h: u32, spacing: u32) -> Size {
    let n = char_count.max(1);
    let w = n * digit_w.max(4) + n.saturating_sub(1) * spacing;
    Size::new(w, digit_h.max(6))
}

/// True for digits, colon, decimal point, and minus.
#[inline]
pub fn is_supported_char(ch: char) -> bool {
    matches!(ch, '0'..='9' | ':' | '.' | '-')
}

/// Segment mask for a digit char, or `None` for non-digits.
pub fn digit_segment_mask(ch: char) -> Option<u8> {
    if ch.is_ascii_digit() {
        Some(DIGIT_SEGS[(ch as u8 - b'0') as usize])
    } else {
        None
    }
}

fn draw_char_cell(
    canvas: &mut Canvas,
    x: i32,
    y: i32,
    ch: char,
    seg: &SegmentGeom,
    fg: Color,
    dim: Color,
) {
    match ch {
        '0'..='9' => {
            let mask = DIGIT_SEGS[(ch as u8 - b'0') as usize];
            draw_segments(canvas, x, y, seg, mask, fg, dim);
        }
        '-' => {
            // Only middle bar lit.
            draw_segments(canvas, x, y, seg, SEG_G, fg, dim);
        }
        ':' => {
            // Two dots stacked; unlit segments stay off (no full ghost digit).
            let t = seg.thickness.max(1);
            let cx = x + seg.cell.w as i32 / 2 - t as i32 / 2;
            let top = y + seg.cell.h as i32 / 3 - t as i32 / 2;
            let bot = y + (2 * seg.cell.h as i32) / 3 - t as i32 / 2;
            canvas.fill_rect(Rect::new(cx, top, t, t), fg);
            canvas.fill_rect(Rect::new(cx, bot, t, t), fg);
        }
        '.' => {
            let t = seg.thickness.max(1).saturating_add(1);
            let px = x + seg.cell.w as i32 - t as i32 - 1;
            let py = y + seg.cell.h as i32 - t as i32;
            canvas.fill_rect(Rect::new(px, py, t, t), fg);
        }
        _ => {
            // Blank / unsupported: draw fully inactive segments for stable footprint.
            draw_segments(canvas, x, y, seg, 0, fg, dim);
        }
    }
}

fn draw_segments(
    canvas: &mut Canvas,
    ox: i32,
    oy: i32,
    seg: &SegmentGeom,
    mask: u8,
    fg: Color,
    dim: Color,
) {
    let paint = |canvas: &mut Canvas, r: Rect, on: bool| {
        let color = if on { fg } else { dim };
        canvas.fill_rect(Rect::new(ox + r.x, oy + r.y, r.w, r.h), color);
    };
    paint(canvas, seg.a, mask & SEG_A != 0);
    paint(canvas, seg.b, mask & SEG_B != 0);
    paint(canvas, seg.c, mask & SEG_C != 0);
    paint(canvas, seg.d, mask & SEG_D != 0);
    paint(canvas, seg.e, mask & SEG_E != 0);
    paint(canvas, seg.f, mask & SEG_F != 0);
    paint(canvas, seg.g, mask & SEG_G != 0);
}

// ── Host-side unit tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_supported_and_rejects_letters() {
        assert!(DigitalNumberWidget::is_supported_str("04:05"));
        assert!(DigitalNumberWidget::is_supported_str("-12.5"));
        assert!(DigitalNumberWidget::is_supported_str("100"));
        assert!(DigitalNumberWidget::validate_str("23:59").is_ok());
        assert_eq!(DigitalNumberWidget::validate_str("12a").err(), Some('a'));
        assert!(!DigitalNumberWidget::is_supported_str("hi"));
    }

    #[test]
    fn measurement_independent_of_value() {
        let mut w = DigitalNumberWidget::new(Rect::new(0, 0, 200, 40))
            .with_digit_size(12, 20)
            .with_spacing(3)
            .with_max_chars(5);
        let m0 = w.measure();
        w.set_value_str("1");
        let m1 = w.measure();
        w.set_value_str("23:59");
        let m2 = w.measure();
        assert_eq!(m0, m1);
        assert_eq!(m1, m2);
        assert_eq!(m0, measure_digital(5, 12, 20, 3));
        assert_eq!(m0.w, 5 * 12 + 4 * 3);
        assert_eq!(m0.h, 20);
    }

    #[test]
    fn leading_zeros_format() {
        let mut w = DigitalNumberWidget::new(Rect::new(0, 0, 100, 30))
            .with_max_chars(4)
            .with_leading_zeros(true);
        w.set_u32(5);
        assert_eq!(core::str::from_utf8(w.value_bytes()).unwrap(), "0005");
        w.set_u32(42);
        assert_eq!(core::str::from_utf8(w.value_bytes()).unwrap(), "0042");
        w.leading_zeros = false;
        w.set_u32(7);
        assert_eq!(core::str::from_utf8(w.value_bytes()).unwrap(), "7");
    }

    #[test]
    fn signed_and_decimal_strings() {
        let mut w = DigitalNumberWidget::new(Rect::new(0, 0, 120, 30)).with_max_chars(6);
        w.set_i32(-12);
        assert_eq!(core::str::from_utf8(w.value_bytes()).unwrap(), "-12");
        assert!(w.set_value_str("-12.5"));
        assert_eq!(core::str::from_utf8(w.value_bytes()).unwrap(), "-12.5");
        assert!(w.set_value_str("04:05"));
        assert_eq!(core::str::from_utf8(w.value_bytes()).unwrap(), "04:05");
    }

    #[test]
    fn unsupported_chars_become_blank_not_corrupt() {
        let mut w = DigitalNumberWidget::new(Rect::new(0, 0, 80, 24)).with_max_chars(3);
        let ok = w.set_value_str("1x2");
        assert!(!ok);
        assert_eq!(w.value_bytes(), b"1 2");
    }

    #[test]
    fn segment_masks_for_digits() {
        assert_eq!(digit_segment_mask('8'), Some(0x7F));
        assert_eq!(digit_segment_mask('1'), Some(SEG_B | SEG_C));
        assert!(digit_segment_mask(':').is_none());
    }

    #[test]
    fn draw_does_not_panic() {
        let theme = Theme::sunlight_dark();
        let mut pixels = vec![0u32; 200 * 60];
        let mut canvas = Canvas::new(&mut pixels, 200, 200, 60);
        let mut w = DigitalNumberWidget::new(Rect::new(10, 10, 180, 40))
            .with_digit_size(14, 24)
            .with_max_chars(5)
            .with_align(DigitalAlign::Center)
            .with_value("04:05");
        w.draw(&mut canvas, &theme);
        // Expect some non-zero pixels (accent-ish).
        assert!(pixels.iter().any(|&p| p != 0));
    }
}
