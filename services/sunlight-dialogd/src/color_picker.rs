//! The dialog host's single color picker. No application owns picker UI state.
use alloc::{string::String, vec::Vec};
use core::fmt::Write;
use sun_font::{draw_text, FontRole, TextStyle};
use sunlight_dialogs::{
    color::{axis_fraction, byte, wrap_hue, Hsv, RecentColors, Rgba, PRESETS},
    ColorPickerRequest, DialogResult,
};
use sunlight_ui::{
    widgets::{Button, ButtonState, TextInput},
    Canvas, Color, Event, Point, Rect, Theme,
};

pub const WIDTH: u32 = 460;
pub const HEIGHT: u32 = 452;
pub const ALPHA_HEIGHT: u32 = 486;

#[derive(Clone, Copy)]
struct Layout {
    sv: Rect,
    hue: Rect,
    alpha: Rect,
    original: Rect,
    preview: Rect,
    fields: [Rect; 4],
    presets: Rect,
    recent: Rect,
    ok: Rect,
    cancel: Rect,
}

impl Layout {
    fn new(width: u32, height: u32, alpha: bool) -> Self {
        // Bound gradient dimensions even if a compositor offers a larger surface.
        let w = width.min(WIDTH).saturating_sub(36);
        let sv_w = w.saturating_sub(128).clamp(1, 288);
        let sv_h = height
            .saturating_sub(if alpha { 346 } else { 312 })
            .clamp(1, 160);
        let sv = Rect::new(18, 42, sv_w, sv_h);
        let hue = Rect::new(18, sv.bottom() + 10, sv_w, 18);
        let a = Rect::new(18, hue.bottom() + 10, sv_w, 18);
        let field_y = if alpha {
            a.bottom() + 26
        } else {
            hue.bottom() + 26
        };
        let field_w = w.saturating_sub(18) / 3;
        let rgb_y = field_y + 50;
        let fields = [
            Rect::new(18, field_y, w.min(180), 28),
            Rect::new(18, rgb_y, field_w, 28),
            Rect::new(18 + field_w as i32 + 9, rgb_y, field_w, 28),
            Rect::new(18 + 2 * (field_w as i32 + 9), rgb_y, field_w, 28),
        ];
        let presets = Rect::new(18, rgb_y + 52, w, 22);
        let recent = Rect::new(18, presets.bottom() + 22, w, 22);
        let ok = Rect::new(
            (width.min(WIDTH) as i32 - 114).max(18),
            height as i32 - 44,
            96,
            28,
        );
        Self {
            sv,
            hue,
            alpha: a,
            fields,
            presets,
            recent,
            ok,
            cancel: Rect::new((ok.x - 106).max(18), ok.y, 96, 28),
            original: Rect::new(sv.right() + 18, 62, w.saturating_sub(sv_w + 18), 42),
            preview: Rect::new(sv.right() + 18, 132, w.saturating_sub(sv_w + 18), 42),
        }
    }

    fn swatch(row: Rect, index: usize) -> Rect {
        let step = (row.w / 10).min(40);
        Rect::new(
            row.x + index as i32 * step as i32,
            row.y,
            step.saturating_sub(5),
            row.h,
        )
    }
}

pub struct ColorPicker {
    title: String,
    original: Rgba,
    color: Rgba,
    hsv: Hsv,
    allow_alpha: bool,
    inputs: [TextInput<'static, 16>; 4],
    valid: [bool; 4],
    // Focus IDs: SV, hue, alpha, HEX, RGB, presets, recents, Cancel, OK.
    focus: usize,
    drag: Option<usize>,
    suppress_click: bool,
    press_origin: Option<Point>,
    hover: Option<Point>,
    recent: RecentColors,
    gradient: Vec<u32>,
    gradient_key: Option<(u32, u32, u32)>,
    layout: Layout,
    result: Option<DialogResult>,
}

impl ColorPicker {
    /// Requests are validated at the shared API/host boundary.
    pub fn new(request: ColorPickerRequest, recent: RecentColors) -> Self {
        let original = Rgba::parse_hex(&request.initial_color, request.allow_alpha)
            .unwrap_or(Rgba::rgb(0, 0, 0));
        let layout = Layout::new(
            WIDTH,
            if request.allow_alpha {
                ALPHA_HEIGHT
            } else {
                HEIGHT
            },
            request.allow_alpha,
        );
        let mut picker = Self {
            title: request.title,
            original,
            color: original,
            hsv: original.hsv(),
            allow_alpha: request.allow_alpha,
            inputs: core::array::from_fn(|i| {
                TextInput::new(layout.fields[i]).with_clipboard_source(b"sunlight-dialogd")
            }),
            valid: [true; 4],
            focus: 0,
            drag: None,
            suppress_click: false,
            press_origin: None,
            hover: None,
            recent,
            gradient: Vec::new(),
            gradient_key: None,
            layout,
            result: None,
        };
        picker.sync_inputs(None);
        picker
    }

