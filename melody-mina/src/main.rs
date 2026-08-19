#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec};

use melody_mina::{
    controller::MelodyMediaController,
    layout::{LayoutMode, MelodyLayout},
    model::{format_time, seek_target_ms, timeline_percent, PlaybackState},
    visualizer::VisualizationFrame,
};
use sun_font::{
    draw_text, draw_text_centered, draw_text_right, draw_text_vcenter, measure_text, FontRole,
    TextStyle,
};
use sunlight_dialogs::{DialogClient, DialogRequest, DialogResult, OpenFileRequest};
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
    App, Canvas, CursorShape, Event, LayoutInvalidation, Point, Rect, ScrollPolicy, ScrollState,
    Size, Theme, UiSymbol, Window, WindowConfig, WindowDecoration, WindowEvent,
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

fn debug_log_u64(mut value: u64) {
    let mut reversed = [0u8; 20];
    let mut len = 0usize;
    if value == 0 {
        debug_log("0");
        return;
    }
    while value != 0 {
        reversed[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    let mut output = [0u8; 20];
    for index in 0..len {
        output[index] = reversed[len - index - 1];
    }
    if let Ok(text) = core::str::from_utf8(&output[..len]) {
        debug_log(text);
    }
}

fn log_media_heap(stage: &str) {
    let heap = sunlight_libc::alloc::heap_stats();
    debug_log("[MELODY-MINA][media-heap] stage=");
    debug_log(stage);
    debug_log(" requested=");
    debug_log_u64(heap.requested_user_bytes as u64);
    debug_log(" live=");
    debug_log_u64(heap.live_allocation_count);
    debug_log(" high_water=");
    debug_log_u64(heap.high_water_allocated_bytes as u64);
    debug_log(" failed=");
    debug_log_u64(heap.failed_allocation_count);
    debug_log("\n");
}

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
    items: &'a [PlaylistItemViewModel<'a>],
    selected: usize,
    hovered: Option<usize>,
    focused: bool,
    scroll: &'a ScrollState,
}

#[derive(Clone, Copy)]
struct PlaylistItemViewModel<'a> {
    title: &'a str,
    artist: Option<&'a str>,
    duration_seconds: Option<u64>,
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
                let mut time_buf = [0u8; 24];
                draw_text_right(
                    &mut clip,
                    Rect::new(0, y, local_w.saturating_sub(24), PLAYLIST_ROW_H),
                    format_time(seconds, &mut time_buf),
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
    media: MelodyMediaController,
    last_media_state: PlaybackState,
    playback_seen: bool,
    visualization_frame: VisualizationFrame,
    last_visualization_ms: u64,
    status: &'static str,
    artwork: Option<TgaImage>,
    track_title: [u8; 128],
    track_title_len: usize,
    has_active_source: bool,
    seek_committed_on_release: bool,
}

impl MelodyMinaApp {
    fn new() -> Self {
        let artwork = TgaImage::parse(PLACEHOLDER_ART_BYTES).ok();
        let media = MelodyMediaController::new();
        let now_playing = media.view();
        let app = Self {
            playlist_scroll: ScrollState::new(),
            timeline: Slider::horizontal(Rect::default())
                .with_range(0, 100)
                .with_value(timeline_percent(&now_playing)),
            volume: Slider::horizontal(Rect::default())
                .with_range(0, 100)
                .with_value(now_playing.volume as u32),
            layout: MelodyLayout::empty(),
            layout_invalidation: LayoutInvalidation::new(),
            client: Size::new(WIN_W, WIN_H),
            hovered_control: None,
            pressed_control: None,
            focus_index: 4,
            window_focused: true,
            media,
            last_media_state: PlaybackState::Idle,
            playback_seen: false,
            visualization_frame: VisualizationFrame::empty(),
            last_visualization_ms: 0,
            status: "Open an Ogg Vorbis file to begin",
            artwork,
            track_title: [0; 128],
            track_title_len: 0,
            has_active_source: false,
            seek_committed_on_release: false,
        };
        log_media_heap("before");
        app
    }

