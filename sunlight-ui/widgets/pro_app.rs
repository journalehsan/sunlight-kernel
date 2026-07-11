use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::image::TgaImage;
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

const HEADER_BUTTON_RADIUS: u32 = 8;
const CHIP_RADIUS: u32 = 12;
const RIBBON_GROUP_RADIUS: u32 = 12;
const RIBBON_BUTTON_RADIUS: u32 = 8;
fn fill_vertical_gradient(canvas: &mut Canvas, rect: Rect, top: Color, bottom: Color) {
    let h = rect.h.max(1);
    for row in 0..h {
        let mix = row * 255 / h;
        let r = ((top.r() as u32 * (255 - mix) + bottom.r() as u32 * mix) / 255) as u8;
        let g = ((top.g() as u32 * (255 - mix) + bottom.g() as u32 * mix) / 255) as u8;
        let b = ((top.b() as u32 * (255 - mix) + bottom.b() as u32 * mix) / 255) as u8;
        canvas.fill_rect(
            Rect::new(rect.x, rect.y + row as i32, rect.w, 1),
            Color::rgb(r, g, b),
        );
    }
}

fn draw_text(
    canvas: &mut Canvas,
    font: Option<&dyn VecText>,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
) {
    if let Some(font) = font {
        font.draw(canvas, text, x, y, color);
    } else {
        canvas.draw_text(x, y, text, color);
    }
}

fn draw_text_vcenter(
    canvas: &mut Canvas,
    font: Option<&dyn VecText>,
    x: i32,
    y: i32,
    h: u32,
    text: &str,
    color: Color,
) {
    if let Some(font) = font {
        font.draw_vcenter(canvas, text, x, y, h, color);
    } else {
        let ty = y + (h as i32 - crate::paint::font::GLYPH_H as i32) / 2;
        canvas.draw_text(x, ty, text, color);
    }
}

fn measure_text_width(font: Option<&dyn VecText>, text: &str) -> u32 {
    font.map(|font| font.measure_w(text))
        .unwrap_or_else(|| Canvas::measure_text(text))
}

fn draw_dropdown_arrow(canvas: &mut Canvas, x: i32, cy: i32, color: Color) {
    canvas.put_pixel(x, cy - 2, color);
    canvas.put_pixel(x + 1, cy - 1, color);
    canvas.put_pixel(x + 2, cy, color);
    canvas.put_pixel(x + 3, cy - 1, color);
    canvas.put_pixel(x + 4, cy - 2, color);
}

#[derive(Clone, Copy)]
pub struct HeaderChip<'a> {
    pub label: &'a str,
    pub icon: Option<&'a TgaImage>,
    pub width: u32,
    pub accent_outline: bool,
}

#[derive(Clone, Copy)]
pub struct HeaderActionButton<'a> {
    pub rect: Rect,
    pub icon: &'a TgaImage,
    pub active: bool,
    pub hovered: bool,
}

pub struct PremiumHeader<'a> {
    pub rect: Rect,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub leading_button: Option<HeaderActionButton<'a>>,
    pub chips: &'a [HeaderChip<'a>],
    pub hovered_chip: Option<usize>,
    pub title_font: Option<&'a dyn VecText>,
    pub subtitle_font: Option<&'a dyn VecText>,
    pub chip_font: Option<&'a dyn VecText>,
}

impl<'a> PremiumHeader<'a> {
    pub fn chip_rect(&self, idx: usize) -> Rect {
        let mut x = self.rect.right() - 24;
        for chip in self.chips[..=idx].iter().rev() {
            x -= chip.width as i32;
            if !core::ptr::eq(chip, &self.chips[idx]) {
                x -= 8;
            }
        }
        Rect::new(x, self.rect.y + 12, self.chips[idx].width, 28)
    }

    pub fn chip_hit(&self, point: Point) -> Option<usize> {
        self.chips
            .iter()
            .enumerate()
            .find_map(|(idx, _)| self.chip_rect(idx).contains(point).then_some(idx))
    }

