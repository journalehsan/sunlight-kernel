#![no_std]
#![no_main]

use core::alloc::{GlobalAlloc, Layout};

use melody_mina::{
    layout::{LayoutMode, MelodyLayout},
    model::{
        timeline_percent, NowPlayingViewModel, PlaybackState, PlaylistItemViewModel,
        DEMO_NOW_PLAYING, DEMO_PLAYLIST,
    },
    visualizer::{
        DemoVisualizationSource, VisualizationFrame, VisualizationSource, MAX_VISUALIZATION_BINS,
    },
};
use sun_font::{
    draw_text, draw_text_centered, draw_text_right, draw_text_vcenter, measure_text, FontRole,
    TextStyle,
};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, process_yield, ProcessExit,
};
use sunlight_ui::{
    draw_scrollbar, hit_test_scrollbar,
    image::TgaImage,
    request_close, set_client_cursor,
    widgets::{ButtonState, IconButton, Slider},
    App, Canvas, CursorShape, Event, LayoutInvalidation, Point, Rect, ScrollPolicy,
    ScrollState, Size, Theme, UiSymbol, Window, WindowConfig, WindowDecoration, WindowEvent,
};

const WIN_W: u32 = 1040;
const WIN_H: u32 = 720;
const KEY_ESC: u8 = 0x01;
const KEY_TAB: u8 = 0x0F;
const KEY_ENTER: u8 = 0x1C;
const KEY_SPACE: u8 = 0x39;
const PLAYLIST_ROW_H: u32 = 52;
const CONTROL_COUNT: usize = 10;
const FRAME_MS_FOCUSED: u64 = 33;
const FRAME_MS_UNFOCUSED: u64 = 100;

static PLACEHOLDER_ART_BYTES: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/audio-x-generic.tga");

struct NoAlloc;

unsafe impl GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        core::ptr::null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[MELODY-MINA] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy)]
struct AlbumArtView {
    rect: Rect,
    image: Option<TgaImage>,
}

impl AlbumArtView {
    fn new(rect: Rect, image: Option<TgaImage>) -> Self {
        Self { rect, image }
    }

    fn cover_rect(viewport: Rect, source: Size) -> Option<Rect> {
        if viewport.w == 0 || viewport.h == 0 || source.w == 0 || source.h == 0 {
            return None;
        }
        let by_width_h = (u64::from(viewport.w) * u64::from(source.h) / u64::from(source.w))
            .min(u64::from(u32::MAX)) as u32;
        let (w, h) = if by_width_h >= viewport.h {
            (viewport.w, by_width_h)
        } else {
            let w = (u64::from(viewport.h) * u64::from(source.w) / u64::from(source.h))
                .min(u64::from(u32::MAX)) as u32;
            (w, viewport.h)
        };
        Some(Rect::new(
            viewport.x - (w.saturating_sub(viewport.w) / 2) as i32,
            viewport.y - (h.saturating_sub(viewport.h) / 2) as i32,
            w,
            h,
        ))
    }

    fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.rect.w == 0 || self.rect.h == 0 {
            return;
        }
        canvas.fill_rounded_rect_with_border(
            self.rect,
            14,
            theme.chrome.card_bg.darken(8),
            theme.chrome.subtle_border,
            1,
        );
        let inner = self.rect.inset(2);
        let mut clipped = canvas.sub_canvas(inner);
        let local = Rect::new(0, 0, inner.w, inner.h);
        clipped.fill_rect(local, theme.panel_alt);
        if let Some(image) = self.image {
            if let Some(dst) = Self::cover_rect(local, Size::new(image.width, image.height)) {
                clipped.draw_tga_icon_tinted_rounded(&image, dst, theme.accent, 12);
            }
        } else {
            clipped.draw_ui_symbol_centered(
                local.inset((local.w.min(local.h) / 3) as i32),
                UiSymbol::Music,
                theme.icon_muted,
            );
            draw_text_centered(
                &mut clipped,
                local,
                "No artwork",
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }
    }
}

