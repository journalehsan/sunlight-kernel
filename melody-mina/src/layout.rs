//! Responsive Melody Mina composition using `sunlight-ui` measure/arrange primitives.

use sunlight_ui::{
    AxisSizing, Column, LayoutBox, LayoutConstraints, Measure, Rect, Row, Size, Sizing,
};

const HEADER_H: u32 = 52;
const OUTER_PAD: u32 = 18;
const NARROW_PAD: u32 = 12;
const GAP: u32 = 12;

const LARGE_ART_MIN: u32 = 280;
const LARGE_DETAIL_MIN: u32 = 440;
pub const LARGE_MIN_CLIENT_W: u32 = OUTER_PAD * 2 + LARGE_ART_MIN + GAP + LARGE_DETAIL_MIN;

const MEDIUM_ART_MIN: u32 = 190;
const MEDIUM_META_MIN: u32 = 300;
pub const MEDIUM_MIN_CLIENT_W: u32 = OUTER_PAD * 2 + MEDIUM_ART_MIN + GAP + MEDIUM_META_MIN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    Large,
    Medium,
    Narrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MelodyLayout {
    pub mode: LayoutMode,
    pub root: Rect,
    pub header: Rect,
    pub header_open: Rect,
    pub header_more: Rect,
    pub album_art: Rect,
    pub metadata: Rect,
    pub playlist: Rect,
    pub visualizer: Rect,
    pub timeline: Rect,
    pub timeline_slider: Rect,
    pub transport: Rect,
    pub transport_buttons: [Rect; 5],
    pub volume_icon: Rect,
    pub volume_slider: Rect,
}

impl MelodyLayout {
    pub const fn empty() -> Self {
        Self {
            mode: LayoutMode::Large,
            root: Rect::new(0, 0, 0, 0),
            header: Rect::new(0, 0, 0, 0),
            header_open: Rect::new(0, 0, 0, 0),
            header_more: Rect::new(0, 0, 0, 0),
            album_art: Rect::new(0, 0, 0, 0),
            metadata: Rect::new(0, 0, 0, 0),
            playlist: Rect::new(0, 0, 0, 0),
            visualizer: Rect::new(0, 0, 0, 0),
            timeline: Rect::new(0, 0, 0, 0),
            timeline_slider: Rect::new(0, 0, 0, 0),
            transport: Rect::new(0, 0, 0, 0),
            transport_buttons: [Rect::new(0, 0, 0, 0); 5],
            volume_icon: Rect::new(0, 0, 0, 0),
            volume_slider: Rect::new(0, 0, 0, 0),
        }
    }

    pub fn arrange(client: Rect) -> Self {
        let constraints = LayoutConstraints::new(client.size());
        let mode = if constraints.available.w >= LARGE_MIN_CLIENT_W {
            LayoutMode::Large
        } else if constraints.available.w >= MEDIUM_MIN_CLIENT_W {
            LayoutMode::Medium
        } else {
            LayoutMode::Narrow
        };
        let header_h = HEADER_H.min(client.h);
        let mut root_children = [fixed_height(header_h), fill()];
        let _ = Column::new(client).arrange(&mut root_children);
        let header = root_children[0].bounds();
        let body = root_children[1].bounds();
        let pad = if mode == LayoutMode::Narrow {
            NARROW_PAD
        } else {
            OUTER_PAD
        };
        let content = body.inset(pad as i32);

        let mut layout = Self {
            mode,
            root: client,
            header,
            ..Self::empty()
        };
        layout.header_open = centered_square_at_left(header.inset(8), 36);
        layout.header_more = centered_square_at_right(header.inset(8), 36);

        match mode {
            LayoutMode::Large => layout.arrange_large(content),
            LayoutMode::Medium => layout.arrange_medium(content),
            LayoutMode::Narrow => layout.arrange_narrow(content),
        }
        layout.arrange_timeline();
        layout.arrange_transport();
        layout
    }

