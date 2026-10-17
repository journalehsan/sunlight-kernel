#[cfg(not(test))]
fn interaction_millis() -> u64 {
    sunlight_ipc::monotonic_millis()
}
#[cfg(test)]
fn interaction_millis() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

use super::*;
use alloc::{format, string::String, vec};
use sunlight_dialogs::{
    ConfirmRequest, ConfirmStyle, DialogButton, DialogClient, DialogCommonOptions, DialogRequest,
    DialogResult, SaveFileRequest, TextInputRequest,
};

pub(super) struct ExportJob {
    fd: libc::Fd,
    temporary: PathBuf,
    destination: PathBuf,
    replace: bool,
    position: u32,
    sampler: Sampler,
    state: State,
    format: FormatKind,
    file_size: u64,
}
impl Drop for ExportJob {
    fn drop(&mut self) {
        let _ = export_io::close(self.fd);
        if !self.temporary.is_empty() {
            let _ = export_io::unlink(self.temporary.as_bytes());
        }
    }
}

impl LightLensApp {
    pub(super) fn comparing(&self) -> bool {
        self.compare_key || self.compare_pointer
    }
    pub(super) fn compare_button(&self) -> Rect {
        let r = self.layout.viewport;
        if r.w < 220 {
            return Rect::new(r.x, r.y, 0, 0);
        }
        Rect::new(r.right() - 122, r.y + 1, 116, 18)
    }
    fn histogram_button(&self) -> Rect {
        let r = self.layout.right;
        if self.workspace.right != 2 {
            return Rect::new(r.x, r.y, 0, 0);
        }
        Rect::new(r.x + 12, r.y + 38, r.w.saturating_sub(24), 24)
    }
    pub(super) fn draw_histogram(&self, canvas: &mut Canvas, theme: &Theme) {
        let mut r = self.histogram_button();
        let panel = self.layout.right;
        let area = Rect::new(panel.x, panel.y + 27, panel.w, panel.h.saturating_sub(27));
        if area.w == 0 || area.h == 0 {
            return;
        }
        let mut sub = canvas.sub_canvas(area);
        let canvas = &mut sub;
        r.x -= area.x;
        r.y -= area.y;
        if r.w == 0 || r.h == 0 {
            return;
        }
        Self::draw_edit_button(
            canvas,
            theme,
            r,
            ["Luminance", "Red", "Green", "Blue", "RGB combined"][self.workspace.histogram_channel],
            true,
            self.histogram_open,
            self.hovered == Some(r),
        );
        let plot = Rect::new(r.x, r.bottom() + 6, r.w, 64);
        canvas.fill_rect(plot, theme.bg);
        canvas.draw_rect(plot, theme.border);
        let text = match (&self.image, &self.histogram) {
            (None, _) => Some("No image"),
            (_, Some((state, _)))
                if *state == self.editor.state
                    && self
                        .histogram_scene
                        .as_ref()
                        .is_some_and(|s| s.same_pixels(&self.editor.scene)) =>
            {
                None
            }
            _ => Some("Updating after edit..."),
        };
        if let Some(text) = text {
            sf_centered(
                canvas,
                plot,
                text,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        } else if let Some((_, hist)) = &self.histogram {
            let channel = self.workspace.histogram_channel;
            let series: &[usize] = match channel {
                1 => &[0],
                2 => &[1],
                3 => &[2],
                4 => &[0, 1, 2],
                _ => &[3],
            };
            let peak = series
                .iter()
                .flat_map(|&i| {
                    if i == 3 {
                        hist.bins.iter()
                    } else {
                        hist.channels[i].iter()
                    }
                })
                .copied()
                .max()
                .unwrap_or(1)
                .max(1);
            for &i in series {
                let bins = if i == 3 {
                    &hist.bins
                } else {
                    &hist.channels[i]
                };
                let color = match i {
                    0 => Color::rgb(240, 90, 90),
                    1 => Color::rgb(85, 210, 130),
                    2 => Color::rgb(100, 150, 255),
                    _ => theme.accent,
                };
                for x in 0..plot.w.saturating_sub(2) {
                    let lo = x as usize * 256 / plot.w.saturating_sub(2) as usize;
                    let hi = (((x + 1) as usize * 256 / plot.w.saturating_sub(2) as usize)
                        .max(lo + 1))
                    .min(256);
                    let value = *bins[lo..hi].iter().max().unwrap_or(&0);
                    let height = (value as u64 * 60 / peak as u64) as u32;
                    canvas.vline(
                        plot.x + 1 + x as i32,
                        plot.bottom() - 1 - height as i32,
                        height,
                        color,
                    );
                }
            }
        }
    }
    fn histogram_tick(&mut self) -> bool {
        if !self.histogram_open
            || self.histogram_button().h == 0
            || self.slider_drag.is_some()
            || self.workspace.gesture.is_some()
            || self.histogram.as_ref().is_some_and(|(s, _)| {
                *s == self.editor.state
                    && self
                        .histogram_scene
                        .as_ref()
                        .is_some_and(|scene| scene.same_pixels(&self.editor.scene))
            })
        {
            return false;
        }
        let Some(image) = self.image.as_ref() else {
            return false;
        };
        self.histogram = Some((
            self.editor.state,
            editor::Histogram::compute_scene(image, self.editor.state, Some(&self.editor.scene)),
        ));
        self.histogram_scene = Some(self.editor.scene.clone());
        true
    }
    fn crop_display(&self, dst: Rect, crop: Rect) -> Rect {
        let (w, h) = self.editor.state.size;
        Rect::new(
            dst.x + (crop.x as i64 * dst.w as i64 / w as i64) as i32,
            dst.y + (crop.y as i64 * dst.h as i64 / h as i64) as i32,
            (crop.w as u64 * dst.w as u64 / w as u64) as u32,
            (crop.h as u64 * dst.h as u64 / h as u64) as u32,
        )
    }
    fn crop_hit(&self, p: Point) -> u8 {
        if !self.crop_mode || !self.preview_area().contains(p) {
            return 0;
        }
        let Some(crop) = self.crop else {
            return 0;
        };
        if crop.w == 0 || crop.h == 0 {
            return 0;
        }
        let r = self.crop_display(self.image_rect(), crop);
        if !r.inset(-6).contains(p) {
            return 0;
        }
        let mut edges = 0;
        if (p.x - r.x).abs() <= 6 {
            edges |= 1;
        } else if (p.x - r.right()).abs() <= 6 {
            edges |= 2;
        }
        if (p.y - r.y).abs() <= 6 {
            edges |= 4;
        } else if (p.y - r.bottom()).abs() <= 6 {
            edges |= 8;
        }
        if edges != 0 {
            edges
        } else if r.contains(p) {
            16
        } else {
            0
        }
    }
    fn crop_cursor(hit: u8) -> CursorShape {
        match hit {
            1 | 2 => CursorShape::ResizeHorizontal,
            4 | 8 => CursorShape::ResizeVertical,
            5 | 10 => CursorShape::ResizeNwse,
            6 | 9 => CursorShape::ResizeNesw,
            16 => CursorShape::Move,
            _ => CursorShape::Pointer,
        }
    }
    fn update_crop(&mut self, p: Point) -> bool {
        let end = editor::image_point(self.image_rect(), self.editor.state.size, p);
        let next = if let Some((r, start, edges)) = self.crop_drag {
            Some(editor::drag_crop(
                r,
                start,
                end,
                edges,
                self.editor.state.size,
                self.crop_ratio,
            ))
        } else {
            self.crop_start
                .map(|start| editor::crop_rect(start, end, self.crop_ratio))
        };
        if next.is_some() && next != self.crop {
            self.crop = next;
            true
        } else {
            false
        }
    }
    fn reset_slider(&mut self, i: usize) -> bool {
        self.finish_slider();
        let before = self.editor.state;
        self.editor.state.adjust[i] = 0;
        self.editor.commit(before);
        self.slider_focus = Some(i);
        before != self.editor.state
    }

    pub(super) fn editable(&self) -> bool {
        self.load_state == LoadState::Ready && self.export.is_none()
    }
    pub(super) fn preview_area(&self) -> Rect {
        let r = self.layout.viewport;
        Rect::new(
            r.x + 24,
            r.y + 44,
            r.w.saturating_sub(48),
            r.h.saturating_sub(68),
        )
    }
    pub(super) fn image_rect(&self) -> Rect {
        self.viewport
            .rect(self.preview_area(), self.editor.state.size)
    }
    pub(super) fn cancel_crop(&mut self) {
        self.crop_mode = false;
        self.crop = None;
        self.crop_start = None;
        self.crop_drag = None;
        self.crop_hover = 0;
    }
    pub(super) fn finish_slider(&mut self) {
        if let Some((_, before)) = self.slider_drag.take() {
            self.editor.commit(before);
        }
    }
    fn release_drag(&mut self, x: i32, y: i32) -> bool {
        let dragging = self.slider_drag.is_some()
            || self.crop_start.is_some()
            || self.crop_drag.is_some()
            || self.pan_start.is_some()
            || self.compare_pointer;
        if let Some((i, _)) = self.slider_drag {
            self.set_slider(i, x);
        }
        if self
            .slider_click
            .is_some_and(|(_, p, _)| (p.x - x).abs() > 5 || (p.y - y).abs() > 5)
        {
            self.slider_click = None;
        }
        self.update_crop(Point::new(x, y));
        self.crop_drag = None;
        self.compare_pointer = false;
        self.finish_slider();
        self.crop_start = None;
        self.pan_start = None;
        dragging
    }
    fn change_geometry(&mut self, geometry: Geometry) {
        if !self.editable() {
            return;
        }
        self.finish_slider();
        match self.editor.geometry(geometry) {
            Ok(()) => {
                self.cancel_crop();
                self.viewport = Viewport::default();
                self.message.set_str("Edited. Ctrl+Z to undo.");
            }
            Err(message) => self.message.set_str(message),
        }
    }
    pub(super) fn history(&mut self, redo: bool) {
        if !self.editable() {
            return;
        }
        self.finish_slider();
        self.cancel_crop();
        self.workspace.selected = None;
        self.workspace.gesture = None;
        if redo {
            self.editor.redo();
        } else {
            self.editor.undo();
        }
        self.viewport
            .clamp(self.preview_area(), self.editor.state.size);
        self.message.set_str(if redo { "Redo" } else { "Undo" });
    }
    pub(super) fn draw_edit_button(
        canvas: &mut Canvas,
        theme: &Theme,
        r: Rect,
        label: &str,
        enabled: bool,
        active: bool,
        hovered: bool,
    ) {
        canvas.fill_rect(
            r,
            if hovered && enabled {
                theme.panel_alt.lighten(8)
            } else {
                theme.panel_alt
            },
        );
        canvas.draw_rect(
            r,
            if enabled && (active || hovered) {
                theme.accent
            } else {
                theme.border
            },
        );
        if r.w < 12 || r.h < 10 {
            return;
        }
        let mut shown: TextBuf<96> = TextBuf::empty();
        shown.set_str(label);
        let label = shown.truncate_to_width(r.w.saturating_sub(8) as i32, FontRole::UiSmall);
        sf_centered(
            canvas,
            r,
            label,
            &TextStyle::new(
                FontRole::UiSmall,
                if !enabled {
                    theme.text_dim
                } else if active {
                    theme.accent
                } else {
                    theme.text
                },
            ),
        );
    }
    fn side_area(&self) -> Rect {
        let r = self.layout.left;
        if self.workspace.collapsed_left {
            return Rect::new(r.x, r.y, 0, 0);
        }
        Rect::new(r.x, r.y + 27, r.w, r.h.saturating_sub(27))
    }
    fn side_button(&self, index: usize) -> Rect {
        let area = self.side_area();
        let row = if index < 14 {
            index / 2
        } else {
            7 + (index - 14) / 2
        };
        let y = if index < 14 {
            8 + row as i32 * 30
        } else {
            386 + (row - 7) as i32 * 30
        };
        let w = area.w.saturating_sub(30) / 2;
        Rect::new(
            area.x + 12 + (index % 2) as i32 * (w as i32 + 6),
            area.y + y - self.side_scroll,
            w,
            26,
        )
    }
    fn slider_rect(&self, index: usize) -> Rect {
        let area = self.side_area();
        Rect::new(
            area.x + 12,
            area.y + 230 + index as i32 * 38 - self.side_scroll,
            area.w.saturating_sub(24),
            14,
        )
    }
    fn side_enabled(&self, index: usize) -> bool {
        self.editable()
            && match index {
                0 => self.editor.can_undo(),
                1 => self.editor.can_redo(),
                12 => self
                    .crop
                    .is_some_and(|r| editor::valid_crop(r, self.editor.state.size)),
                13 => self.crop_mode,
                17 => false,
                _ => true,
            }
    }
    pub(super) fn draw_left_panel(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        if self.workspace.collapsed_left {
            canvas.fill_rect(rect, theme.panel);
            self.draw_tabs(canvas, theme, true);
            return;
        }
        if self.workspace.left == 1 {
            self.draw_annotations_panel(canvas, theme);
            return;
        }
        canvas.fill_rect(rect, theme.panel);
        self.draw_tabs(canvas, theme, true);
        let area = self.side_area();
        if area.w == 0 || area.h == 0 {
            return;
        }
        let mut c = canvas.sub_canvas(area);
        for index in 0..18 {
            let label = match index {
                0 => "Undo",
                1 => "Redo",
                2 => "Fit",
                3 => "100%",
                4 => "Flip V",
                5 => "Reset All",
                6 => "Save",
                7 => "Save As",
                8 => "Resize",
                9 => {
                    if self.keep_aspect {
                        "Aspect: On"
                    } else {
                        "Aspect: Off"
                    }
                }
                10 => "Crop",
                11 => ["Free", "1:1", "4:3", "16:9"][self.crop_ratio],
                12 => "Apply",
                13 => "Cancel",
                14 => self.editor.state.effect.label(),
                15 => "No Filter",
                16 => "Reset Adjust",
                _ => "AI: Future",
            };
            let r = self.side_button(index);
            let r = Rect::new(r.x - area.x, r.y - area.y, r.w, r.h);
            Self::draw_edit_button(
                &mut c,
                theme,
                r,
                label,
                self.side_enabled(index),
                (index == 10 && self.crop_mode)
                    || (index == 14 && self.editor.state.effect != editor::Effect::None),
                self.hovered == Some(self.side_button(index)),
            );
        }
        for (i, label) in ["Brightness", "Contrast", "Saturation", "Gamma"]
            .iter()
            .enumerate()
        {
            let r = self.slider_rect(i);
            let r = Rect::new(r.x - area.x, r.y - area.y, r.w, r.h);
            let value = self.editor.state.adjust[i];
            sf_draw(
                &mut c,
                label,
                r.x,
                r.y - 14,
                &TextStyle::new(
                    FontRole::UiSmall,
                    if value != 0 { theme.accent } else { theme.text },
                ),
            );
            sf_right(
                &mut c,
                Rect::new(r.x, r.y - 16, r.w.saturating_sub(40), 14),
                &format!("{:+}", value),
                &TextStyle::new(
                    FontRole::UiSmall,
                    if value != 0 { theme.accent } else { theme.text },
                ),
                0,
            );
            sf_right(
                &mut c,
                Rect::new(r.x, r.y - 16, r.w, 14),
                "Reset",
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                0,
            );
            if self.slider_focus == Some(i) {
                c.draw_rect(r.inset(-1), theme.accent);
            }
            c.fill_rect(Rect::new(r.x, r.y + 5, r.w, 4), theme.border);
            c.vline(r.x + r.w as i32 / 2, r.y, r.h, theme.text_dim);
            let knob_x = r.x + (value as i32 + 100) * (r.w.saturating_sub(8)) as i32 / 200;
            c.fill_rect(
                Rect::new(knob_x, r.y + 1, 8, 12),
                if self.editable() {
                    theme.accent
                } else {
                    theme.text_dim
                },
            );
        }
        sf_draw(
            &mut c,
            "Filter: click to cycle",
            12,
            370 - self.side_scroll,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        sf_draw(
            &mut c,
            "Wheel here to scroll",
            12,
            454 - self.side_scroll,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }
    pub(super) fn draw_edited_photo(&self, canvas: &mut Canvas, image: &RgbaImage, dst: Rect) {
        if dst.w == 0 || dst.h == 0 {
            return;
        }
        let state = if self.comparing() {
            State::new(image.width, image.height)
        } else {
            self.editor.state
        };
        let size = state.size;
        let x0 = dst.x.max(0) as u32;
        let y0 = dst.y.max(0) as u32;
        let x1 = dst.right().min(canvas.width as i32).max(0) as u32;
        let y1 = dst.bottom().min(canvas.height as i32).max(0) as u32;
        let clip = Rect::new(
            x0 as i32,
            y0 as i32,
            x1.saturating_sub(x0),
            y1.saturating_sub(y0),
        );
        let mut cache = self.preview_cache.borrow_mut();
        let cached = cache.prepare(image, state, dst, clip, self.slider_drag.is_some());
        let sampler = if cached {
            None
        } else {
            Some(Sampler::new(state))
        };
        for y in y0..y1 {
            for x in x0..x1 {
                let p = if cached {
                    cache.pixel(x, y)
                } else {
                    let sx = ((x as i64 - dst.x as i64) * size.0 as i64 / dst.w as i64) as u32;
                    let sy = ((y as i64 - dst.y as i64) * size.1 as i64 / dst.h as i64) as u32;
                    sampler.as_ref().unwrap().pixel(image, sx, sy)
                };
                let bg_layer = &self.editor.scene.layers[0];
                let alpha = if self.comparing() {
                    p >> 24
                } else if bg_layer.visible {
                    ((p >> 24) * bg_layer.opacity as u32 + 127) / 255
                } else {
                    0
                };
                let idx = y as usize * canvas.stride as usize + x as usize;
                let bg = canvas.pixels[idx];
                let mut out = 0xff000000;
                for shift in [0, 8, 16] {
                    let v = (((p >> shift) & 255) * alpha
                        + ((bg >> shift) & 255) * (255 - alpha)
                        + 127)
                        / 255;
                    out |= v << shift;
                }
                canvas.pixels[idx] = out;
            }
        }
    }
    pub(super) fn draw_crop_overlay(&self, canvas: &mut Canvas, dst: Rect, theme: &Theme) {
        if !self.crop_mode {
            return;
        }
        sf_draw(
            canvas,
            "Crop: drag, Enter apply, Esc cancel",
            6,
            4,
            &TextStyle::new(FontRole::UiSmall, theme.accent),
        );
        if let Some(crop) = self.crop {
            let r = self.crop_display(dst, crop);
            // Dim the already composited viewport, without touching source pixels.
            for y in dst.y.max(0)..dst.bottom().min(canvas.height as i32) {
                for x in dst.x.max(0)..dst.right().min(canvas.width as i32) {
                    if !r.contains(Point::new(x, y)) {
                        let i = y as usize * canvas.stride as usize + x as usize;
                        canvas.pixels[i] = 0xff000000 | ((canvas.pixels[i] & 0x00fefefe) >> 1);
                    }
                }
            }
            canvas.draw_rect(r, theme.accent);
            canvas.draw_rect(r.inset(-1), Color::rgb(255, 255, 255));
            for (x, y, hit) in [
                (r.x, r.y, 5),
                (r.right(), r.y, 6),
                (r.x, r.bottom(), 9),
                (r.right(), r.bottom(), 10),
                (r.x + r.w as i32 / 2, r.y, 4),
                (r.x + r.w as i32 / 2, r.bottom(), 8),
                (r.x, r.y + r.h as i32 / 2, 1),
                (r.right(), r.y + r.h as i32 / 2, 2),
            ] {
                canvas.fill_rect(
                    Rect::new(x - 3, y - 3, 7, 7),
                    if self.crop_hover == hit {
                        theme.accent
                    } else {
                        theme.text
                    },
                );
            }
            for third in 1..3 {
                canvas.vline(r.x + (r.w * third / 3) as i32, r.y, r.h, theme.accent);
                canvas.hline(r.x, r.y + (r.h * third / 3) as i32, r.w, theme.accent);
            }
        }
    }
    fn set_slider(&mut self, index: usize, x: i32) -> bool {
        let r = self.slider_rect(index);
        if r.w <= 8 {
            return false;
        }
        let value = (((x - r.x - 4) * 200 / (r.w - 8) as i32) - 100).clamp(-100, 100) as i16;
        if self.editor.state.adjust[index] == value {
            return false;
        }
        self.editor.state.adjust[index] = value;
        true
    }
    fn side_action(&mut self, index: usize) {
        if !self.side_enabled(index) {
            return;
        }
        self.finish_slider();
        self.slider_focus = None;
        self.compare_key = false;
        self.compare_pointer = false;
        match index {
            0 => self.history(false),
            1 => self.history(true),
            2 => self.viewport = Viewport::default(),
            3 => {
                self.viewport = Viewport {
                    fit: false,
                    ..Viewport::default()
                };
                self.viewport
                    .clamp(self.preview_area(), self.editor.state.size);
            }
            4 => self.change_geometry(Geometry::FlipV),
            5 => {
                self.editor.reset();
                self.cancel_crop();
                self.viewport = Viewport::default();
                self.message.set_str("Reset to original. Ctrl+Z to undo.");
            }
            6 => self.save(false),
            7 => self.save(true),
            8 => self.resize_dialog(),
            9 => self.keep_aspect = !self.keep_aspect,
            10 => {
                self.workspace.left = 0;
                self.workspace.gesture = None;
                self.crop_mode = !self.crop_mode;
                self.crop = None;
                self.crop_start = None;
                self.crop_drag = None;
            }
            11 => {
                self.crop_ratio = (self.crop_ratio + 1) % 4;
                self.crop = None;
                self.crop_drag = None;
            }
            12 => {
                if let Some(r) = self.crop {
                    self.change_geometry(Geometry::Crop(r));
                }
            }
            13 => self.cancel_crop(),
            14..=16 => {
                let before = self.editor.state;
                match index {
                    14 => self.editor.state.effect = self.editor.state.effect.next(),
                    15 => self.editor.state.effect = editor::Effect::None,
                    _ => self.editor.state.adjust = [0; 4],
                }
                self.editor.commit(before);
            }
            _ => {}
        }
    }
    fn resize_dialog(&mut self) {
        let (w, h) = self.editor.state.size;
        let prompt = if self.keep_aspect {
            "Width in pixels (height follows aspect ratio):"
        } else {
            "Enter width x height in pixels:"
        };
        let default_value = if self.keep_aspect {
            format!("{w}")
        } else {
            format!("{w} x {h}")
        };
        let req = DialogRequest::TextInput(TextInputRequest {
            common: DialogCommonOptions::new("Resize (nearest neighbor)", prompt),
            default_value,
            allow_empty: false,
        });
        match DialogClient::new().show(&req) {
            Ok(DialogResult::TextSubmitted(text)) => {
                let parsed = if self.keep_aspect {
                    text.trim().parse::<u32>().ok().and_then(|nw| {
                        let nh = (nw as u64 * h as u64 + w as u64 / 2) / w as u64;
                        u32::try_from(nh).ok().map(|nh| (nw, nh))
                    })
                } else {
                    text.split_once(['x', 'X']).and_then(|(a, b)| {
                        Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?))
                    })
                };
                if let Some((w, h)) = parsed {
                    self.change_geometry(Geometry::Resize(w, h));
                } else {
                    self.message
                        .set_str("Invalid dimensions. Enter positive whole numbers.");
                }
            }
            Err(_) => self.message.set_str("Resize dialog unavailable."),
            _ => {}
        }
    }
    fn confirm(&mut self, title: &str, message: &str) -> bool {
        match DialogClient::new().show(&DialogRequest::Confirm(ConfirmRequest {
            common: DialogCommonOptions::new(title, message),
            style: ConfirmStyle::YesNo,
            default_button: DialogButton::No,
        })) {
            Ok(DialogResult::Yes) => true,
            Err(_) => {
                self.message
                    .set_str("Confirmation dialog unavailable; action cancelled.");
                false
            }
            _ => false,
        }
    }
    pub(super) fn confirm_leave(&mut self) -> bool {
        if self.export.is_some() {
            self.message
                .set_str("Export in progress. Escape cancels it.");
            return false;
        }
        self.finish_slider();
        (!self.editor.dirty() && !self.editor.scene.has_annotations())
            || self.confirm(
                "Unsaved image",
                if self.editor.scene.has_annotations() {"Discard editable layers? Export saves flattened pixels only; objects are kept only in memory."}else{"Discard unsaved edits? Choose No to return and Save As."},
            )
    }
    fn save(&mut self, save_as: bool) {
        self.compare_key = false;
        self.compare_pointer = false;
        if !self.editable() {
            return;
        }
        self.finish_slider();
        if self.editor.scene.has_annotations() && !self.confirm("Export flattened image?","Visible layers will be flattened. Editable objects remain in memory only and cannot be reopened.") {return;}
        let mut path = self.current_path;
        if save_as
            || !matches!(
                format_from_name(path.file_name_bytes()),
                FormatKind::SimgLegacy | FormatKind::Tga
            )
        {
            let parent = path
                .parent()
                .and_then(|p| core::str::from_utf8(p.as_bytes()).ok().map(String::from));
            let req = DialogRequest::SaveFile(SaveFileRequest {
                title: String::from("Export image (TGA or SIMG v2)"),
                initial_dir: parent,
                suggested_name: Some(format!("{}.edited.simg", path.file_name_str())),
                default_extension: Some(String::from("simg")),
                allowed_extensions: vec![String::from("simg"), String::from("tga")],
                overwrite_confirm: false,
                confirm_button_label: Some(String::from("Export")),
            });
            match DialogClient::new().show(&req) {
                Ok(DialogResult::SavePathSelected(value)) => {
                    let Some(p) = PathBuf::from_bytes(value.as_bytes()) else {
                        self.message.set_str("Invalid or too long export path.");
                        return;
                    };
                    path = p;
                }
                Err(_) => {
                    self.message.set_str("Save dialog unavailable.");
                    return;
                }
                _ => {
                    self.message.set_str("Save cancelled.");
                    return;
                }
            }
        }
        let kind = match format_from_name(path.file_name_bytes()) {
            FormatKind::SimgLegacy => FormatKind::SimgV2,
            FormatKind::Tga => FormatKind::Tga,
            _ => {
                self.message
                    .set_str("Export requires .simg or .tga. PNG/JPEG encoding unavailable.");
                return;
            }
        };
        let replace = match libc::stat(path.as_bytes()) {
            Ok(_) => true,
            Err(libc::Errno::NoEntry) => false,
            Err(_) => {
                self.message
                    .set_str("Could not inspect export destination.");
                return;
            }
        };
        if replace
            && !self.confirm(
                "Replace image?",
                "Replace the selected existing file with the edited image?",
            )
        {
            return;
        }
        let result = self.start_export(path, kind, replace);
        if let Err(message) = result {
            self.message.set_str(message);
        }
    }
    fn start_export(
        &mut self,
        path: PathBuf,
        format: FormatKind,
        replace: bool,
    ) -> Result<(), &'static str> {
        let parent = path
            .parent()
            .ok_or("Export requires an absolute file path.")?;
        let mut temporary = None;
        for attempt in 0..32 {
            let name = format!(".light-lens-{}-{}.tmp", export_io::pid(), attempt);
            let candidate = parent
                .join(name.as_bytes())
                .ok_or("Temporary path is too long.")?;
            if candidate == path {
                continue;
            }
            match export_io::open_with_flags_mode(
                candidate.as_bytes(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            ) {
                Ok(fd) => {
                    temporary = Some((candidate, fd));
                    break;
                }
                Err(_) => continue,
            }
        }
        let (temporary, fd) = temporary.ok_or("Cannot reserve an export temporary file.")?;
        let state = self.editor.state;
        let (w, h) = state.size;
        let job = ExportJob {
            fd,
            temporary,
            destination: path,
            replace,
            position: 0,
            sampler: Sampler::new(state),
            state,
            format,
            file_size: w as u64 * h as u64 * 4 + if format == FormatKind::Tga { 18 } else { 36 },
        };
        let header = if format == FormatKind::Tga {
            sun_img::encode_tga_rgba32_header(w, h)
                .map_err(|_| "Invalid TGA export size.")?
                .to_vec()
        } else {
            sun_img::simg_v2::encode_raw_header(w, h)
                .map_err(|_| "Invalid SIMG export size.")?
                .to_vec()
        };
        export_io::write_all(fd, &header).map_err(|_| "Could not write export header.")?;
        self.export = Some(job);
        self.message
            .set_str("Exporting... Escape cancels. Editing paused.");
        Ok(())
    }
    fn export_tick(&mut self) -> bool {
        let Some(mut job) = self.export.take() else {
            return false;
        };
        let Some(image) = self.image.as_ref() else {
            return true;
        };
        let mut bytes = [0u8; 8192];
        let (w, h) = job.state.size;
        let len = job.sampler.write_bgra(image, job.position, &mut bytes);
        for (i, pixel) in bytes[..len].chunks_exact_mut(4).enumerate() {
            let n = job.position + i as u32;
            let base = u32::from_le_bytes(pixel.try_into().unwrap());
            let color = self.editor.scene.pixel(
                base,
                sunlight_light_lens::annotations::P::new(
                    (n % w) as f32 + 0.5,
                    (n / w) as f32 + 0.5,
                ),
            );
            pixel.copy_from_slice(&color.to_le_bytes());
        }
        let end = job.position + (len / 4) as u32;
        if export_io::write_all(job.fd, &bytes[..len]).is_err() {
            self.message
                .set_str("Export write failed; destination unchanged.");
            return true;
        }
        job.position = end;
        if end < w * h {
            self.export = Some(job);
            return false;
        }
        // Both rename APIs publish a complete file in the same directory. The
        // no-replace variant also protects a destination created after stat.
        let result = if job.replace {
            export_io::rename(job.temporary.as_bytes(), job.destination.as_bytes())
        } else {
            export_io::rename_no_replace(job.temporary.as_bytes(), job.destination.as_bytes())
        };
        if result.is_err() {
            self.message
                .set_str("Could not publish export; destination unchanged.");
            return true;
        }
        job.temporary = PathBuf::empty();
        self.current_path = job.destination;
        self.info.format = job.format;
        self.info.size_bytes = job.file_size;
        self.editor.mark_saved();
        self.message
            .set_str(if self.editor.scene.has_annotations() {
                "Flattened export saved. Editable layers remain in memory only."
            } else {
                "Saved image with alpha preserved."
            });
        #[cfg(not(test))]
        self.collect_siblings();
        true
    }
    pub(super) fn editor_event(&mut self, event: Event) -> bool {
        // Window decodes unmodified printable presses and Enter as Key,
        // while releases and Ctrl shortcuts remain raw KeyPress events.
        let event = match event {
            Event::Key(ch) => {
                let keycode = match ch {
                    '\\' => 0x2b,
                    '+' | '=' => 0x0d,
                    '-' => 0x0c,
                    'f' | 'F' => 0x21,
                    '1' => 0x02,
                    '\n' | '\r' => 0x1c,
                    _ => return false,
                };
                Event::key_press(keycode, true, false, false, false, false)
            }
            other => other,
        };
        if let Event::KeyPress {
            keycode: 0x2b,
            pressed,
            ctrl,
            alt,
            super_key,
            ..
        } = event
        {
            let active = pressed
                && !ctrl
                && !alt
                && !super_key
                && self.editable()
                && self.slider_drag.is_none()
                && self.crop_start.is_none()
                && self.crop_drag.is_none()
                && self.pan_start.is_none()
                && self.workspace.gesture.is_none();
            let changed = self.compare_key != active;
            self.compare_key = active;
            return changed;
        }
        if self.comparing() {
            match event {
                Event::Click { .. } | Event::MouseUp { button: 0, .. } => {
                    let changed = self.compare_pointer;
                    self.compare_pointer = false;
                    if matches!(event, Event::MouseUp { .. }) {
                        self.suppress_click = changed;
                    }
                    return changed;
                }
                Event::KeyPress {
                    keycode: KEY_ESC,
                    pressed: true,
                    ..
                } => {
                    self.compare_key = false;
                    self.compare_pointer = false;
                    return true;
                }
                Event::FocusChanged { focused: false }
                | Event::PointerOwnership {
                    owned: false,
                    captured: false,
                } => {
                    self.compare_key = false;
                    self.compare_pointer = false;
                    return true;
                }
                Event::Tick => return self.histogram_tick(),
                _ => return false,
            }
        }

        if self.export.is_some() {
            return match event {
                Event::Tick => self.export_tick(),
                Event::KeyPress {
                    keycode: KEY_ESC,
                    pressed: true,
                    ..
                } => {
                    self.export = None;
                    self.message
                        .set_str("Export cancelled; destination unchanged.");
                    true
                }
                _ => false,
            };
        }
        if !(matches!(event, Event::Click { .. }) && self.suppress_click) {
            if let Some(changed) = self.workspace_event(&event) {
                return changed;
            }
        }
        match event {
            Event::Tick => self.histogram_tick(),
            Event::Click { x, y } => {
                if self.suppress_click {
                    self.suppress_click = false;
                    return false;
                }
                // Window::poll_event emits Click instead of MouseUp for the
                // native left-button release. Complete captured drags first.
                if self.release_drag(x, y) {
                    return true;
                }
                let p = Point::new(x, y);
                if self.histogram_button().contains(p) {
                    self.workspace.histogram_channel = (self.workspace.histogram_channel + 1) % 5;
                    self.histogram_open = true;
                    return true;
                }
                if self.layout.back.contains(p) {
                    return self.show_previous();
                }
                if self.layout.next.contains(p) {
                    return self.show_next();
                }
                if !self.editable() {
                    return false;
                }
                for i in 0..6 {
                    if self.layout.tools[i].contains(p) {
                        self.finish_slider();
                        match i {
                            0 | 1 => {
                                let a = self.preview_area();
                                self.viewport.zoom_at(
                                    a,
                                    self.editor.state.size,
                                    Point::new(a.x + a.w as i32 / 2, a.y + a.h as i32 / 2),
                                    i == 0,
                                );
                            }
                            2 => self.change_geometry(Geometry::RotateLeft),
                            3 => self.change_geometry(Geometry::RotateRight),
                            4 => self.side_action(10),
                            _ => self.change_geometry(Geometry::FlipH),
                        }
                        return true;
                    }
                }
                if self.side_area().contains(p) {
                    for i in 0..18 {
                        if self.side_button(i).contains(p) {
                            self.side_action(i);
                            return true;
                        }
                    }
                    for i in 0..4 {
                        let r = self.slider_rect(i);
                        if Rect::new(r.right() - 34, r.y - 16, 34, 14).contains(p) {
                            return self.reset_slider(i);
                        }
                    }
                }
                false
            }
            Event::MouseWheel { x, y, delta } => {
                if delta == 0 {
                    return false;
                }
                let p = Point::new(x, y);
                if self.side_area().contains(p) {
                    let max = (478 - self.side_area().h as i32).max(0);
                    self.side_scroll =
                        (self.side_scroll + if delta > 0 { 32 } else { -32 }).clamp(0, max);
                    return true;
                }
                if self.editable()
                    && self.preview_area().contains(p)
                    && self.crop_start.is_none()
                    && self.crop_drag.is_none()
                    && self.pan_start.is_none()
                    && delta != 0
                {
                    return self.viewport.wheel_zoom(
                        self.preview_area(),
                        self.editor.state.size,
                        p,
                        delta,
                    );
                }
                false
            }
            Event::MouseDown { x, y, button: 0 } => {
                self.suppress_click = false;
                if !self.editable() {
                    return false;
                }
                let p = Point::new(x, y);
                if self.compare_button().contains(p) {
                    self.compare_pointer = true;
                    self.slider_focus = None;
                    return true;
                }
                if self.side_area().contains(p) {
                    for i in 0..4 {
                        let r = self.slider_rect(i);
                        let label = Rect::new(r.x, r.y - 16, r.w.saturating_sub(36), 16);
                        if r.contains(p) || label.contains(p) {
                            self.finish_slider();
                            let now = interaction_millis();
                            let double = self.slider_click.is_some_and(|(last, point, at)| {
                                last == i
                                    && now.saturating_sub(at) <= 400
                                    && (point.x - x).abs() <= 5
                                    && (point.y - y).abs() <= 5
                            });
                            self.slider_focus = Some(i);
                            self.slider_click = Some((i, p, now));
                            if double {
                                self.slider_click = None;
                                self.reset_slider(i);
                                self.suppress_click = true;
                            } else if r.contains(p) {
                                self.slider_drag = Some((i, self.editor.state));
                                self.set_slider(i, x);
                            }
                            return true;
                        }
                    }
                }
                self.slider_focus = None;
                self.slider_click = None;
                if self.preview_area().contains(p) {
                    if self.crop_mode {
                        let hit = self.crop_hit(p);
                        if hit != 0 {
                            let start =
                                editor::image_point(self.image_rect(), self.editor.state.size, p);
                            self.crop_drag = self.crop.map(|r| (r, start, hit));
                            self.crop_hover = hit;
                            set_client_cursor(Self::crop_cursor(hit));
                            return true;
                        }
                        if !self.image_rect().contains(p) {
                            return false;
                        }
                        let start =
                            editor::image_point(self.image_rect(), self.editor.state.size, p);
                        self.crop_start = Some(start);
                        self.crop = Some(Rect::new(start.x, start.y, 0, 0));
                    } else {
                        let dst = self.image_rect();
                        let area = self.preview_area();
                        if self.viewport.fit || (dst.w <= area.w && dst.h <= area.h) {
                            return false;
                        }
                        self.pan_start = Some(p);
                    }
                    return true;
                }
                false
            }
            Event::MouseMove { x, y } => {
                let p = Point::new(x, y);
                let mut hovered = None;
                if self.editable() {
                    if self.side_area().contains(p) {
                        hovered = (0..18)
                            .filter(|&i| self.side_enabled(i))
                            .map(|i| self.side_button(i))
                            .find(|r| r.contains(p));
                    }
                    if hovered.is_none() {
                        hovered = self
                            .layout
                            .tools
                            .iter()
                            .copied()
                            .chain([self.compare_button()])
                            .find(|r| r.contains(p));
                    }
                }
                for (r, enabled) in [
                    (self.layout.back, self.has_previous()),
                    (self.layout.next, self.has_next()),
                    (self.histogram_button(), true),
                ] {
                    if enabled && r.contains(p) {
                        hovered = Some(r);
                    }
                }
                let hover_changed = self.hovered != hovered;
                self.hovered = hovered;
                if let Some((i, _)) = self.slider_drag {
                    return self.set_slider(i, x);
                }
                if self.crop_start.is_some() || self.crop_drag.is_some() {
                    return self.update_crop(Point::new(x, y));
                }
                if let Some(start) = self.pan_start {
                    let old_pan = self.viewport.pan;
                    self.viewport.pan.x += x - start.x;
                    self.viewport.pan.y += y - start.y;
                    self.pan_start = Some(Point::new(x, y));
                    self.viewport
                        .clamp(self.preview_area(), self.editor.state.size);
                    return old_pan != self.viewport.pan;
                }
                let hover = self.crop_hit(Point::new(x, y));
                set_client_cursor(Self::crop_cursor(hover));
                let changed = self.crop_hover != hover;
                self.crop_hover = hover;
                changed || hover_changed
            }
            Event::MouseUp { x, y, button: 0 } => {
                let dragging = self.release_drag(x, y);
                self.suppress_click = dragging;
                dragging
            }
            Event::FocusChanged { focused: false }
            | Event::PointerOwnership {
                owned: false,
                captured: false,
            } => {
                self.finish_slider();
                self.crop_start = None;
                self.crop_drag = None;
                self.pan_start = None;
                self.slider_focus = None;
                self.slider_click = None;
                self.hovered = None;
                set_client_cursor(CursorShape::Pointer);
                true
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ctrl,
                shift,
                alt,
                super_key,
            } => {
                if alt || super_key {
                    return false;
                }
                self.finish_slider();
                self.pan_start = None;
                self.crop_start = None;
                self.crop_drag = None;
                if keycode == KEY_ESC {
                    if self.slider_focus.take().is_some() {
                        return true;
                    }
                    if self.crop_mode {
                        self.cancel_crop();
                    } else if self.confirm_leave() {
                        request_close();
                    }
                    return true;
                }
                if ctrl {
                    match keycode {
                        0x0b => {
                            self.side_action(2);
                            return self.editable();
                        }
                        0x02 => {
                            self.side_action(3);
                            return self.editable();
                        }
                        KEY_Q => {
                            if self.confirm_leave() {
                                request_close();
                            }
                            return true;
                        }
                        0x2c => {
                            self.history(shift);
                            return true;
                        }
                        0x15 => {
                            self.history(true);
                            return true;
                        }
                        0x1f => {
                            self.save(shift);
                            return true;
                        }
                        _ => {}
                    }
                }
                if ctrl {
                    return false;
                }
                if keycode == 0x1c && self.crop_mode {
                    self.side_action(12);
                    return true;
                }
                if !self.editable() {
                    return false;
                }
                if let Some(i) = self.slider_focus {
                    let delta = match keycode {
                        KEY_LEFT | 0x50 => -1,
                        KEY_RIGHT | 0x48 => 1,
                        _ => 0,
                    };
                    if delta != 0 {
                        let before = self.editor.state;
                        self.editor.state.adjust[i] =
                            (self.editor.state.adjust[i] + delta).clamp(-100, 100);
                        self.editor.commit(before);
                        return before != self.editor.state;
                    }
                    if keycode == 0x1c {
                        self.slider_focus = None;
                        return true;
                    }
                }
                match keycode {
                    KEY_LEFT if !self.crop_mode && self.slider_focus.is_none() => {
                        self.show_previous()
                    }
                    KEY_RIGHT if !self.crop_mode && self.slider_focus.is_none() => self.show_next(),
                    0x21 => {
                        self.side_action(2);
                        true
                    }
                    0x02 => {
                        self.side_action(3);
                        true
                    }
                    0x0d | 0x4e => {
                        let a = self.preview_area();
                        self.viewport.zoom_at(
                            a,
                            self.editor.state.size,
                            Point::new(a.x + a.w as i32 / 2, a.y + a.h as i32 / 2),
                            true,
                        );
                        true
                    }
                    0x0c | 0x4a => {
                        let a = self.preview_area();
                        self.viewport.zoom_at(
                            a,
                            self.editor.state.size,
                            Point::new(a.x + a.w as i32 / 2, a.y + a.h as i32 / 2),
                            false,
                        );
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> LightLensApp {
        let mut app = LightLensApp::new(None);
        app.set_client_bounds(WIN_W, WIN_H);
        app.ensure_layout();
        app.editor = Editor::new(100, 100);
        app.image = Some(RgbaImage {
            width: 100,
            height: 100,
            pixels: vec![0x80402010; 10000],
        });
        app.load_state = LoadState::Ready;
        app
    }
    fn key(code: u8, pressed: bool, ctrl: bool, shift: bool) -> Event {
        Event::key_press(code, pressed, shift, ctrl, false, false)
    }
    fn paint(app: &mut LightLensApp) {
        let r = app.client_bounds;
        let mut pixels = vec![0; r.w as usize * r.h as usize];
        app.view(
            &mut Canvas::new(&mut pixels, r.w, r.w, r.h),
            &Theme::sunlight_dark(),
        );
    }
    #[test]
    fn native_character_events_and_control_shortcuts_follow_keyboard_routing() {
        let mut app = ready();
        app.update(Event::key('\\'));
        assert!(app.comparing());
        app.update(key(0x2b, false, false, false));
        assert!(!app.comparing());
        app.update(key(0x02, true, true, false));
        assert!(!app.viewport.fit);
        assert_eq!(app.viewport.zoom, 1000);
        app.update(Event::key('+'));
        assert_eq!(app.viewport.zoom, 1250);
        app.update(Event::key('-'));
        assert_eq!(app.viewport.zoom, 1000);
        app.update(key(0x0b, true, true, false));
        assert!(app.viewport.fit);
        app.update(Event::key('1'));
        assert!(!app.viewport.fit);
        app.update(Event::key('f'));
        assert!(app.viewport.fit);
        app.crop_mode = true;
        app.crop = Some(Rect::new(10, 20, 50, 60));
        app.update(Event::key('\n'));
        assert_eq!(app.editor.state.size, (50, 60));
        app.update(key(0x2c, true, true, false));
        assert_eq!(app.editor.state.size, (100, 100));
        app.update(key(0x2c, true, true, true));
        assert_eq!(app.editor.state.size, (50, 60));
        app.update(key(0x2c, true, true, false));
        app.update(key(0x15, true, true, false));
        assert_eq!(app.editor.state.size, (50, 60));
        let state = app.editor.state;
        assert!(!app.update(Event::key('x')));
        assert!(!app.update(Event::key_press(0x02, true, false, false, true, false)));
        assert_eq!(app.editor.state, state);
    }
    #[test]
    fn comparison_preserves_state_checkpoint_history_and_viewport() {
        let mut app = ready();
        app.change_geometry(Geometry::Crop(Rect::new(10, 20, 40, 60)));
        app.change_geometry(Geometry::RotateRight);
        app.editor.mark_saved();
        app.side_action(3);
        app.viewport.zoom = 8000;
        app.viewport.pan = Point::new(-12, 18);
        let state = app.editor.state;
        let view = app.viewport;
        app.update(key(0x2b, true, false, false));
        assert!(app.comparing());
        paint(&mut app);
        assert!(!app.update(key(0x2b, true, false, false))); // repeat
        app.update(key(0x2b, false, false, false));
        assert!(!app.comparing());
        assert_eq!(app.editor.state, state);
        assert_eq!(app.viewport.pan, view.pan);
        assert_eq!(app.viewport.zoom, view.zoom);
        assert!(!app.editor.dirty());
        app.history(false);
        assert_eq!(app.editor.state.size, (40, 60));
        let b = app.compare_button();
        app.update(Event::mouse_down(b.x + 2, b.y + 2, 0));
        assert!(app.comparing());
        app.update(Event::FocusChanged { focused: false });
        assert!(!app.comparing());
        app.update(Event::mouse_down(b.x + 2, b.y + 2, 0));
        app.update(Event::click(0, 0));
        assert!(!app.comparing());
    }
    #[test]
    fn focused_sliders_fine_adjust_reset_and_noop_moves() {
        let mut app = ready();
        let r = app.slider_rect(1);
        app.update(Event::mouse_down(r.x + 10, r.y - 8, 0)); // label focuses
        app.update(Event::click(r.x + 10, r.y - 8));
        assert!(!app.editor.dirty());
        app.update(key(KEY_RIGHT, true, false, false));
        assert_eq!(app.editor.state.adjust[1], 1);
        app.update(key(KEY_LEFT, true, false, false));
        assert!(!app.editor.dirty());
        app.update(key(KEY_RIGHT, true, false, false));
        app.update(Event::mouse_down(r.x + 10, r.y - 8, 0)); // second label click resets
        app.update(Event::click(r.x + 10, r.y - 8));
        assert_eq!(app.editor.state.adjust[1], 0);
        app.history(false);
        assert_eq!(app.editor.state.adjust[1], 1);
        app.slider_click = None;
        app.update(Event::mouse_down(r.x + 80, r.y + 5, 0));
        assert!(!app.update(Event::mouse_move(r.x + 80, r.y + 5)));
        app.update(Event::FocusChanged { focused: false });
        assert!(app.slider_drag.is_none());
        assert!(app.slider_focus.is_none());
    }
    #[test]
    fn crop_handles_capture_move_and_reuse_preview_under_zoom_pan() {
        let mut app = ready();
        app.crop_mode = true;
        app.viewport = Viewport {
            zoom: 6000,
            fit: false,
            pan: Point::new(-30, 20),
        };
        app.viewport
            .clamp(app.preview_area(), app.editor.state.size);
        app.crop = Some(Rect::new(30, 30, 30, 30));
        paint(&mut app);
        let samples = app.preview_cache.borrow().sampled_pixels;
        let dst = app.image_rect();
        let r = app.crop_display(dst, app.crop.unwrap());
        let before = app.editor.state;
        app.update(Event::mouse_down(r.x, r.y, 0));
        assert!(app.crop_drag.is_some());
        assert!(app.pan_start.is_none());
        app.update(Event::mouse_move(r.x - 12, r.y - 12));
        paint(&mut app);
        assert_eq!(app.preview_cache.borrow().sampled_pixels, samples);
        app.update(Event::click(r.x - 12, r.y - 12));
        assert_eq!(app.crop, Some(Rect::new(28, 28, 32, 32)));
        assert_eq!(app.editor.state, before);
        app.update(key(0x1c, true, false, false));
        assert_eq!(app.editor.state.size, (32, 32));
        app.history(false);
        assert_eq!(app.editor.state, before);
        app.crop_mode = true;
        app.update(key(KEY_ESC, true, false, false));
        assert!(!app.crop_mode);
    }
    #[test]
    fn histogram_waits_for_slider_release_and_caches_current_edits() {
        let mut app = ready();
        app.histogram_open = true;
        app.workspace.right = 2;
        assert!(app.update(Event::Tick));
        assert!(!app.update(Event::Tick));
        let r = app.slider_rect(0);
        app.update(Event::mouse_down(r.x + 10, r.y + 4, 0));
        assert!(!app.update(Event::Tick));
        assert_ne!(app.histogram.as_ref().unwrap().0, app.editor.state);
        app.update(Event::click(r.x + 10, r.y + 4));
        assert!(app.update(Event::Tick));
        assert_eq!(app.histogram.as_ref().unwrap().0, app.editor.state);
        app.side_action(14);
        assert!(app.update(Event::Tick));
        app.history(false);
        assert!(app.update(Event::Tick));
    }
    struct TestDir(std::path::PathBuf);
    impl TestDir {
        fn new() -> Self {
            static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            let p = std::path::PathBuf::from(format!(
                "/tmp/light-lens-export-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn path(&self, name: &str) -> PathBuf {
            PathBuf::from_bytes(self.0.join(name).as_os_str().as_encoded_bytes()).unwrap()
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn save_publication_failure_cancel_and_write_failure_preserve_identity_and_checkpoint() {
        let dir = TestDir::new();
        let mut app = ready();
        app.current_path = dir.path("original.simg");
        std::fs::write(dir.0.join("original.simg"), b"original bytes").unwrap();
        let original = app.current_path;
        let before = app.editor.state;
        app.editor.state.adjust[0] = 20;
        app.editor.commit(before);
        // Simulate disk-full while writing the header.
        export_io::FAIL_WRITE.set(true);
        let failed = app.start_export(dir.path("header.simg"), FormatKind::SimgV2, false);
        export_io::FAIL_WRITE.set(false);
        assert!(failed.is_err());
        assert!(app.export.is_none());
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
        app.start_export(dir.path("cancel.simg"), FormatKind::SimgV2, false)
            .unwrap();
        app.update(key(KEY_ESC, true, false, false));
        assert!(app.export.is_none());
        app.start_export(original, FormatKind::SimgV2, true)
            .unwrap();
        export_io::FAIL_WRITE.set(true);
        app.export_tick();
        export_io::FAIL_WRITE.set(false);
        assert!(app.export.is_none());
        assert_eq!(
            std::fs::read(dir.0.join("original.simg")).unwrap(),
            b"original bytes"
        );
        app.start_export(dir.path("raced.simg"), FormatKind::SimgV2, false)
            .unwrap();
        std::fs::write(dir.0.join("raced.simg"), b"racing file").unwrap();
        while app.export.is_some() {
            app.export_tick();
        }
        assert_eq!(app.current_path.as_bytes(), original.as_bytes());
        assert!(app.editor.dirty());
        assert_eq!(
            std::fs::read(dir.0.join("raced.simg")).unwrap(),
            b"racing file"
        );
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
        let destination = dir.path("saved.simg");
        app.start_export(destination, FormatKind::SimgV2, false)
            .unwrap();
        assert_eq!(app.current_path.as_bytes(), original.as_bytes());
        assert!(app.editor.dirty());
        while app.export.is_some() {
            app.export_tick();
        }
        assert_eq!(app.current_path.as_bytes(), destination.as_bytes());
        assert!(!app.editor.dirty());
        let bytes = std::fs::read(dir.0.join("saved.simg")).unwrap();
        assert!(sun_img::decode_image(&bytes).is_ok());
        app.history(false);
        assert!(app.editor.dirty());
        app.history(true);
        assert!(!app.editor.dirty());
    }
    #[test]
    fn native_click_release_commits_one_slider_transaction() {
        let mut app = ready();
        let r = app.slider_rect(0);
        app.update(Event::mouse_down(r.x + 10, r.y + 4, 0));
        app.update(Event::mouse_move(r.x + 40, r.y + 4));
        app.update(Event::mouse_move(r.x + 80, r.y + 4));
        app.update(Event::click(r.right() - 1, r.y + 4));
        assert!(app.slider_drag.is_none());
        assert!(app.editor.dirty());
        app.update(Event::mouse_move(r.x, r.y));
        assert_eq!(app.editor.state.adjust[0], 100);
        app.history(false);
        assert_eq!(app.editor.state.adjust[0], 0);
        assert!(!app.editor.can_undo());
    }
    #[test]
    fn zero_crop_can_be_restarted_without_invalid_handle_math() {
        let mut app = ready();
        app.crop_mode = true;
        let r = app.image_rect();
        let p = Point::new(r.x + 20, r.y + 20);
        app.update(Event::mouse_down(p.x, p.y, 0));
        app.update(Event::click(p.x, p.y));
        assert!(!app.side_enabled(12));
        app.update(Event::mouse_down(p.x, p.y, 0));
        app.update(Event::click(p.x + 100, p.y + 100));
        assert!(app.side_enabled(12));
    }
    #[test]
    fn crop_selection_does_not_edit_and_native_release_stops_capture() {
        let mut app = ready();
        app.crop_mode = true;
        app.viewport = Viewport {
            zoom: 4000,
            fit: false,
            pan: Point::new(20, -10),
        };
        app.viewport
            .clamp(app.preview_area(), app.editor.state.size);
        let dst = app.image_rect();
        let before = app.editor.state;
        let a = Point::new(dst.x + 80, dst.y + 80);
        let b = Point::new(dst.x + 240, dst.y + 280);
        app.update(Event::mouse_down(a.x, a.y, 0));
        app.update(Event::mouse_move(b.x, b.y));
        app.update(Event::click(b.x, b.y));
        assert_eq!(app.crop, Some(Rect::new(20, 20, 40, 50)));
        assert!(app.crop_start.is_none());
        assert!(app.pan_start.is_none());
        assert_eq!(app.editor.state, before);
        app.side_action(12);
        assert_eq!(app.editor.state.size, (40, 50));
        assert!(app.editor.dirty());
        app.history(false);
        assert_eq!(app.editor.state, before);
        assert!(app.image.unwrap().pixels.iter().all(|p| *p == 0x80402010));
    }
    #[test]
    fn clicking_fitted_image_preserves_fit_after_resize() {
        let mut app = ready();
        let r = app.image_rect();
        let p = Point::new(r.x + 10, r.y + 10);
        app.update(Event::mouse_down(p.x, p.y, 0));
        assert_eq!(app.image_rect(), r);
        app.update(Event::click(p.x + 10, p.y));
        assert!(app.pan_start.is_none());
        assert!(app.viewport.fit);
        app.window_event(WindowEvent::Resized {
            width: 1024,
            height: 768,
        });
        assert_eq!(
            app.image_rect(),
            editor::fit_rect(app.preview_area(), app.editor.state.size).unwrap()
        );
        assert!(!app.editor.dirty());
    }
    #[test]
    fn edited_preview_clips_and_composites_alpha() {
        let app = ready();
        let mut pixels = vec![0xff204060; 400];
        let mut c = Canvas::new(&mut pixels, 20, 20, 20);
        {
            let mut preview = c.sub_canvas(Rect::new(0, 5, 20, 15));
            app.draw_edited_photo(
                &mut preview,
                app.image.as_ref().unwrap(),
                Rect::new(-5, -5, 30, 30),
            );
        }
        assert!(pixels[..100].iter().all(|p| *p == 0xff204060));
        assert!(pixels[100..].iter().all(|p| *p == 0xff303038));
    }
    #[test]
    fn scrolled_buttons_and_sliders_use_painted_coordinates() {
        let mut app = ready();
        app.set_client_bounds(900, 420);
        let area = app.side_area();
        app.update(Event::mouse_wheel(area.x + 8, area.y + 8, 120));
        assert!(app.side_scroll > 0);
        let r = app.side_button(9);
        app.update(Event::click(r.x + 5, r.y + 5));
        assert!(!app.keep_aspect);
        for _ in 0..4 {
            app.update(Event::mouse_wheel(area.x + 8, area.y + 8, 120));
        }
        let r = app.slider_rect(2);
        app.update(Event::mouse_down(r.x + 20, r.y + 4, 0));
        app.update(Event::click(r.x + 20, r.y + 4));
        assert!(app.editor.dirty());
    }
    #[test]
    fn sample_crop_histogram_and_comparison_render_at_requested_sizes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/images/Samples");
        let file = std::fs::read_dir(dir)
            .unwrap()
            .map(|f| f.unwrap().path())
            .find(|p| p.extension().and_then(|e| e.to_str()) == Some("simg"))
            .unwrap();
        let image = decode_simg(&std::fs::read(&file).unwrap()).unwrap();
        let mut app = ready();
        app.editor = Editor::new(image.width, image.height);
        app.image = Some(image);
        app.current_path = PathBuf::from_bytes(b"/Pictures/Sample.simg").unwrap();
        app.message.clear();
        app.crop_mode = true;
        app.crop = Some(Rect::new(200, 150, 800, 450));
        app.histogram_open = true;
        app.workspace.right = 2;
        app.histogram_tick();
        for (w, h) in [(1024, 768), (1366, 768), (1600, 900), (1920, 1080)] {
            app.window_event(WindowEvent::Resized {
                width: w,
                height: h,
            });
            let hist = app.histogram_button();
            assert!(hist.bottom() + 70 <= app.layout.right.bottom());
            let mut pixels = vec![0; w as usize * h as usize];
            app.view(
                &mut Canvas::new(&mut pixels, w, w, h),
                &Theme::sunlight_dark(),
            );
            if std::env::var_os("LIGHT_LENS_SCREENSHOT").is_some() {
                let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
                for p in &pixels {
                    bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
                }
                std::fs::write(format!("/tmp/light-lens-phase2-{w}.ppm"), bytes).unwrap();
            }
            app.update(key(0x2b, true, false, false));
            app.view(
                &mut Canvas::new(&mut pixels, w, w, h),
                &Theme::sunlight_dark(),
            );
            app.update(key(0x2b, false, false, false));
        }
    }
    #[test]
    fn editor_renders_at_small_and_large_client_sizes() {
        let mut app = ready();
        for (w, h) in [
            (1220, 760),
            (1024, 768),
            (1366, 768),
            (1600, 900),
            (1920, 1080),
            (900, 520),
            (760, 420),
            (400, 120),
            (1, 1),
            (0, 0),
        ] {
            app.window_event(WindowEvent::Resized {
                width: w,
                height: h,
            });
            let mut pixels = vec![0; w as usize * h as usize];
            let mut canvas = Canvas::new(&mut pixels, w, w, h);
            app.view(&mut canvas, &Theme::sunlight_dark());
            if w == 1220 && std::env::var_os("LIGHT_LENS_SCREENSHOT").is_some() {
                let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
                for pixel in pixels {
                    bytes.extend_from_slice(&[
                        (pixel >> 16) as u8,
                        (pixel >> 8) as u8,
                        pixel as u8,
                    ]);
                }
                std::fs::write("/tmp/light-lens-preview.ppm", bytes).unwrap();
            }
        }
    }
    fn annotation_point(app: &LightLensApp, x: f32, y: f32) -> Point {
        let r = app.image_rect();
        let (w, h) = app.editor.state.size;
        Point::new(
            r.x + (x * r.w as f32 / w as f32) as i32,
            r.y + (y * r.h as f32 / h as f32) as i32,
        )
    }
    fn annotation_ready() -> LightLensApp {
        let mut app = ready();
        app.workspace.left = 1;
        app.editor
            .edit_scene(|s| {
                s.add_layer()?;
                Ok(())
            })
            .unwrap();
        app
    }
    #[test]
    fn annotation_drag_selection_resize_cancel_delete_are_single_transactions() {
        use sunlight_light_lens::annotations::Tool;
        let mut app = annotation_ready();
        app.workspace.tool = Tool::Rectangle;
        let a = annotation_point(&app, 20., 20.);
        let b = annotation_point(&app, 60., 50.);
        app.update(Event::mouse_down(a.x, a.y, 0));
        app.update(Event::mouse_move(b.x, b.y));
        assert_eq!(app.editor.scene.count(), 0);
        app.update(Event::click(b.x, b.y));
        assert_eq!(app.editor.scene.count(), 1);
        let id = app.workspace.selected.unwrap();
        let original = app.editor.scene.object(id).unwrap().clone();
        app.history(false);
        assert_eq!(app.editor.scene.count(), 0);
        app.history(true);
        assert_eq!(app.editor.scene.count(), 1);
        app.workspace.tool = Tool::Select;
        let start = annotation_point(&app, 20., 30.);
        let end = annotation_point(&app, 30., 40.);
        app.update(Event::mouse_down(start.x, start.y, 0));
        app.update(Event::mouse_move(end.x, end.y));
        app.update(Event::click(end.x, end.y));
        let moved = app.editor.scene.object(id).unwrap().clone();
        assert_ne!(moved.transform, original.transform);
        app.history(false);
        assert_eq!(app.editor.scene.object(id).unwrap(), &original);
        app.history(true);
        // Re-select, then drag the visible lower-right resize handle.
        app.update(Event::mouse_down(end.x, end.y, 0));
        app.update(Event::click(end.x, end.y));
        let (_, corner) = app.editor.scene.object(id).unwrap().bounds();
        let p = annotation_point(&app, corner.x, corner.y);
        let q = annotation_point(&app, corner.x + 10., corner.y + 10.);
        app.update(Event::mouse_down(p.x, p.y, 0));
        assert!(app.workspace.gesture.as_ref().unwrap().resizing);
        app.update(Event::mouse_move(q.x, q.y));
        app.update(Event::click(q.x, q.y));
        assert_ne!(
            app.editor.scene.object(id).unwrap().transform,
            moved.transform
        );
        app.history(false);
        assert_eq!(app.editor.scene.object(id).unwrap(), &moved);
        app.workspace.tool = Tool::Ellipse;
        app.update(Event::mouse_down(a.x, a.y, 0));
        app.update(Event::mouse_move(b.x, b.y));
        app.update(key(KEY_ESC, true, false, false));
        app.update(Event::click(b.x, b.y));
        assert_eq!(app.editor.scene.count(), 1);
        app.workspace.tool = Tool::Select;
        app.update(Event::mouse_down(end.x, end.y, 0));
        app.update(Event::click(end.x, end.y));
        assert_eq!(app.workspace.selected, Some(id));
        app.update(key(0x53, true, false, false));
        assert_eq!(app.editor.scene.count(), 0);
        app.history(false);
        assert_eq!(app.editor.scene.count(), 1);
    }
    #[test]
    fn zoom_pan_mapping_and_annotation_interactions_reuse_background_samples() {
        use sunlight_light_lens::annotations::{Tool, P};
        let mut app = annotation_ready();
        app.viewport = Viewport {
            fit: false,
            zoom: 400,
            pan: Point::new(10, -10),
        };
        app.viewport
            .clamp(app.preview_area(), app.editor.state.size);
        paint(&mut app);
        let samples = app.preview_cache.borrow().sampled_pixels;
        app.workspace.tool = Tool::Line;
        let a = annotation_point(&app, 20., 30.);
        let b = annotation_point(&app, 50., 60.);
        app.update(Event::mouse_down(a.x, a.y, 0));
        for n in 0..4 {
            app.update(Event::mouse_move(b.x + n, b.y));
            paint(&mut app);
        }
        app.update(Event::click(b.x, b.y));
        paint(&mut app);
        let id = app.workspace.selected.unwrap();
        let o = app.editor.scene.object(id).unwrap();
        assert_eq!(o.a, P::new(20., 30.));
        assert_eq!(o.b, P::new(50., 60.));
        assert_eq!(app.preview_cache.borrow().sampled_pixels, samples);
        app.workspace.right = 2;
        app.histogram_open = true;
        assert!(app.update(Event::Tick));
        assert!(!app.update(Event::Tick));
        let cached = app.histogram_scene.clone().unwrap();
        app.editor
            .edit_scene(|s| {
                s.layer_mut().unwrap().visible = false;
                Ok(())
            })
            .unwrap();
        assert!(app.update(Event::Tick));
        assert_ne!(cached.layers, app.histogram_scene.as_ref().unwrap().layers);
        app.workspace.histogram_channel = 4;
        assert!(!app.update(Event::Tick));
        app.editor
            .edit_scene(|s| {
                let l = s.layer_mut().unwrap();
                l.name = String::from("Notes");
                l.locked = true;
                Ok(())
            })
            .unwrap();
        assert!(!app.update(Event::Tick));
    }
    #[test]
    fn flattened_streaming_exports_match_composite_and_retain_editable_history() {
        use sunlight_light_lens::annotations::{Object, Style, Tool, P};
        let dir = TestDir::new();
        let mut app = annotation_ready();
        let mut o = Object::new(
            Tool::Rectangle,
            P::new(10., 10.),
            Style {
                fill: Some(0xffe03020),
                color: 0xffe03020,
                ..Style::default()
            },
        );
        o.b = P::new(60., 60.);
        app.editor
            .edit_scene(|s| {
                s.insert(o)?;
                s.layer_mut().unwrap().opacity = 128;
                s.add_layer()?;
                let mut hidden = Object::new(Tool::Ellipse, P::new(5., 5.), Style::default());
                hidden.b = P::new(80., 80.);
                s.insert(hidden)?;
                s.layer_mut().unwrap().visible = false;
                Ok(())
            })
            .unwrap();
        let scene = app.editor.scene.clone();
        for (name, kind) in [
            ("layers.tga", FormatKind::Tga),
            ("layers.simg", FormatKind::SimgV2),
        ] {
            app.start_export(dir.path(name), kind, false).unwrap();
            while app.export.is_some() {
                app.export_tick();
            }
            let bytes = std::fs::read(dir.0.join(name)).unwrap();
            let image = if kind == FormatKind::Tga {
                decode_simg(&bytes).unwrap()
            } else {
                decode_simg(&bytes).unwrap()
            };
            assert_eq!(image.width, 100);
            for (x, y) in [(0, 0), (10, 10), (20, 20), (99, 99)] {
                let base = app.image.as_ref().unwrap().pixel(x, y);
                assert_eq!(
                    image.pixel(x, y),
                    scene.pixel(base, P::new(x as f32 + 0.5, y as f32 + 0.5))
                );
            }
            assert_eq!(app.editor.scene, scene);
            assert!(!app.editor.dirty());
            assert!(app.editor.scene.has_annotations());
            assert!(app.message.as_str().contains("Flattened"));
        }
        app.history(false);
        assert!(app.editor.dirty());
        app.history(true);
        assert!(!app.editor.dirty());
        assert_eq!(app.editor.scene, scene);
        let path = app.current_path;
        export_io::FAIL_WRITE.set(true);
        assert!(app
            .start_export(dir.path("failed.simg"), FormatKind::SimgV2, false)
            .is_err());
        export_io::FAIL_WRITE.set(false);
        assert!(app.current_path == path);
        assert_eq!(app.editor.scene, scene);
        assert!(!app.editor.dirty());
    }
    #[test]
    fn panels_collapse_keep_tabs_accessible_and_scroll_at_small_heights() {
        let mut app = annotation_ready();
        let full = app.layout.viewport.w;
        for left in [true, false] {
            let r = app.panel_toggle(left);
            app.update(Event::click(r.x + 2, r.y + 2));
        }
        assert!(app.workspace.collapsed_left && app.workspace.collapsed_right);
        assert!(app.layout.viewport.w > full + 300);
        paint(&mut app);
        for left in [true, false] {
            let r = app.tab_rect(left, 1);
            app.update(Event::click(r.x + 2, r.y + 2));
        }
        assert!(!app.workspace.collapsed_left && !app.workspace.collapsed_right);
        assert_eq!(app.workspace.left, 1);
        assert_eq!(app.workspace.right, 1);
        for (w, h) in [(0, 0), (320, 150), (640, 400), (800, 600), (1024, 768)] {
            app.set_client_bounds(w, h);
            for tab in 0..3 {
                app.workspace.right = tab;
                paint(&mut app);
            }
            app.workspace.left = 0;
            paint(&mut app);
            app.workspace.left = 1;
            paint(&mut app);
        }
        app.set_client_bounds(640, 400);
        app.workspace.right = 0;
        let r = app.layout.right;
        app.update(Event::mouse_wheel(r.x + 20, r.y + 60, 120));
        assert!(app.workspace.layer_scroll > 0);
        paint(&mut app);
    }
    #[test]
    fn annotated_sample_renders_tabs_and_layers_at_multiple_sizes() {
        use sunlight_light_lens::annotations::{Object, Style, TextMask, Tool, P};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/images/Samples");
        let file = std::fs::read_dir(dir)
            .unwrap()
            .map(|f| f.unwrap().path())
            .find(|p| p.extension().and_then(|s| s.to_str()) == Some("simg"))
            .unwrap();
        let image = decode_simg(&std::fs::read(file).unwrap()).unwrap();
        let mut app = ready();
        app.editor = Editor::new(image.width, image.height);
        app.image = Some(image);
        app.workspace.left = 1;
        app.workspace.scroll = 90;
        app.editor
            .edit_scene(|s| {
                s.add_layer()?;
                for (tool, a, b) in [
                    (Tool::Rectangle, P::new(160., 180.), P::new(620., 580.)),
                    (Tool::Arrow, P::new(850., 180.), P::new(650., 390.)),
                    (Tool::Ellipse, P::new(750., 420.), P::new(1120., 680.)),
                ] {
                    let mut o = Object::new(
                        tool,
                        a,
                        Style {
                            width: 6,
                            ..Style::default()
                        },
                    );
                    o.b = b;
                    s.insert(o)?;
                }
                let mut text = Object::new(
                    Tool::Text,
                    P::new(200., 100.),
                    Style {
                        font_size: 44,
                        ..Style::default()
                    },
                );
                text.text = Some(alloc::rc::Rc::new(TextMask::new("Light Lens")?));
                s.insert(text)?;
                Ok(())
            })
            .unwrap();
        for (w, h) in [(640, 480), (1024, 768), (1366, 768), (1920, 1080)] {
            app.set_client_bounds(w, h);
            let mut pixels = vec![0; w as usize * h as usize];
            app.view(
                &mut Canvas::new(&mut pixels, w, w, h),
                &Theme::sunlight_dark(),
            );
            if std::env::var_os("LIGHT_LENS_SCREENSHOT").is_some() {
                let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
                for p in &pixels {
                    bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
                }
                std::fs::write(format!("/tmp/light-lens-phase3-{w}.ppm"), bytes).unwrap();
            }
        }
    }
    #[test]
    fn text_content_style_and_layer_commands_share_history_without_changing_glyph_cache() {
        use alloc::rc::Rc;
        use sunlight_light_lens::annotations::{Object, Style, TextMask, Tool, P};
        let mut app = annotation_ready();
        let mut text = Object::new(Tool::Text, P::new(10., 10.), Style::default());
        text.text = Some(Rc::new(TextMask::new("First").unwrap()));
        let mut id = 0;
        app.editor
            .edit_scene(|s| {
                id = s.insert(text)?;
                Ok(())
            })
            .unwrap();
        let first = app.editor.scene.clone();
        let mut text = app.editor.scene.object(id).unwrap().clone();
        text.text = Some(Rc::new(TextMask::new("Edited").unwrap()));
        text.style.font_size = 40;
        text.style.color = 0xff00ff00;
        app.editor.edit_scene(|s| s.replace(text)).unwrap();
        let edited = app.editor.scene.clone();
        app.history(false);
        assert_eq!(app.editor.scene, first);
        app.history(true);
        assert_eq!(app.editor.scene, edited);
        let glyphs = app.editor.scene.object(id).unwrap().text.clone().unwrap();
        app.change_geometry(Geometry::RotateRight);
        assert!(Rc::ptr_eq(
            &glyphs,
            app.editor.scene.object(id).unwrap().text.as_ref().unwrap()
        ));
        app.history(false);
        app.workspace.selected = Some(id);
        let before = app.editor.scene.object(id).unwrap().transform;
        app.update(key(KEY_RIGHT, true, false, false));
        assert_ne!(app.editor.scene.object(id).unwrap().transform, before);
        app.history(false);
        assert_eq!(app.editor.scene.object(id).unwrap().transform, before);
        app.editor
            .edit_scene(|s| {
                s.layer_mut().unwrap().locked = true;
                Ok(())
            })
            .unwrap();
        app.workspace.selected = Some(id);
        app.update(key(0x53, true, false, false));
        assert!(app.editor.scene.object(id).is_some());
        app.history(false);
        app.workspace.selected = Some(id);
        app.update(key(0x53, true, false, false));
        assert!(app.editor.scene.object(id).is_none());
        app.history(false);
        assert!(app.editor.scene.object(id).is_some());
    }

    #[test]
    fn shared_color_picker_preserves_cancel_opacity_and_scene_history() {
        use sunlight_light_lens::annotations::{Object, Style, Tool, P};
        let mut app = annotation_ready();
        let mut id = 0;
        let mut other_id = 0;
        app.editor
            .edit_scene(|s| {
                let mut object = Object::new(
                    Tool::Rectangle,
                    P::new(10., 10.),
                    Style {
                        opacity: 96,
                        fill: Some(0xff123456),
                        ..Style::default()
                    },
                );
                object.b = P::new(25., 25.);
                id = s.insert(object)?;
                let mut other = Object::new(Tool::Ellipse, P::new(30., 30.), Style::default());
                other.b = P::new(50., 50.);
                other_id = s.insert(other)?;
                Ok(())
            })
            .unwrap();
        app.workspace.selected = Some(id);
        app.workspace.style = app.selected_object().unwrap().style;
        let scene = app.editor.scene.clone();
        let style = app.workspace.style;
        app.apply_annotation_color_result(Ok(DialogResult::Cancelled));
        app.apply_annotation_color_result(Err(sunlight_dialogs::DialogError::HostUnavailable));
        app.apply_annotation_color_result(Ok(DialogResult::ColorSelected(String::from(
            "#11223344",
        ))));
        assert_eq!(app.workspace.style, style);
        assert_eq!(app.editor.scene, scene);
        app.apply_annotation_color_result(Ok(DialogResult::ColorSelected(String::from("#00aaff"))));
        let after = app.editor.scene.clone();
        assert_eq!(app.workspace.style.color, 0xff00aaff);
        assert_eq!(app.editor.scene.object(id).unwrap().style.opacity, 96);
        assert_eq!(
            app.editor.scene.object(id).unwrap().style.fill,
            Some(0xff123456)
        );
        assert_eq!(app.editor.scene.object(other_id), scene.object(other_id));
        app.history(false);
        assert_eq!(app.editor.scene, scene);
        app.history(true);
        assert_eq!(app.editor.scene, after);
        // Cancelling and reselecting the same color create no extra transaction.
        app.workspace.selected = Some(id);
        app.apply_annotation_color_result(Ok(DialogResult::Cancelled));
        app.apply_annotation_color_result(Ok(DialogResult::ColorSelected(String::from("#00AAFF"))));
        app.history(false);
        assert_eq!(app.editor.scene, scene);
        // Undo may leave the tool's current style unchanged. Selecting that
        // color again still recolors the selected object through one transaction.
        app.workspace.selected = Some(id);
        app.apply_annotation_color_result(Ok(DialogResult::ColorSelected(String::from("#00AAFF"))));
        assert_eq!(app.editor.scene, after);
        app.history(false);
        assert_eq!(app.editor.scene, scene);
        app.workspace.selected = None;
        app.apply_annotation_color_result(Ok(DialogResult::ColorSelected(String::from("#FF0000"))));
        assert_eq!(app.workspace.style.color, 0xffff0000);
        assert_eq!(app.editor.scene, scene);
    }

    #[test]
    fn dialog_hex_wire_output_updates_label_and_new_annotation_pixels() {
        use sunlight_dialogs::{decode_result, encode_result};
        use sunlight_light_lens::annotations::{Tool, P};
        for hex in ["#302513", "#00aaff", "#000000", "#FFFFFF", "12AB34"] {
            let mut app = annotation_ready();
            let payload = encode_result(&DialogResult::ColorSelected(String::from(hex)));
            let decoded = decode_result(&payload).unwrap();
            app.apply_annotation_color_result(Ok(decoded));
            let expected =
                0xff000000 | u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
            assert_eq!(app.workspace.style.color, expected, "{hex}");
            assert_eq!(
                app.annotation_labels()[8],
                format!("Color #{:06X}", expected & 0xffffff)
            );
            assert_eq!(
                app.message.as_str(),
                format!("Annotation color: #{:06X}", expected & 0xffffff)
            );
            app.workspace.tool = Tool::Rectangle;
            let a = annotation_point(&app, 20., 20.);
            let b = annotation_point(&app, 60., 50.);
            app.update(Event::mouse_down(a.x, a.y, 0));
            app.update(Event::mouse_move(b.x, b.y));
            app.update(Event::click(b.x, b.y));
            let object = app.selected_object().unwrap();
            assert_eq!(object.style.color, expected);
            let edge = P::new(object.a.x, (object.a.y + object.b.y) / 2.);
            assert_eq!(app.editor.scene.pixel(0xff000000, edge), expected);
        }
    }

    #[test]
    fn color_result_errors_are_visible_and_preserve_current_color() {
        let mut app = annotation_ready();
        let original = app.workspace.style;
        app.apply_annotation_color_result(Err(sunlight_dialogs::DialogError::Corrupt));
        assert_eq!(app.message.as_str(), "Native color dialog failed: Corrupt.");
        app.apply_annotation_color_result(Ok(DialogResult::TextSubmitted(String::from("#123456"))));
        assert_eq!(app.message.as_str(), "Unexpected color dialog result.");
        app.apply_annotation_color_result(Ok(DialogResult::Error(String::from(
            "Dialog host failed",
        ))));
        assert_eq!(app.message.as_str(), "Dialog host failed");
        assert_eq!(app.workspace.style, original);
    }
}
