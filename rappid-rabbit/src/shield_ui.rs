//! Small browser settings surface, using existing native Checkbox controls.
use super::*;
use core::sync::atomic::Ordering;
use rappid_rabbit::content_shield::{CanonicalUrl, CosmeticPolicy};
use sunlight_ui::widgets::Checkbox;

impl RabbitApp {
    pub(super) fn shield_site_active(&self) -> bool {
        self.shield_site()
            .is_none_or(|site| !self.shield.settings.site_exceptions.contains(&site))
            && self.shield.active_lists().next().is_some()
    }
    pub(super) fn shield_settings_rect(&self) -> Rect {
        let width = self.client_bounds.w.saturating_sub(16).min(410);
        Rect::new(
            (self.client_bounds.w.saturating_sub(width + 8)) as i32,
            TOP_BAR_H as i32 + 4,
            width,
            (220 + self.shield.lists.len() as u32 * 28)
                .min(self.client_bounds.h.saturating_sub(TOP_BAR_H + 12)),
        )
    }
    fn shield_site(&self) -> Option<String> {
        self.render_state
            .as_ref()
            .map(|s| s.final_url.as_str())
            .or_else(|| (!self.navigation_url.is_empty()).then_some(self.navigation_url.as_str()))
            .and_then(CanonicalUrl::parse)
            .map(|u| u.host)
    }
    fn shield_checkbox_rect(&self, index: u32) -> Rect {
        let r = self.shield_settings_rect();
        Rect::new(
            r.x + 14,
            r.y + 38 + (index * 28) as i32,
            r.w.saturating_sub(28),
            24,
        )
    }
    pub(super) fn draw_shield_settings(&self, canvas: &mut Canvas, theme: &Theme) {
        if !self.shield_settings_open {
            return;
        }
        let rect = self.shield_settings_rect();
        canvas.fill_rounded_rect(rect, 8, theme.panel);
        canvas.stroke_rounded_rect(rect, 8, 1, theme.accent);
        let title = Rect::new(rect.x + 14, rect.y + 8, rect.w.saturating_sub(28), 24);
        Label::new(title, "Content Shield")
            .with_font(&F_UI)
            .draw(canvas, theme);
        let mut enable =
            Checkbox::new(self.shield_checkbox_rect(0), "Enable Content Shield").with_font(&F_UI);
        enable.checked = self.shield.settings.enabled;
        enable.draw(canvas, theme);
        for (index, list) in self.shield.lists.iter().enumerate() {
            let mut cb = Checkbox::new(self.shield_checkbox_rect(index as u32 + 1), &list.name)
                .with_font(&F_UI);
            cb.checked = list.enabled && !self.shield.settings.disabled_lists.contains(&list.id);
            cb.draw(canvas, theme);
        }
        let site = self.shield_site();
        let mut cb = Checkbox::new(
            self.shield_checkbox_rect(self.shield.lists.len() as u32 + 1),
            "Disable for the current site",
        )
        .with_font(&F_UI);
        cb.checked = site
            .as_ref()
            .is_some_and(|s| self.shield.settings.site_exceptions.contains(s));
        cb.draw(canvas, theme);
        let supported: usize = self
            .shield
            .active_lists()
            .map(|s| s.compiled.diagnostics.supported)
            .sum();
        let unsupported: usize = self
            .shield
            .active_lists()
            .map(|s| s.compiled.diagnostics.unsupported)
            .sum();
        let hidden = self
            .render_state
            .as_ref()
            .map_or(0, |s| s.styles.shield_hidden_count());
        let lines = [
            site.unwrap_or_else(|| "No HTTP(S) site".into()),
            format!("{supported} supported / {unsupported} unsupported rules"),
            format!(
                "{} requests blocked / {hidden} elements hidden",
                self.shield_stats.blocked.load(Ordering::Relaxed)
            ),
            "Reload to apply network rule changes.".into(),
            self.shield_persistence_status.clone(),
        ];
        let y = rect.y + 42 + (self.shield.lists.len() as i32 + 2) * 28;
        for (i, line) in lines.iter().enumerate() {
            let text = super::browser_ui::fit_text(line, rect.w.saturating_sub(28), &F_SMALL);
            Label::new(
                Rect::new(
                    rect.x + 14,
                    y + i as i32 * 20,
                    rect.w.saturating_sub(28),
                    20,
                ),
                &text,
            )
            .with_font(&F_SMALL)
            .draw(canvas, theme);
        }
    }
    pub(super) fn shield_changed(&mut self) {
        self.shield.changed();
        self.shield_stats.cancelled.store(true, Ordering::Release);
        self.shield_stats = Default::default();
        self.image_completion = None;
        self.viewport_version = self.viewport_version.wrapping_add(1);
        self.shield_reflow_pending = true;
        self.form_state.focused_control = None;
        self.form_state.pressed_control = None;
        if self.runtime.is_none() {
            if let Some(state) = self.render_state.as_mut() {
                state.set_shield_policy(
                    CosmeticPolicy::for_document(&self.shield, &state.final_url),
                    &RABBIT_FONTS,
                );
            }
            self.shield_reflow_pending = false;
        }
        self.shield_persistence_status =
            if rappid_rabbit::content_shield::save_settings(&self.shield.settings) {
                "Preferences saved"
            } else {
                "Preferences apply to this session (KV unavailable)"
            }
            .into();
    }
    pub(super) fn update_shield_ui(&mut self, event: Event) -> bool {
        if matches!(
            event,
            Event::KeyPress {
                keycode: 0x1F,
                pressed: true,
                ctrl: true,
                shift: true,
                ..
            }
        ) {
            self.shield_settings_open = !self.shield_settings_open;
            self.url_input.active = false;
            return true;
        }
        if let Event::Click { x, y } = event {
            let point = Point::new(x, y);
            if self.layout.shield.contains(point) {
                self.shield_settings_open = !self.shield_settings_open;
                self.url_input.active = false;
                return true;
            }
            if self.shield_settings_open {
                if !self.shield_settings_rect().contains(point) {
                    self.shield_settings_open = false;
                    return true;
                }
                let mut cb = Checkbox::new(self.shield_checkbox_rect(0), "");
                if cb.update(event) {
                    self.shield.settings.enabled = !self.shield.settings.enabled;
                    self.shield_changed();
                    return true;
                }
                for index in 0..self.shield.lists.len() {
                    let mut cb = Checkbox::new(self.shield_checkbox_rect(index as u32 + 1), "");
                    if cb.update(event) {
                        let id = self.shield.lists[index].id.clone();
                        if !self.shield.settings.disabled_lists.remove(&id) {
                            self.shield.settings.disabled_lists.insert(id);
                        }
                        self.shield_changed();
                        return true;
                    }
                }
                let mut cb = Checkbox::new(
                    self.shield_checkbox_rect(self.shield.lists.len() as u32 + 1),
                    "",
                );
                if cb.update(event) {
                    if let Some(site) = self.shield_site() {
                        let disabled = !self.shield.settings.site_exceptions.contains(&site);
                        self.shield.settings.set_site_exception(&site, disabled);
                        self.shield_changed();
                    }
                    return true;
                }
                return true;
            }
        }
        if self.shield_settings_open
            && matches!(
                event,
                Event::MouseDown { .. }
                    | Event::MouseUp { .. }
                    | Event::MouseWheel { .. }
                    | Event::Key(_)
                    | Event::KeyPress { .. }
            )
        {
            return true;
        }
        // Rebuilding policy is asynchronous. Do not dispatch pointer/key events
        // to objects from a presentation awaiting the new policy.
        self.shield_reflow_pending
            && self.document_view == DocumentView::Render
            && event.pos().is_some_and(|p| self.layout.content.contains(p))
            && matches!(
                event,
                Event::MouseDown { .. } | Event::MouseUp { .. } | Event::Click { .. }
            )
    }
}
