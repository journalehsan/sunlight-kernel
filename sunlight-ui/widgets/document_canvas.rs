use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

const HOST_RADIUS: u32 = 20;
const PAGE_RADIUS: u32 = 10;
const SURFACE_RADIUS: u32 = 12;
const PAGE_HEADER_H: u32 = 62;
const PAGE_FOOTER_H: u32 = 42;
const PAGE_INSET: i32 = 24;
const SURFACE_INSET_X: i32 = 24;
const SURFACE_TOP_GAP: i32 = 18;
const CONTENT_INSET: i32 = 28;

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

fn clip_text_to_width<'a>(font: Option<&dyn VecText>, text: &'a str, max_w: u32) -> &'a str {
    if max_w == 0 {
        return "";
    }
    if measure_text_width(font, text) <= max_w {
        return text;
    }

    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if measure_text_width(font, &text[..next]) > max_w {
            break;
        }
        end = next;
    }
    &text[..end]
}

fn draw_line_clipped(
    canvas: &mut Canvas,
    bounds: Rect,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    style: DocumentStrokeStyle,
) {
    let mut x = x1;
    let mut y = y1;
    let dx = (x2 - x1).abs();
    let sx = if x1 <= x2 { 1 } else { -1 };
    let dy = -(y2 - y1).abs();
    let sy = if y1 <= y2 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        let thickness = style.thickness.max(1) as i32;
        let half = thickness / 2;
        for py in 0..thickness {
            for px in 0..thickness {
                let tx = x + px - half;
                let ty = y + py - half;
                if bounds.contains(Point::new(tx, ty)) {
                    canvas.put_pixel(tx, ty, style.color);
                }
            }
        }

        if x == x2 && y == y2 {
            break;
        }
        let e2 = err * 2;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentCanvasMode {
    Editable,
    ReadOnly,
}

impl Default for DocumentCanvasMode {
    fn default() -> Self {
        Self::Editable
    }
}

#[derive(Clone, Copy)]
pub struct DocumentTextStyle<'a> {
    pub font: Option<&'a dyn VecText>,
    pub color: Color,
}

impl<'a> DocumentTextStyle<'a> {
    pub const fn new(font: Option<&'a dyn VecText>, color: Color) -> Self {
        Self { font, color }
    }
}

