//! Chrome and page interaction adapters for the single retained document.
use super::*;

impl RabbitApp {
    pub(super) fn document_rect(&mut self) -> Rect {
        let panel = self.source_panel_rect();
        // Reserve a stable gutter even while the page fits. Late image layout
        // cannot toggle the layout width and cause a second reflow or cover links.
        Rect::new(
            panel.x,
            panel.y,
            panel.w.saturating_sub(SCROLLBAR_WIDTH),
            panel.h,
        )
    }

    pub(super) fn sync_document_scroll(&mut self) {
        let viewport = self.document_rect().size();
        #[cfg(feature = "dom")]
        let content = self
            .render_state
            .as_ref()
            .map(|s| s.current_scene.content_size)
            .unwrap_or_default();
        #[cfg(not(feature = "dom"))]
        let content = Size::default();
        self.document_scroll
            .set_geometry(viewport.w, viewport.h, content.w, content.h);
        if !self.document_scroll.can_scroll_y() {
            self.document_scroll.end_drag();
        }
    }

    pub(super) fn draw_document_scrollbar(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let region = self.source_panel_rect();
        let track = self.document_scroll.track_rect(region);
        canvas.fill_rect(track, theme.panel);
        draw_scrollbar(
            canvas,
            theme,
            region,
            &self.document_scroll,
            ScrollPolicy::Auto,
        );
    }

    pub(super) fn handle_document_scroll(&mut self, event: Event) -> bool {
        // A release produces a separate Click in the native event loop. Consume
        // it even when a drag ended outside the track, so it cannot activate a link.
        if matches!(event, Event::Click { .. }) && self.suppress_scroll_click {
            self.suppress_scroll_click = false;
            self.document_scroll.end_drag();
            return true;
        }
        if self.document_view != DocumentView::Render {
            return false;
        }
        let region = self.source_panel_rect();
        let track = self.document_scroll.track_rect(region);
        match event {
            Event::MouseWheel { x, y, delta } if region.contains(Point::new(x, y)) => {
                // No layout invalidation: presentation and resource priority read
                // this same offset on their next tick.
                self.document_scroll.scroll_by_wheel(delta, 48)
            }
            Event::MouseDown { x, y, button: 0 }
                if self.document_scroll.can_scroll_y()
                    && region.contains(Point::new(x, y))
                    && track.contains(Point::new(x, y)) =>
            {
                self.suppress_scroll_click = true;
                self.url_input.active = false;
                self.focus = FocusPane::Render;
                if self
                    .document_scroll
                    .thumb_rect(track)
                    .is_some_and(|thumb| thumb.contains(Point::new(x, y)))
                {
                    self.document_scroll.start_drag(track, y);
                } else {
                    self.document_scroll.handle_track_click(track, y);
                }
                true
            }
            Event::Click { x, y }
                if self.document_scroll.can_scroll_y()
                    && region.contains(Point::new(x, y))
                    && track.contains(Point::new(x, y)) =>
            {
                self.document_scroll.handle_track_click(track, y);
                true
            }
            Event::MouseMove { y, .. } if self.document_scroll.dragging => {
                self.document_scroll.update_drag(track, y);
                true
            }
            Event::MouseUp { button: 0, .. } if self.document_scroll.dragging => {
                self.document_scroll.end_drag();
                true
            }
            _ => false,
        }
    }

