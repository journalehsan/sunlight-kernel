#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec, vec::Vec};

use melody_mina::{
    controller::MelodyMediaController,
    layout::{LayoutMode, MelodyLayout, MelodyView},
    library::{
        builtin_entry, read_metadata_bytes, scan_music_directory, supported_extension, MediaEntry,
        MediaSource,
    },
    metadata,
    model::{format_time, next_playlist_index, seek_target_ms, timeline_percent, PlaybackState},
    visualizer::VisualizationFrame,
};
use sun_font::{
    draw_text, draw_text_centered, draw_text_right, draw_text_vcenter, measure_text, FontRole,
    TextStyle,
};
use sunlight_dialogs::{
    DialogClient, DialogRequest, DialogResult, OpenFileRequest, OpenFolderRequest,
};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, process_yield, ProcessExit,
};
use sunlight_ui::{
    draw_scrollbar, hit_test_scrollbar,
    image::{decode_image, RgbaImage},
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
const CONTROL_COUNT: usize = 13;
const FRAME_MS_FOCUSED: u64 = 33;
const FRAME_MS_UNFOCUSED: u64 = 100;

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

struct AlbumArtView<'a> {
    rect: Rect,
    image: Option<&'a RgbaImage>,
}

impl<'a> AlbumArtView<'a> {
    fn new(rect: Rect, image: Option<&'a RgbaImage>) -> Self {
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
                clipped.draw_rgba_image(image, dst);
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
                "Unknown",
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }
    }
}

struct PlaylistView<'a> {
    rect: Rect,
    items: &'a [PlaylistItemViewModel<'a>],
    selected: Option<usize>,
    playing: Option<usize>,
    collection: bool,
    hovered: Option<usize>,
    scroll: &'a ScrollState,
}

#[derive(Clone, Copy)]
struct PlaylistItemViewModel<'a> {
    title: &'a str,
    artist: Option<&'a str>,
    duration_seconds: Option<u64>,
    artwork: Option<&'a RgbaImage>,
}