    pub fn result(&self) -> Option<&DialogResult> {
        self.result.as_ref()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.layout = Layout::new(
            width.min(i32::MAX as u32),
            height.min(i32::MAX as u32),
            self.allow_alpha,
        );
        for (i, input) in self.inputs.iter_mut().enumerate() {
            input.rect = self.layout.fields[i];
        }
        self.drag = None;
    }

    fn set_focus(&mut self, focus: usize) {
        self.focus = focus;
        for (i, input) in self.inputs.iter_mut().enumerate() {
            input.active = focus == i + 3;
        }
    }

    fn sync_inputs(&mut self, except: Option<usize>) {
        // Stack storage keeps pointer drags free of temporary heap buffers.
        for i in 0..4 {
            if except != Some(i) {
                let mut value = heapless::String::<16>::new();
                match i {
                    0 => {
                        let _ = write!(
                            value,
                            "#{:02X}{:02X}{:02X}",
                            self.color.r, self.color.g, self.color.b
                        );
                        if self.allow_alpha {
                            let _ = write!(value, "{:02X}", self.color.a);
                        }
                    }
                    1 => {
                        let _ = write!(value, "{}", self.color.r);
                    }
                    2 => {
                        let _ = write!(value, "{}", self.color.g);
                    }
                    _ => {
                        let _ = write!(value, "{}", self.color.b);
                    }
                }
                self.inputs[i].set_text(&value);
                self.valid[i] = true;
            }
        }
    }

    fn set_color(&mut self, mut color: Rgba, editing: Option<usize>) {
        if !self.allow_alpha {
            color.a = 255;
        }
        self.color = color;
        let mut hsv = color.hsv();
        // Neutral colors retain the chosen hue so increasing saturation works.
        if hsv.s == 0. {
            hsv.h = self.hsv.h;
        }
        self.hsv = hsv;
        self.sync_inputs(editing);
    }

    fn edit_input(&mut self, i: usize) {
        let text = self.inputs[i].value();
        let next = if i == 0 {
            Rgba::parse_hex(text, self.allow_alpha)
        } else if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
            text.parse::<u8>().ok().map(|n| {
                let mut c = self.color;
                match i {
                    1 => c.r = n,
                    2 => c.g = n,
                    _ => c.b = n,
                }
                c
            })
        } else {
            None
        };
        self.valid[i] = next.is_some();
        if let Some(color) = next {
            self.set_color(color, Some(i));
        }
    }

    fn pointer(&mut self, control: usize, x: i32, y: i32) {
        match control {
            0 => {
                let r = self.layout.sv;
                self.hsv.s = axis_fraction(x, r.x, r.w);
                self.hsv.v = 1. - axis_fraction(y, r.y, r.h);
            }
            1 => {
                let r = self.layout.hue;
                self.hsv.h = axis_fraction(x, r.x, r.w) * 360.;
            }
            2 => {
                let r = self.layout.alpha;
                self.color.a = byte(axis_fraction(x, r.x, r.w));
            }
            _ => return,
        }
        self.color = self.hsv.rgba(self.color.a);
        self.sync_inputs(None);
    }

    fn control_at(&self, p: Point) -> Option<usize> {
        if self.layout.sv.contains(p) {
            Some(0)
        } else if self.layout.hue.contains(p) {
            Some(1)
        } else if self.allow_alpha && self.layout.alpha.contains(p) {
            Some(2)
        } else {
            None
        }
    }

    fn drag_to(&mut self, control: usize, x: i32, y: i32) -> bool {
        if self.control_at(Point::new(x, y)) != Some(control) {
            return false;
        }
        self.pointer(control, x, y);
        true
    }

