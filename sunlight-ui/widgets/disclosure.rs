//! Generic, allocation-free disclosure and property-list widgets.
//!
//! They deliberately accept only presentation data.  Callers own domain
//! state, formatting, IPC, focus traversal, and lifecycle.

use crate::{font::VecText, Canvas, Event, Point, Rect, Theme};

use super::{BadgeKind, StatusBadge};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisclosureEvent {
    None,
    Toggled,
}

/// State supplied by the owning application for deterministic focus and input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DisclosureState {
    pub expanded: bool,
    pub focused: bool,
}

/// An expandable card header.  Content is drawn separately into
/// [`content_rect`](Self::content_rect), keeping this widget independent of
/// any data model or layout framework.
pub struct DisclosureGroup<'a> {
    pub rect: Rect,
    pub title: &'a str,
    pub subtitle: Option<&'a str>,
    pub status: Option<(&'a str, BadgeKind)>,
    pub state: DisclosureState,
    /// Optional MiniType/vector font supplied by the application.
    pub font: Option<&'a dyn VecText>,
}

impl<'a> DisclosureGroup<'a> {
    pub const HEADER_HEIGHT: u32 = 42;

    pub fn new(rect: Rect, title: &'a str) -> Self {
        Self {
            rect,
            title,
            subtitle: None,
            status: None,
            state: DisclosureState::default(),
            font: None,
        }
    }

    pub fn with_subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    pub fn with_status(mut self, label: &'a str, kind: BadgeKind) -> Self {
        self.status = Some((label, kind));
        self
    }

    pub fn with_state(mut self, state: DisclosureState) -> Self {
        self.state = state;
        self
    }

    /// Use a Sunlight vector font instead of the early-boot bitmap fallback.
    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    pub fn header_rect(&self) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y,
            self.rect.w,
            Self::HEADER_HEIGHT.min(self.rect.h),
        )
    }

    pub fn content_rect(&self) -> Rect {
        Rect::new(
            self.rect.x + 10,
            self.rect.y + Self::HEADER_HEIGHT as i32,
            self.rect.w.saturating_sub(20),
            self.rect.h.saturating_sub(Self::HEADER_HEIGHT + 8),
        )
    }

    pub fn hit_test(&self, point: Point) -> bool {
        self.header_rect().contains(point)
    }

    pub fn handle_event(&self, event: Event, window_focused: bool) -> DisclosureEvent {
        match event {
            Event::Click { x, y } if window_focused && self.hit_test(Point::new(x, y)) => {
                DisclosureEvent::Toggled
            }
            Event::KeyPress {
                keycode: 0x1c | 0x39,
                pressed: true,
                ..
            } if self.state.focused => DisclosureEvent::Toggled,
            _ => DisclosureEvent::None,
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rounded_rect(self.rect, 8, theme.panel);
        // Keep every card outlined, including the focused one. Focus is a
        // restrained leading accent rather than a bright full-card glow.
        canvas.stroke_rounded_rect(self.rect, 8, 1, theme.border);
        let header = self.header_rect();
        canvas.fill_rect(header, theme.panel_alt);
        if self.state.focused {
            canvas.fill_rounded_rect(
                Rect::new(header.x + 3, header.y + 8, 3, header.h.saturating_sub(16)),
                2,
                theme.accent,
            );
        }
        draw_text(
            self.font,
            canvas,
            self.title,
            header.x + 12,
            header.y + 5,
            16,
            theme.text,
        );
        if let Some(subtitle) = self.subtitle {
            draw_text(
                self.font,
                canvas,
                subtitle,
                header.x + 12,
                header.y + 22,
                14,
                theme.text_dim,
            );
        }
        if let Some((status, kind)) = self.status {
            let status_width = measure_width(self.font, status) as i32 + 16;
            StatusBadge::new(header.right() - status_width - 16, header.y + 16, kind)
                .with_label(status)
                .with_font_if(self.font)
                .draw(canvas, theme);
        }
        draw_text(
            self.font,
            canvas,
            if self.state.expanded { "⌃" } else { "⌄" },
            header.right() - 16,
            header.y + 13,
            16,
            theme.text_dim,
        );
        if self.state.expanded {
            canvas.hbar(
                header.x + 8,
                header.bottom() - 1,
                header.w.saturating_sub(16),
                1,
                theme.border,
            );
        }
    }
}

/// Borrowed row data for [`PropertyGrid`]. Values are presentation text, not
/// editable fields. `height_for_width` is deterministic and allocation-free.
#[derive(Clone, Copy)]
pub struct PropertyRow<'a> {
    pub label: &'a str,
    pub value: &'a str,
}