    pub fn leading_button_hit(&self, point: Point) -> bool {
        self.leading_button
            .map(|button| button.rect.contains(point))
            .unwrap_or(false)
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        fill_vertical_gradient(
            canvas,
            self.rect,
            theme.panel.lighten(10),
            theme.panel.darken(24),
        );
        canvas.hbar(
            self.rect.x,
            self.rect.bottom() - 1,
            self.rect.w,
            1,
            theme.border,
        );
        canvas.hbar(
            self.rect.x,
            self.rect.bottom() - 2,
            self.rect.w,
            1,
            theme.accent.darken(110),
        );

        if let Some(button) = self.leading_button {
            let fill = if button.active {
                theme.accent.darken(42)
            } else if button.hovered {
                theme.panel_alt.lighten(18)
            } else {
                theme.panel_alt
            };
            canvas.fill_rounded_rect(button.rect, HEADER_BUTTON_RADIUS, fill);
            canvas.stroke_rounded_rect(
                button.rect,
                HEADER_BUTTON_RADIUS,
                1,
                if button.active {
                    theme.accent
                } else {
                    theme.border
                },
            );
            canvas.draw_tga_icon_tinted(
                button.icon,
                Rect::new(button.rect.x + 10, button.rect.y + 8, 20, 20),
                if button.active {
                    theme.text_on_accent
                } else {
                    theme.icon_foreground
                },
            );
        }

        let text_x = self
            .leading_button
            .map(|button| button.rect.right() + 12)
            .unwrap_or(self.rect.x + 14);
        draw_text(
            canvas,
            self.title_font,
            text_x,
            self.rect.y + 7,
            self.title,
            theme.text,
        );
        draw_text(
            canvas,
            self.subtitle_font,
            text_x,
            self.rect.y + 26,
            self.subtitle,
            theme.text_dim,
        );

        for (idx, chip) in self.chips.iter().enumerate() {
            let rect = self.chip_rect(idx);
            let hovered = self.hovered_chip == Some(idx);
            canvas.fill_rounded_rect(
                rect,
                CHIP_RADIUS,
                if hovered {
                    theme.panel_alt.lighten(14)
                } else {
                    theme.panel
                },
            );
            canvas.stroke_rounded_rect(
                rect,
                CHIP_RADIUS,
                1,
                if hovered {
                    theme.accent.darken(72)
                } else if chip.accent_outline {
                    theme.accent.darken(90)
                } else {
                    theme.border
                },
            );
            if let Some(icon) = chip.icon {
                canvas.draw_tga_icon_tinted(
                    icon,
                    Rect::new(rect.x + 8, rect.y + 6, 16, 16),
                    theme.accent,
                );
            }
            let text_x = if chip.icon.is_some() {
                rect.x + 28
            } else {
                rect.x + 12
            };
            draw_text_vcenter(
                canvas,
                self.chip_font,
                text_x,
                rect.y,
                rect.h,
                chip.label,
                if chip.accent_outline {
                    theme.text
                } else {
                    theme.text_muted
                },
            );
        }
    }
}

#[derive(Clone, Copy)]
pub struct AppMenuCommand<'a> {
    pub label: &'a str,
    pub icon: Option<&'a TgaImage>,
    pub has_secondary: bool,
}

#[derive(Clone, Copy)]
pub struct AppMenuSecondaryItem<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub icon: Option<&'a TgaImage>,
}

pub struct TwoPaneAppMenu<'a> {
    pub rect: Rect,
    pub left_width: u32,
    pub right_width: u32,
    pub header_title: &'a str,
    pub header_subtitle: &'a str,
    pub secondary_title: &'a str,
    pub secondary_subtitle: &'a str,
    pub commands: &'a [AppMenuCommand<'a>],
    pub secondary_items: &'a [AppMenuSecondaryItem<'a>],
    pub active_command: Option<usize>,
    pub active_secondary: Option<usize>,
    pub show_secondary: bool,
    pub title_font: Option<&'a dyn VecText>,
    pub label_font: Option<&'a dyn VecText>,
    pub small_font: Option<&'a dyn VecText>,
}

impl<'a> TwoPaneAppMenu<'a> {
    pub fn left_rect(&self) -> Rect {
        Rect::new(self.rect.x, self.rect.y, self.left_width, self.rect.h)
    }