    fn confirm(&mut self) {
        if self.valid.iter().all(|v| *v) {
            self.result = Some(DialogResult::ColorSelected(
                self.color.to_hex(self.allow_alpha),
            ));
            self.drag = None;
        }
    }

    fn action_at(&self, p: Point) -> Option<usize> {
        if self.layout.ok.contains(p) {
            return Some(26);
        }
        if self.layout.cancel.contains(p) {
            return Some(25);
        }
        for i in 0..10 {
            if Layout::swatch(self.layout.presets, i).contains(p) {
                return Some(7 + i);
            }
        }
        for i in 0..self.recent.colors().len() {
            if Layout::swatch(self.layout.recent, i).contains(p) {
                return Some(17 + i);
            }
        }
        None
    }

    fn activate(&mut self) {
        match self.focus {
            7..=16 => self.preset(PRESETS[self.focus - 7]),
            17..=24 => {
                if let Some(c) = self.recent.colors().get(self.focus - 17).copied() {
                    self.set_color(c, None);
                }
            }
            25 => self.result = Some(DialogResult::Cancelled),
            _ => self.confirm(),
        }
    }

    fn preset(&mut self, mut c: Rgba) {
        c.a = self.color.a;
        self.set_color(c, None);
    }

    fn next_focus(&mut self, backwards: bool) {
        let mut i = self.focus;
        loop {
            i = if backwards {
                (i + 26) % 27
            } else {
                (i + 1) % 27
            };
            if i == 2 && !self.allow_alpha {
                continue;
            }
            if (17..=24).contains(&i) && i - 17 >= self.recent.colors().len() {
                continue;
            }
            self.set_focus(i);
            break;
        }
    }

    fn arrows(&mut self, key: u8, shift: bool) -> bool {
        let direction = if key == 0x4b || key == 0x50 { -1. } else { 1. };
        let step = if shift { 10. } else { 1. };
        match self.focus {
            0 => {
                if key == 0x4b || key == 0x4d {
                    self.hsv.s = (self.hsv.s + direction * step / 100.).clamp(0., 1.);
                } else {
                    self.hsv.v = (self.hsv.v + direction * step / 100.).clamp(0., 1.);
                }
            }
            1 => self.hsv.h = wrap_hue(self.hsv.h + direction * step),
            2 if self.allow_alpha => {
                self.color.a = byte(self.color.a as f32 / 255. + direction * step / 100.)
            }
            _ => return false,
        }
        self.color = self.hsv.rgba(self.color.a);
        self.sync_inputs(None);
        true
    }