struct PlaylistView<'a> {
    rect: Rect,
    items: &'a [PlaylistItemViewModel],
    selected: usize,
    hovered: Option<usize>,
    focused: bool,
    scroll: &'a ScrollState,
}

impl<'a> PlaylistView<'a> {
    fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rounded_rect_with_border(
            self.rect,
            10,
            theme.chrome.card_bg,
            if self.focused {
                theme.accent
            } else {
                theme.border
            },
            1,
        );
        let viewport = self.rect.inset(2);
        if viewport.w == 0 || viewport.h == 0 {
            return;
        }
        let mut clip = canvas.sub_canvas(viewport);
        let local_w = viewport
            .w
            .saturating_sub(if self.scroll.can_scroll_y() { 10 } else { 0 });
        for (index, item) in self.items.iter().enumerate() {
            let y = index as i32 * PLAYLIST_ROW_H as i32 - self.scroll.offset_y;
            if y + PLAYLIST_ROW_H as i32 <= 0 || y >= viewport.h as i32 {
                continue;
            }
            let row = Rect::new(0, y, local_w, PLAYLIST_ROW_H);
            let selected = index == self.selected;
            if selected {
                clip.fill_rounded_rect(row.inset(3), 7, theme.chrome.selection);
                clip.fill_rounded_rect(
                    Rect::new(3, y + 7, 3, PLAYLIST_ROW_H.saturating_sub(14)),
                    2,
                    theme.accent,
                );
            } else if self.hovered == Some(index) {
                clip.fill_rounded_rect(row.inset(3), 7, theme.panel_alt);
            }

            let text_x = 14;
            let right_reserve = 66u32.min(local_w / 3);
            let max_text_w = local_w
                .saturating_sub(text_x as u32)
                .saturating_sub(right_reserve);
            let mut title_buf = [0u8; 96];
            let title = elide(item.title, FontRole::UiMedium, max_text_w, &mut title_buf);
            draw_text(
                &mut clip,
                title,
                text_x,
                y + 8,
                &TextStyle::new(
                    FontRole::UiMedium,
                    if selected {
                        theme.accent_hover
                    } else {
                        theme.text
                    },
                ),
            );
            if let Some(artist) = item.artist {
                let mut artist_buf = [0u8; 96];
                let artist = elide(artist, FontRole::UiSmall, max_text_w, &mut artist_buf);
                draw_text(
                    &mut clip,
                    artist,
                    text_x,
                    y + 28,
                    &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                );
            }
            if let Some(seconds) = item.duration_seconds {
                let mut time_buf = [0u8; 8];
                draw_text_right(
                    &mut clip,
                    Rect::new(0, y, local_w.saturating_sub(24), PLAYLIST_ROW_H),
                    time_text(seconds, &mut time_buf),
                    &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                    4,
                );
            }
            clip.draw_ui_symbol_centered(
                Rect::new(local_w.saturating_sub(22) as i32, y + 13, 18, 26),
                UiSymbol::MoreHorizontal,
                if self.hovered == Some(index) {
                    theme.icon_foreground
                } else {
                    theme.icon_muted
                },
            );
        }
        draw_scrollbar(
            canvas,
            theme,
            self.rect.inset(2),
            self.scroll,
            ScrollPolicy::Auto,
        );
    }
}

struct VisualizerView {
    rect: Rect,
    frame: VisualizationFrame,
}

impl VisualizerView {
    fn desired_bins(width: u32) -> usize {
        (width / 14).clamp(24, MAX_VISUALIZATION_BINS as u32) as usize
    }

    fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rounded_rect(self.rect, 9, theme.panel);
        let inner = self.rect.inset(8);
        let bins = self.frame.bins();
        if bins.is_empty() || inner.w == 0 || inner.h == 0 {
            return;
        }
        let gap = 3u32;
        let total_gap = gap.saturating_mul(bins.len().saturating_sub(1) as u32);
        let bar_w = inner
            .w
            .saturating_sub(total_gap)
            .checked_div(bins.len() as u32)
            .unwrap_or(0)
            .max(2);
        let used = bar_w
            .saturating_mul(bins.len() as u32)
            .saturating_add(total_gap);
        let mut x = inner.x + (inner.w.saturating_sub(used) / 2) as i32;
        for (index, amplitude) in bins.iter().enumerate() {
            let h = (inner.h.saturating_mul(u32::from(*amplitude)) / 100)
                .max(2)
                .min(inner.h);
            let bar = Rect::new(x, inner.bottom() - h as i32, bar_w, h);
            let emphasis = ((index * 7 + bins.len()) % 11) < 3;
            canvas.fill_rounded_rect(
                bar,
                (bar_w / 2).min(3),
                if emphasis {
                    theme.accent_hover
                } else {
                    theme.accent.darken(48)
                },
            );
            x += (bar_w + gap) as i32;
        }
    }
}

struct MelodyMinaApp {
    now_playing: NowPlayingViewModel,
    selected: usize,
    hovered_item: Option<usize>,
    playlist_scroll: ScrollState,
    timeline: Slider,
    volume: Slider,
    layout: MelodyLayout,
    layout_invalidation: LayoutInvalidation,
    client: Size,
    hovered_control: Option<usize>,
    pressed_control: Option<usize>,
    focus_index: usize,
    window_focused: bool,
    visualizer_source: DemoVisualizationSource,
    visualization_frame: VisualizationFrame,
    last_visualization_ms: u64,
    status: &'static str,
    artwork: Option<TgaImage>,
}

impl MelodyMinaApp {
    fn new() -> Self {
        let artwork = TgaImage::parse(PLACEHOLDER_ART_BYTES).ok();
        Self {
            now_playing: DEMO_NOW_PLAYING,
            selected: 0,
            hovered_item: None,
            playlist_scroll: ScrollState::new(),
            timeline: Slider::horizontal(Rect::default())
                .with_range(0, 100)
                .with_value(timeline_percent(&DEMO_NOW_PLAYING)),
            volume: Slider::horizontal(Rect::default())
                .with_range(0, 100)
                .with_value(68),
            layout: MelodyLayout::empty(),
            layout_invalidation: LayoutInvalidation::new(),
            client: Size::new(WIN_W, WIN_H),
            hovered_control: None,
            pressed_control: None,
            focus_index: 4,
            window_focused: true,
            visualizer_source: DemoVisualizationSource::new(),
            visualization_frame: VisualizationFrame::empty(),
            last_visualization_ms: 0,
            status: "Phase 1 UI demo / no audio backend",
            artwork,
        }
    }

    fn ensure_layout(&mut self) {
        let root = Rect::new(0, 0, self.client.w, self.client.h);
        if self.layout_invalidation.update(root) {
            self.layout = MelodyLayout::arrange(root);
            self.timeline.rect = self.layout.timeline_slider;
            self.volume.rect = self.layout.volume_slider;
            self.playlist_scroll.set_geometry(
                self.layout.playlist.w,
                self.layout.playlist.h,
                self.layout.playlist.w,
                PLAYLIST_ROW_H.saturating_mul(DEMO_PLAYLIST.len() as u32),
            );
        }
    }

