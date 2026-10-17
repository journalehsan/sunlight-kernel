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
        let _ = libc::close(self.fd);
        if !self.temporary.is_empty() {
            let _ = libc::unlink(self.temporary.as_bytes());
        }
    }
}

impl LightLensApp {
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
    }
    pub(super) fn finish_slider(&mut self) {
        if let Some((_, before)) = self.slider_drag.take() {
            self.editor.commit(before);
        }
    }
    fn release_drag(&mut self, x: i32, y: i32) -> bool {
        let dragging =
            self.slider_drag.is_some() || self.crop_start.is_some() || self.pan_start.is_some();
        if let Some((i, _)) = self.slider_drag {
            self.set_slider(i, x);
        }
        if let Some(start) = self.crop_start {
            let end =
                editor::image_point(self.image_rect(), self.editor.state.size, Point::new(x, y));
            self.crop = Some(editor::crop_rect(start, end, self.crop_ratio));
        }
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
    fn history(&mut self, redo: bool) {
        if !self.editable() {
            return;
        }
        self.finish_slider();
        self.cancel_crop();
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
    ) {
        Self::draw_placeholder_chip(canvas, theme, r, None, label);
        if enabled {
            if active {
                canvas.draw_rect(r, theme.accent);
            }
            sf_centered(
                canvas,
                r,
                label,
                &TextStyle::new(
                    FontRole::UiSmall,
                    if active { theme.accent } else { theme.text },
                ),
            );
        }
    }
    fn side_area(&self) -> Rect {
        let r = self.layout.left;
        Rect::new(r.x, r.y + 20, r.w, r.h.saturating_sub(20))
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
                12 => self.crop.is_some_and(|r| r.w > 0 && r.h > 0),
                13 => self.crop_mode,
                17 => false,
                _ => true,
            }
    }
    pub(super) fn draw_left_panel(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let area = Self::draw_panel(canvas, theme, rect, "Edit Tools");
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
                &format!("{} {:+}", label, value),
                r.x,
                r.y - 14,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
            sf_right(
                &mut c,
                Rect::new(r.x, r.y - 16, r.w, 14),
                "Reset",
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                0,
            );
            c.fill_rect(Rect::new(r.x, r.y + 5, r.w, 4), theme.border);
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
        let sampler = Sampler::new(self.editor.state);
        let size = self.editor.state.size;
        let x0 = dst.x.max(0) as u32;
        let y0 = dst.y.max(0) as u32;
        let x1 = dst.right().min(canvas.width as i32).max(0) as u32;
        let y1 = dst.bottom().min(canvas.height as i32).max(0) as u32;
        for y in y0..y1 {
            let sy = ((y as i64 - dst.y as i64) * size.1 as i64 / dst.h as i64) as u32;
            for x in x0..x1 {
                let sx = ((x as i64 - dst.x as i64) * size.0 as i64 / dst.w as i64) as u32;
                let p = sampler.pixel(image, sx, sy);
                let alpha = p >> 24;
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
            let (w, h) = self.editor.state.size;
            let x = dst.x + (crop.x as i64 * dst.w as i64 / w as i64) as i32;
            let y = dst.y + (crop.y as i64 * dst.h as i64 / h as i64) as i32;
            let r = Rect::new(
                x,
                y,
                (crop.w as u64 * dst.w as u64 / w as u64) as u32,
                (crop.h as u64 * dst.h as u64 / h as u64) as u32,
            );
            canvas.draw_rect(r, theme.accent);
            canvas.draw_rect(r.inset(-1), Color::rgb(255, 255, 255));
            for third in 1..3 {
                canvas.vline(r.x + (r.w * third / 3) as i32, r.y, r.h, theme.accent);
                canvas.hline(r.x, r.y + (r.h * third / 3) as i32, r.w, theme.accent);
            }
        }
    }
    fn set_slider(&mut self, index: usize, x: i32) {
        let r = self.slider_rect(index);
        if r.w <= 8 {
            return;
        }
        self.editor.state.adjust[index] =
            (((x - r.x - 4) * 200 / (r.w - 8) as i32) - 100).clamp(-100, 100) as i16;
    }
    fn side_action(&mut self, index: usize) {
        if !self.side_enabled(index) {
            return;
        }
        self.finish_slider();
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
                self.crop_mode = !self.crop_mode;
                self.crop = None;
                self.crop_start = None;
            }
            11 => {
                self.crop_ratio = (self.crop_ratio + 1) % 4;
                self.crop = None;
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
        !self.editor.dirty()
            || self.confirm(
                "Unsaved image",
                "Discard unsaved edits? Choose No to return and Save As.",
            )
    }
    fn save(&mut self, save_as: bool) {
        if !self.editable() {
            return;
        }
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
            let name = format!(".light-lens-{}-{}.tmp", sunlight_ipc::getpid(), attempt);
            let candidate = parent
                .join(name.as_bytes())
                .ok_or("Temporary path is too long.")?;
            if candidate == path {
                continue;
            }
            match libc::open_with_flags_mode(
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
        libc::write_all(fd, &header).map_err(|_| "Could not write export header.")?;
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
        let end = job.position + (len / 4) as u32;
        if libc::write_all(job.fd, &bytes[..len]).is_err() {
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
            libc::rename(job.temporary.as_bytes(), job.destination.as_bytes())
        } else {
            libc::rename_no_replace(job.temporary.as_bytes(), job.destination.as_bytes())
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
        self.message.set_str("Saved image with alpha preserved.");
        self.collect_siblings();
        true
    }
    pub(super) fn editor_event(&mut self, event: Event) -> bool {
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
        match event {
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
                            let before = self.editor.state;
                            self.editor.state.adjust[i] = 0;
                            self.editor.commit(before);
                            return true;
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
                    && self.pan_start.is_none()
                    && delta != 0
                {
                    self.viewport.zoom_at(
                        self.preview_area(),
                        self.editor.state.size,
                        p,
                        delta < 0,
                    );
                    return true;
                }
                false
            }
            Event::MouseDown { x, y, button: 0 } => {
                self.suppress_click = false;
                if !self.editable() {
                    return false;
                }
                let p = Point::new(x, y);
                if self.side_area().contains(p) {
                    for i in 0..4 {
                        if self.slider_rect(i).contains(p) {
                            self.slider_drag = Some((i, self.editor.state));
                            self.set_slider(i, x);
                            return true;
                        }
                    }
                }
                if self.preview_area().contains(p) {
                    if self.crop_mode {
                        if !self.image_rect().contains(p) {
                            return false;
                        }
                        let start =
                            editor::image_point(self.image_rect(), self.editor.state.size, p);
                        self.crop_start = Some(start);
                        self.crop = Some(Rect::new(start.x, start.y, 0, 0));
                    } else {
                        if self.viewport.fit {
                            self.viewport.zoom =
                                Viewport::fit_zoom(self.preview_area(), self.editor.state.size);
                            self.viewport.fit = false;
                        }
                        self.pan_start = Some(p);
                    }
                    return true;
                }
                false
            }
            Event::MouseMove { x, y } => {
                if let Some((i, _)) = self.slider_drag {
                    self.set_slider(i, x);
                    return true;
                }
                if let Some(start) = self.crop_start {
                    let end = editor::image_point(
                        self.image_rect(),
                        self.editor.state.size,
                        Point::new(x, y),
                    );
                    self.crop = Some(editor::crop_rect(start, end, self.crop_ratio));
                    return true;
                }
                if let Some(start) = self.pan_start {
                    self.viewport.pan.x += x - start.x;
                    self.viewport.pan.y += y - start.y;
                    self.pan_start = Some(Point::new(x, y));
                    self.viewport
                        .clamp(self.preview_area(), self.editor.state.size);
                    return true;
                }
                false
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
                self.pan_start = None;
                true
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ctrl,
                shift,
                ..
            } => {
                self.finish_slider();
                self.pan_start = None;
                self.crop_start = None;
                if keycode == KEY_ESC {
                    if self.crop_mode {
                        self.cancel_crop();
                    } else if self.confirm_leave() {
                        request_close();
                    }
                    return true;
                }
                if ctrl {
                    match keycode {
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
                if keycode == 0x1c && self.crop_mode {
                    self.side_action(12);
                    return true;
                }
                if !self.editable() {
                    return false;
                }
                match keycode {
                    KEY_LEFT => self.show_previous(),
                    KEY_RIGHT => self.show_next(),
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
    fn pan_starts_at_fit_scale_and_stops_on_click() {
        let mut app = ready();
        let r = app.image_rect();
        let p = Point::new(r.x + 10, r.y + 10);
        app.update(Event::mouse_down(p.x, p.y, 0));
        assert_eq!(app.image_rect(), r);
        app.update(Event::click(p.x + 10, p.y));
        assert!(app.pan_start.is_none());
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
    fn editor_renders_at_small_and_large_client_sizes() {
        let mut app = ready();
        for (w, h) in [
            (1220, 760),
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
}