    pub fn right_rect(&self) -> Rect {
        let left = self.left_rect();
        Rect::new(left.right(), left.y, self.right_width, left.h)
    }

    pub fn command_rect(&self, idx: usize) -> Rect {
        let left = self.left_rect();
        Rect::new(
            left.x + 8,
            left.y + 38 + idx as i32 * 30,
            self.left_width - 16,
            30,
        )
    }

    pub fn secondary_rect(&self, idx: usize) -> Rect {
        let right = self.right_rect();
        Rect::new(
            right.x + 10,
            right.y + 38 + idx as i32 * 42,
            right.w - 20,
            38,
        )
    }

    pub fn command_hit(&self, point: Point) -> Option<usize> {
        self.commands
            .iter()
            .enumerate()
            .find_map(|(idx, _)| self.command_rect(idx).contains(point).then_some(idx))
    }

    pub fn secondary_hit(&self, point: Point) -> Option<usize> {
        if !self.show_secondary {
            return None;
        }
        self.secondary_items
            .iter()
            .enumerate()
            .find_map(|(idx, _)| self.secondary_rect(idx).contains(point).then_some(idx))
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rounded_rect(self.rect, 14, theme.panel.lighten(4));
        canvas.stroke_rounded_rect(self.rect, 14, 1, theme.border);

        let left = self.left_rect();
        canvas.fill_rect(left, theme.panel.lighten(4));
        draw_text(
            canvas,
            self.title_font,
            left.x + 12,
            left.y + 10,
            self.header_title,
            theme.text,
        );
        draw_text(
            canvas,
            self.small_font,
            left.x + 12,
            left.y + 22,
            self.header_subtitle,
            theme.text_dim,
        );
        canvas.fill_rect(Rect::new(left.x + 8, left.y + 8, 4, 18), theme.accent);

        for (idx, command) in self.commands.iter().enumerate() {
            let rect = self.command_rect(idx);
            let active = self.active_command == Some(idx);
            canvas.fill_rounded_rect(
                rect,
                8,
                if active {
                    theme.accent.darken(34)
                } else if idx % 2 == 0 {
                    theme.panel_alt
                } else {
                    theme.panel
                },
            );
            if active {
                canvas.stroke_rounded_rect(rect, 8, 1, theme.accent);
            }
            if let Some(icon) = command.icon {
                canvas.draw_tga_icon_tinted(
                    icon,
                    Rect::new(rect.x + 8, rect.y + 6, 18, 18),
                    if active {
                        theme.accent_hover
                    } else {
                        theme.icon_foreground
                    },
                );
            }
            draw_text_vcenter(
                canvas,
                self.label_font,
                rect.x + 34,
                rect.y,
                rect.h,
                command.label,
                if active { theme.text } else { theme.text_muted },
            );
            if command.has_secondary {
                draw_text_vcenter(
                    canvas,
                    self.label_font,
                    rect.right() - 18,
                    rect.y,
                    rect.h,
                    ">",
                    theme.text_dim,
                );
            }
        }

        if !self.show_secondary {
            return;
        }

        let right = self.right_rect();
        canvas.fill_rect(right, theme.panel_alt.lighten(6));
        canvas.vline(right.x, right.y + 10, right.h - 20, theme.border);
        draw_text(
            canvas,
            self.title_font,
            right.x + 16,
            right.y + 10,
            self.secondary_title,
            theme.text,
        );
        draw_text(
            canvas,
            self.small_font,
            right.x + 16,
            right.y + 22,
            self.secondary_subtitle,
            theme.text_dim,
        );

        for (idx, item) in self.secondary_items.iter().enumerate() {
            let rect = self.secondary_rect(idx);
            let hovered = self.active_secondary == Some(idx);
            canvas.fill_rounded_rect(
                rect,
                8,
                if hovered {
                    theme.panel.lighten(10)
                } else {
                    theme.panel
                },
            );
            canvas.stroke_rounded_rect(
                rect,
                8,
                1,
                if hovered {
                    theme.accent.darken(70)
                } else {
                    theme.border
                },
            );
            if let Some(icon) = item.icon {
                canvas.draw_tga_icon_tinted(
                    icon,
                    Rect::new(rect.x + 8, rect.y + 10, 16, 16),
                    if hovered {
                        theme.accent
                    } else {
                        theme.icon_muted
                    },
                );
            }
            draw_text(
                canvas,
                self.label_font,
                rect.x + 30,
                rect.y + 8,
                item.title,
                theme.text,
            );
            draw_text(
                canvas,
                self.small_font,
                rect.x + 30,
                rect.y + 22,
                item.subtitle,
                theme.text_dim,
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RibbonButtonKind {
    Dropdown,
    Toggle,
    IconButton,
    WideButton,
}

#[derive(Clone, Copy)]
pub struct RibbonButtonSpec<'a> {
    pub label: &'a str,
    pub icon: Option<&'a TgaImage>,
    pub width: u32,
    pub kind: RibbonButtonKind,
    pub row: u8,
}

pub struct RibbonGroupSpec<'a> {
    pub title: &'a str,
    pub buttons: &'a [RibbonButtonSpec<'a>],
}

impl<'a> RibbonGroupSpec<'a> {
    fn row_width(&self, row: u8) -> u32 {
        let mut width = 0;
        let mut count = 0u32;
        for button in self.buttons {
            if button.row == row {
                width += button.width;
                count += 1;
            }
        }
        if count > 1 {
            width + (count - 1) * 8
        } else {
            width
        }
    }