    fn button_state(&self, index: usize) -> ButtonState {
        if self.pressed_control == Some(index) {
            ButtonState::Pressed
        } else if self.hovered_control == Some(index) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        }
    }

    fn draw_header(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.layout.header, theme.chrome.titlebar_active);
        canvas.hbar(
            self.layout.header.x,
            self.layout.header.bottom() - 1,
            self.layout.header.w,
            1,
            theme.chrome.titlebar_divider_active,
        );
        IconButton::new(self.layout.header_open, UiSymbol::Folder)
            .with_state(self.button_state(0))
            .with_focus(self.focus_index == 0 && self.window_focused)
            .draw(canvas, theme);
        IconButton::new(self.layout.header_more, UiSymbol::MoreHorizontal)
            .with_state(self.button_state(1))
            .with_focus(self.focus_index == 1 && self.window_focused)
            .draw(canvas, theme);

        let title_left = self.layout.header_open.right() + 10;
        let title_right = self.layout.header_more.x - 10;
        let title_rect = Rect::new(
            title_left,
            self.layout.header.y,
            (title_right - title_left).max(0) as u32,
            self.layout.header.h,
        );
        canvas.draw_ui_symbol(
            title_rect.x,
            title_rect.y + (title_rect.h as i32 - 9) / 2,
            UiSymbol::Music,
            theme.accent,
        );
        draw_text_vcenter(
            canvas,
            "Melody Mina",
            title_rect.x + 17,
            title_rect.y,
            title_rect.h,
            &TextStyle::new(FontRole::UiBold, theme.text),
        );
        if title_rect.w > 330 {
            draw_text_right(
                canvas,
                title_rect,
                self.status,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                0,
            );
        }
    }

    fn draw_metadata(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.layout.metadata;
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        let mut title_buf = [0u8; 96];
        let title = elide(
            self.now_playing.title,
            FontRole::UiTitle,
            rect.w,
            &mut title_buf,
        );
        draw_text(
            canvas,
            title,
            rect.x,
            rect.y + 2,
            &TextStyle::new(FontRole::UiTitle, theme.text),
        );
        let mut artist_buf = [0u8; 96];
        let artist = elide(
            self.now_playing.artist,
            FontRole::UiMedium,
            rect.w,
            &mut artist_buf,
        );
        draw_text(
            canvas,
            artist,
            rect.x,
            rect.y + 28,
            &TextStyle::new(FontRole::UiMedium, theme.accent_hover),
        );
        if rect.h >= 58 {
            let mut album_buf = [0u8; 96];
            let album = elide(
                self.now_playing.album,
                FontRole::UiSmall,
                rect.w,
                &mut album_buf,
            );
            draw_text(
                canvas,
                album,
                rect.x,
                rect.y + 49,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }
        if rect.h >= 82 {
            draw_text(
                canvas,
                "Demo presentation state",
                rect.x,
                rect.y + 68,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
        }
    }

    fn draw_timeline(&self, canvas: &mut Canvas, theme: &Theme) {
        let mut elapsed = [0u8; 8];
        let mut duration = [0u8; 8];
        draw_text_vcenter(
            canvas,
            time_text(self.now_playing.elapsed_seconds, &mut elapsed),
            self.layout.timeline.x,
            self.layout.timeline.y,
            self.layout.timeline.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        draw_text_right(
            canvas,
            Rect::new(
                self.layout.timeline.right() - 44,
                self.layout.timeline.y,
                44,
                self.layout.timeline.h,
            ),
            time_text(self.now_playing.duration_seconds, &mut duration),
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            0,
        );
        self.timeline.draw(canvas, theme);
        if self.focus_index == 8 && self.window_focused {
            canvas.draw_rect(self.layout.timeline_slider, theme.accent_hover);
        }
    }

    fn draw_transport(&self, canvas: &mut Canvas, theme: &Theme) {
        let symbols = [
            UiSymbol::Shuffle,
            UiSymbol::PreviousTrack,
            if self.now_playing.playback_state == PlaybackState::PlayingPresentation {
                UiSymbol::Pause
            } else {
                UiSymbol::Play
            },
            UiSymbol::NextTrack,
            UiSymbol::Repeat,
        ];
        for (index, symbol) in symbols.iter().enumerate() {
            IconButton::new(self.layout.transport_buttons[index], *symbol)
                .primary(index == 2)
                .with_state(self.button_state(index + 2))
                .with_focus(self.focus_index == index + 2 && self.window_focused)
                .draw(canvas, theme);
        }
        canvas.draw_ui_symbol_centered(self.layout.volume_icon, UiSymbol::Volume, theme.icon_muted);
        self.volume.draw(canvas, theme);
        if self.focus_index == 9 && self.window_focused {
            canvas.draw_rect(self.layout.volume_slider, theme.accent_hover);
        }
    }

    fn hit_control(&self, point: Point) -> Option<usize> {
        if self.layout.header_open.contains(point) {
            return Some(0);
        }
        if self.layout.header_more.contains(point) {
            return Some(1);
        }
        for (index, rect) in self.layout.transport_buttons.iter().enumerate() {
            if rect.contains(point) {
                return Some(index + 2);
            }
        }
        None
    }

    fn playlist_item_at(&self, point: Point) -> Option<usize> {
        if !self.layout.playlist.contains(point) {
            return None;
        }
        if hit_test_scrollbar(
            self.layout.playlist.inset(2),
            &self.playlist_scroll,
            point.x,
            point.y,
        )
        .is_some()
        {
            return None;
        }
        let local = point.y - self.layout.playlist.y - 2 + self.playlist_scroll.offset_y;
        if local < 0 {
            return None;
        }
        let index = local as usize / PLAYLIST_ROW_H as usize;
        (index < DEMO_PLAYLIST.len()).then_some(index)
    }

    fn activate_control(&mut self, index: usize) -> bool {
        match index {
            0 => self.status = "Open media arrives with the future media backend",
            1 => self.status = "Options placeholder / Phase 1",
            2 => self.status = "Shuffle is a presentation-only control",
            3 => self.status = "Previous is unavailable without a media backend",
            4 => {
                self.now_playing.playback_state = match self.now_playing.playback_state {
                    PlaybackState::Paused => PlaybackState::PlayingPresentation,
                    PlaybackState::PlayingPresentation => PlaybackState::Paused,
                };
                self.status = "Visual state only / no audio is playing";
            }
            5 => self.status = "Next is unavailable without a media backend",
            6 => self.status = "Repeat is a presentation-only control",
            _ => return false,
        }
        true
    }

    fn keyboard_activate(&mut self) -> bool {
        if self.focus_index <= 6 {
            self.activate_control(self.focus_index)
        } else if self.focus_index == 7 {
            self.playlist_scroll.focused = true;
            true
        } else {
            false
        }
    }

    fn move_focus(&mut self, reverse: bool) {
        self.focus_index = if reverse {
            (self.focus_index + CONTROL_COUNT - 1) % CONTROL_COUNT
        } else {
            (self.focus_index + 1) % CONTROL_COUNT
        };
        self.playlist_scroll.focused = self.focus_index == 7;
    }

    fn update_visualizer(&mut self) -> bool {
        let now = monotonic_millis();
        let interval = if self.window_focused {
            FRAME_MS_FOCUSED
        } else {
            FRAME_MS_UNFOCUSED
        };
        if now.saturating_sub(self.last_visualization_ms) < interval {
            return false;
        }
        self.last_visualization_ms = now;
        let bins = VisualizerView::desired_bins(self.layout.visualizer.w.saturating_sub(16));
        self.visualization_frame = self.visualizer_source.next_frame(bins);
        true
    }
}

impl App for MelodyMinaApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.ensure_layout();
        canvas.fill_rect(
            Rect::new(0, 0, canvas.width, canvas.height),
            theme.chrome.window_bg,
        );
        self.draw_header(canvas, theme);
        AlbumArtView::new(self.layout.album_art, self.artwork).draw(canvas, theme);
        self.draw_metadata(canvas, theme);
        PlaylistView {
            rect: self.layout.playlist,
            items: &DEMO_PLAYLIST,
            selected: self.selected,
            hovered: self.hovered_item,
            focused: self.focus_index == 7 && self.window_focused,
            scroll: &self.playlist_scroll,
        }
        .draw(canvas, theme);
        VisualizerView {
            rect: self.layout.visualizer,
            frame: self.visualization_frame,
        }
        .draw(canvas, theme);
        self.draw_timeline(canvas, theme);
        self.draw_transport(canvas, theme);

        if self.layout.mode == LayoutMode::Narrow && self.layout.header.w <= 330 {
            draw_text_centered(
                canvas,
                Rect::new(
                    self.layout.header.x + 52,
                    self.layout.header.y,
                    self.layout.header.w.saturating_sub(104),
                    self.layout.header.h,
                ),
                "Melody Mina",
                &TextStyle::new(FontRole::UiBold, theme.text),
            );
        }
    }

    fn update(&mut self, event: Event) -> bool {
        self.ensure_layout();
        match event {
            Event::Tick => self.update_visualizer(),
            Event::FocusChanged { focused } => {
                self.window_focused = focused;
                self.pressed_control = None;
                true
            }
            Event::PointerOwnership {
                owned: false,
                captured: false,
            } => {
                let changed =
                    self.hovered_control.take().is_some() || self.hovered_item.take().is_some();
                self.playlist_scroll.hovered = false;
                changed
            }
            Event::MouseMove { x, y } => {
                let point = Point::new(x, y);
                let old_control = self.hovered_control;
                let old_item = self.hovered_item;
                self.hovered_control = self.hit_control(point);
                self.hovered_item = self.playlist_item_at(point);
                let track = self
                    .playlist_scroll
                    .track_rect(self.layout.playlist.inset(2));
                let old_scroll_hover = self.playlist_scroll.hovered;
                self.playlist_scroll.hovered = track.contains(point);
                let dragged = self.playlist_scroll.update_drag(track, y);
                let timeline_changed = self.timeline.update(event);
                let volume_changed = self.volume.update(event);
                if self.hovered_control.is_some()
                    || self.hovered_item.is_some()
                    || self.timeline.active
                    || self.volume.active
                {
                    set_client_cursor(CursorShape::Hand);
                } else {
                    set_client_cursor(CursorShape::Pointer);
                }
                old_control != self.hovered_control
                    || old_item != self.hovered_item
                    || old_scroll_hover != self.playlist_scroll.hovered
                    || dragged
                    || timeline_changed
                    || volume_changed
            }
            Event::MouseDown { x, y, button: 0 } => {
                let point = Point::new(x, y);
                if let Some(index) = self.hit_control(point) {
                    self.pressed_control = Some(index);
                    self.focus_index = index;
                    return true;
                }
                if let Some(hit_thumb) =
                    hit_test_scrollbar(self.layout.playlist.inset(2), &self.playlist_scroll, x, y)
                {
                    let track = self
                        .playlist_scroll
                        .track_rect(self.layout.playlist.inset(2));
                    if hit_thumb {
                        self.playlist_scroll.start_drag(track, y);
                    } else {
                        self.playlist_scroll.handle_track_click(track, y);
                    }
                    self.focus_index = 7;
                    return true;
                }
                if let Some(index) = self.playlist_item_at(point) {
                    self.selected = index;
                    self.focus_index = 7;
                    self.status = "Queue selection is demo presentation state";
                    return true;
                }
                let timeline_changed = self.timeline.update(event);
                let volume_changed = self.volume.update(event);
                if self.layout.timeline_slider.contains(point) {
                    self.focus_index = 8;
                }
                if self.layout.volume_slider.contains(point) {
                    self.focus_index = 9;
                }
                timeline_changed || volume_changed
            }
            Event::Click { x, y } => {
                let point = Point::new(x, y);
                let activated = self
                    .pressed_control
                    .take()
                    .filter(|index| self.hit_control(point) == Some(*index))
                    .map(|index| self.activate_control(index))
                    .unwrap_or(false);
                self.playlist_scroll.end_drag();
                let timeline_changed = self.timeline.update(event);
                if timeline_changed {
                    self.now_playing.elapsed_seconds = self
                        .now_playing
                        .duration_seconds
                        .saturating_mul(self.timeline.value)
                        / 100;
                    self.status = "Timeline preview only / media was not seeked";
                }
                let volume_changed = self.volume.update(event);
                if volume_changed {
                    self.status = "Volume preview only / system audio unchanged";
                }
                activated || timeline_changed || volume_changed || self.hovered_control.is_some()
            }
            Event::MouseWheel { x, y, delta } => {
                if self.layout.playlist.contains(Point::new(x, y)) {
                    self.focus_index = 7;
                    self.playlist_scroll.focused = true;
                    return self
                        .playlist_scroll
                        .scroll_by_wheel(delta, PLAYLIST_ROW_H as i32);
                }
                false
            }
            Event::KeyPress {
                keycode: KEY_ESC,
                pressed: true,
                ..
            } => {
                request_close();
                true
            }
            Event::KeyPress {
                keycode: KEY_TAB,
                pressed: true,
                shift,
                ..
            } => {
                self.move_focus(shift);
                true
            }
            Event::KeyPress {
                keycode: KEY_ENTER | KEY_SPACE,
                pressed: true,
                ..
            } => self.keyboard_activate(),
            Event::KeyPress {
                keycode: 0x48,
                pressed: true,
                ..
            } if self.focus_index == 7 => self.playlist_scroll.scroll_by(-(PLAYLIST_ROW_H as i32)),
            Event::KeyPress {
                keycode: 0x50,
                pressed: true,
                ..
            } if self.focus_index == 7 => self.playlist_scroll.scroll_by(PLAYLIST_ROW_H as i32),
            _ => false,
        }
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        match event {
            WindowEvent::Resized { width, height } => {
                self.client = Size::new(width, height);
                self.layout_invalidation.invalidate();
                true
            }
        }
    }

    fn poll_timeout_ms(&self) -> u64 {
        if self.window_focused {
            FRAME_MS_FOCUSED
        } else {
            FRAME_MS_UNFOCUSED
        }
    }
}

