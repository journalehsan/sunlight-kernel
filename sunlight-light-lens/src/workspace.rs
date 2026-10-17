//! Native workspace controls and annotation gestures; background sampling stays cached.
use super::*;
use alloc::{format, rc::Rc, string::String};
use sunlight_dialogs::{
    ColorPickerRequest, DialogClient, DialogCommonOptions, DialogRequest, DialogResult,
    TextInputRequest,
};
use sunlight_light_lens::annotations::{self, Object, Style, TextMask, Tool, P};

pub(super) struct Gesture {
    pub object: Object,
    pub start: P,
    pub moving: bool,
    pub resizing: bool,
}
pub(super) struct Workspace {
    pub collapsed_left: bool,
    pub collapsed_right: bool,
    pub histogram_channel: usize,
    pub left: usize,
    pub right: usize,
    pub tool: Tool,
    pub style: Style,
    pub selected: Option<u64>,
    pub gesture: Option<Gesture>,
    pub scroll: i32,
    pub layer_scroll: i32,
    pub info_scroll: i32,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            collapsed_left: false,
            collapsed_right: false,
            histogram_channel: 0,
            left: 0,
            right: 0,
            tool: Tool::Select,
            style: Style::default(),
            selected: None,
            gesture: None,
            scroll: 0,
            layer_scroll: 0,
            info_scroll: 0,
        }
    }
}
impl LightLensApp {
    pub(super) fn panel_toggle(&self, left: bool) -> Rect {
        let r = self.layout.header;
        if r.w < 480 {
            return Rect::new(0, 0, 0, 0);
        }
        Rect::new(r.x + 134 + if left { 0 } else { 72 }, r.y + 5, 66, 26)
    }
    fn refresh_workspace_layout(&mut self) {
        self.layout_invalidation.invalidate();
        self.ensure_layout();
        self.viewport
            .clamp(self.preview_area(), self.editor.state.size);
    }
    pub(super) fn apply_panel_collapse(&mut self) {
        if self.workspace.collapsed_left {
            let delta = self.layout.left.w.saturating_sub(26);
            self.layout.left.w = self.layout.left.w.min(26);
            self.layout.viewport.x -= delta as i32;
            self.layout.viewport.w += delta;
        }
        if self.workspace.collapsed_right {
            let delta = self.layout.right.w.saturating_sub(26);
            self.layout.right.x += delta as i32;
            self.layout.right.w = self.layout.right.w.min(26);
            self.layout.viewport.w += delta;
        }
    }
    pub(super) fn tab_rect(&self, left: bool, i: usize) -> Rect {
        let r = if left {
            self.layout.left
        } else {
            self.layout.right
        };
        let collapsed = if left {
            self.workspace.collapsed_left
        } else {
            self.workspace.collapsed_right
        };
        if collapsed {
            return Rect::new(r.x, r.y + i as i32 * 30, r.w, 26.min(r.h));
        }
        let n = if left { 2 } else { 3 };
        let w = r.w / n;
        Rect::new(r.x + i as i32 * w as i32, r.y, w, 26.min(r.h))
    }
    pub(super) fn draw_tabs(&self, c: &mut Canvas, t: &Theme, left: bool) {
        let labels: &[&str] = if left {
            &["Edit", "Annotate"]
        } else {
            &["Layers", "Info", "Histogram"]
        };
        for (i, label) in labels.iter().enumerate() {
            let collapsed = if left {
                self.workspace.collapsed_left
            } else {
                self.workspace.collapsed_right
            };
            let label = if collapsed { &label[..1] } else { label };
            let r = self.tab_rect(left, i);
            Self::draw_edit_button(
                c,
                t,
                r,
                label,
                true,
                if left {
                    self.workspace.left == i
                } else {
                    self.workspace.right == i
                },
                false,
            );
        }
    }
    fn annotation_button(&self, i: usize) -> Rect {
        let r = self.layout.left;
        Rect::new(
            r.x + 10,
            r.y + 34 + i as i32 * 30 - self.workspace.scroll,
            r.w.saturating_sub(20),
            26,
        )
    }
    fn layer_button(&self, i: usize) -> Rect {
        let r = self.layout.right;
        let w = r.w.saturating_sub(26) / 2;
        Rect::new(
            r.x + 10 + (i % 2) as i32 * (w as i32 + 6),
            r.y + 34 + (i / 2) as i32 * 30 - self.workspace.layer_scroll,
            w,
            26,
        )
    }
    fn layer_row(&self, i: usize) -> Rect {
        let r = self.layout.right;
        Rect::new(
            r.x + 10,
            r.y + 224 + (self.editor.scene.layers.len() - 1 - i) as i32 * 32
                - self.workspace.layer_scroll,
            r.w.saturating_sub(20),
            28,
        )
    }
    pub(super) fn draw_annotations_panel(&self, c: &mut Canvas, t: &Theme) {
        let r = self.layout.left;
        c.fill_rect(r, t.panel);
        self.draw_tabs(c, t, true);
        let area = Rect::new(r.x, r.y + 27, r.w, r.h.saturating_sub(27));
        if area.w == 0 || area.h == 0 {
            return;
        }
        let mut sub = c.sub_canvas(area);
        let labels = self.annotation_labels();
        for (i, label) in labels.iter().enumerate() {
            let mut b = self.annotation_button(i);
            b.x -= area.x;
            b.y -= area.y;
            let selected = i < 8 && Tool::ALL[i] == self.workspace.tool;
            let enabled = self.annotation_enabled(i);
            Self::draw_edit_button(&mut sub, t, b, label, enabled, selected, false);
        }
        let y = 34 + labels.len() as i32 * 30 - self.workspace.scroll - 27;
        sf_draw(
            &mut sub,
            "Editable in memory",
            10,
            y,
            &TextStyle::new(FontRole::UiSmall, t.text_dim),
        );
        sf_draw(
            &mut sub,
            "Save exports pixels",
            10,
            y + 18,
            &TextStyle::new(FontRole::UiSmall, t.text_dim),
        );
    }
    fn annotation_enabled(&self, i: usize) -> bool {
        self.editable()
            && match i {
                11 if !matches!(self.property_tool(), Tool::Rectangle | Tool::Ellipse) => {
                    self.editor.scene.layers.len() < annotations::MAX_LAYERS
                }
                12 => self.editor.can_undo(),
                13 => self.selected_object().is_some(),
                14 => self.selected_object().is_some_and(|o| o.tool == Tool::Text),
                _ => true,
            }
    }
    fn property_tool(&self) -> Tool {
        self.selected_object()
            .map(|o| o.tool)
            .unwrap_or(self.workspace.tool)
    }
    pub(super) fn annotation_labels(&self) -> alloc::vec::Vec<String> {
        let mut out: alloc::vec::Vec<String> =
            Tool::ALL.iter().map(|t| String::from(t.label())).collect();
        out.push(format!(
            "Color #{:06X}",
            self.workspace.style.color & 0xffffff
        ));
        out.push(if self.property_tool() == Tool::Text {
            format!("Font size: {}", self.workspace.style.font_size)
        } else {
            format!("Stroke: {} px", self.workspace.style.width)
        });
        out.push(format!(
            "Opacity: {}%",
            self.workspace.style.opacity as u32 * 100 / 255
        ));
        out.push(
            if matches!(self.property_tool(), Tool::Rectangle | Tool::Ellipse) {
                if self.workspace.style.fill.is_some() {
                    format!(
                        "Fill #{:06X}",
                        self.workspace.style.fill.unwrap() & 0xffffff
                    )
                } else {
                    String::from("Fill: none")
                }
            } else {
                String::from("Layer: add")
            },
        );
        out.push(String::from("Undo"));
        out.push(String::from("Delete object"));
        if self.selected_object().is_some_and(|o| o.tool == Tool::Text) {
            out.push(String::from("Edit text"));
        }
        out
    }
    pub(super) fn draw_layers_panel(&self, c: &mut Canvas, t: &Theme) {
        let r = self.layout.right;
        c.fill_rect(r, t.panel);
        self.draw_tabs(c, t, false);
        if self.workspace.collapsed_right {
            return;
        }
        let area = Rect::new(
            r.x + 1,
            r.y + 27,
            r.w.saturating_sub(2),
            r.h.saturating_sub(28),
        );
        if area.w == 0 || area.h == 0 {
            return;
        }
        let mut sub = c.sub_canvas(area);
        for (i, label) in [
            "Add layer",
            "Delete layer",
            "Visible",
            "Lock",
            "Raise",
            "Lower",
            "Rename",
            "Opacity -",
            "Opacity +",
            "Delete object",
            "Undo",
            "Redo",
        ]
        .iter()
        .enumerate()
        {
            let layer = self.editor.scene.layer();
            let active = match i {
                2 => layer.is_some_and(|l| l.visible),
                3 => layer.is_some_and(|l| l.locked),
                _ => false,
            };
            let enabled = self.layer_enabled(i);
            let mut button = self.layer_button(i);
            button.x -= area.x;
            button.y -= area.y;
            Self::draw_edit_button(&mut sub, t, button, label, enabled, active, false);
        }
        for (i, l) in self.editor.scene.layers.iter().enumerate().rev() {
            let mut row = self.layer_row(i);
            row.x -= area.x;
            row.y -= area.y;
            let label = format!(
                "{}{} {}  {}%",
                if l.visible { "" } else { "[hidden] " },
                if l.locked { "[L]" } else { "" },
                l.name,
                l.opacity as u32 * 100 / 255
            );
            Self::draw_edit_button(
                &mut sub,
                t,
                row,
                &label,
                true,
                l.id == self.editor.scene.active,
                false,
            );
        }
    }
    fn layer_enabled(&self, i: usize) -> bool {
        if !self.editable() {
            return false;
        }
        let scene = &self.editor.scene;
        let layer = scene.layer();
        let pos = scene
            .layers
            .iter()
            .position(|l| l.id == scene.active)
            .unwrap_or(0);
        match i {
            0 => scene.layers.len() < annotations::MAX_LAYERS,
            1 => scene.active != 0 && layer.is_some_and(|l| !l.locked),
            2 => true,
            3 => scene.active != 0,
            4 => pos > 0 && pos + 1 < scene.layers.len(),
            5 => pos > 1,
            6 => scene.active != 0,
            7 => layer.is_some_and(|l| l.opacity > 0),
            8 => layer.is_some_and(|l| l.opacity < 255),
            9 => self.selected_object().is_some(),
            10 => self.editor.can_undo(),
            11 => self.editor.can_redo(),
            _ => false,
        }
    }
    pub(super) fn selected_object(&self) -> Option<&Object> {
        let id = self.workspace.selected?;
        self.editor
            .scene
            .layers
            .iter()
            .filter(|l| l.visible && !l.locked && l.opacity > 0)
            .flat_map(|l| l.objects.iter())
            .find(|o| o.id == id)
            .map(|o| o.as_ref())
    }
    fn scene_action(
        &mut self,
        edit: impl FnOnce(&mut annotations::Scene) -> Result<(), &'static str>,
    ) {
        if let Err(e) = self.editor.edit_scene(edit) {
            self.message.set_str(e);
        }
    }
    fn input(&mut self, title: &str, prompt: &str, value: &str) -> Option<String> {
        self.finish_slider();
        self.slider_focus = None;
        let request = DialogRequest::TextInput(TextInputRequest {
            common: DialogCommonOptions::new(title, prompt),
            default_value: String::from(value),
            allow_empty: false,
        });
        match DialogClient::new().show(&request) {
            Ok(DialogResult::TextSubmitted(value)) => Some(value),
            Err(_) => {
                self.message.set_str("Native text dialog unavailable.");
                None
            }
            _ => None,
        }
    }
    fn edit_text(&mut self, mut object: Object, new: bool) {
        let old = object
            .text
            .as_ref()
            .map(|t| t.text.as_str())
            .unwrap_or("Text");
        if let Some(value) = self.input(
            "Annotation text",
            "Single line, up to 128 UTF-8 bytes:",
            old,
        ) {
            match TextMask::new(&value) {
                Ok(mask) => {
                    object.text = Some(Rc::new(mask));
                    if new {
                        self.commit_object(object);
                    } else {
                        self.scene_action(|s| s.replace(object));
                    }
                }
                Err(e) => self.message.set_str(e),
            }
        }
    }
    fn commit_object(&mut self, object: Object) {
        let mut id = None;
        self.scene_action(|s| {
            id = Some(s.insert(object)?);
            Ok(())
        });
        if id.is_some() {
            self.workspace.selected = id;
        }
    }
    fn layer_action(&mut self, i: usize) {
        if !self.layer_enabled(i) {
            return;
        }
        if i == 10 || i == 11 {
            self.history(i == 11);
            return;
        }
        if i == 9 {
            if let Some(id) = self.workspace.selected {
                self.scene_action(|s| s.delete(id));
                self.workspace.selected = None;
            }
            return;
        }
        if i == 6 {
            let old = self
                .editor
                .scene
                .layer()
                .map(|l| l.name.clone())
                .unwrap_or_default();
            if let Some(name) = self.input("Rename layer", "Layer name (up to 32 bytes):", &old) {
                if name.trim().is_empty() || name.len() > 32 || name.chars().any(char::is_control) {
                    self.message.set_str("Layer name: 1-32 bytes, single line.");
                } else {
                    self.scene_action(|s| {
                        s.layer_mut().unwrap().name = name;
                        Ok(())
                    });
                }
            }
            return;
        }
        self.scene_action(|s| {
            match i {
                0 => {
                    s.add_layer()?;
                }
                1 => s.remove_layer()?,
                2 => {
                    let l = s.layer_mut().unwrap();
                    l.visible = !l.visible;
                }
                3 => {
                    let l = s.layer_mut().unwrap();
                    l.locked = !l.locked;
                }
                4 | 5 => s.reorder(i == 4),
                7 | 8 => {
                    let l = s.layer_mut().unwrap();
                    l.opacity = if i == 7 {
                        l.opacity.saturating_sub(32)
                    } else {
                        l.opacity.saturating_add(32)
                    };
                }
                _ => {}
            }
            Ok(())
        });
        self.workspace.selected = None;
    }
    fn annotation_action(&mut self, i: usize) {
        if !self.annotation_enabled(i) {
            return;
        }
        if i < 8 {
            self.workspace.selected = None;
            self.workspace.tool = Tool::ALL[i];
            self.workspace.style.opacity = if i == 7 { 96 } else { 255 };
            return;
        }
        if i == 11 && !matches!(self.property_tool(), Tool::Rectangle | Tool::Ellipse) {
            self.layer_action(0);
            return;
        }
        if i == 12 {
            self.history(false);
            return;
        }
        if i == 13 {
            if let Some(id) = self.workspace.selected {
                self.scene_action(|s| s.delete(id));
                self.workspace.selected = None;
            }
            return;
        }
        if i == 14 {
            if let Some(o) = self.selected_object().cloned() {
                self.edit_text(o, false);
            }
            return;
        }
        match i {
            8 => {
                self.finish_slider();
                self.slider_focus = None;
                let request = DialogRequest::ColorPicker(ColorPickerRequest {
                    title: String::from("Annotation color"),
                    initial_color: format!("#{:06X}", self.workspace.style.color & 0xffffff),
                    // Annotation opacity remains its own existing style control.
                    allow_alpha: false,
                });
                let result = DialogClient::new().show(&request);
                self.apply_annotation_color_result(result);
                return;
            }
            9 => {
                let text = self.property_tool() == Tool::Text;
                let old = if text {
                    self.workspace.style.font_size
                } else {
                    self.workspace.style.width
                };
                if let Some(v) = self.input(
                    if text { "Font size" } else { "Stroke width" },
                    "Size in image pixels (1-128):",
                    &format!("{old}"),
                ) {
                    if let Ok(n) = v.trim().parse::<u16>() {
                        if (1..=128).contains(&n) {
                            if text {
                                self.workspace.style.font_size = n;
                            } else {
                                self.workspace.style.width = n;
                            }
                        } else {
                            self.message.set_str("Size must be 1-128.");
                        }
                    } else {
                        self.message.set_str("Size must be a whole number, 1-128.");
                    }
                }
            }
            10 => {
                self.workspace.style.opacity = match self.workspace.style.opacity {
                    255 => 192,
                    192 => 128,
                    128 => 64,
                    64 => 32,
                    _ => 255,
                };
            }
            11 => {
                let old = self
                    .workspace
                    .style
                    .fill
                    .map(|c| format!("{:06X}", c & 0xffffff))
                    .unwrap_or_else(|| String::from("none"));
                if let Some(value) = self.input("Shape fill", "Hex RGB or none for no fill:", &old)
                {
                    if value.trim().eq_ignore_ascii_case("none") {
                        self.workspace.style.fill = None;
                    } else if let Ok(color) =
                        u32::from_str_radix(value.trim().trim_start_matches('#'), 16)
                    {
                        if color <= 0xffffff {
                            self.workspace.style.fill = Some(0xff000000 | color);
                        } else {
                            self.message.set_str("Invalid fill RGB.");
                        }
                    } else {
                        self.message.set_str("Invalid fill RGB.");
                    }
                }
            }
            _ => {}
        }
        if let Some(mut o) = self.selected_object().cloned() {
            o.style = self.workspace.style;
            self.scene_action(|s| s.replace(o));
        }
    }