    pub fn required_width(&self) -> u32 {
        self.row_width(0).max(self.row_width(1)).max(80) + 24
    }

    pub fn button_rect(&self, group_rect: Rect, idx: usize) -> Rect {
        let target = self.buttons[idx];
        let mut x = group_rect.x + 12;
        for button in &self.buttons[..idx] {
            if button.row == target.row {
                x += button.width as i32 + 8;
            }
        }
        let y = group_rect.y + 12 + target.row as i32 * 40;
        Rect::new(x, y, target.width, 32)
    }

    pub fn hit_test(&self, group_rect: Rect, point: Point) -> Option<usize> {
        self.buttons.iter().enumerate().find_map(|(idx, _)| {
            self.button_rect(group_rect, idx)
                .contains(point)
                .then_some(idx)
        })
    }

    pub fn draw(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        group_rect: Rect,
        hovered_button: Option<usize>,
        label_font: Option<&dyn VecText>,
        small_font: Option<&dyn VecText>,
    ) {
        canvas.fill_rounded_rect(group_rect, RIBBON_GROUP_RADIUS, theme.panel.lighten(4));
        canvas.stroke_rounded_rect(group_rect, RIBBON_GROUP_RADIUS, 1, theme.border);
        canvas.hbar(
            group_rect.x + 10,
            group_rect.bottom() - 24,
            group_rect.w - 20,
            1,
            theme.border,
        );

        for (idx, button) in self.buttons.iter().enumerate() {
            let rect = self.button_rect(group_rect, idx);
            let hovered = hovered_button == Some(idx);
            let fill = match button.kind {
                RibbonButtonKind::Dropdown => theme.panel_alt.lighten(6),
                RibbonButtonKind::Toggle => {
                    if hovered {
                        theme.accent.darken(36)
                    } else {
                        theme.panel_alt
                    }
                }
                RibbonButtonKind::IconButton | RibbonButtonKind::WideButton => {
                    if hovered {
                        theme.panel_alt.lighten(18)
                    } else {
                        theme.panel_alt
                    }
                }
            };
            let border = if hovered {
                theme.accent.darken(70)
            } else {
                theme.border
            };
            canvas.fill_rounded_rect(rect, RIBBON_BUTTON_RADIUS, fill);
            canvas.stroke_rounded_rect(rect, RIBBON_BUTTON_RADIUS, 1, border);

            match button.kind {
                RibbonButtonKind::Dropdown => {
                    draw_text_vcenter(
                        canvas,
                        label_font,
                        rect.x + 10,
                        rect.y,
                        rect.h,
                        button.label,
                        theme.text,
                    );
                    draw_dropdown_arrow(
                        canvas,
                        rect.right() - 18,
                        rect.y + rect.h as i32 / 2,
                        theme.text_dim,
                    );
                }
                RibbonButtonKind::Toggle | RibbonButtonKind::IconButton => {
                    if let Some(icon) = button.icon {
                        canvas.draw_tga_icon_tinted(
                            icon,
                            Rect::new(rect.x + ((rect.w as i32 - 18) / 2), rect.y + 7, 18, 18),
                            if hovered {
                                theme.accent
                            } else {
                                theme.icon_foreground
                            },
                        );
                    } else {
                        let w = measure_text_width(label_font, button.label);
                        draw_text_vcenter(
                            canvas,
                            label_font,
                            rect.x + ((rect.w as i32 - w as i32) / 2),
                            rect.y,
                            rect.h,
                            button.label,
                            theme.text,
                        );
                    }
                }
                RibbonButtonKind::WideButton => {
                    if let Some(icon) = button.icon {
                        canvas.draw_tga_icon_tinted(
                            icon,
                            Rect::new(rect.x + 8, rect.y + 7, 18, 18),
                            if hovered {
                                theme.accent
                            } else {
                                theme.icon_foreground
                            },
                        );
                    }
                    draw_text_vcenter(
                        canvas,
                        label_font,
                        rect.x + if button.icon.is_some() { 30 } else { 10 },
                        rect.y,
                        rect.h,
                        button.label,
                        theme.text,
                    );
                }
            }
        }

        draw_text_vcenter(
            canvas,
            small_font,
            group_rect.x + 12,
            group_rect.bottom() - 22,
            18,
            self.title,
            theme.text_dim,
        );
    }
}

pub struct RibbonBar<'a> {
    pub rect: Rect,
    pub groups: &'a [RibbonGroupSpec<'a>],
    pub hovered: Option<(usize, usize)>,
    pub label_font: Option<&'a dyn VecText>,
    pub small_font: Option<&'a dyn VecText>,
}