    fn track_title(&self) -> &str {
        if self.track_title_len == 0 {
            "No media loaded"
        } else {
            core::str::from_utf8(&self.track_title[..self.track_title_len])
                .unwrap_or("Selected audio")
        }
    }

    fn set_track_title(&mut self, path: &str) {
        let filename = path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(path);
        let name = filename
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .filter(|stem| !stem.is_empty())
            .unwrap_or(filename);
        self.track_title_len = name.len().min(self.track_title.len());
        while !name.is_char_boundary(self.track_title_len) {
            self.track_title_len -= 1;
        }
        self.track_title[..self.track_title_len]
            .copy_from_slice(&name.as_bytes()[..self.track_title_len]);
    }

    fn open_media(&mut self) {
        let request = DialogRequest::OpenFile(OpenFileRequest {
            title: String::from("Open Ogg Vorbis Audio"),
            initial_dir: Some(String::from("/home/user/Music")),
            allowed_mime_types: vec![String::from("audio/ogg")],
            allowed_extensions: vec![String::from("ogg")],
            allow_multiple: false,
            show_preview: false,
            confirm_button_label: Some(String::from("Open")),
        });
        match DialogClient::new().show(&request) {
            Ok(DialogResult::FileSelected(path)) => self.load_media_path(&path),
            Ok(DialogResult::Cancelled | DialogResult::Cancel | DialogResult::Dismissed) => {
                self.status = "Open cancelled";
            }
            Ok(_) => self.status = "No audio file was selected",
            Err(_) => self.status = "File picker unavailable",
        }
    }

    fn load_media_path(&mut self, path: &str) {
        match self.media.open(path) {
            Ok(()) => {
                self.set_track_title(path);
                self.has_active_source = true;
                self.status = "Loading Ogg Vorbis audio...";
            }
            Err(error) => self.status = error.kind.user_message(),
        }
    }