pub struct PropertyGrid<'a> {
    pub rect: Rect,
    pub rows: &'a [PropertyRow<'a>],
    pub label_font: Option<&'a dyn VecText>,
    pub value_font: Option<&'a dyn VecText>,
}

impl<'a> PropertyGrid<'a> {
    pub const ROW_HEIGHT: u32 = 24;

    pub const fn new(rect: Rect, rows: &'a [PropertyRow<'a>]) -> Self {
        Self {
            rect,
            rows,
            label_font: None,
            value_font: None,
        }
    }

    /// Supply separate presentation fonts for labels and read-only values.
    pub fn with_fonts(mut self, label_font: &'a dyn VecText, value_font: &'a dyn VecText) -> Self {
        self.label_font = Some(label_font);
        self.value_font = Some(value_font);
        self
    }

    pub fn height_for_width(rows: &'a [PropertyRow<'a>], width: u32) -> u32 {
        let value_columns = if width < 300 { 1 } else { 2 };
        let values_per_line = (width / (value_columns * 7)).max(12) as usize;
        rows.iter().fold(0u32, |height, row| {
            let lines =
                ((row.value.chars().count() + values_per_line - 1) / values_per_line).max(1) as u32;
            height.saturating_add(Self::ROW_HEIGHT.saturating_mul(lines))
        })
    }

    pub fn row_rect(&self, index: usize) -> Option<Rect> {
        if index >= self.rows.len() {
            return None;
        }
        Some(Rect::new(
            self.rect.x,
            self.rect.y + index as i32 * Self::ROW_HEIGHT as i32,
            self.rect.w,
            Self::ROW_HEIGHT,
        ))
    }

    pub fn hit_test(&self, point: Point) -> Option<usize> {
        self.rows.iter().enumerate().find_map(|(index, _)| {
            self.row_rect(index)
                .filter(|rect| rect.contains(point))
                .map(|_| index)
        })
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let narrow = self.rect.w < 300;
        for (index, row) in self.rows.iter().enumerate() {
            let Some(rect) = self.row_rect(index) else {
                continue;
            };
            draw_text(
                self.label_font,
                canvas,
                row.label,
                rect.x,
                rect.y + 3,
                18,
                theme.text_dim,
            );
            if narrow {
                draw_text(
                    self.value_font,
                    canvas,
                    row.value,
                    rect.x,
                    rect.y + 15,
                    18,
                    theme.text,
                );
            } else {
                draw_text(
                    self.value_font,
                    canvas,
                    row.value,
                    rect.x + (rect.w as i32 / 2),
                    rect.y + 3,
                    18,
                    theme.text,
                );
            }
            if index + 1 < self.rows.len() {
                canvas.hbar(
                    rect.x,
                    rect.bottom() - 1,
                    rect.w,
                    1,
                    theme.border.darken(24),
                );
            }
        }
    }
}

fn draw_text(
    font: Option<&dyn VecText>,
    canvas: &mut Canvas,
    text: &str,
    x: i32,
    y: i32,
    height: u32,
    color: crate::Color,
) {
    if let Some(font) = font {
        font.draw_vcenter(canvas, text, x, y, height, color);
    } else {
        canvas.draw_text(x, y + 5, text, color);
    }
}

fn measure_width(font: Option<&dyn VecText>, text: &str) -> u32 {
    font.map_or_else(|| Canvas::measure_text(text), |font| font.measure_w(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disclosure_toggles_from_keyboard_and_only_focused_clicks() {
        let group =
            DisclosureGroup::new(Rect::new(0, 0, 200, 80), "Title").with_state(DisclosureState {
                expanded: false,
                focused: true,
            });
        assert_eq!(
            group.handle_event(
                Event::key_press(0x1c, true, false, false, false, false),
                true
            ),
            DisclosureEvent::Toggled
        );
        assert_eq!(
            group.handle_event(Event::click(10, 10), false),
            DisclosureEvent::None
        );
    }

    #[test]
    fn property_grid_measurement_and_hitboxes_are_bounded() {
        let rows = [PropertyRow {
            label: "Address",
            value: "192.0.2.1/24",
        }];
        assert!(PropertyGrid::height_for_width(&rows, 160) >= PropertyGrid::ROW_HEIGHT);
        let grid = PropertyGrid::new(Rect::new(0, 0, 200, 24), &rows);
        assert_eq!(grid.hit_test(Point::new(4, 4)), Some(0));
    }
}