impl<'a> PlaylistView<'a> {
    fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rounded_rect_with_border(
            self.rect,
            10,
            theme.chrome.card_bg,
            theme.border,
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
            let selected = Some(index) == self.selected;
            let playing = Some(index) == self.playing;
            if selected {
                clip.fill_rounded_rect(row.inset(3), 7, theme.chrome.selection);
            } else if self.hovered == Some(index) {
                clip.fill_rounded_rect(row.inset(3), 7, theme.panel_alt);
            }
            if playing {
                clip.fill_rounded_rect(
                    Rect::new(3, y + 7, 3, PLAYLIST_ROW_H.saturating_sub(14)),
                    2,
                    theme.accent,
                );
            }

            let text_x = if self.collection {
                let cover = Rect::new(9, y + 7, 38, 38);
                if item.artwork.is_some() {
                    AlbumArtView::new(cover, item.artwork).draw(&mut clip, theme);
                } else {
                    clip.draw_ui_symbol_centered(
                        cover.inset(8),
                        UiSymbol::Music,
                        theme.icon_muted,
                    );
                }
                56
            } else if playing {
                clip.draw_ui_symbol_centered(
                    Rect::new(9, y + 17, 14, 18),
                    UiSymbol::Play,
                    theme.accent,
                );
                30
            } else {
                14
            };
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
                    if playing || selected {
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
                if self.collection {
                    UiSymbol::Forward
                } else {
                    UiSymbol::MoreHorizontal
                },
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

/// Groups refer to the existing queue; browsing never replaces its entries.
struct CollectionGroup {
    title: String,
    subtitle: String,
    tracks: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepeatMode {
    Off,
    All,
    One,
}

fn collection_key(entry: &MediaEntry) -> (&str, Option<&str>) {
    let folder = entry
        .path
        .rsplit_once('/')
        .map(|(folder, _)| folder)
        .unwrap_or("");
    match entry.album.as_deref().filter(|album| !album.is_empty()) {
        Some(album) => (folder, Some(album)),
        None => (folder, None),
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
    playlist: alloc::vec::Vec<MediaEntry>,
    active_queue: Vec<usize>,
    view: MelodyView,
    collections: Vec<CollectionGroup>,
    selected_collection: usize,
    playlist_collection: usize,
    active_playlist: Option<usize>,
    active_queue_pos: Option<usize>,
    shuffle_remaining: Vec<usize>,
    shuffle_history: Vec<usize>,
    shuffle: bool,
    // The visible list owns this scroll state. Navigation swaps it with the
    // other view's saved state, retaining both offsets across view changes.
    playlist_scroll: ScrollState,
    other_view_scroll: ScrollState,
    selected_playlist: usize,
    hovered_playlist: Option<usize>,
    pressed_playlist: Option<usize>,
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
    artwork: Option<RgbaImage>,
    track_title: String,
    track_artist: String,
    track_album: String,
    has_active_source: bool,
    repeat_mode: RepeatMode,
    seek_committed_on_release: bool,
}

impl MelodyMinaApp {
    fn new() -> Self {
        let media = MelodyMediaController::new();
        let now_playing = media.view();
        let mut playlist = vec![builtin_entry()];
        let music = music_directory();
        for entry in scan_music_directory(&music) {
            if !playlist.iter().any(|existing| existing.path == entry.path) {
                playlist.push(entry);
            }
        }
        let mut app = Self {
            playlist,
            active_queue: Vec::new(),
            view: MelodyView::Collection,
            collections: Vec::new(),
            selected_collection: 0,
            playlist_collection: 0,
            active_playlist: None,
            active_queue_pos: None,
            shuffle_remaining: Vec::new(),
            shuffle_history: Vec::new(),
            shuffle: false,
            playlist_scroll: ScrollState::new(),
            other_view_scroll: ScrollState::new(),
            selected_playlist: 0,
            hovered_playlist: None,
            pressed_playlist: None,
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
            status: "Choose a collection to begin",
            artwork: None,
            track_title: String::from("No track selected"),
            track_artist: String::new(),
            track_album: String::new(),
            has_active_source: false,
            repeat_mode: RepeatMode::Off,
            seek_committed_on_release: false,
        };
        app.rebuild_collections();
        log_media_heap("before");
        app
    }

    fn track_title(&self) -> &str {
        &self.track_title
    }

    fn rebuild_collections(&mut self) {
        // Discovery only appends entries, so group order and selection stay stable.
        let mut groups = vec![CollectionGroup {
            title: String::from("All Tracks"),
            subtitle: format!("{} tracks", self.playlist.len()),
            tracks: (0..self.playlist.len()).collect(),
        }];
        for (index, entry) in self.playlist.iter().enumerate() {
            let key = collection_key(entry);
            if let Some(group) = groups
                .iter_mut()
                .skip(1)
                .find(|group| collection_key(&self.playlist[group.tracks[0]]) == key)
            {
                group.tracks.push(index);
            } else {
                let title = key.1.unwrap_or_else(|| {
                    key.0
                        .rsplit('/')
                        .find(|part| !part.is_empty())
                        .unwrap_or("Music")
                });
                groups.push(CollectionGroup {
                    title: String::from(title),
                    subtitle: String::new(),
                    tracks: vec![index],
                });
            }
        }
        for group in groups.iter_mut().skip(1) {
            let folder = collection_key(&self.playlist[group.tracks[0]]).0;
            group.subtitle = format!("{} tracks · {}", group.tracks.len(), folder);
        }
        self.collections = groups;
    }

    fn switch_view(&mut self, view: MelodyView) {
        if self.view == view {
            return;
        }
        self.playlist_scroll.end_drag();
        self.playlist_scroll.hovered = false;
        self.playlist_scroll.focused = false;
        core::mem::swap(&mut self.playlist_scroll, &mut self.other_view_scroll);
        self.view = view;
        self.hovered_playlist = None;
        self.pressed_playlist = None;
        self.hovered_control = None;
        self.layout_invalidation.invalidate();
        self.ensure_layout();
        self.playlist_scroll.focused = self.focus_index == 8;
    }

    fn list_len(&self) -> usize {
        match self.view {
            MelodyView::Collection => self.collections.len(),
            MelodyView::NowPlaying => self.active_queue.len(),
        }
    }

    fn selected_list_row(&self) -> Option<usize> {
        match self.view {
            MelodyView::Collection => Some(self.selected_collection),
            MelodyView::NowPlaying => self
                .active_queue
                .iter()
                .position(|index| *index == self.selected_playlist),
        }
    }

    fn ensure_playlist_selection_visible(&mut self) {
        if let Some(row) = self
            .active_queue
            .iter()
            .position(|index| *index == self.selected_playlist)
        {
            let scroll = if self.view == MelodyView::NowPlaying {
                &mut self.playlist_scroll
            } else {
                &mut self.other_view_scroll
            };
            scroll.ensure_visible(
                (row as i32).saturating_mul(PLAYLIST_ROW_H as i32),
                PLAYLIST_ROW_H,
            );
        }
    }

    fn activate_list_row(&mut self, row: usize) -> bool {
        if self.view == MelodyView::Collection {
            self.select_collection(row)
        } else {
            self.activate_queue_position(row, true, true)
        }
    }

    fn show_now_playing(&mut self) {
        self.switch_view(MelodyView::NowPlaying);
        self.ensure_layout();
        self.ensure_playlist_selection_visible();
    }

    fn select_collection(&mut self, collection: usize) -> bool {
        let Some(tracks) = self
            .collections
            .get(collection)
            .map(|group| group.tracks.clone())
        else {
            return false;
        };
        self.selected_collection = collection;
        self.playlist_collection = collection;
        self.active_queue = tracks;
        self.shuffle_history.clear();
        self.active_queue_pos = None;
        self.active_playlist = None;
        self.clear_active_track();
        self.playlist_scroll.scroll_to_y(0);
        if let Some(first) = self.active_queue.first().copied() {
            self.active_queue_pos = Some(0);
            self.selected_playlist = first;
            self.reset_shuffle_cycle();
            let path = self.playlist[first].path.clone();
            self.load_media_path_with_autoplay(&path, false);
        } else {
            self.shuffle_remaining.clear();
            self.status = "This collection has no playable tracks";
        }
        self.switch_view(MelodyView::NowPlaying);
        self.ensure_layout();
        self.ensure_playlist_selection_visible();
        true
    }

    fn clear_active_track(&mut self) {
        if self.media.view().controls().stop {
            let _ = self.media.stop();
        }
        self.active_playlist = None;
        self.active_queue_pos = None;
        self.has_active_source = false;
        self.artwork = None;
        self.track_title = String::from("No track selected");
        self.track_artist.clear();
        self.track_album.clear();
        self.visualization_frame = VisualizationFrame::empty();
    }

    fn reset_shuffle_cycle(&mut self) {
        self.shuffle_remaining.clear();
        if !self.shuffle {
            return;
        }
        for position in 0..self.active_queue.len() {
            if Some(position) != self.active_queue_pos {
                self.shuffle_remaining.push(position);
            }
        }
    }

    fn random_index(&self, len: usize) -> usize {
        if len <= 1 {
            return 0;
        }
        let mut bytes = [0u8; 8];
        let entropy = sunlight_libc::getrandom(&mut bytes, sunlight_libc::GRND_NONCRYPTO);
        let value = if entropy == bytes.len() as isize {
            u64::from_le_bytes(bytes)
        } else {
            monotonic_millis()
        };
        (value % len as u64) as usize
    }

    fn next_shuffle_position(&mut self) -> Option<usize> {
        if self.shuffle_remaining.is_empty() {
            if self.repeat_mode == RepeatMode::All {
                self.reset_shuffle_cycle();
            }
        }
        if self.shuffle_remaining.is_empty() {
            return None;
        }
        let index = self.random_index(self.shuffle_remaining.len());
        Some(self.shuffle_remaining.swap_remove(index))
    }

    fn activate_queue_position(
        &mut self,
        position: usize,
        autoplay: bool,
        record_history: bool,
    ) -> bool {
        let Some(index) = self.active_queue.get(position).copied() else {
            return false;
        };
        if record_history {
            if let Some(previous) = self.active_queue_pos {
                if previous != position {
                    self.shuffle_history.push(previous);
                }
            }
        }
        self.active_queue_pos = Some(position);
        if self.shuffle {
            self.shuffle_remaining.retain(|candidate| *candidate != position);
        }
        self.selected_playlist = index;
        let path = self.playlist[index].path.clone();
        self.load_media_path_with_autoplay(&path, autoplay);
        self.ensure_playlist_selection_visible();
        true
    }

    fn open_media(&mut self) {
        let request = DialogRequest::OpenFile(OpenFileRequest {
            title: String::from("Open Audio"),
            initial_dir: Some(music_directory()),
            allowed_mime_types: vec![
                String::from("audio/ogg"),
                String::from("audio/wav"),
                String::from("audio/mpeg"),
            ],
            allowed_extensions: vec![
                String::from("ogg"),
                String::from("oga"),
                String::from("wav"),
                String::from("mp3"),
            ],
            allow_multiple: false,
            show_preview: false,
            confirm_button_label: Some(String::from("Open")),
        });
        match DialogClient::new().show(&request) {
            Ok(DialogResult::FileSelected(path)) => {
                self.load_media_path(&path);
                self.show_now_playing();
            }
            Ok(DialogResult::Cancelled | DialogResult::Cancel | DialogResult::Dismissed) => {
                self.status = "Open cancelled";
            }
            Ok(_) => self.status = "No audio file was selected",
            Err(_) => self.status = "File picker unavailable",
        }
    }

    fn open_music_folder(&mut self) {
        let request = DialogRequest::OpenFolder(OpenFolderRequest {
            title: String::from("Add Music Folder"),
            initial_dir: Some(music_directory()),
            confirm_button_label: Some(String::from("Add Folder")),
        });
        match DialogClient::new().show(&request) {
            Ok(DialogResult::FolderSelected(path)) => {
                let added = self.add_music_directory(&path);
                self.status = if added == 0 {
                    "No supported audio files found"
                } else {
                    "Music folder added"
                };
            }
            Ok(DialogResult::Cancelled | DialogResult::Cancel | DialogResult::Dismissed) => {
                self.status = "Folder selection cancelled";
            }
            Ok(_) => self.status = "No folder was selected",
            Err(_) => self.status = "Folder picker unavailable",
        }
    }

    fn add_music_directory(&mut self, root: &str) -> usize {
        let mut added = 0;
        for entry in scan_music_directory(root) {
            if self
                .playlist
                .iter()
                .any(|existing| existing.path == entry.path)
            {
                continue;
            }
            self.playlist.push(entry);
            added += 1;
        }
        if added != 0 {
            self.rebuild_collections();
        }
        added
    }

    fn load_media_path(&mut self, path: &str) {
        let index = self.playlist.iter().position(|entry| entry.path == path).or_else(|| {
            let format = supported_extension(path)?;
            let filename = path.rsplit('/').next().unwrap_or(path);
            let title = filename
                .rsplit_once('.')
                .map(|(stem, _)| stem)
                .filter(|stem| !stem.is_empty())
                .unwrap_or(filename);
            self.playlist.push(MediaEntry {
                path: String::from(path),
                display_title: String::from(title),
                artist: None,
                album: None,
                duration_ms: None,
                format,
                source: MediaSource::UserMusic,
            });
            self.rebuild_collections();
            Some(self.playlist.len() - 1)
        });
        if let Some(index) = index {
            if !self.active_queue.contains(&index) {
                self.active_queue.clear();
                self.active_queue.push(index);
                self.playlist_collection = 0;
                self.active_queue_pos = Some(0);
                self.shuffle_history.clear();
                self.reset_shuffle_cycle();
            }
        }
        self.load_media_path_with_autoplay(path, true);
    }

    fn load_media_path_with_autoplay(&mut self, path: &str, auto_play: bool) {
        let index = self.playlist.iter().position(|entry| entry.path == path);
        if let Some(index) = index {
            self.selected_playlist = index;
            self.active_playlist = Some(index);
            if let Some(position) = self.active_queue.iter().position(|entry| *entry == index) {
                self.active_queue_pos = Some(position);
            }
        }
        self.artwork = None;
        self.track_title = String::from("Loading");
        self.track_artist.clear();
        self.track_album.clear();
        self.has_active_source = false;
        let opened = if auto_play {
            self.media.open(path)
        } else {
            self.media.open_paused(path)
        };
        match opened {
            Ok(()) => {
                let fallback_title = index
                    .and_then(|index| self.playlist.get(index))
                    .map(|entry| entry.display_title.clone())
                    .unwrap_or_else(|| String::from("Unknown"));
                let tags = supported_extension(path)
                    .and_then(|format| {
                        read_metadata_bytes(path, format)
                            .map(|bytes| metadata::parse(format, &bytes))
                    })
                    .unwrap_or_default();
                self.track_title = tags.title.unwrap_or(fallback_title);
                self.track_artist = tags.artist.unwrap_or_else(|| String::from("Unknown"));
                self.track_album = tags.album.unwrap_or_else(|| String::from("Unknown"));
                self.artwork = tags.cover.as_deref().and_then(|cover| {
                    let (width, height) =
                        sunlight_ui::image::inspect_image_dimensions(cover).ok()?;
                    (u64::from(width) * u64::from(height) <= 1024 * 1024)
                        .then(|| decode_image(cover).ok())
                        .flatten()
                });
                self.has_active_source = true;
                self.status = "Loading audio...";
                self.ensure_layout();
                self.ensure_playlist_selection_visible();
            }
            Err(error) => {
                self.active_playlist = None;
                self.active_queue_pos = None;
                self.status = error.user_message();
                debug_log("[MELODY-MINA][media-open-error] source=");
                debug_log(path);
                debug_log(" kind=");
                debug_log_u64(error.kind as u64);
                debug_log(" detail=");
                debug_log_u64(error.detail as u64);
                debug_log(" message=");
                debug_log(error.user_message());
                debug_log("\n");
            }
        }
    }

    fn playlist_index_at(&self, point: Point) -> Option<usize> {
        let viewport = self.layout.playlist.inset(2);
        if !viewport.contains(point) {
            return None;
        }
        let local_y = point
            .y
            .saturating_sub(viewport.y)
            .saturating_add(self.playlist_scroll.offset_y);
        let index = (local_y as u32 / PLAYLIST_ROW_H) as usize;
        (index < self.list_len()).then_some(index)
    }

    fn navigate_playlist(&mut self, delta: isize) -> bool {
        if self.active_queue.is_empty() {
            return false;
        }
        if delta < 0 && self.shuffle && !self.shuffle_history.is_empty() {
            if let Some(previous) = self.shuffle_history.pop() {
                return self.activate_queue_position(previous, true, false);
            }
        }
        if delta > 0 && self.shuffle {
            let Some(next) = self.next_shuffle_position() else {
                return false;
            };
            return self.activate_queue_position(next, true, true);
        }
        let current = self.active_queue_pos.unwrap_or(0);
        let Some(next) = next_playlist_index(current, self.active_queue.len(), delta) else {
            return false;
        };
        self.activate_queue_position(next, true, true)
    }

    fn advance_after_completion(&mut self) {
        let Some(current) = self.active_queue_pos else {
            return;
        };
        if self.repeat_mode == RepeatMode::One {
            let _ = self.activate_queue_position(current, true, false);
            return;
        }
        let next = if self.shuffle
            && self.active_queue.len() == 1
            && self.repeat_mode == RepeatMode::All
        {
            Some(current)
        } else if self.shuffle {
            self.next_shuffle_position()
        } else if current + 1 < self.active_queue.len() {
            Some(current + 1)
        } else if self.repeat_mode == RepeatMode::All && !self.active_queue.is_empty() {
            Some(0)
        } else {
            None
        };
        if let Some(next) = next {
            let _ = self.activate_queue_position(next, true, true);
        } else {
            self.status = "Playlist finished";
        }
    }

    fn move_playlist_selection(&mut self, delta: isize) -> bool {
        let next = match self.selected_list_row() {
            Some(current) => next_playlist_index(current, self.list_len(), delta),
            None if self.list_len() != 0 => Some(if delta < 0 {
                self.list_len() - 1
            } else {
                0
            }),
            None => None,
        };
        let Some(next) = next else {
            return false;
        };
        if self.view == MelodyView::Collection {
            self.selected_collection = next;
        } else {
            self.selected_playlist = self.active_queue[next];
        }
        self.playlist_scroll.ensure_visible(
            (next as i32).saturating_mul(PLAYLIST_ROW_H as i32),
            PLAYLIST_ROW_H,
        );
        true
    }

    fn sync_media(&mut self) -> bool {
        let changed = self.media.refresh();
        let now_playing = self.media.view();
        let previous_state = self.last_media_state;
        let state_changed = now_playing.playback_state != self.last_media_state;
        self.last_media_state = now_playing.playback_state;
        let mut advancing = false;
        if state_changed && now_playing.playback_state == PlaybackState::Ended {
            self.advance_after_completion();
            advancing = self.media.view().playback_state == PlaybackState::Loading;
        }
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
                debug_log(" message=");
                debug_log(error.user_message());
                debug_log("\n");
            }
        }
        self.status = if advancing {
            "Loading audio..."
        } else if self.active_queue.is_empty() && !self.has_active_source {
            "Choose a collection to begin"
        } else if let Some(error) = now_playing.error {
            error.user_message()
        } else {
            match now_playing.playback_state {
                PlaybackState::Idle => "Select an audio file",
                PlaybackState::Loading => "Loading audio...",
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
            self.layout = MelodyLayout::arrange_view(root, self.view);
            self.timeline.rect = self.layout.timeline_slider;
            self.volume.rect = self.layout.volume_slider;
        }
        let viewport = self.layout.playlist.inset(2);
        self.playlist_scroll.set_geometry(
            viewport.w,
            viewport.h,
            viewport.w,
            (self.list_len() as u32).saturating_mul(PLAYLIST_ROW_H),
        );
    }

    fn button_state(&self, index: usize) -> ButtonState {
        if !self.control_enabled(index) {
            ButtonState::Disabled
        } else if self.pressed_control == Some(index) {
            ButtonState::Pressed
        } else if index == 6 && self.repeat_mode != RepeatMode::Off {
            ButtonState::Pressed
        } else if index == 7 && self.shuffle {
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
            1 => true,
            2 => controls.stop,
            4 => controls.play_pause,
            3 | 5 => self.has_active_source && !self.active_queue.is_empty(),
            6 | 7 => controls.play_pause,
            11 | 12 => true,
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
        let collection_empty = self.view == MelodyView::Collection && self.active_queue.is_empty();
        let title_source = if collection_empty {
            "Your music collection"
        } else {
            self.track_title()
        };
        let title = elide(
            title_source,
            FontRole::UiTitle,
            rect.w,
            &mut title_buf,
        );
        let mut clip = canvas.sub_canvas(rect);
        let local = Rect::new(0, 0, rect.w, rect.h);
        let centered =
            self.view == MelodyView::NowPlaying && self.layout.mode != LayoutMode::Medium;
        let title_rect = Rect::new(0, 2, rect.w, 24.min(rect.h));
        let title_style = TextStyle::new(FontRole::UiTitle, theme.text);
        if centered {
            draw_text_centered(&mut clip, title_rect, title, &title_style);
        } else {
            draw_text(&mut clip, title, 0, 2, &title_style);
        }
        let mut artist_buf = [0u8; 96];
        let artist_source = if collection_empty {
            "Choose a collection to prepare playback"
        } else {
            self.track_artist.as_str()
        };
        let artist = elide(
            artist_source,
            FontRole::UiMedium,
            rect.w,
            &mut artist_buf,
        );
        let artist_style = TextStyle::new(FontRole::UiMedium, theme.accent_hover);
        if centered {
            draw_text_centered(
                &mut clip,
                Rect::new(0, 28, local.w, 20),
                artist,
                &artist_style,
            );
        } else {
            draw_text(&mut clip, artist, 0, 28, &artist_style);
        }
        if rect.h >= 58 {
            let mut album_buf = [0u8; 96];
            let album = elide(&self.track_album, FontRole::UiSmall, rect.w, &mut album_buf);
            let style = TextStyle::new(FontRole::UiSmall, theme.text_dim);
            if centered {
                draw_text_centered(&mut clip, Rect::new(0, 49, local.w, 18), album, &style);
            } else {
                draw_text(&mut clip, album, 0, 49, &style);
            }
        }
        if rect.h >= 82 {
            let mut status_buf = [0u8; 96];
            let status = elide(self.status, FontRole::UiSmall, rect.w, &mut status_buf);
            let style = TextStyle::new(FontRole::UiSmall, theme.text_muted);
            if centered {
                draw_text_centered(&mut clip, Rect::new(0, 68, local.w, 16), status, &style);
            } else {
                draw_text(&mut clip, status, 0, 68, &style);
            }
        }
    }

    fn draw_navigation(&self, canvas: &mut Canvas, theme: &Theme) {
        for (offset, label) in ["Collection", "Now Playing"].iter().enumerate() {
            let index = 11 + offset;
            let rect = self.layout.navigation[offset];
            let active = matches!(
                (self.view, offset),
                (MelodyView::Collection, 0) | (MelodyView::NowPlaying, 1)
            );
            let bg = match self.button_state(index) {
                ButtonState::Pressed => theme.chrome.selection,
                ButtonState::Hovered => theme.panel_alt,
                _ if active => theme.chrome.selection,
                _ => theme.panel,
            };
            canvas.fill_rounded_rect_with_border(
                rect,
                7,
                bg,
                if active || (self.focus_index == index && self.window_focused) {
                    theme.accent
                } else {
                    theme.border
                },
                1,
            );
            let mut clip = canvas.sub_canvas(rect);
            let mut label_buf = [0u8; 32];
            let label = elide(
                label,
                FontRole::UiMedium,
                rect.w.saturating_sub(12),
                &mut label_buf,
            );
            draw_text_centered(
                &mut clip,
                Rect::new(0, 0, rect.w, rect.h),
                label,
                &TextStyle::new(
                    FontRole::UiMedium,
                    if active { theme.accent_hover } else { theme.text },
                ),
            );
        }
    }

    fn draw_list(&self, canvas: &mut Canvas, theme: &Theme) {
        let heading = self.layout.list_heading;
        let heading_title = if self.view == MelodyView::Collection {
            String::from("Collection / Browse")
        } else {
            self.collections
                .get(self.playlist_collection)
                .map(|group| format!("Playlist · {}", group.title))
                .unwrap_or_else(|| String::from("Playlist"))
        };
        let mut title_buf = [0u8; 128];
        let title = elide(&heading_title, FontRole::UiSmall, heading.w, &mut title_buf);
        let mut heading_clip = canvas.sub_canvas(heading);
        draw_text_vcenter(
            &mut heading_clip,
            title,
            0,
            0,
            heading.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        let collection = self.view == MelodyView::Collection;
        let items: Vec<_> = if collection {
            self.collections
                .iter()
                .map(|group| PlaylistItemViewModel {
                    title: &group.title,
                    artist: Some(&group.subtitle),
                    duration_seconds: None,
                    // Reuse the current decoded cover without extra artwork reads.
                    artwork: self
                        .active_playlist
                        .filter(|active| group.tracks.contains(active))
                        .and(self.artwork.as_ref()),
                })
                .collect()
        } else {
            self.active_queue
                .iter()
                .map(|index| {
                    let entry = &self.playlist[*index];
                    PlaylistItemViewModel {
                        title: &entry.display_title,
                        artist: entry.artist.as_deref(),
                        duration_seconds: if Some(*index) == self.active_playlist {
                            self.media
                                .view()
                                .duration_ms
                                .or(entry.duration_ms)
                                .map(|value| value / 1_000)
                        } else {
                            entry.duration_ms.map(|value| value / 1_000)
                        },
                        artwork: None,
                    }
                })
                .collect()
        };
        let playing = if collection {
            None
        } else {
            self.active_queue_pos
        };
        PlaylistView {
            rect: self.layout.playlist,
            items: &items,
            selected: self.selected_list_row(),
            playing,
            collection,
            hovered: self.hovered_playlist,
            scroll: &self.playlist_scroll,
        }
        .draw(canvas, theme);
        if !collection && self.active_queue.is_empty() {
            let empty = self.layout.playlist.inset(16);
            let mut empty_buf = [0u8; 96];
            let empty_hint = elide(
                "Choose a collection to add tracks to your playlist.",
                FontRole::UiSmall,
                empty.w,
                &mut empty_buf,
            );
            draw_text_centered(
                canvas,
                Rect::new(empty.x, empty.y + 30, empty.w, 24),
                "Your playlist is empty",
                &TextStyle::new(FontRole::UiMedium, theme.text),
            );
            draw_text_centered(
                canvas,
                Rect::new(empty.x, empty.y + 58, empty.w, 40),
                empty_hint,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
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
        if self.focus_index == 9 && self.window_focused {
            canvas.draw_rect(self.layout.timeline_slider, theme.accent_hover);
        }
    }

    fn draw_transport(&self, canvas: &mut Canvas, theme: &Theme) {
        let symbols = [
            UiSymbol::Stop,
            UiSymbol::PreviousTrack,
            if self.media.shows_pause() {
                UiSymbol::Pause
            } else {
                UiSymbol::Play
            },
            UiSymbol::NextTrack,
            UiSymbol::Repeat,
            UiSymbol::Shuffle,
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
        if self.focus_index == 10 && self.window_focused {
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
        for (offset, rect) in self.layout.navigation.iter().enumerate() {
            if rect.contains(point) {
                return Some(11 + offset);
            }
        }
        if self.view == MelodyView::Collection
            && (self.layout.album_art.contains(point) || self.layout.metadata.contains(point))
        {
            return Some(12);
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
            1 => self.open_music_folder(),
            3 => {
                self.navigate_playlist(-1);
            }
            2 => match self.media.stop() {
                Ok(()) => self.status = "Stopping playback...",
                Err(error) => self.status = error.user_message(),
            },
            4 => {
                if let Err(error) = self.media.play_pause() {
                    self.status = error.user_message();
                }
            }
            5 => {
                self.navigate_playlist(1);
            }
            6 => {
                self.repeat_mode = match self.repeat_mode {
                    RepeatMode::Off => RepeatMode::One,
                    RepeatMode::One => RepeatMode::All,
                    RepeatMode::All => RepeatMode::Off,
                };
                self.status = match self.repeat_mode {
                    RepeatMode::Off => "Repeat off",
                    RepeatMode::All => "Repeat all",
                    RepeatMode::One => "Repeat one",
                };
            }
            7 => {
                self.shuffle = !self.shuffle;
                self.shuffle_history.clear();
                self.reset_shuffle_cycle();
                self.status = if self.shuffle {
                    "Shuffle on"
                } else {
                    "Shuffle off"
                };
            }
            11 => self.switch_view(MelodyView::Collection),
            12 => self.show_now_playing(),
            _ => return false,
        }
        true
    }

    fn keyboard_activate(&mut self) -> bool {
        if self.focus_index <= 7 || self.focus_index >= 11 {
            self.activate_control(self.focus_index)
        } else if self.focus_index == 8 {
            self.activate_list_row(self.selected_list_row().unwrap_or(0))
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
        self.playlist_scroll.focused = self.focus_index == 8;
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
        self.draw_navigation(canvas, theme);
        AlbumArtView::new(self.layout.album_art, self.artwork.as_ref()).draw(canvas, theme);
        self.draw_metadata(canvas, theme);
        self.draw_list(canvas, theme);
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
                self.hovered_playlist = None;
                self.pressed_playlist = None;
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
                let old_playlist_hover = self.hovered_playlist;
                self.hovered_playlist = self.playlist_index_at(point);
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
                    || old_playlist_hover != self.hovered_playlist
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
                    self.focus_index = 8;
                    return true;
                }
                if let Some(index) = self.playlist_index_at(point) {
                    self.pressed_playlist = Some(index);
                    self.focus_index = 8;
                    self.playlist_scroll.focused = true;
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
                    self.focus_index = 9;
                }
                if self.layout.volume_slider.contains(point) {
                    self.focus_index = 10;
                }
                timeline_changed || volume_changed
            }
            Event::MouseUp { x, y, .. } => {
                let timeline_changed = if self.media.interaction().seek_drag_active {
                    let _ = self.timeline.update(Event::MouseMove { x, y });
                    let changed = self.timeline.update(event);
                    match self.media.commit_seek(self.timeline.value) {
                        Ok(()) => self.status = "Seeking...",
                        Err(error) => self.status = error.user_message(),
                    }
                    self.seek_committed_on_release = true;
                    let _ = changed;
                    true
                } else {
                    false
                };
                let was_volume_dragging = self.volume.dragging;
                if was_volume_dragging {
                    let _ = self.volume.update(Event::MouseMove { x, y });
                }
                let volume_changed = self.volume.update(event);
                if was_volume_dragging {
                    match self.media.set_volume(self.volume.value) {
                        Ok(()) => self.status = "Stream volume changed",
                        Err(error) => self.status = error.user_message(),
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
                let playlist_activated = self
                    .pressed_playlist
                    .take()
                    .filter(|index| self.playlist_index_at(point) == Some(*index))
                    .map(|index| self.activate_list_row(index))
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
                        Err(error) => self.status = error.user_message(),
                    }
                }
                let volume_changed = self.volume.update(event);
                if volume_changed {
                    match self.media.set_volume(self.volume.value) {
                        Ok(()) => self.status = "Stream volume changed",
                        Err(error) => self.status = error.user_message(),
                    }
                }
                activated
                    || playlist_activated
                    || timeline_changed
                    || volume_changed
                    || self.hovered_control.is_some()
            }
            Event::MouseWheel { x, y, delta } => {
                if self.layout.playlist.contains(Point::new(x, y)) {
                    self.focus_index = 8;
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
            } if self.focus_index == 8 => self.move_playlist_selection(-1),
            Event::KeyPress {
                keycode: 0x50,
                pressed: true,
                ..
            } if self.focus_index == 8 => self.move_playlist_selection(1),
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
        if self.media.has_pending_action() {
            return FRAME_MS_FOCUSED;
        }
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
    for pointer in raw.iter().take(count).skip(1) {
        if pointer.is_null() {
            continue;
        }
        let len = unsafe { sunlight_libc::crt0::cstr_len(*pointer, 4096) };
        let bytes = unsafe { core::slice::from_raw_parts(*pointer, len) };
        let Ok(argument) = core::str::from_utf8(bytes) else {
            continue;
        };
        // The launcher prepends this bookkeeping argument to GUI processes.
        // It is not a media path and must never be handed to MediaPlayer.
        if argument.starts_with("--sunlight-launch=") {
            continue;
        }
        if argument.starts_with('/') {
            return Some(String::from(argument));
        }
    }
    None
}

fn music_directory() -> String {
    sunlight_libc::env::getenv(b"HOME")
        .map(|home| {
            let mut path = String::from(home);
            if !path.ends_with('/') {
                path.push('/');
            }
            path.push_str("Music");
            path
        })
        .unwrap_or_else(|| String::from("/root/Music"))
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, envp: *const *const u8) -> ! {
    sunlight_libc::env::init(envp);
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
        app.show_now_playing();
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