    fn arrange_large(&mut self, content: Rect) {
        let visual_h = 72.min(content.h / 5);
        let timeline_h = 42.min(content.h / 7);
        let transport_h = 50.min(content.h / 6);
        let mut sections = [
            LayoutBox::new(Rect::default())
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Flex(1))),
            fixed_height(visual_h),
            fixed_height(timeline_h),
            fixed_height(transport_h),
        ];
        let _ = Column::new(content).with_gap(GAP).arrange(&mut sections);
        let hero = sections[0].bounds();
        self.visualizer = sections[1].bounds();
        self.timeline = sections[2].bounds();
        self.transport = sections[3].bounds();

        let art_size = hero.h.min(320).min(hero.w.saturating_sub(GAP));
        let mut hero_columns = [
            LayoutBox::new(Rect::default()).with_sizing(Sizing::new(
                AxisSizing::Fixed(art_size),
                AxisSizing::Fixed(art_size),
            )),
            fill(),
        ];
        let _ = Row::new(hero).with_gap(GAP).arrange(&mut hero_columns);
        self.album_art = hero_columns[0].bounds();
        let details = hero_columns[1].bounds();
        let metadata_h = 84.min(details.h / 2);
        let mut detail_rows = [fixed_height(metadata_h), fill()];
        let _ = Column::new(details).with_gap(GAP).arrange(&mut detail_rows);
        self.metadata = detail_rows[0].bounds();
        self.playlist = detail_rows[1].bounds();
    }

    fn arrange_medium(&mut self, content: Rect) {
        let intro_h = 190.min(content.h.saturating_mul(2) / 5);
        let visual_h = 62.min(content.h / 7);
        let timeline_h = 42.min(content.h / 9);
        let transport_h = 48.min(content.h / 8);
        let mut sections = [
            fixed_height(intro_h),
            LayoutBox::new(Rect::default())
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Flex(1))),
            fixed_height(visual_h),
            fixed_height(timeline_h),
            fixed_height(transport_h),
        ];
        let _ = Column::new(content).with_gap(GAP).arrange(&mut sections);
        let intro = sections[0].bounds();
        self.playlist = sections[1].bounds();
        self.visualizer = sections[2].bounds();
        self.timeline = sections[3].bounds();
        self.transport = sections[4].bounds();

        let art_size = intro.h.min(220).min(intro.w.saturating_sub(GAP));
        let mut intro_columns = [
            LayoutBox::new(Rect::default()).with_sizing(Sizing::new(
                AxisSizing::Fixed(art_size),
                AxisSizing::Fixed(art_size),
            )),
            fill(),
        ];
        let _ = Row::new(intro).with_gap(GAP).arrange(&mut intro_columns);
        self.album_art = intro_columns[0].bounds();
        self.metadata = intro_columns[1].bounds();
    }

    fn arrange_narrow(&mut self, content: Rect) {
        let compact = content.h < 520;
        let metadata_h = if compact { 48 } else { 66 }.min(content.h / 5);
        let visual_h = if compact { 36 } else { 52 }.min(content.h / 7);
        let timeline_h = if compact { 32 } else { 40 }.min(content.h / 8);
        let transport_h = if compact { 40 } else { 48 }.min(content.h / 7);
        let gaps = if compact { 6 } else { 10 };
        let fixed_without_art = metadata_h
            .saturating_add(visual_h)
            .saturating_add(timeline_h)
            .saturating_add(transport_h)
            .saturating_add(gaps * 5);
        let art_budget = content
            .h
            .saturating_sub(fixed_without_art)
            .saturating_mul(3)
            / 5;
        let art_size = content
            .w
            .min(if compact { 150 } else { 230 })
            .min(art_budget);
        let mut sections = [
            fixed_height(art_size),
            fixed_height(metadata_h),
            LayoutBox::new(Rect::default())
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Flex(1))),
            fixed_height(visual_h),
            fixed_height(timeline_h),
            fixed_height(transport_h),
        ];
        let _ = Column::new(content).with_gap(gaps).arrange(&mut sections);
        let album_slot = sections[0].bounds();
        self.album_art = Rect::new(
            album_slot.x + (album_slot.w.saturating_sub(art_size) / 2) as i32,
            album_slot.y,
            art_size,
            art_size,
        );
        self.metadata = sections[1].bounds();
        self.playlist = sections[2].bounds();
        self.visualizer = sections[3].bounds();
        self.timeline = sections[4].bounds();
        self.transport = sections[5].bounds();
    }

    fn arrange_timeline(&mut self) {
        let label_w = if self.mode == LayoutMode::Narrow {
            54
        } else {
            64
        };
        let mut row = [fixed_width(label_w), fill(), fixed_width(label_w)];
        let _ = Row::new(self.timeline).with_gap(6).arrange(&mut row);
        self.timeline_slider = row[1].bounds().inset(2);
    }

    fn arrange_transport(&mut self) {
        let button = if self.mode == LayoutMode::Narrow {
            34
        } else {
            38
        }
        .min(self.transport.h);
        let slider_w = if self.mode == LayoutMode::Narrow {
            64
        } else {
            104
        };
        let gap = if self.mode == LayoutMode::Narrow {
            4
        } else {
            7
        };
        let mut row = [
            LayoutBox::new(Rect::default())
                .with_sizing(Sizing::new(AxisSizing::Flex(1), AxisSizing::Fill)),
            square(button),
            square(button),
            square(button),
            square(button),
            square(button),
            fixed_width(18),
            fixed_width(slider_w),
            LayoutBox::new(Rect::default())
                .with_sizing(Sizing::new(AxisSizing::Flex(1), AxisSizing::Fill)),
        ];
        let _ = Row::new(self.transport).with_gap(gap).arrange(&mut row);
        for (index, target) in self.transport_buttons.iter_mut().enumerate() {
            *target = center_vertically(row[index + 1].bounds(), button);
        }
        self.volume_icon = center_vertically(row[6].bounds(), 18);
        self.volume_slider = center_vertically(row[7].bounds(), 24);
    }
}

