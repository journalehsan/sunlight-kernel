//! Measured, grapheme-safe one/two-line labels using the existing font renderer.
use alloc::string::String;
use alloc::vec::Vec;
use unicode_segmentation::UnicodeSegmentation;

use super::VecText;
use crate::{theme::Color, Canvas, Rect};

#[derive(Debug)]
pub struct LabelLayout {
    pub lines: [String; 2],
    pub widths: [u32; 2],
    pub line_count: usize,
}

fn fitting_end(text: &str, width: u32, font: &dyn VecText) -> usize {
    let mut end = 0;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let next = offset + grapheme.len();
        if font.measure_w(&text[..next]) > width {
            break;
        }
        end = next;
    }
    end
}

fn line_end(text: &str, width: u32, font: &dyn VecText) -> usize {
    let end = fitting_end(text, width, font);
    text[..end]
        .grapheme_indices(true)
        .rev()
        .find(|(_, g)| {
            g.chars()
                .any(|c| c.is_whitespace() || matches!(c, '_' | '-' | '.'))
        })
        .map_or(end, |(offset, g)| offset + g.len())
}

/// Full measured wrapping for a filename tooltip. Allocation is bounded by the
/// input's grapheme count. An over-wide cluster stays intact for clipped drawing.
pub fn wrap_label(mut text: &str, width: u32, font: &dyn VecText) -> Vec<String> {
    let mut lines = Vec::new();
    while !text.is_empty() {
        let end = if font.measure_w(text) <= width {
            text.len()
        } else {
            let end = line_end(text, width, font);
            if end == 0 {
                text.graphemes(true).next().unwrap().len()
            } else {
                end
            }
        };
        lines.push(String::from(&text[..end]));
        text = &text[end..];
    }
    lines
}

fn ellipsize(text: &str, width: u32, font: &dyn VecText) -> String {
    if font.measure_w(text) <= width {
        return String::from(text);
    }
    let mut result = String::from("…");
    if font.measure_w(&result) > width {
        return String::new();
    }
    // Measure the complete candidate, so font/cluster behavior is respected.
    for (offset, grapheme) in text.grapheme_indices(true) {
        let end = offset + grapheme.len();
        let mut candidate = String::from(&text[..end]);
        candidate.push('…');
        if font.measure_w(&candidate) > width {
            break;
        }
        result = candidate;
    }
    result
}

impl LabelLayout {
    /// At most two lines. Break after filename separators where possible;
    /// otherwise break at extended grapheme boundaries, including emoji/marks.
    /// An over-wide first cluster falls back to an ellipsis without splitting it.
    pub fn new(text: &str, width: u32, max_lines: usize, font: &dyn VecText) -> Self {
        let mut lines = [String::new(), String::new()];
        let mut line_count = 1;
        if max_lines < 2 || font.measure_w(text) <= width {
            lines[0] = ellipsize(text, width, font);
        } else {
            let end = line_end(text, width, font);
            if end == 0 {
                lines[0] = ellipsize(text, width, font);
            } else {
                lines[0] = String::from(&text[..end]);
                lines[1] = ellipsize(&text[end..], width, font);
                line_count = 2;
            }
        }
        let widths = [font.measure_w(&lines[0]), font.measure_w(&lines[1])];
        Self {
            lines,
            widths,
            line_count,
        }
    }