impl<'a> RibbonBar<'a> {
    fn available_width(&self) -> u32 {
        let gaps = self.groups.len().saturating_sub(1) as u32 * 10;
        self.rect.w.saturating_sub(32 + gaps)
    }

    fn total_required_width(&self) -> u32 {
        self.groups
            .iter()
            .map(RibbonGroupSpec::required_width)
            .sum()
    }

    fn group_width(&self, idx: usize) -> u32 {
        let required = self.groups[idx].required_width();
        let available = self.available_width();
        let total_required = self.total_required_width();
        if total_required >= available || self.groups.is_empty() {
            return required;
        }
        let extra = available - total_required;
        let base_extra = extra / self.groups.len() as u32;
        let remainder = extra % self.groups.len() as u32;
        required + base_extra + if idx < remainder as usize { 1 } else { 0 }
    }

    pub fn group_rect(&self, idx: usize) -> Rect {
        let mut x = self.rect.x + 16;
        for prev in 0..idx {
            x += self.group_width(prev) as i32 + 10;
        }
        Rect::new(x, self.rect.y + 12, self.group_width(idx), self.rect.h - 20)
    }

    pub fn hit_test(&self, point: Point) -> Option<(usize, usize)> {
        for (group_idx, group) in self.groups.iter().enumerate() {
            if let Some(button_idx) = group.hit_test(self.group_rect(group_idx), point) {
                return Some((group_idx, button_idx));
            }
        }
        None
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        fill_vertical_gradient(
            canvas,
            self.rect,
            theme.panel_alt.lighten(8),
            theme.panel.darken(12),
        );
        canvas.hbar(
            self.rect.x,
            self.rect.bottom() - 1,
            self.rect.w,
            1,
            theme.border,
        );

        for (group_idx, group) in self.groups.iter().enumerate() {
            group.draw(
                canvas,
                theme,
                self.group_rect(group_idx),
                self.hovered.and_then(|(hover_group, hover_button)| {
                    (hover_group == group_idx).then_some(hover_button)
                }),
                self.label_font,
                self.small_font,
            );
        }
    }
}