fn elide<'a>(text: &str, role: FontRole, max_w: u32, output: &'a mut [u8]) -> &'a str {
    if measure_text(text, role).w <= max_w {
        let len = text.len().min(output.len());
        output[..len].copy_from_slice(&text.as_bytes()[..len]);
        return core::str::from_utf8(&output[..len]).unwrap_or("");
    }
    let dots = "...";
    let budget = max_w.saturating_sub(measure_text(dots, role).w);
    let mut end = 0usize;
    for (index, ch) in text.char_indices() {
        let next = index + ch.len_utf8();
        if next + dots.len() > output.len() || measure_text(&text[..next], role).w > budget {
            break;
        }
        end = next;
    }
    output[..end].copy_from_slice(&text.as_bytes()[..end]);
    output[end..end + dots.len()].copy_from_slice(dots.as_bytes());
    core::str::from_utf8(&output[..end + dots.len()]).unwrap_or("...")
}

fn time_text(seconds: u32, output: &mut [u8; 8]) -> &str {
    let minutes = (seconds / 60).min(999);
    let secs = seconds % 60;
    let mut index = 0usize;
    if minutes >= 100 {
        output[index] = b'0' + (minutes / 100) as u8;
        index += 1;
    }
    if minutes >= 10 {
        output[index] = b'0' + ((minutes / 10) % 10) as u8;
        index += 1;
    }
    output[index] = b'0' + (minutes % 10) as u8;
    index += 1;
    output[index] = b':';
    index += 1;
    output[index] = b'0' + (secs / 10) as u8;
    index += 1;
    output[index] = b'0' + (secs % 10) as u8;
    index += 1;
    core::str::from_utf8(&output[..index]).unwrap_or("0:00")
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=melody-mina",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let mut app = MelodyMinaApp::new();
    let Some(mut window) = Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Melody Mina",
        decoration: WindowDecoration::Normal,
    }) else {
        debug_log("[MELODY-MINA] failed to connect window\n");
        ProcessExit::exit(1)
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_geometry_preserves_source_ratio_and_crops_centered() {
        let viewport = Rect::new(10, 20, 200, 200);
        let cover = AlbumArtView::cover_rect(viewport, Size::new(400, 200)).unwrap();
        assert_eq!((cover.w, cover.h), (400, 200));
        assert_eq!(cover.x, -90);
        assert_eq!(cover.y, 20);
    }

    #[test]
    fn time_formatting_is_allocation_free_and_stable() {
        let mut buf = [0u8; 8];
        assert_eq!(time_text(42, &mut buf), "0:42");
        assert_eq!(time_text(207, &mut buf), "3:27");
    }
}