    /// Clip actual glyph ink (including overhang) to the label's rectangle.
    /// Cached widths eliminate measurement during painting.
    pub fn draw(
        &self,
        canvas: &mut Canvas,
        rect: Rect,
        font: &dyn VecText,
        color: Color,
        centered: bool,
    ) {
        let left = rect.x.max(0);
        let top = rect.y.max(0);
        let right = rect.right().min(canvas.width as i32);
        let bottom = rect.bottom().min(canvas.height as i32);
        if right <= left || bottom <= top {
            return;
        }
        let start = top as usize * canvas.stride as usize + left as usize;
        let mut clip = Canvas::new(
            &mut canvas.pixels[start..],
            canvas.stride,
            (right - left) as u32,
            (bottom - top) as u32,
        );
        for i in 0..self.line_count {
            let x = rect.x - left
                + if centered {
                    (rect.w.saturating_sub(self.widths[i]) / 2) as i32
                } else {
                    0
                };
            font.draw(
                &mut clip,
                &self.lines[i],
                x,
                rect.y - top + (i as u32 * font.line_height()) as i32,
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Metrics;
    impl VecText for Metrics {
        fn measure_w(&self, text: &str) -> u32 {
            text.chars()
                .map(|c| match c {
                    'W' => 11,
                    'i' => 2,
                    _ => 5,
                })
                .sum()
        }
        fn line_height(&self) -> u32 {
            8
        }
        fn draw(&self, canvas: &mut Canvas, _text: &str, x: i32, y: i32, color: Color) -> i32 {
            // Deliberate ink overhang to test the label's hard clipping boundary.
            canvas.fill_rect(Rect::new(x - 5, y - 5, 100, 100), color);
            x
        }
        fn draw_vcenter(
            &self,
            canvas: &mut Canvas,
            text: &str,
            x: i32,
            y: i32,
            _height: u32,
            color: Color,
        ) -> i32 {
            self.draw(canvas, text, x, y, color)
        }
    }

    #[test]
    fn short_names_and_proportional_metrics() {
        let short = LabelLayout::new("a.txt", 30, 2, &Metrics);
        assert_eq!(short.lines[0], "a.txt");
        assert_eq!(short.line_count, 1);
        assert_eq!(LabelLayout::new("iiii", 12, 2, &Metrics).line_count, 1);
        assert_eq!(LabelLayout::new("WWWW", 12, 2, &Metrics).line_count, 2);
    }

    #[test]
    fn full_tooltip_wrap_retains_every_grapheme() {
        let text = "very_long_فایل_👩🏽‍💻_e\u{301}.tar.gz";
        for width in [0, 1, 10, 25, 50, 200] {
            let lines = wrap_label(text, width, &Metrics);
            assert_eq!(lines.concat(), text);
            assert!(lines.len() <= text.graphemes(true).count());
        }
    }

    #[test]
    fn two_lines_wrap_at_filename_boundaries() {
        for text in ["abc_def", "abc-def", "abc.def", "abc def"] {
            let l = LabelLayout::new(text, 25, 2, &Metrics);
            assert_eq!(l.line_count, 2);
            assert_eq!(l.lines[0], &text[..4]);
            assert_eq!(l.lines[1], &text[4..]);
        }
    }

    #[test]
    fn ellipsis_and_unseparated_names() {
        let l = LabelLayout::new("abcdefghijklmnopqrstuvwxyz", 25, 2, &Metrics);
        assert_eq!(l.lines, ["abcde", "fghi…"]);
        let l = LabelLayout::new("abcdef", 20, 1, &Metrics);
        assert_eq!(l.lines[0], "abc…");
        assert!(l.lines[1].is_empty());
        assert!(LabelLayout::new("abcdef", 1, 2, &Metrics).lines[0].is_empty());
    }

    #[test]
    fn extended_graphemes_are_never_split() {
        for cluster in ["e\u{301}", "👩🏽‍💻", "🇮🇷", "क्\u{200d}ष", "\r\n"] {
            let text = alloc::format!("a{cluster}bbbbb");
            // Only 'a' fits on line one; the whole second cluster does not.
            let l = LabelLayout::new(&text, 9, 2, &Metrics);
            assert_eq!(l.lines[0], "a");
            assert_eq!(l.lines[1], "…");
        }
    }

    #[test]
    fn unicode_mixed_names_and_narrow_widths() {
        for text in [
            "فایل_خیلی_طولانی.png",
            "abc_فارسی_123.tar.gz",
            "😀e\u{301}filename",
            "",
            "a.b.c.d",
        ] {
            for width in 0..100 {
                let l = LabelLayout::new(text, width, 2, &Metrics);
                assert!(l.line_count <= 2);
                assert!(l.widths.iter().all(|w| *w <= width));
                for line in &l.lines {
                    let prefix = line.strip_suffix('…').unwrap_or(line);
                    if !prefix.is_empty() {
                        let start = if text.starts_with(prefix) {
                            0
                        } else {
                            l.lines[0].len()
                        };
                        let end = start + prefix.len();
                        assert!(
                            end == text.len()
                                || text.grapheme_indices(true).any(|(offset, _)| offset == end)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ink_is_clipped_with_stride_and_partial_scrolling() {
        for rect in [Rect::new(7, 5, 12, 10), Rect::new(-3, -2, 12, 10)] {
            let mut pixels = alloc::vec![0; 40 * 30];
            let mut canvas = Canvas::new(&mut pixels, 40, 30, 30);
            LabelLayout::new("W", 12, 2, &Metrics).draw(
                &mut canvas,
                rect,
                &Metrics,
                Color(0xffffffff),
                true,
            );
            for y in 0..30 {
                for x in 0..40 {
                    if x >= 30 || !rect.contains(crate::Point::new(x, y)) {
                        assert_eq!(canvas.pixels[y as usize * 40 + x as usize], 0);
                    }
                }
            }
            assert!(canvas.pixels.iter().any(|p| *p != 0));
        }
    }
}