    /// True only when something visible changes. Tick never schedules painting.
    /// The compositor owns pointer capture; keep routing captured motion until
    /// release, and discard it when focus/capture is lost.
    pub fn update(&mut self, event: Event) -> bool {
        if self.result.is_some() {
            return false;
        }
        if let Some(i) = self.inputs.iter().position(|i| i.context_menu_open()) {
            let changed = self.inputs[i].update(event);
            if changed {
                self.edit_input(i);
            }
            return changed;
        }
        match event {
            Event::Tick => return false,
            Event::FocusChanged { focused: false }
            | Event::PointerOwnership {
                owned: false,
                captured: false,
            } => {
                self.drag = None;
                self.press_origin = None;
                self.hover = None;
                for input in &mut self.inputs {
                    input.update(event);
                }
                return true;
            }
            Event::FocusChanged { focused: true } => {
                self.set_focus(self.focus);
                return true;
            }
            Event::KeyPress {
                keycode: 0x01,
                pressed: true,
                ..
            }
            | Event::Key('\u{1b}') => {
                self.drag = None;
                self.result = Some(DialogResult::Cancelled);
                return true;
            }
            Event::KeyPress {
                keycode: 0x0f,
                pressed: true,
                shift,
                ..
            } => {
                self.next_focus(shift);
                return true;
            }
            Event::KeyPress {
                keycode: 0x1c,
                pressed: true,
                ..
            }
            | Event::Key('\n')
            | Event::Key('\r') => {
                self.activate();
                return true;
            }
            Event::Key(' ') if self.focus >= 7 => {
                self.activate();
                return true;
            }
            Event::KeyPress {
                keycode: key @ (0x48 | 0x50 | 0x4b | 0x4d),
                pressed: true,
                shift,
                ctrl: false,
                alt: false,
                ..
            } if self.focus < 3 => return self.arrows(key, shift),
            Event::MouseDown { x, y, button: 0 } => {
                let p = Point::new(x, y);
                self.press_origin = Some(p);
                self.suppress_click = self.action_at(p).is_none();
                if let Some(control) = self.control_at(p) {
                    self.set_focus(control);
                    self.drag = Some(control);
                    self.suppress_click = true;
                    self.pointer(control, x, y);
                    return true;
                }
            }
            Event::MouseMove { x, y } => {
                if let Some(control) = self.drag {
                    return self.drag_to(control, x, y);
                }
                let p = Some(Point::new(x, y));
                let changed = [self.layout.ok, self.layout.cancel].iter().any(|r| {
                    self.hover.is_some_and(|p| r.contains(p)) != p.is_some_and(|p| r.contains(p))
                });
                self.hover = p;
                // TextInput owns text selection/cursor updates below.
                let mut dirty = changed;
                for input in &mut self.inputs {
                    dirty |= input.update(event);
                }
                return dirty;
            }
            Event::MouseUp { x, y, button: 0 } => {
                if let Some(control) = self.drag.take() {
                    self.drag_to(control, x, y);
                    return true;
                }
            }
            Event::Click { x, y } => {
                let origin = self.press_origin.take();
                // Native Window emits Click, rather than MouseUp, on left
                // release. End capture before suppressing its button action.
                if let Some(control) = self.drag.take() {
                    self.suppress_click = false;
                    self.drag_to(control, x, y);
                    return true;
                }
                if self.suppress_click {
                    self.suppress_click = false;
                    return false;
                }
                let p = Point::new(x, y);
                // A release over another action must not activate that action.
                if origin.is_some_and(|origin| self.action_at(origin) != self.action_at(p)) {
                    return false;
                }
                if let Some(action) = self.action_at(p) {
                    self.set_focus(action);
                    self.activate();
                    return true;
                }
                if let Some(c) = self.control_at(p) {
                    self.set_focus(c);
                    self.pointer(c, x, y);
                    return true;
                }
            }
            _ => {}
        }
        if let Event::MouseDown { x, y, .. } | Event::Click { x, y } = event {
            if let Some(i) = self
                .inputs
                .iter()
                .position(|input| input.rect.contains(Point::new(x, y)))
            {
                self.set_focus(i + 3);
            }
        }
        let mut changed = false;
        for i in 0..4 {
            let mut before = heapless::String::<16>::new();
            let _ = before.push_str(self.inputs[i].value());
            changed |= self.inputs[i].update(event);
            if before.as_str() != self.inputs[i].value() {
                self.edit_input(i);
            }
        }
        changed
    }

    fn cache_gradient(&mut self) {
        let r = self.layout.sv;
        let key = (self.hsv.h.to_bits(), r.w, r.h);
        if self.gradient_key == Some(key) {
            return;
        }
        let Some(len) = (r.w as usize).checked_mul(r.h as usize) else {
            return;
        };
        if len > 288 * 160 {
            return;
        }
        if len > self.gradient.len()
            && self
                .gradient
                .try_reserve_exact(len - self.gradient.len())
                .is_err()
        {
            return;
        }
        self.gradient.resize(len, 0);
        for y in 0..r.h {
            for x in 0..r.w {
                self.gradient[y as usize * r.w as usize + x as usize] = Hsv {
                    h: self.hsv.h,
                    s: axis_fraction(x as i32, 0, r.w),
                    v: 1. - axis_fraction(y as i32, 0, r.h),
                }
                .rgba(255)
                .argb();
            }
        }
        self.gradient_key = Some(key);
    }