struct CompositionRequirements;

impl Measure for CompositionRequirements {
    fn measure(&self, constraints: LayoutConstraints) -> Size {
        constraints.constrain(Size::new(MEDIUM_MIN_CLIENT_W, 420))
    }
}

pub fn measured_minimum(constraints: LayoutConstraints) -> Size {
    CompositionRequirements.measure(constraints)
}

const fn fill() -> LayoutBox {
    LayoutBox::new(Rect::new(0, 0, 0, 0))
        .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fill))
}

const fn fixed_height(height: u32) -> LayoutBox {
    LayoutBox::new(Rect::new(0, 0, 0, height))
        .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(height)))
}

const fn fixed_width(width: u32) -> LayoutBox {
    LayoutBox::new(Rect::new(0, 0, width, 0))
        .with_sizing(Sizing::new(AxisSizing::Fixed(width), AxisSizing::Fill))
}

const fn square(size: u32) -> LayoutBox {
    LayoutBox::new(Rect::new(0, 0, size, size)).with_sizing(Sizing::new(
        AxisSizing::Fixed(size),
        AxisSizing::Fixed(size),
    ))
}

fn centered_square_at_left(rect: Rect, size: u32) -> Rect {
    Rect::new(
        rect.x,
        rect.y + (rect.h.saturating_sub(size) / 2) as i32,
        size.min(rect.w),
        size.min(rect.h),
    )
}

fn centered_square_at_right(rect: Rect, size: u32) -> Rect {
    let actual = size.min(rect.w).min(rect.h);
    Rect::new(
        rect.right() - actual as i32,
        rect.y + (rect.h.saturating_sub(actual) / 2) as i32,
        actual,
        actual,
    )
}

fn center_vertically(rect: Rect, height: u32) -> Rect {
    let actual = height.min(rect.h);
    Rect::new(
        rect.x,
        rect.y + (rect.h.saturating_sub(actual) / 2) as i32,
        rect.w,
        actual,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositions_follow_measured_width_requirements() {
        assert_eq!(
            MelodyLayout::arrange(Rect::new(0, 0, LARGE_MIN_CLIENT_W, 720)).mode,
            LayoutMode::Large
        );
        assert_eq!(
            MelodyLayout::arrange(Rect::new(0, 0, LARGE_MIN_CLIENT_W - 1, 720)).mode,
            LayoutMode::Medium
        );
        assert_eq!(
            MelodyLayout::arrange(Rect::new(0, 0, MEDIUM_MIN_CLIENT_W - 1, 720)).mode,
            LayoutMode::Narrow
        );
    }

    #[test]
    fn album_art_is_square_in_every_mode() {
        for size in [
            Size::new(1100, 720),
            Size::new(680, 700),
            Size::new(420, 720),
        ] {
            let layout = MelodyLayout::arrange(Rect::new(0, 0, size.w, size.h));
            assert_eq!(layout.album_art.w, layout.album_art.h);
        }
    }

    #[test]
    fn realistic_aggressive_resize_keeps_regions_inside_client() {
        for size in [
            Size::new(1100, 720),
            Size::new(650, 560),
            Size::new(360, 420),
        ] {
            let layout = MelodyLayout::arrange(Rect::new(0, 0, size.w, size.h));
            for rect in [
                layout.album_art,
                layout.metadata,
                layout.playlist,
                layout.visualizer,
                layout.timeline,
                layout.transport,
            ] {
                assert!(rect.x >= 0 && rect.y >= 0);
                assert!(rect.right() <= size.w as i32, "{rect:?} outside {size:?}");
                assert!(rect.bottom() <= size.h as i32, "{rect:?} outside {size:?}");
            }
        }
    }
}