    fn sync_media(&mut self) -> bool {
        let changed = self.media.refresh();
        let now_playing = self.media.view();
        let previous_state = self.last_media_state;
        let state_changed = now_playing.playback_state != self.last_media_state;
        self.last_media_state = now_playing.playback_state;
        if state_changed && now_playing.playback_state == PlaybackState::Playing {
            self.playback_seen = true;
            log_media_heap("during");
        } else if state_changed
            && now_playing.playback_state == PlaybackState::Ready
            && self.playback_seen
            && previous_state != PlaybackState::Loading
        {
            log_media_heap("after-stop");
        }
        if !self.media.interaction().seek_drag_active {
            self.timeline.value = timeline_percent(&now_playing);
        }
        if !self.volume.dragging {
            self.volume.value = now_playing.volume as u32;
        }
        let visualization_changed = if now_playing.playback_state == PlaybackState::Playing {
            let frame = self.media.visualization();
            let old = self.visualization_frame;
            self.visualization_frame.set_len(frame.bins().len());
            for (index, value) in frame.bins().iter().enumerate() {
                self.visualization_frame.set_bin(index, *value);
            }
            old != self.visualization_frame
        } else {
            self.visualization_frame.decay(8)
        };
        if state_changed && now_playing.playback_state == PlaybackState::Error {
            if let Some(error) = now_playing.error {
                debug_log("[MELODY-MINA][media-error] kind=");
                debug_log_u64(error.kind as u64);
                debug_log(" detail=");
                debug_log_u64(error.detail as u64);
                debug_log("\n");
            }
        }
        self.status = if let Some(error) = now_playing.error {
            error.kind.user_message()
        } else {
            match now_playing.playback_state {
                PlaybackState::Idle => "Open an Ogg Vorbis file to begin",
                PlaybackState::Loading => "Loading Ogg Vorbis audio...",
                PlaybackState::Ready => "Ready",
                PlaybackState::Playing => "Playing through Sunlight Media",
                PlaybackState::Paused => "Paused",
                PlaybackState::Ended => "Playback ended",
                PlaybackState::Error => "Playback failed",
            }
        };
        changed || state_changed || visualization_changed
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
                PLAYLIST_ROW_H,
            );
        }
    }

    fn button_state(&self, index: usize) -> ButtonState {
        if !self.control_enabled(index) {
            ButtonState::Disabled
        } else if self.pressed_control == Some(index) {
            ButtonState::Pressed
        } else if self.hovered_control == Some(index) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        }
    }

    fn control_enabled(&self, index: usize) -> bool {
        let controls = self.media.view().controls();
        match index {
            0 => controls.open,
            2 => controls.stop,
            4 => controls.play_pause,
            // Options, Previous, Next, and Repeat remain visible to preserve
            // the accepted layout but this single-file phase does not own a
            // playlist/sequencing engine.
            1 | 3 | 5 | 6 => false,
            _ => false,
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
            self.track_title(),
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
            if self.has_active_source {
                "Unknown Artist"
            } else {
                "Choose a local audio file"
            },
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
                if self.has_active_source {
                    "Local media"
                } else {
                    ""
                },
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
                "Decoded and timed by the reusable media backend",
                rect.x,
                rect.y + 68,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
        }
    }

    fn draw_timeline(&self, canvas: &mut Canvas, theme: &Theme) {
        let view = self.media.view();
        let interaction = self.media.interaction();
        let displayed_ms = if interaction.seek_drag_active {
            seek_target_ms(&view, interaction.seek_preview_percent).unwrap_or(view.position_ms)
        } else {
            view.position_ms
        };
        let mut elapsed = [0u8; 24];
        let mut duration = [0u8; 24];
        draw_text_vcenter(
            canvas,
            format_time(displayed_ms / 1_000, &mut elapsed),
            self.layout.timeline.x,
            self.layout.timeline.y,
            self.layout.timeline.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        draw_text_right(
            canvas,
            Rect::new(
                self.layout.timeline_slider.right() + 6,
                self.layout.timeline.y,
                (self.layout.timeline.right() - self.layout.timeline_slider.right() - 6).max(0)
                    as u32,
                self.layout.timeline.h,
            ),
            view.duration_ms
                .map(|value| format_time(value / 1_000, &mut duration))
                .unwrap_or("--:--"),
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            0,
        );
        self.timeline.draw(canvas, theme);
        if self.focus_index == 8 && self.window_focused {
            canvas.draw_rect(self.layout.timeline_slider, theme.accent_hover);
        }
    }

    fn draw_transport(&self, canvas: &mut Canvas, theme: &Theme) {
        let now_playing = self.media.view();
        let symbols = [
            UiSymbol::Stop,
            UiSymbol::PreviousTrack,
            if now_playing.shows_pause() {
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
        if self.control_enabled(0) && self.layout.header_open.contains(point) {
            return Some(0);
        }
        if self.control_enabled(1) && self.layout.header_more.contains(point) {
            return Some(1);
        }
        for (index, rect) in self.layout.transport_buttons.iter().enumerate() {
            if self.control_enabled(index + 2) && rect.contains(point) {
                return Some(index + 2);
            }
        }
        None
    }

    fn activate_control(&mut self, index: usize) -> bool {
        if !self.control_enabled(index) {
            return false;
        }
        match index {
            0 => self.open_media(),
            2 => match self.media.stop() {
                Ok(()) => self.status = "Stopping playback...",
                Err(error) => self.status = error.kind.user_message(),
            },
            4 => {
                if let Err(error) = self.media.play_pause() {
                    self.status = error.kind.user_message();
                }
            }
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
        self.sync_media()
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
        let now_playing = self.media.view();
        let active_item = [PlaylistItemViewModel {
            title: self.track_title(),
            artist: self.has_active_source.then_some("Unknown Artist"),
            duration_seconds: now_playing.duration_ms.map(|value| value / 1_000),
        }];
        PlaylistView {
            rect: self.layout.playlist,
            items: &active_item,
            selected: 0,
            hovered: None,
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
                let changed = self.hovered_control.take().is_some();
                self.playlist_scroll.hovered = false;
                self.media.cancel_seek();
                self.timeline.dragging = false;
                self.timeline.active = false;
                self.volume.dragging = false;
                changed
            }
            Event::MouseMove { x, y } => {
                let point = Point::new(x, y);
                let old_control = self.hovered_control;
                self.hovered_control = self.hit_control(point);
                let track = self
                    .playlist_scroll
                    .track_rect(self.layout.playlist.inset(2));
                let old_scroll_hover = self.playlist_scroll.hovered;
                self.playlist_scroll.hovered = track.contains(point);
                let dragged = self.playlist_scroll.update_drag(track, y);
                let timeline_changed =
                    if self.media.seek_enabled() || self.media.interaction().seek_drag_active {
                        self.timeline.update(event)
                    } else {
                        false
                    };
                if timeline_changed {
                    self.media.preview_seek(self.timeline.value);
                }
                let volume_changed = self.volume.update(event);
                if self.hovered_control.is_some() || self.timeline.active || self.volume.active {
                    set_client_cursor(CursorShape::Hand);
                } else {
                    set_client_cursor(CursorShape::Pointer);
                }
                old_control != self.hovered_control
                    || old_scroll_hover != self.playlist_scroll.hovered
                    || dragged
                    || timeline_changed
                    || volume_changed
            }
            Event::MouseDown { x, y, button: 0 } => {
                self.seek_committed_on_release = false;
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
                let timeline_changed = if self.media.seek_enabled() {
                    let changed = self.timeline.update(event);
                    if self.layout.timeline_slider.contains(point) {
                        let _ = self.media.begin_seek(self.timeline.value);
                    }
                    changed
                } else {
                    false
                };
                let volume_changed = self.volume.update(event);
                if self.layout.timeline_slider.contains(point) {
                    self.focus_index = 8;
                }
                if self.layout.volume_slider.contains(point) {
                    self.focus_index = 9;
                }
                timeline_changed || volume_changed
            }
            Event::MouseUp { .. } => {
                let timeline_changed = if self.media.interaction().seek_drag_active {
                    let changed = self.timeline.update(event);
                    match self.media.commit_seek(self.timeline.value) {
                        Ok(()) => self.status = "Seeking...",
                        Err(error) => self.status = error.kind.user_message(),
                    }
                    self.seek_committed_on_release = true;
                    let _ = changed;
                    true
                } else {
                    false
                };
                let was_volume_dragging = self.volume.dragging;
                let volume_changed = self.volume.update(event);
                if was_volume_dragging {
                    match self.media.set_volume(self.volume.value) {
                        Ok(()) => self.status = "Stream volume changed",
                        Err(error) => self.status = error.kind.user_message(),
                    }
                }
                timeline_changed || volume_changed || was_volume_dragging
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
                let suppress_seek = core::mem::replace(&mut self.seek_committed_on_release, false);
                let timeline_changed = if self.media.seek_enabled() && !suppress_seek {
                    self.timeline.update(event)
                } else {
                    false
                };
                if timeline_changed && !self.media.interaction().seek_drag_active {
                    let _ = self.media.begin_seek(self.timeline.value);
                    match self.media.commit_seek(self.timeline.value) {
                        Ok(()) => self.status = "Seeking...",
                        Err(error) => self.status = error.kind.user_message(),
                    }
                }
                let volume_changed = self.volume.update(event);
                if volume_changed {
                    match self.media.set_volume(self.volume.value) {
                        Ok(()) => self.status = "Stream volume changed",
                        Err(error) => self.status = error.kind.user_message(),
                    }
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
        match self.media.view().playback_state {
            PlaybackState::Playing if self.window_focused => FRAME_MS_FOCUSED,
            PlaybackState::Playing | PlaybackState::Loading => FRAME_MS_UNFOCUSED,
            _ => 250,
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

fn media_path_arg(argc: u64, argv: *const *const u8) -> Option<String> {
    let mut raw = [core::ptr::null(); 8];
    let count = unsafe { sunlight_libc::crt0::collect_raw_args(argc, argv, &mut raw) };
    if count < 2 || raw[1].is_null() {
        return None;
    }
    let len = unsafe { sunlight_libc::crt0::cstr_len(raw[1], 4096) };
    let bytes = unsafe { core::slice::from_raw_parts(raw[1], len) };
    core::str::from_utf8(bytes).ok().map(String::from)
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
    if let Some(path) = media_path_arg(argc, argv) {
        app.load_media_path(&path);
    }
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
    drop(app);
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
}