impl<'a> Default for DocumentTextStyle<'a> {
    fn default() -> Self {
        Self {
            font: None,
            color: Color::rgb(0x24, 0x24, 0x28),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentStrokeStyle {
    pub color: Color,
    pub thickness: u32,
}

impl DocumentStrokeStyle {
    pub const fn new(color: Color, thickness: u32) -> Self {
        Self { color, thickness }
    }
}

impl Default for DocumentStrokeStyle {
    fn default() -> Self {
        Self {
            color: Color::rgb(0xD7, 0xD3, 0xCD),
            thickness: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentRectStyle {
    pub fill: Color,
    pub border: Option<DocumentStrokeStyle>,
}

impl DocumentRectStyle {
    pub const fn new(fill: Color, border: Option<DocumentStrokeStyle>) -> Self {
        Self { fill, border }
    }
}

impl Default for DocumentRectStyle {
    fn default() -> Self {
        Self {
            fill: Color::rgb(0xF9, 0xF7, 0xF2),
            border: Some(DocumentStrokeStyle::default()),
        }
    }
}

#[derive(Clone, Copy)]
pub enum DocumentCanvasItem<'a> {
    Text {
        x: i32,
        y: i32,
        text: &'a str,
        style: DocumentTextStyle<'a>,
    },
    Rect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: DocumentRectStyle,
    },
    Line {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        style: DocumentStrokeStyle,
    },
    LinkText {
        x: i32,
        y: i32,
        text: &'a str,
        url: &'a str,
        style: DocumentTextStyle<'a>,
    },
    ImagePlaceholder {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        label: &'a str,
    },
}

#[derive(Clone, Copy)]
pub struct DocumentCanvas<'a> {
    pub rect: Rect,
    pub mode: DocumentCanvasMode,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub empty_label: &'a str,
    pub footer_note: &'a str,
    pub show_guides: bool,
    pub items: &'a [DocumentCanvasItem<'a>],
    pub title_font: Option<&'a dyn VecText>,
    pub subtitle_font: Option<&'a dyn VecText>,
    pub body_font: Option<&'a dyn VecText>,
    pub small_font: Option<&'a dyn VecText>,
}

impl<'a> DocumentCanvas<'a> {
    pub fn new(rect: Rect, items: &'a [DocumentCanvasItem<'a>]) -> Self {
        Self {
            rect,
            mode: DocumentCanvasMode::Editable,
            title: "Document Canvas",
            subtitle: "Reusable fixed-coordinate page surface",
            empty_label: "Document Canvas Ready",
            footer_note: "Fixed-coordinate page surface",
            show_guides: true,
            items,
            title_font: None,
            subtitle_font: None,
            body_font: None,
            small_font: None,
        }
    }

    pub fn with_mode(mut self, mode: DocumentCanvasMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_titles(mut self, title: &'a str, subtitle: &'a str) -> Self {
        self.title = title;
        self.subtitle = subtitle;
        self
    }

    pub fn with_empty_label(mut self, empty_label: &'a str) -> Self {
        self.empty_label = empty_label;
        self
    }

    pub fn with_footer_note(mut self, footer_note: &'a str) -> Self {
        self.footer_note = footer_note;
        self
    }

    pub fn with_guides(mut self, show_guides: bool) -> Self {
        self.show_guides = show_guides;
        self
    }

    pub fn with_fonts(
        mut self,
        title_font: Option<&'a dyn VecText>,
        subtitle_font: Option<&'a dyn VecText>,
        body_font: Option<&'a dyn VecText>,
        small_font: Option<&'a dyn VecText>,
    ) -> Self {
        self.title_font = title_font;
        self.subtitle_font = subtitle_font;
        self.body_font = body_font;
        self.small_font = small_font;
        self
    }

    pub fn host_rect(&self) -> Rect {
        self.rect.inset(18)
    }

    pub fn page_rect(&self) -> Rect {
        let host = self.host_rect();
        let desired_w = 860u32.min(host.w.saturating_sub(96)).max(620);
        let desired_h = host.h.saturating_sub(56).max(420);
        let x = host.x + ((host.w as i32 - desired_w as i32) / 2);
        let y = host.y + 26;
        Rect::new(x, y, desired_w, desired_h)
    }

    pub fn document_rect(&self) -> Rect {
        let page = self.page_rect();
        let top = page.y + PAGE_HEADER_H as i32 + SURFACE_TOP_GAP;
        let bottom = page.bottom() - PAGE_FOOTER_H as i32;
        Rect::new(
            page.x + SURFACE_INSET_X,
            top,
            page.w.saturating_sub((SURFACE_INSET_X * 2) as u32),
            (bottom - top).max(0) as u32,
        )
    }

    pub fn content_rect(&self) -> Rect {
        self.document_rect().inset(CONTENT_INSET)
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        fill_vertical_gradient(canvas, self.rect, theme.bg.lighten(14), theme.bg.darken(8));

        let host = self.host_rect();
        canvas.fill_rounded_rect(host, HOST_RADIUS, theme.panel.darken(8));
        canvas.stroke_rounded_rect(host, HOST_RADIUS, 1, theme.border);

        let page = self.page_rect();
        let shadow = page.translate(10, 12);
        canvas.fill_rounded_rect(shadow, PAGE_RADIUS, Color::rgb(0x16, 0x16, 0x18));
        canvas.fill_rounded_rect(page, PAGE_RADIUS, Color::rgb(0xFB, 0xFA, 0xF7));
        canvas.stroke_rounded_rect(page, PAGE_RADIUS, 1, Color::rgb(0xD7, 0xD3, 0xCD));

        let page_top = Rect::new(page.x, page.y, page.w, PAGE_HEADER_H);
        fill_vertical_gradient(
            canvas,
            page_top,
            Color::rgb(0xFF, 0xFF, 0xFF),
            Color::rgb(0xF4, 0xF1, 0xED),
        );
        canvas.hbar(
            page.x,
            page.y + PAGE_HEADER_H as i32 - 1,
            page.w,
            1,
            Color::rgb(0xE7, 0xE1, 0xD8),
        );

        draw_text(
            canvas,
            self.title_font,
            page.x + PAGE_INSET,
            page.y + 22,
            self.title,
            Color::rgb(0x24, 0x24, 0x28),
        );
        draw_text(
            canvas,
            self.subtitle_font,
            page.x + PAGE_INSET,
            page.y + 40,
            self.subtitle,
            Color::rgb(0x72, 0x72, 0x7C),
        );

        let document = self.document_rect();
        canvas.fill_rounded_rect(document, SURFACE_RADIUS, Color::rgb(0xFF, 0xFF, 0xFF));
        canvas.stroke_rounded_rect(document, SURFACE_RADIUS, 1, Color::rgb(0xE0, 0xDB, 0xD4));
        canvas.hbar(
            document.x + 1,
            document.y + 1,
            document.w.saturating_sub(2),
            4,
            theme.accent.lighten(34),
        );

        let content = self.content_rect();
        if self.show_guides {
            self.draw_guides(canvas, content);
        }

        if self.items.is_empty() {
            self.draw_empty_label(canvas, content, theme);
        } else {
            self.draw_items(canvas, content);
        }

        draw_text(
            canvas,
            self.small_font,
            page.x + PAGE_INSET,
            page.bottom() - 30,
            self.footer_note,
            Color::rgb(0x86, 0x82, 0x7B),
        );
    }

    fn draw_guides(&self, canvas: &mut Canvas, content: Rect) {
        let guide_color = Color::rgb(0xEE, 0xE8, 0xE0);
        let margin_color = Color::rgb(0xF2, 0xEA, 0xE0);
        let guide_margin_x = content.x + 54;
        canvas.fill_rect(
            Rect::new(guide_margin_x, content.y, 1, content.h),
            margin_color,
        );

        let mut y = content.y + 26;
        while y < content.bottom() - 14 {
            canvas.fill_rect(
                Rect::new(content.x, y, content.w, 1),
                if (y - content.y) % 56 == 0 {
                    guide_color.darken(8)
                } else {
                    guide_color
                },
            );
            y += 28;
        }
    }

    fn draw_empty_label(&self, canvas: &mut Canvas, content: Rect, theme: &Theme) {
        let badge_w = measure_text_width(self.body_font, self.empty_label) + 28;
        let badge = Rect::new(
            content.x + ((content.w as i32 - badge_w as i32) / 2),
            content.y + (content.h as i32 / 2) - 18,
            badge_w,
            36,
        );
        canvas.fill_rounded_rect(badge, 18, theme.panel);
        canvas.stroke_rounded_rect(badge, 18, 1, theme.accent.darken(80));
        draw_text_vcenter(
            canvas,
            self.body_font,
            badge.x + 14,
            badge.y,
            badge.h,
            self.empty_label,
            theme.text,
        );
    }

    fn draw_items(&self, canvas: &mut Canvas, content: Rect) {
        for item in self.items {
            match *item {
                DocumentCanvasItem::Text { x, y, text, style } => {
                    let px = content.x + x;
                    let py = content.y + y;
                    if py < content.y || py >= content.bottom() {
                        continue;
                    }
                    let max_w = (content.right() - px).max(0) as u32;
                    let visible = clip_text_to_width(style.font, text, max_w);
                    if !visible.is_empty() {
                        draw_text(canvas, style.font, px, py, visible, style.color);
                    }
                }
                DocumentCanvasItem::Rect { x, y, w, h, style } => {
                    let rect = Rect::new(content.x + x, content.y + y, w, h);
                    if let Some(clipped) = rect.intersect(content) {
                        canvas.fill_rect(clipped, style.fill);
                        if let Some(border) = style.border {
                            canvas.draw_rect(clipped, border.color);
                        }
                    }
                }
                DocumentCanvasItem::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    style,
                } => {
                    draw_line_clipped(
                        canvas,
                        content,
                        content.x + x1,
                        content.y + y1,
                        content.x + x2,
                        content.y + y2,
                        style,
                    );
                }
                DocumentCanvasItem::LinkText {
                    x,
                    y,
                    text,
                    url,
                    style,
                } => {
                    let px = content.x + x;
                    let py = content.y + y;
                    if py < content.y || py >= content.bottom() {
                        continue;
                    }
                    let max_w = (content.right() - px).max(0) as u32;
                    let visible = clip_text_to_width(style.font, text, max_w);
                    if visible.is_empty() {
                        continue;
                    }
                    let color = if url.is_empty() {
                        style.color
                    } else {
                        style.color
                    };
                    draw_text(canvas, style.font, px, py, visible, color);
                    let underline_w = measure_text_width(style.font, visible);
                    let underline_y = py + crate::paint::font::GLYPH_H as i32 + 2;
                    if underline_y < content.bottom() {
                        canvas.hbar(px, underline_y, underline_w, 1, color.lighten(18));
                    }
                }
                DocumentCanvasItem::ImagePlaceholder { x, y, w, h, label } => {
                    let rect = Rect::new(content.x + x, content.y + y, w, h);
                    if let Some(clipped) = rect.intersect(content) {
                        canvas.fill_rect(clipped, Color::rgb(0xF6, 0xF3, 0xEE));
                        canvas.draw_rect(clipped, Color::rgb(0xD3, 0xCD, 0xC4));
                        draw_line_clipped(
                            canvas,
                            content,
                            rect.x + 8,
                            rect.y + 8,
                            rect.right() - 9,
                            rect.bottom() - 9,
                            DocumentStrokeStyle::new(Color::rgb(0xD8, 0xD1, 0xC8), 1),
                        );
                        draw_line_clipped(
                            canvas,
                            content,
                            rect.right() - 9,
                            rect.y + 8,
                            rect.x + 8,
                            rect.bottom() - 9,
                            DocumentStrokeStyle::new(Color::rgb(0xD8, 0xD1, 0xC8), 1),
                        );
                        let label_rect = Rect::new(
                            rect.x + 10,
                            rect.y + (rect.h as i32 / 2) - 10,
                            rect.w.saturating_sub(20),
                            20,
                        );
                        let visible =
                            clip_text_to_width(self.small_font, label, label_rect.w.max(1));
                        draw_text_vcenter(
                            canvas,
                            self.small_font,
                            label_rect.x,
                            label_rect.y,
                            label_rect.h,
                            visible,
                            Color::rgb(0x8B, 0x86, 0x80),
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentCanvas, DocumentCanvasItem, DocumentCanvasMode, DocumentStrokeStyle,
        DocumentTextStyle,
    };
    use crate::{Canvas, Color, Rect, Theme};

    #[test]
    fn mode_defaults_to_editable() {
        assert_eq!(DocumentCanvasMode::default(), DocumentCanvasMode::Editable);
    }

    #[test]
    fn content_rect_stays_within_page() {
        let widget = DocumentCanvas::new(Rect::new(0, 0, 1240, 680), &[]);
        assert!(widget
            .page_rect()
            .intersect(widget.content_rect())
            .is_some());
        assert!(widget.content_rect().right() <= widget.page_rect().right());
        assert!(widget.content_rect().bottom() <= widget.page_rect().bottom());
    }

    #[test]
    fn empty_document_draw_does_not_panic() {
        let mut pixels = [0u32; 320 * 240];
        let mut canvas = Canvas::new(&mut pixels, 320, 320, 240);
        let widget = DocumentCanvas::new(Rect::new(0, 0, 320, 240), &[])
            .with_mode(DocumentCanvasMode::ReadOnly);
        widget.draw(&mut canvas, &Theme::sunlight_dark());
    }

    #[test]
    fn sample_items_can_be_constructed() {
        let items = [
            DocumentCanvasItem::Text {
                x: 0,
                y: 0,
                text: "Sample",
                style: DocumentTextStyle::default(),
            },
            DocumentCanvasItem::Line {
                x1: 0,
                y1: 12,
                x2: 60,
                y2: 12,
                style: DocumentStrokeStyle::new(Color::rgb(1, 2, 3), 1),
            },
        ];
        let widget = DocumentCanvas::new(Rect::new(0, 0, 640, 480), &items);
        assert_eq!(widget.items.len(), 2);
    }
}