    pub fn draw(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.cache_gradient();
        let l = self.layout;
        canvas.fill_rect(Rect::new(0, 0, canvas.width, canvas.height), theme.bg);
        canvas.draw_rect(
            Rect::new(
                8,
                8,
                canvas.width.saturating_sub(16),
                canvas.height.saturating_sub(16),
            ),
            theme.border,
        );
        label(
            canvas,
            theme,
            if self.title.is_empty() {
                "Choose color"
            } else {
                &self.title
            },
            18,
            17,
        );
        if self.gradient_key == Some((self.hsv.h.to_bits(), l.sv.w, l.sv.h)) {
            for y in 0..l.sv.h {
                for x in 0..l.sv.w {
                    canvas.put_pixel(
                        l.sv.x + x as i32,
                        l.sv.y + y as i32,
                        Color(self.gradient[y as usize * l.sv.w as usize + x as usize]),
                    );
                }
            }
        }
        marker(
            canvas,
            Point::new(
                l.sv.x + (self.hsv.s * l.sv.w.saturating_sub(1) as f32) as i32,
                l.sv.y + ((1. - self.hsv.v) * l.sv.h.saturating_sub(1) as f32) as i32,
            ),
        );
        for x in 0..l.hue.w {
            let c = Hsv {
                h: axis_fraction(x as i32, 0, l.hue.w) * 360.,
                s: 1.,
                v: 1.,
            }
            .rgba(255);
            canvas.vline(l.hue.x + x as i32, l.hue.y, l.hue.h, Color(c.argb()));
        }
        track_marker(canvas, l.hue, self.hsv.h / 360.);
        label(canvas, theme, "Original", l.original.x, l.original.y - 18);
        preview(canvas, l.original, self.original);
        label(canvas, theme, "New", l.preview.x, l.preview.y - 18);
        preview(canvas, l.preview, self.color);
        if self.allow_alpha {
            for x in 0..l.alpha.w {
                let mut c = self.color;
                c.a = byte(axis_fraction(x as i32, 0, l.alpha.w));
                preview(
                    canvas,
                    Rect::new(l.alpha.x + x as i32, l.alpha.y, 1, l.alpha.h),
                    c,
                );
            }
            track_marker(canvas, l.alpha, self.color.a as f32 / 255.);
            let mut opacity = heapless::String::<20>::new();
            let _ = write!(
                opacity,
                "{}% opacity",
                (self.color.a as u32 * 100 + 127) / 255
            );
            label(canvas, theme, &opacity, l.preview.x, l.alpha.y);
        }
        for (i, name) in ["HEX", "Red", "Green", "Blue"].iter().enumerate() {
            let r = self.inputs[i].rect;
            label(canvas, theme, name, r.x, r.y - 17);
            self.inputs[i].draw_field(canvas, theme);
            if !self.valid[i] {
                canvas.draw_rect(r, theme.text_dim);
            }
        }
        if self.valid.iter().any(|v| !v) {
            label(
                canvas,
                theme,
                "Check HEX / RGB (0-255)",
                l.fields[0].right() + 10,
                l.fields[0].y + 6,
            );
        }
        label(canvas, theme, "Presets", l.presets.x, l.presets.y - 17);
        for (i, c) in PRESETS.iter().enumerate() {
            let r = Layout::swatch(l.presets, i);
            preview(canvas, r, *c);
            canvas.draw_rect(
                r,
                if self.focus == 7 + i {
                    theme.accent
                } else {
                    theme.border
                },
            );
        }
        label(canvas, theme, "Recent", l.recent.x, l.recent.y - 17);
        for (i, c) in self.recent.colors().iter().enumerate() {
            let r = Layout::swatch(l.recent, i);
            preview(canvas, r, *c);
            canvas.draw_rect(
                r,
                if self.focus == 17 + i {
                    theme.accent
                } else {
                    theme.border
                },
            );
        }
        if self.recent.colors().is_empty() {
            label(
                canvas,
                theme,
                "Confirmed colors appear here",
                l.recent.x,
                l.recent.y + 3,
            );
        }
        for (i, r) in [l.sv, l.hue, l.alpha].iter().enumerate() {
            if i == 2 && !self.allow_alpha {
                continue;
            }
            canvas.draw_rect(
                r.inset(-1),
                if self.focus == i {
                    theme.accent
                } else {
                    theme.border
                },
            );
        }
        let mut ok = Button::new(l.ok, "OK");
        let mut cancel = Button::secondary(l.cancel, "Cancel");
        if self.hover.is_some_and(|p| l.ok.contains(p)) {
            ok.state = ButtonState::Hovered;
        }
        if self.valid.iter().any(|v| !v) {
            ok.state = ButtonState::Disabled;
        }
        if self.hover.is_some_and(|p| l.cancel.contains(p)) {
            cancel.state = ButtonState::Hovered;
        }
        ok.draw(canvas, theme);
        cancel.draw(canvas, theme);
        if self.focus == 25 {
            canvas.draw_rect(l.cancel.inset(-2), theme.accent);
        }
        if self.focus == 26 {
            canvas.draw_rect(l.ok.inset(-2), theme.accent);
        }
        for input in &self.inputs {
            input.draw_context_menu(canvas, theme);
        }
    }
}

