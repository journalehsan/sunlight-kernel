use crate::event::Event;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderOrientation {
    Horizontal,
    Vertical,
}

pub struct Slider {
    pub rect: Rect,
    pub min: u32,
    pub max: u32,
    pub value: u32,
    pub orientation: SliderOrientation,
    pub active: bool,
    pub dragging: bool,
}

impl Slider {
    const TRACK_THICKNESS: u32 = 4;
    const THUMB_SIZE: u32 = 12;
    const PADDING: i32 = 4;

    pub const fn new(rect: Rect) -> Self {
        Self {
            rect,
            min: 0,
            max: 100,
            value: 0,
            orientation: SliderOrientation::Horizontal,
            active: false,
            dragging: false,
        }
    }

    pub const fn horizontal(rect: Rect) -> Self {
        Self::new(rect)
    }

    pub const fn vertical(rect: Rect) -> Self {
        Self {
            orientation: SliderOrientation::Vertical,
            ..Self::new(rect)
        }
    }

    pub fn with_orientation(mut self, orientation: SliderOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn with_range(mut self, min: u32, max: u32) -> Self {
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        self.min = min;
        self.max = max;
        self.value = self.value.clamp(self.min_value(), self.max_value());
        self
    }

    pub fn with_value(mut self, value: u32) -> Self {
        self.value = value.clamp(self.min_value(), self.max_value());
        self
    }

    pub fn set_range(&mut self, min: u32, max: u32) {
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        self.min = min;
        self.max = max;
        self.value = self.value.clamp(self.min_value(), self.max_value());
    }

    pub fn set_value(&mut self, value: u32) {
        self.value = value.clamp(self.min_value(), self.max_value());
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.rect.w == 0 || self.rect.h == 0 {
            return;
        }

        canvas.fill_rect(self.rect, theme.panel);
        canvas.draw_rect(
            self.rect,
            if self.active || self.dragging {
                theme.accent
            } else {
                theme.border
            },
        );

        match self.orientation {
            SliderOrientation::Horizontal => self.draw_horizontal(canvas, theme),
            SliderOrientation::Vertical => self.draw_vertical(canvas, theme),
        }
    }

    pub fn update(&mut self, event: Event) -> bool {
        match event {
            Event::MouseDown { x, y, button } if button == 0 => {
                if self.rect.contains(Point::new(x, y)) {
                    self.active = true;
                    self.dragging = true;
                    let _ = self.set_from_pointer(x, y);
                    return true;
                }
                self.active = false;
                false
            }
            Event::MouseMove { x, y } => {
                let hovered = self.rect.contains(Point::new(x, y));
                let mut changed = false;
                if self.dragging {
                    changed = self.set_from_pointer(x, y);
                }
                if self.active != hovered && !self.dragging {
                    self.active = hovered;
                    return true;
                }
                changed
            }
            Event::MouseUp { x, y, button } if button == 0 => {
                let was_dragging = self.dragging;
                self.dragging = false;
                let hovered = self.rect.contains(Point::new(x, y));
                let changed = self.active != hovered;
                self.active = hovered;
                was_dragging || changed
            }
            Event::Click { x, y } => {
                if self.rect.contains(Point::new(x, y)) {
                    self.active = true;
                    self.dragging = false;
                    let _ = self.set_from_pointer(x, y);
                    return true;
                }
                self.active = false;
                false
            }
            _ => false,
        }
    }

    fn min_value(&self) -> u32 {
        self.min.min(self.max)
    }

    fn max_value(&self) -> u32 {
        self.min.max(self.max)
    }

    fn range(&self) -> u32 {
        self.max_value().saturating_sub(self.min_value())
    }

    fn normalized_value(&self) -> f32 {
        let min = self.min_value();
        let range = self.range();
        if range == 0 {
            0.5
        } else {
            (self.value.clamp(min, self.max_value()) - min) as f32 / range as f32
        }
    }

    fn thumb_size(&self) -> u32 {
        self.rect.w.min(self.rect.h).min(Self::THUMB_SIZE).max(1)
    }

    fn set_from_pointer(&mut self, x: i32, y: i32) -> bool {
        let next = self.pointer_value(x, y);
        if next != self.value {
            self.value = next;
            true
        } else {
            false
        }
    }

    fn pointer_value(&self, x: i32, y: i32) -> u32 {
        let min = self.min_value();
        let max = self.max_value();
        let range = max.saturating_sub(min);
        if range == 0 {
            return min;
        }

        let thumb = self.thumb_size() as i32;
        let pad = Self::PADDING;

        let t = match self.orientation {
            SliderOrientation::Horizontal => {
                let start = self.rect.x + pad + thumb / 2;
                let end = self.rect.right() - pad - thumb / 2 - 1;
                let span = (end - start).max(0);
                if span == 0 {
                    0.5
                } else {
                    let px = x.clamp(start, end);
                    (px - start) as f32 / span as f32
                }
            }
            SliderOrientation::Vertical => {
                let start = self.rect.bottom() - pad - thumb / 2 - 1;
                let end = self.rect.y + pad + thumb / 2;
                let span = (start - end).max(0);
                if span == 0 {
                    0.5
                } else {
                    let py = y.clamp(end, start);
                    (start - py) as f32 / span as f32
                }
            }
        };

        min + (t.clamp(0.0, 1.0) * range as f32) as u32
    }

    fn draw_horizontal(&self, canvas: &mut Canvas, theme: &Theme) {
        let thumb = self.thumb_size();
        let pad = Self::PADDING;
        let min = self.min_value();
        let max = self.max_value();
        let range = max.saturating_sub(min);
        let t = self.normalized_value();

        let thumb_cx = if range == 0 {
            self.rect.x + self.rect.w as i32 / 2
        } else {
            let start = self.rect.x + pad + thumb as i32 / 2;
            let end = self.rect.right() - pad - thumb as i32 / 2 - 1;
            start + ((end - start).max(0) as f32 * t) as i32
        };
        let thumb_rect = Rect::new(
            thumb_cx - thumb as i32 / 2,
            self.rect.y + self.rect.h as i32 / 2 - thumb as i32 / 2,
            thumb,
            thumb,
        );
        let track_y = self.rect.y + self.rect.h as i32 / 2 - Self::TRACK_THICKNESS as i32 / 2;
        let track_left = self.rect.x + pad + thumb as i32 / 2;
        let track_right = self.rect.right() - pad - thumb as i32 / 2 - 1;
        let track_w = (track_right - track_left + 1).max(0) as u32;
        if track_w > 0 {
            canvas.fill_rect(
                Rect::new(track_left, track_y, track_w, Self::TRACK_THICKNESS),
                theme.panel_alt,
            );
            let fill_w = (thumb_cx - track_left).max(0) as u32;
            if fill_w > 0 {
                canvas.fill_rect(
                    Rect::new(track_left, track_y, fill_w, Self::TRACK_THICKNESS),
                    if self.dragging {
                        theme.accent_hover
                    } else {
                        theme.accent
                    },
                );
            }
        }

        canvas.fill_rounded_rect_with_border(
            thumb_rect,
            4,
            theme.panel_alt,
            if self.dragging || self.active {
                theme.accent_hover
            } else {
                theme.border
            },
            1,
        );
    }

    fn draw_vertical(&self, canvas: &mut Canvas, theme: &Theme) {
        let thumb = self.thumb_size();
        let pad = Self::PADDING;
        let min = self.min_value();
        let max = self.max_value();
        let range = max.saturating_sub(min);
        let t = self.normalized_value();

        let thumb_cy = if range == 0 {
            self.rect.y + self.rect.h as i32 / 2
        } else {
            let start = self.rect.bottom() - pad - thumb as i32 / 2 - 1;
            let end = self.rect.y + pad + thumb as i32 / 2;
            start - ((start - end).max(0) as f32 * t) as i32
        };
        let thumb_rect = Rect::new(
            self.rect.x + self.rect.w as i32 / 2 - thumb as i32 / 2,
            thumb_cy - thumb as i32 / 2,
            thumb,
            thumb,
        );
        let track_x = self.rect.x + self.rect.w as i32 / 2 - Self::TRACK_THICKNESS as i32 / 2;
        let track_top = self.rect.y + pad + thumb as i32 / 2;
        let track_bottom = self.rect.bottom() - pad - thumb as i32 / 2 - 1;
        let track_h = (track_bottom - track_top + 1).max(0) as u32;
        if track_h > 0 {
            canvas.fill_rect(
                Rect::new(track_x, track_top, Self::TRACK_THICKNESS, track_h),
                theme.panel_alt,
            );
            let fill_top = thumb_cy;
            let fill_h = (track_bottom - fill_top + 1).max(0) as u32;
            if fill_h > 0 {
                canvas.fill_rect(
                    Rect::new(track_x, fill_top, Self::TRACK_THICKNESS, fill_h),
                    if self.dragging {
                        theme.accent_hover
                    } else {
                        theme.accent
                    },
                );
            }
        }

        canvas.fill_rounded_rect_with_border(
            thumb_rect,
            4,
            theme.panel_alt,
            if self.dragging || self.active {
                theme.accent_hover
            } else {
                theme.border
            },
            1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_value_to_range() {
        let slider = Slider::new(Rect::new(0, 0, 120, 20))
            .with_range(20, 80)
            .with_value(100);
        assert_eq!(slider.value, 80);
    }

    #[test]
    fn horizontal_click_updates_value() {
        let mut slider = Slider::horizontal(Rect::new(0, 0, 120, 20)).with_range(0, 100);

        assert!(slider.update(Event::click(0, 10)));
        assert_eq!(slider.value, 0);

        assert!(slider.update(Event::click(119, 10)));
        assert_eq!(slider.value, 100);
    }

    #[test]
    fn vertical_click_updates_value() {
        let mut slider = Slider::vertical(Rect::new(0, 0, 20, 120)).with_range(0, 100);

        assert!(slider.update(Event::click(10, 0)));
        assert_eq!(slider.value, 100);

        assert!(slider.update(Event::click(10, 119)));
        assert_eq!(slider.value, 0);
    }
}