    /// Commit through the existing scene transaction only after a valid HEX
    /// selection. Cancellation never creates a history entry or touches objects.
    pub(super) fn apply_annotation_color_result(
        &mut self,
        result: Result<DialogResult, sunlight_dialogs::DialogError>,
    ) {
        #[cfg(not(test))]
        sunlight_ipc::debug_log(&format!("[LIGHT-LENS] color picker result: {result:?}\n"));
        let value = match result {
            Ok(DialogResult::ColorSelected(value)) => value,
            Err(error) => {
                self.message
                    .set_str(&format!("Native color dialog failed: {error:?}."));
                return;
            }
            Ok(DialogResult::Cancel | DialogResult::Cancelled | DialogResult::Dismissed) => return,
            Ok(DialogResult::Error(error)) => {
                self.message.set_str(&error);
                return;
            }
            Ok(_) => {
                self.message.set_str("Unexpected color dialog result.");
                return;
            }
        };
        let digits = value.trim().trim_start_matches('#');
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            self.message.set_str("Invalid RGB color.");
            return;
        }
        if let Ok(color) = u32::from_str_radix(digits, 16) {
            let color = 0xff000000 | color;
            self.workspace.style.color = color;
            self.message
                .set_str(&format!("Annotation color: #{:06X}", color & 0xffffff));
            #[cfg(not(test))]
            sunlight_ipc::debug_log(&format!(
                "[LIGHT-LENS] annotation color set: #{:06X}\n",
                color & 0xffffff
            ));
            if let Some(mut object) = self.selected_object().cloned() {
                if object.style.color != color {
                    object.style.color = color;
                    self.scene_action(|s| s.replace(object));
                }
            }
        }
    }
    fn doc_point(&self, p: Point) -> P {
        let dst = self.image_rect();
        let (w, h) = self.editor.state.size;
        P::new(
            ((p.x - dst.x) as f32 * w as f32 / dst.w.max(1) as f32).clamp(0., w as f32),
            ((p.y - dst.y) as f32 * h as f32 / dst.h.max(1) as f32).clamp(0., h as f32),
        )
    }
    /// Returns Some when this workspace owns the event, including no-op releases.
    pub(super) fn workspace_event(&mut self, event: &Event) -> Option<bool> {
        match *event {
            Event::Click { x, y } => {
                let p = Point::new(x, y);
                if self.slider_drag.is_some()
                    || self.crop_start.is_some()
                    || self.crop_drag.is_some()
                    || self.pan_start.is_some()
                {
                    return None;
                }
                if self.workspace.gesture.is_some() {
                    self.annotation_release(p);
                    return Some(true);
                }
                for left in [true, false] {
                    if self.panel_toggle(left).contains(p) {
                        if left {
                            self.workspace.collapsed_left = !self.workspace.collapsed_left;
                        } else {
                            self.workspace.collapsed_right = !self.workspace.collapsed_right;
                        }
                        self.refresh_workspace_layout();
                        return Some(true);
                    }
                }
                for left in [true, false] {
                    for i in 0..if left { 2 } else { 3 } {
                        if self.tab_rect(left, i).contains(p) {
                            self.finish_slider();
                            self.cancel_crop();
                            self.slider_focus = None;
                            self.workspace.gesture = None;
                            if left {
                                self.workspace.left = i;
                                self.workspace.selected = None;
                                self.workspace.collapsed_left = false;
                                self.side_scroll = 0;
                            } else {
                                self.workspace.right = i;
                                self.workspace.collapsed_right = false;
                                self.histogram_open = i == 2;
                            }
                            self.refresh_workspace_layout();
                            return Some(true);
                        }
                    }
                }
                if self.workspace.collapsed_left && self.layout.left.contains(p)
                    || self.workspace.collapsed_right && self.layout.right.contains(p)
                {
                    return Some(false);
                }
                if self.workspace.left == 1 && self.layout.left.contains(p) {
                    if self.editable() {
                        for i in 0..self.annotation_labels().len() {
                            if self.annotation_button(i).contains(p) {
                                self.annotation_action(i);
                                break;
                            }
                        }
                    }
                    return Some(true);
                }
                if self.workspace.right == 0 && self.layout.right.contains(p) {
                    for i in 0..12 {
                        if self.layer_button(i).contains(p) {
                            self.layer_action(i);
                            return Some(true);
                        }
                    }
                    for i in 0..self.editor.scene.layers.len() {
                        if self.layer_row(i).contains(p) {
                            self.editor.activate(self.editor.scene.layers[i].id);
                            self.workspace.selected = None;
                            return Some(true);
                        }
                    }
                    return Some(false);
                }
            }
            Event::MouseDown { x, y, button: 0 } if self.workspace.left == 1 && self.editable() => {
                self.suppress_click = false;
                let p = Point::new(x, y);
                if self.layout.left.contains(p) {
                    return Some(false);
                }
                if self.preview_area().contains(p) && self.image_rect().contains(p) {
                    self.slider_focus = None;
                    self.cancel_crop();
                    let q = self.doc_point(p);
                    if self.workspace.tool == Tool::Select {
                        let tolerance = 6. * self.editor.state.size.0 as f32
                            / self.image_rect().w.max(1) as f32;
                        if let Some(o) = self.selected_object().filter(|o| {
                            matches!(
                                o.tool,
                                Tool::Rectangle
                                    | Tool::Ellipse
                                    | Tool::Line
                                    | Tool::Arrow
                                    | Tool::Text
                            )
                        }) {
                            let (_, b) = o.bounds();
                            if (b.x - q.x).abs() <= tolerance && (b.y - q.y).abs() <= tolerance {
                                self.workspace.gesture = Some(Gesture {
                                    object: o.clone(),
                                    start: q,
                                    moving: false,
                                    resizing: true,
                                });
                                return Some(true);
                            }
                        }
                        self.workspace.selected = None;
                        if let Some((layer, id)) = self.editor.scene.hit(q, tolerance) {
                            self.editor.activate(layer);
                            self.workspace.selected = Some(id);
                            let object = self.editor.scene.object(id).unwrap().clone();
                            self.workspace.style = object.style;
                            self.workspace.gesture = Some(Gesture {
                                object,
                                start: q,
                                moving: true,
                                resizing: false,
                            });
                        }
                    } else {
                        if !self.editor.scene.writable() {
                            self.message
                                .set_str("Add/select an unlocked annotation layer in Layers.");
                            return Some(true);
                        }
                        let o = Object::new(self.workspace.tool, q, self.workspace.style);
                        if self.workspace.tool == Tool::Text {
                            self.edit_text(o, true);
                            self.suppress_click = true;
                        } else {
                            self.workspace.gesture = Some(Gesture {
                                object: o,
                                start: q,
                                moving: false,
                                resizing: false,
                            });
                        }
                    }
                    return Some(true);
                }
            }
            Event::MouseMove { x, y } if self.workspace.gesture.is_some() => {
                let q = self.doc_point(Point::new(x, y));
                let original = self.workspace.gesture.as_ref().unwrap();
                let mut o = if original.moving || original.resizing {
                    self.editor
                        .scene
                        .object(original.object.id)
                        .cloned()
                        .unwrap_or_else(|| original.object.clone())
                } else {
                    original.object.clone()
                };
                if original.moving {
                    o.translate(q.x - original.start.x, q.y - original.start.y);
                } else if original.resizing {
                    o.resize(q);
                } else if matches!(o.tool, Tool::Pen | Tool::Highlighter) {
                    o.add_point(q);
                } else {
                    o.b = q;
                }
                self.workspace.gesture.as_mut().unwrap().object = o;
                return Some(true);
            }
            Event::MouseUp { x, y, button: 0 } if self.workspace.gesture.is_some() => {
                self.annotation_release(Point::new(x, y));
                self.suppress_click = true;
                return Some(true);
            }
            Event::MouseWheel { x, y, delta } => {
                let p = Point::new(x, y);
                if self.workspace.gesture.is_some() {
                    return Some(false);
                }
                if self.workspace.left == 1 && self.layout.left.contains(p) {
                    let max = (self.annotation_labels().len() as i32 * 30 + 80
                        - self.layout.left.h as i32)
                        .max(0);
                    self.workspace.scroll =
                        (self.workspace.scroll + if delta > 0 { 32 } else { -32 }).clamp(0, max);
                    return Some(true);
                }
                if self.workspace.right == 1 && self.layout.right.contains(p) {
                    let max = (354 - self.layout.right.h as i32).max(0);
                    self.workspace.info_scroll = (self.workspace.info_scroll
                        + if delta > 0 { 32 } else { -32 })
                    .clamp(0, max);
                    return Some(true);
                }
                if self.workspace.right == 0 && self.layout.right.contains(p) {
                    let max = (self.editor.scene.layers.len() as i32 * 32 + 224
                        - self.layout.right.h as i32)
                        .max(0);
                    self.workspace.layer_scroll = (self.workspace.layer_scroll
                        + if delta > 0 { 32 } else { -32 })
                    .clamp(0, max);
                    return Some(true);
                }
            }
            Event::KeyPress {
                keycode: KEY_ESC,
                pressed: true,
                ..
            } if self.workspace.gesture.is_some() || self.workspace.selected.is_some() => {
                self.workspace.gesture = None;
                self.workspace.selected = None;
                return Some(true);
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ctrl: false,
                alt: false,
                super_key: false,
                shift,
            } if self.selected_object().is_some()
                && matches!(keycode, 0x4b | 0x4d | 0x48 | 0x50) =>
            {
                let mut o = self.selected_object().unwrap().clone();
                let d = if shift { 10. } else { 1. };
                let (dx, dy) = match keycode {
                    0x4b => (-d, 0.),
                    0x4d => (d, 0.),
                    0x48 => (0., -d),
                    _ => (0., d),
                };
                o.translate(dx, dy);
                self.scene_action(|s| s.replace(o));
                return Some(true);
            }
            Event::KeyPress {
                keycode: 0x53,
                pressed: true,
                ..
            } if self.workspace.selected.is_some() => {
                self.layer_action(9);
                return Some(true);
            }
            Event::KeyPress {
                keycode: 0x1c,
                pressed: true,
                ..
            } if self.selected_object().is_some_and(|o| o.tool == Tool::Text) => {
                let o = self.selected_object().unwrap().clone();
                self.edit_text(o, false);
                return Some(true);
            }
            Event::FocusChanged { focused: false }
            | Event::PointerOwnership {
                owned: false,
                captured: false,
            } => {
                self.workspace.gesture = None;
            }
            Event::KeyPress { pressed: true, .. } if self.workspace.gesture.is_some() => {
                return Some(false);
            }
            _ => {}
        }
        None
    }
    fn annotation_release(&mut self, p: Point) {
        self.workspace_event(&Event::MouseMove { x: p.x, y: p.y });
        if let Some(g) = self.workspace.gesture.take() {
            if g.moving || g.resizing {
                self.scene_action(|s| s.replace(g.object));
            } else if g.object.valid() {
                self.commit_object(g.object);
            }
        }
    }
    pub(super) fn draw_annotation_overlay(&self, c: &mut Canvas, dst: Rect, t: &Theme) {
        let (w, h) = self.editor.state.size;
        if w == 0 || h == 0 || dst.w == 0 || dst.h == 0 {
            return;
        }
        let to_screen = |p: P| {
            Point::new(
                dst.x + (p.x * dst.w as f32 / w as f32) as i32,
                dst.y + (p.y * dst.h as f32 / h as f32) as i32,
            )
        };
        let hidden = self
            .workspace
            .gesture
            .as_ref()
            .filter(|g| g.moving || g.resizing)
            .map(|g| g.object.id);
        for l in self
            .editor
            .scene
            .layers
            .iter()
            .skip(1)
            .filter(|l| l.visible && l.opacity > 0)
        {
            let mut objects: alloc::vec::Vec<&Object> = l
                .objects
                .iter()
                .filter(|o| hidden != Some(o.id))
                .map(|o| o.as_ref())
                .collect();
            if l.id == self.editor.scene.active {
                if let Some(g) = &self.workspace.gesture {
                    objects.push(&g.object);
                }
            }
            if l.opacity == 255 {
                for o in objects {
                    self.paint_object(c, dst, o, 255);
                }
            } else {
                self.paint_group(c, dst, &objects, l.opacity);
            }
        }
        if let Some(o) = self
            .workspace
            .gesture
            .as_ref()
            .map(|g| &g.object)
            .or_else(|| self.selected_object())
        {
            let (a, b) = o.bounds();
            let a = to_screen(a);
            let b = to_screen(b);
            let r = Rect::new(
                a.x,
                a.y,
                (b.x - a.x).max(1) as u32,
                (b.y - a.y).max(1) as u32,
            );
            c.draw_rect(r, t.accent);
            if matches!(
                o.tool,
                Tool::Rectangle | Tool::Ellipse | Tool::Line | Tool::Arrow | Tool::Text
            ) {
                c.fill_rect(Rect::new(b.x - 3, b.y - 3, 7, 7), t.accent);
            }
        }
    }
    fn paint_group(&self, c: &mut Canvas, dst: Rect, objects: &[&Object], opacity: u8) {
        if objects.is_empty() {
            return;
        }
        let (w, h) = self.editor.state.size;
        let mut lo = P::new(w as f32, h as f32);
        let mut hi = P::new(0., 0.);
        for o in objects {
            let (a, b) = o.bounds();
            lo.x = lo.x.min(a.x);
            lo.y = lo.y.min(a.y);
            hi.x = hi.x.max(b.x);
            hi.y = hi.y.max(b.y);
        }
        let x0 = (dst.x + (lo.x * dst.w as f32 / w as f32) as i32)
            .max(dst.x)
            .max(0);
        let y0 = (dst.y + (lo.y * dst.h as f32 / h as f32) as i32)
            .max(dst.y)
            .max(0);
        let x1 = (dst.x + (hi.x * dst.w as f32 / w as f32) as i32 + 1)
            .min(dst.right())
            .min(c.width as i32);
        let y1 = (dst.y + (hi.y * dst.h as f32 / h as f32) as i32 + 1)
            .min(dst.bottom())
            .min(c.height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = P::new(
                    (x as f32 + 0.5 - dst.x as f32) * w as f32 / dst.w as f32,
                    (y as f32 + 0.5 - dst.y as f32) * h as f32 / dst.h as f32,
                );
                let mut group = 0;
                for o in objects {
                    group = annotations::over(group, o.pixel(p, 255));
                }
                let i = y as usize * c.stride as usize + x as usize;
                c.pixels[i] =
                    annotations::over(c.pixels[i], annotations::with_opacity(group, opacity));
            }
        }
    }
    fn paint_object(&self, c: &mut Canvas, dst: Rect, o: &Object, opacity: u8) {
        let (w, h) = self.editor.state.size;
        let (a, b) = o.bounds();
        let x0 = (dst.x + (a.x * dst.w as f32 / w as f32) as i32)
            .max(dst.x)
            .max(0);
        let y0 = (dst.y + (a.y * dst.h as f32 / h as f32) as i32)
            .max(dst.y)
            .max(0);
        let x1 = (dst.x + (b.x * dst.w as f32 / w as f32) as i32 + 1)
            .min(dst.right())
            .min(c.width as i32);
        let y1 = (dst.y + (b.y * dst.h as f32 / h as f32) as i32 + 1)
            .min(dst.bottom())
            .min(c.height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = P::new(
                    (x as f32 + 0.5 - dst.x as f32) * w as f32 / dst.w as f32,
                    (y as f32 + 0.5 - dst.y as f32) * h as f32 / dst.h as f32,
                );
                let src = o.pixel(p, opacity);
                if src >> 24 > 0 {
                    let i = y as usize * c.stride as usize + x as usize;
                    c.pixels[i] = annotations::over(c.pixels[i], src);
                }
            }
        }
    }
}