    pub(super) fn cursor_at(&mut self, point: Point) -> CursorShape {
        if self.url_input.context_menu_open() {
            return CursorShape::Pointer;
        }
        if self.document_scroll.dragging {
            return CursorShape::ResizeVertical;
        }
        if self.url_text_rect().contains(point) {
            return CursorShape::Text;
        }
        if self.devtools_button_rect().contains(point)
            || self.view_button_rect().contains(point)
            || (!self.navigation_draining && self.url_action_rect().contains(point))
        {
            return CursorShape::Hand;
        }
        let layout = self.developer_panel_layout();
        if layout
            .resize_handle_rect
            .is_some_and(|rect| rect.contains(point))
        {
            return CursorShape::ResizeVertical;
        }
        if self.document_view != DocumentView::Render {
            return CursorShape::Pointer;
        }
        if self.document_scroll.can_scroll_y()
            && self
                .document_scroll
                .thumb_rect(self.document_scroll.track_rect(layout.main_rect))
                .is_some_and(|rect| rect.contains(point))
        {
            return CursorShape::ResizeVertical;
        }
        #[cfg(feature = "dom")]
        {
            let target = self.document_canvas().hit_test(point).map(|object| {
                (
                    object.owner_node_id,
                    match object.interaction {
                        Some(RenderInteraction::Control { owner_node_id }) => Some(owner_node_id),
                        _ => None,
                    },
                    matches!(object.interaction, Some(RenderInteraction::Link { .. })),
                )
            });
            let Some((owner, control, link)) = target else {
                return CursorShape::Pointer;
            };
            let Some(state) = self.render_state.as_ref() else {
                return CursorShape::Pointer;
            };
            let dom = &state.dom;
            if let Some(owner_node_id) = control {
                let id = owner_node_id.0 as usize;
                // Runtime form values outlive a scene. Interaction attributes
                // come from the current DOM so SunJS enable/type changes cannot
                // leave a stale disabled or text cursor behind.
                if dom.get_attribute(id, "disabled").is_some() {
                    return CursorShape::Pointer;
                }
                let text = dom
                    .tag_name(id)
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("textarea"))
                    || (dom
                        .tag_name(id)
                        .is_some_and(|tag| tag.eq_ignore_ascii_case("input"))
                        && !matches!(
                            dom.get_attribute(id, "type")
                                .unwrap_or("text")
                                .to_ascii_lowercase()
                                .as_str(),
                            "submit"
                                | "reset"
                                | "button"
                                | "checkbox"
                                | "radio"
                                | "file"
                                | "image"
                                | "hidden"
                        ));
                return if text {
                    CursorShape::Text
                } else {
                    CursorShape::Hand
                };
            }
            if link {
                return CursorShape::Hand;
            }
            // Textarea/contenteditable have no dedicated Canvas interaction yet.
            // Inspect only the topmost object's ancestry, never objects beneath it.
            let mut node = Some(owner.0 as usize);
            while let Some(id) = node {
                if dom.get_attribute(id, "disabled").is_some() {
                    return CursorShape::Pointer;
                }
                if dom
                    .tag_name(id)
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("textarea"))
                {
                    return CursorShape::Text;
                }
                if let Some(editable) = dom.get_attribute(id, "contenteditable") {
                    return if editable.is_empty()
                        || editable.eq_ignore_ascii_case("true")
                        || editable.eq_ignore_ascii_case("plaintext-only")
                    {
                        CursorShape::Text
                    } else {
                        CursorShape::Pointer
                    };
                }
                node = dom.parent(id);
            }
        }
        CursorShape::Pointer
    }

    pub(super) fn refresh_cursor(&mut self) {
        self.cursor = self
            .pointer
            .map(|point| self.cursor_at(point))
            .unwrap_or(CursorShape::Pointer);
        let region = self.source_panel_rect();
        let track = self.document_scroll.track_rect(region);
        self.document_scroll.hovered = self.document_view == DocumentView::Render
            && self.pointer.is_some_and(|point| track.contains(point));
    }

    pub(super) fn refresh_tab_title(&mut self) {
        if self.pending_fetch {
            return;
        }
        #[cfg(feature = "dom")]
        if let Some(state) = self.render_state.as_ref() {
            let revision = (state.document_id, state.dom_revision);
            if self.title_revision == Some(revision) {
                return;
            }
            let title = state
                .dom
                .find_first_element("title")
                .and_then(|id| state.dom.element_text_content(id))
                .unwrap_or_default();
            self.tab_title = compact_title(if title.trim().is_empty() {
                &state.final_url
            } else {
                &title
            });
            self.title_revision = Some(revision);
            return;
        }
        self.tab_title = if self.navigation_url.is_empty() {
            "New page".into()
        } else {
            compact_title(&self.navigation_url)
        };
        self.title_revision = None;
    }

    pub(super) fn reload(&mut self) {
        if self.pending_fetch || self.navigation_draining {
            return;
        }
        #[cfg(feature = "dom")]
        if let Some(state) = self.render_state.as_ref() {
            self.url_input.set_text(&state.final_url);
        } else if !self.navigation_url.is_empty() {
            self.url_input.set_text(&self.navigation_url);
        }
        self.queue_fetch();
    }

    pub(super) fn stop_navigation(&mut self) {
        if !self.pending_fetch {
            return;
        }
        self.pending_fetch = false;
        // Keep the receiver until the single blocking worker finishes. Repeated
        // Stop/Reload cannot accumulate queued network closures or DOM buffers.
        self.navigation_draining = self.completion.is_some();
        self.document_lifecycle.begin_navigation();
        self.document_scroll.end_drag();
        if self.navigation_draining {
            if let Some(request_id) = self.active_navigation_request_id {
                self.developer_tools.network.fail_request(
                    request_id,
                    None,
                    None,
                    None,
                    "Navigation stopped",
                );
            }
        }
        self.status = "Stopped".into();
        self.render_status = "Navigation stopped".into();
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            "Navigation stopped; any pending worker result will be discarded.",
        );
    }

    pub(super) fn poll_stopped_navigation(&mut self) -> bool {
        if let Some(receiver) = self.completion.as_mut() {
            if matches!(
                receiver.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ) {
                return false;
            }
        }
        self.completion = None;
        self.navigation_draining = false;
        true
    }

    pub(super) fn draw_chrome_hint(&self, canvas: &mut Canvas, theme: &Theme) {
        let Some(point) = self.pointer else {
            return;
        };
        let label = if self.devtools_button_rect().contains(point) {
            "Developer tools (F12)"
        } else if self.view_button_rect().contains(point) {
            "Page source / Render"
        } else if self.layout.back.contains(point) {
            "Back: no history available"
        } else if self.layout.forward.contains(point) {
            "Forward: no history available"
        } else if self.url_action_rect().contains(point) {
            if self.pending_fetch {
                "Stop loading"
            } else if self.navigation_draining {
                "Waiting for stopped request"
            } else {
                "Reload (Ctrl+R)"
            }
        } else {
            return;
        };
        let width = (F_SMALL.measure_w(label) + 16).min(self.client_bounds.w);
        let rect = Rect::new(
            (point.x - width as i32 / 2)
                .clamp(0, self.client_bounds.w.saturating_sub(width) as i32),
            self.top_bar_rect().bottom() + 3,
            width,
            24,
        );
        canvas.fill_rounded_rect(rect, 4, theme.panel_alt);
        canvas.stroke_rounded_rect(rect, 4, 1, theme.chrome.subtle_border);
        let label = fit_text(label, width.saturating_sub(16), &F_SMALL);
        Label::new(
            Rect::new(rect.x + 8, rect.y, width.saturating_sub(16), rect.h),
            &label,
        )
        .with_font(&F_SMALL)
        .draw(canvas, theme);
    }

    pub(super) fn draw_top_bar(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let top = self.top_bar_rect();
        canvas.fill_rect(top, theme.panel);
        canvas.hbar(
            top.x,
            top.bottom() - 1,
            top.w,
            1,
            theme.chrome.subtle_border,
        );
        let tab = self.layout.tab;
        canvas.fill_rounded_rect(tab, 6, theme.chrome.input_bg);
        canvas.hbar(
            tab.x + 8,
            tab.bottom() - 1,
            tab.w.saturating_sub(16),
            2,
            theme.accent,
        );
        canvas.fill_rounded_rect(Rect::new(tab.x + 9, tab.y + 8, 6, 6), 3, theme.accent);
        let label = if self.pending_fetch {
            "Loading…"
        } else {
            self.tab_title.as_str()
        };
        let label = fit_text(label, tab.w.saturating_sub(36), &F_SMALL);
        Label::new(
            Rect::new(tab.x + 23, tab.y, tab.w.saturating_sub(29), tab.h),
            &label,
        )
        .with_font(&F_SMALL)
        .draw(canvas, theme);
        // No history exists in this browser yet. Disabled arrows deliberately
        // have no event action, and are omitted at narrow widths.
        for (rect, forward) in [(self.layout.back, false), (self.layout.forward, true)] {
            if rect.w == 0 {
                continue;
            }
            let cx = rect.x + rect.w as i32 / 2;
            let cy = rect.y + rect.h as i32 / 2;
            let color = theme.chrome.disabled_fg;
            canvas.hbar(cx - 5, cy, 11, 1, color);
            for dy in -4i32..=4 {
                let dx = 5 - dy.abs();
                canvas.fill_rect(
                    Rect::new(cx + if forward { dx } else { -dx }, cy + dy, 1, 1),
                    color,
                );
            }
        }
        self.draw_url_bar(canvas, theme);
        for (rect, label, active) in [
            (
                self.view_button_rect(),
                "<>",
                self.document_view == DocumentView::Source,
            ),
            (
                self.devtools_button_rect(),
                "{}",
                self.developer_tools.panel.open,
            ),
        ] {
            let hovered = self.pointer.is_some_and(|point| rect.contains(point));
            if hovered || active {
                canvas.fill_rounded_rect(rect, 5, theme.chrome.control_hover);
            }
            let color = if active {
                theme.accent
            } else {
                theme.icon_muted
            };
            F_SMALL.draw_vcenter(
                canvas,
                label,
                rect.x + (rect.w as i32 - F_SMALL.measure_w(label) as i32) / 2,
                rect.y,
                rect.h,
                color,
            );
        }
    }
}

fn compact_title(text: &str) -> String {
    let mut title = String::new();
    let mut count = 0;
    for word in text.split_whitespace() {
        if !title.is_empty() {
            title.push(' ');
            count += 1;
        }
        for ch in word.chars().filter(|c| !c.is_control()) {
            if count == 96 {
                return title;
            }
            title.push(ch);
            count += 1;
        }
        if count == 96 {
            return title;
        }
    }
    title
}

fn fit_text(text: &str, width: u32, font: &dyn VecText) -> String {
    if font.measure_w(text) <= width {
        return text.into();
    }
    let mut result = String::new();
    for ch in text.chars() {
        result.push(ch);
        if font.measure_w(&result).saturating_add(font.measure_w("…")) > width {
            result.pop();
            break;
        }
    }
    if font.measure_w("…") <= width {
        result.push('…');
    }
    result
}