fn label(canvas: &mut Canvas, theme: &Theme, text: &str, x: i32, y: i32) {
    // Clip long request titles within the native surface.
    draw_text(
        canvas,
        text,
        x,
        y,
        &TextStyle::new(FontRole::UiSmall, theme.text),
    );
}

fn preview(canvas: &mut Canvas, r: Rect, color: Rgba) {
    for y in r.y.max(0)..r.bottom().min(canvas.height as i32) {
        for x in r.x.max(0)..r.right().min(canvas.width as i32) {
            let n = if (x / 6 + y / 6) % 2 == 0 { 155 } else { 95 };
            canvas.put_pixel(x, y, Color(color.argb()).blend_over(Color::rgb(n, n, n)));
        }
    }
}

fn marker(canvas: &mut Canvas, p: Point) {
    canvas.draw_rect(Rect::new(p.x - 3, p.y - 3, 7, 7), Color::rgb(0, 0, 0));
    canvas.draw_rect(Rect::new(p.x - 2, p.y - 2, 5, 5), Color::rgb(255, 255, 255));
}

fn track_marker(canvas: &mut Canvas, r: Rect, t: f32) {
    let x = r.x + (t.clamp(0., 1.) * r.w.saturating_sub(1) as f32) as i32;
    canvas.draw_rect(Rect::new(x - 2, r.y - 2, 5, r.h + 4), Color::rgb(0, 0, 0));
    canvas.draw_rect(
        Rect::new(x - 1, r.y - 1, 3, r.h + 2),
        Color::rgb(255, 255, 255),
    );
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::{format, vec};

    fn picker(alpha: bool) -> ColorPicker {
        ColorPicker::new(
            ColorPickerRequest {
                title: String::from("Annotation color"),
                initial_color: String::from(if alpha { "#FFAA0080" } else { "#FFAA00" }),
                allow_alpha: alpha,
            },
            RecentColors::default(),
        )
    }

    fn key(keycode: u8, shift: bool) -> Event {
        Event::key_press(keycode, true, shift, false, false, false)
    }

    #[test]
    fn lifecycle_cancel_and_confirm() {
        let mut p = picker(false);
        assert!(p.result().is_none());
        assert!(!p.update(Event::Tick));
        p.pointer(0, 20, 90);
        assert_ne!(p.color, p.original);
        p.update(key(0x01, false));
        assert_eq!(p.result(), Some(&DialogResult::Cancelled));
        assert_eq!(p.original.to_hex(false), "#FFAA00");
        assert!(!p.update(key(0x1c, false)));
        let mut p = picker(false);
        p.update(key(0x1c, false));
        assert_eq!(
            p.result(),
            Some(&DialogResult::ColorSelected(String::from("#FFAA00")))
        );
        assert!(picker(false).result().is_none());
    }

    #[test]
    fn hex_rgb_and_hsv_stay_in_sync_without_replacing_edits() {
        let mut p = picker(false);
        p.inputs[0].set_text("#00ff00");
        p.edit_input(0);
        assert_eq!(
            p.hsv,
            Hsv {
                h: 120.,
                s: 1.,
                v: 1.
            }
        );
        assert_eq!(p.inputs[0].value(), "#00ff00");
        assert_eq!(p.inputs[2].value(), "255");
        p.inputs[0].set_text("#00f");
        p.edit_input(0);
        assert_eq!(p.color, Rgba::rgb(0, 255, 0));
        assert!(!p.valid[0]);
        p.confirm();
        assert!(p.result().is_none());
        p.inputs[0].set_text("#0000FF");
        p.edit_input(0);
        p.inputs[1].set_text("256");
        p.edit_input(1);
        assert_eq!(p.color, Rgba::rgb(0, 0, 255));
        p.inputs[1].set_text("128");
        p.edit_input(1);
        assert_eq!(p.inputs[0].value(), "#8000FF");
        assert_eq!(p.color.hsv(), p.hsv);
        p.confirm();
        assert_eq!(
            p.result(),
            Some(&DialogResult::ColorSelected(String::from("#8000FF")))
        );
    }

    #[test]
    fn text_events_preserve_cursor_and_incomplete_value() {
        let mut p = picker(false);
        p.set_focus(3);
        p.update(Event::Key('\u{8}'));
        assert_eq!(p.inputs[0].value(), "#FFAA0");
        assert_eq!(p.color.to_hex(false), "#FFAA00");
        p.update(Event::Key('f'));
        assert_eq!(p.inputs[0].value(), "#FFAA0f");
        assert_eq!(p.color.to_hex(false), "#FFAA0F");
    }

    #[test]
    fn drag_stays_inside_control_and_release_does_not_press_ok() {
        let mut p = picker(false);
        let r = p.layout.sv;
        p.update(Event::mouse_down(r.x, r.y, 0));
        p.update(Event::mouse_move(-1000, -1000));
        assert_eq!(p.hsv.s, 0.);
        assert_eq!(p.hsv.v, 1.);
        p.update(Event::mouse_move(1000, 1000));
        assert_eq!(p.hsv.s, 0.);
        assert_eq!(p.hsv.v, 1.);
        p.update(Event::mouse_move(r.right() - 1, r.bottom() - 1));
        assert_eq!(p.hsv.s, 1.);
        assert_eq!(p.hsv.v, 0.);
        let b = p.layout.ok;
        p.update(Event::mouse_up(b.x, b.y, 0));
        p.update(Event::click(b.x, b.y));
        assert!(p.result().is_none());
        assert!(p.drag.is_none());
        p.update(Event::mouse_down(r.x, r.y, 0));
        p.update(Event::FocusChanged { focused: false });
        assert!(p.drag.is_none());
        let old = p.color;
        p.update(Event::mouse_move(1000, 1000));
        assert_eq!(p.color, old);
        let cancel = p.layout.cancel;
        p.update(Event::mouse_down(cancel.x, cancel.y, 0));
        p.update(Event::mouse_up(b.x, b.y, 0));
        p.update(Event::click(b.x, b.y));
        assert!(p.result().is_none());
        let input = p.layout.fields[0];
        p.update(Event::mouse_down(input.x, input.y, 0));
        p.update(Event::mouse_up(b.x, b.y, 0));
        p.update(Event::click(b.x, b.y));
        assert!(p.result().is_none());
        p.update(Event::mouse_down(b.x, b.y, 0));
        p.update(Event::mouse_up(b.x, b.y, 0));
        p.update(Event::click(b.x, b.y));
        assert!(matches!(p.result(), Some(DialogResult::ColorSelected(_))));
    }

    #[test]
    fn native_click_release_finishes_each_control_and_hover_keeps_selection() {
        for control in 0..3 {
            let mut p = picker(true);
            let r = [p.layout.sv, p.layout.hue, p.layout.alpha][control];
            let before = p.color;
            p.update(Event::mouse_move(r.x, r.y));
            assert_eq!(p.color, before);
            p.update(Event::mouse_down(r.x, r.y, 0));
            p.update(Event::mouse_move(r.x + r.w as i32 / 2, r.y));
            // Window::poll_event emits Click directly on left release.
            p.update(Event::click(r.right() - 1, r.y));
            assert!(p.drag.is_none());
            assert!(p.press_origin.is_none());
            let selected = p.color;
            let hsv = p.hsv;
            p.update(Event::mouse_move(r.x, r.bottom() - 1));
            p.update(Event::mouse_move(-1000, -1000));
            assert_eq!(p.color, selected);
            assert_eq!(p.hsv, hsv);
            let ok = p.layout.ok;
            p.update(Event::mouse_down(ok.x, ok.y, 0));
            p.update(Event::click(ok.x, ok.y));
            assert_eq!(
                p.result(),
                Some(&DialogResult::ColorSelected(selected.to_hex(true)))
            );
        }
    }

    #[test]
    fn native_release_outside_finishes_drag_at_last_inside_color() {
        let mut p = picker(false);
        let r = p.layout.sv;
        p.update(Event::mouse_down(r.x + 50, r.y + 50, 0));
        let selected = p.color;
        let ok = p.layout.ok;
        p.update(Event::mouse_move(ok.x, ok.y));
        assert_eq!(p.color, selected);
        p.update(Event::click(ok.x, ok.y));
        assert!(p.drag.is_none());
        assert!(p.result().is_none());
        p.update(Event::mouse_move(r.x, r.y));
        assert_eq!(p.color, selected);
        // A subsequent click starts and completes a fresh selection.
        p.update(Event::mouse_down(r.x, r.y, 0));
        p.update(Event::click(r.x, r.y));
        assert!(p.drag.is_none());
        assert_eq!(p.color, Rgba::rgb(255, 255, 255));
    }

    #[test]
    fn hue_sv_alpha_coordinates_and_preset_alpha() {
        let mut p = picker(true);
        let r = p.layout.hue;
        p.pointer(1, r.x, r.y);
        assert_eq!(p.hsv.h, 0.);
        p.pointer(1, r.right() - 1, r.y);
        assert_eq!(p.hsv.h, 360.);
        let r = p.layout.sv;
        p.pointer(0, r.right() - 1, r.y);
        assert_eq!(p.color.r, 255);
        let r = p.layout.alpha;
        p.pointer(2, r.x, r.y);
        assert_eq!(p.color.a, 0);
        p.pointer(2, r.right() - 1, r.y);
        assert_eq!(p.color.a, 255);
        p.color.a = 128;
        p.preset(Rgba::rgb(0, 0, 255));
        assert_eq!(p.color.a, 128);
        p.confirm();
        assert_eq!(
            p.result(),
            Some(&DialogResult::ColorSelected(String::from("#0000FF80")))
        );
    }

    #[test]
    fn keyboard_focus_and_recents() {
        let mut p = picker(false);
        p.update(key(0x0f, false));
        assert_eq!(p.focus, 1);
        p.update(key(0x0f, false));
        assert_eq!(p.focus, 3);
        p.set_focus(0);
        let old = p.hsv.s;
        p.update(key(0x4b, false));
        assert!(p.hsv.s < old);
        p.set_focus(16);
        p.update(key(0x0f, false));
        assert_eq!(p.focus, 25);
        p.update(key(0x0f, true));
        assert_eq!(p.focus, 16);
        assert!(p.recent.colors().is_empty());
        p.recent.record(Rgba {
            r: 1,
            g: 2,
            b: 3,
            a: 0,
        });
        p.set_focus(17);
        p.update(Event::Key(' '));
        assert_eq!(p.color.to_hex(false), "#010203");
        assert_eq!(p.color.a, 255);
        assert_eq!(p.recent.colors().len(), 1);
    }

    #[test]
    fn gradient_cache_reuses_storage_and_only_changes_for_hue_or_size() {
        let mut p = picker(false);
        p.cache_gradient();
        let ptr = p.gradient.as_ptr();
        let key = p.gradient_key;
        let r = p.layout.sv;
        p.pointer(0, r.x + 25, r.y + 30);
        p.cache_gradient();
        assert_eq!(p.gradient_key, key);
        assert_eq!(p.gradient.as_ptr(), ptr);
        p.hsv.h = 200.;
        p.cache_gradient();
        assert_ne!(p.gradient_key, key);
        assert_eq!(p.gradient.as_ptr(), ptr);
        p.resize(300, 400);
        p.cache_gradient();
        assert_eq!(
            p.gradient.len(),
            p.layout.sv.w as usize * p.layout.sv.h as usize
        );
    }

    #[test]
    fn layout_and_rendering_are_safe_at_native_and_small_sizes() {
        for alpha in [false, true] {
            let mut p = picker(alpha);
            assert!(p.layout.recent.bottom() < p.layout.ok.y);
            assert!(p.layout.fields[3].right() <= WIDTH as i32 - 18);
            for (w, h) in [
                (WIDTH, if alpha { ALPHA_HEIGHT } else { HEIGHT }),
                (220, 280),
                (1, 1),
                (0, 0),
            ] {
                p.resize(w, h);
                let mut pixels = vec![0; (w * h) as usize];
                p.draw(
                    &mut Canvas::new(&mut pixels, w, w, h),
                    &Theme::sunlight_dark(),
                );
                if w == WIDTH && std::env::var_os("SUNLIGHT_COLOR_PREVIEW").is_some() {
                    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                    for pixel in &pixels {
                        ppm.extend_from_slice(&[
                            (pixel >> 16) as u8,
                            (pixel >> 8) as u8,
                            *pixel as u8,
                        ]);
                    }
                    std::fs::write(
                        format!(
                            "/tmp/sunlight-color-picker-{}.ppm",
                            if alpha { "rgba" } else { "rgb" }
                        ),
                        ppm,
                    )
                    .unwrap();
                }
                assert!(!p.update(Event::Tick));
            }
        }
    }
}
