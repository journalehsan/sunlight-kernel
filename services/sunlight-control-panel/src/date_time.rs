//! Control Panel page: Date & Time.
//!
//! Three sections:
//!   1. Current time (SolarClock + digital/date/zone metadata)
//!   2. Time synchronization (timed status + Sync now)
//!   3. Timezone selection (catalog search + WorldMap + Apply)
//!
//! Widgets never query services. All IPC goes through `sunlight_tz::client`.
//! City search and map nearest-hit use the shared `sunlight_tz::catalog`.

use core::fmt::Write;

use sun_font::{self, FontRole, TextStyle, Typography};

use sunlight_ipc::monotonic_millis;
use sunlight_tz::{
    catalog::{
        deg_to_md, location_by_zone_id, md_to_deg, nearest_locations, search_locations,
        selection_is_pending, DEFAULT_MAX_DISTANCE_MD, MAX_SEARCH_RESULTS,
    },
    local_now, tz_by_id, LocalTimeSnapshot, SyncStatusSnapshot, TimeClient, TimeClientError,
    TzClient, TzClientError, TzLocation,
};
use sunlight_ui::{
    widgets::{
        BoundedSearchField, Button, ButtonState, DigitalAlign, DigitalNumberWidget, GeoCoord,
        MapHit, MapMarker, SolarClockSnapshot, SolarClockWidget, WorldMapWidget,
    },
    Canvas, Color, Event, MaterialPalette, Point, Rect, Theme,
};

use crate::sysinfo::FixedStr;

const KEY_ESC: u8 = 0x01;
const KEY_ENTER: u8 = 0x1C;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_BACKSPACE: u8 = 0x0E;

/// Clock second refresh while the page is visible (not a continuous repaint loop).
const CLOCK_TICK_MS: u64 = 1_000;
/// How often to re-query service status when idle (not while a sync is in flight).
const STATUS_REFRESH_MS: u64 = 5_000;
/// Result rows that fit the reserved list band without colliding with preview.
const SEARCH_VISIBLE: usize = 4;
const RESULT_ROW_H: i32 = 18;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DateTimeAction {
    None,
    Back,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncUiPhase {
    Idle,
    /// User requested sync; next tick performs the blocking IPC call.
    Queued,
    /// IPC in progress (button stays disabled).
    Active,
}

pub struct DateTimePageState {
    // --- Applied (authoritative) ---
    applied_zone: FixedStr<64>,
    local: Option<LocalTimeSnapshot>,
    last_snapshot: SolarClockSnapshot,
    date_line: FixedStr<24>,
    offset_line: FixedStr<40>,

    // --- Sync ---
    sync: Option<SyncStatusSnapshot>,
    sync_error: FixedStr<80>,
    timed_error: FixedStr<64>,
    sync_phase: SyncUiPhase,
    sync_feedback: FixedStr<80>,

    // --- Timezone selection (single authoritative proposed state) ---
    /// Canonical zone id for Apply (search or map — same owner).
    proposed_zone: FixedStr<64>,
    proposed_city: FixedStr<40>,
    proposed_country: FixedStr<32>,
    proposed_coord: Option<GeoCoord>,
    search: BoundedSearchField<48>,
    search_hits_zones: [FixedStr<64>; MAX_SEARCH_RESULTS],
    search_hits_cities: [FixedStr<40>; MAX_SEARCH_RESULTS],
    search_hits_countries: [FixedStr<32>; MAX_SEARCH_RESULTS],
    search_hits_len: usize,
    selected_result: usize,
    /// Transient map/search feedback only (never shares the preview row).
    map_feedback: FixedStr<64>,
    /// Apply / set-zone outcome only (dedicated action feedback row).
    apply_feedback: FixedStr<80>,
    tz_error: FixedStr<64>,

    // --- Scheduling ---
    next_clock_ms: u64,
    next_status_ms: u64,
    page_active: bool,
}

impl DateTimePageState {
    pub fn new() -> Self {
        Self {
            applied_zone: FixedStr::empty(),
            local: None,
            last_snapshot: SolarClockSnapshot::new(0, 0, 0),
            date_line: FixedStr::empty(),
            offset_line: FixedStr::empty(),
            sync: None,
            sync_error: FixedStr::empty(),
            timed_error: FixedStr::empty(),
            sync_phase: SyncUiPhase::Idle,
            sync_feedback: FixedStr::empty(),
            proposed_zone: FixedStr::empty(),
            proposed_city: FixedStr::empty(),
            proposed_country: FixedStr::empty(),
            proposed_coord: None,
            search: BoundedSearchField::new(),
            search_hits_zones: [FixedStr::empty(); MAX_SEARCH_RESULTS],
            search_hits_cities: [FixedStr::empty(); MAX_SEARCH_RESULTS],
            search_hits_countries: [FixedStr::empty(); MAX_SEARCH_RESULTS],
            search_hits_len: 0,
            selected_result: 0,
            map_feedback: FixedStr::empty(),
            apply_feedback: FixedStr::empty(),
            tz_error: FixedStr::empty(),
            next_clock_ms: 0,
            next_status_ms: 0,
            page_active: false,
        }
    }

    /// Enter the page: load authoritative state and arm deadlines.
    pub fn activate(&mut self) -> bool {
        self.page_active = true;
        self.sync_phase = SyncUiPhase::Idle;
        self.search.clear();
        self.search.active = false;
        self.map_feedback.clear();
        self.apply_feedback.clear();
        self.sync_feedback.clear();
        self.refresh_all();
        self.run_search();
        // Seed proposed from applied.
        if self.proposed_zone.is_empty() {
            self.seed_proposed_from_applied();
        }
        true
    }

    fn seed_proposed_from_applied(&mut self) {
        self.proposed_zone.set(self.applied_zone.as_str());
        if let Some(loc) = location_by_zone_id(self.applied_zone.as_str()) {
            self.proposed_city.set(loc.city);
            self.proposed_country.set(loc.country);
            self.proposed_coord =
                Some(GeoCoord::new(md_to_deg(loc.lon_md), md_to_deg(loc.lat_md)));
        } else {
            self.proposed_city.clear();
            self.proposed_country.clear();
            self.proposed_coord = None;
        }
    }

    /// Leave the page: cancel deadlines and drop transient UI state.
    pub fn deactivate(&mut self) {
        self.page_active = false;
        self.next_clock_ms = u64::MAX;
        self.next_status_ms = u64::MAX;
        self.sync_phase = SyncUiPhase::Idle;
        self.search.active = false;
        // Drop service snapshots so reopening reloads cleanly.
        *self = Self::new();
    }

    pub fn refresh_due(&self) -> bool {
        if !self.page_active {
            return false;
        }
        let now = monotonic_millis();
        now >= self.next_clock_ms
            || now >= self.next_status_ms
            || matches!(self.sync_phase, SyncUiPhase::Queued)
    }

    pub fn on_tick(&mut self) -> bool {
        if !self.refresh_due() {
            return false;
        }
        let mut dirty = false;
        let now = monotonic_millis();

        if matches!(self.sync_phase, SyncUiPhase::Queued) {
            dirty |= self.perform_sync();
        }

        if now >= self.next_clock_ms {
            dirty |= self.refresh_local_time();
            self.next_clock_ms = now.saturating_add(CLOCK_TICK_MS);
        }

        if now >= self.next_status_ms && matches!(self.sync_phase, SyncUiPhase::Idle) {
            dirty |= self.refresh_sync_status();
            self.next_status_ms = now.saturating_add(STATUS_REFRESH_MS);
        }

        dirty
    }

    fn refresh_all(&mut self) {
        let now = monotonic_millis();
        let _ = self.refresh_zone();
        let _ = self.refresh_local_time();
        let _ = self.refresh_sync_status();
        self.next_clock_ms = now.saturating_add(CLOCK_TICK_MS);
        self.next_status_ms = now.saturating_add(STATUS_REFRESH_MS);
    }

    fn refresh_zone(&mut self) -> bool {
        match TzClient::connect() {
            Ok(client) => match client.get_zone() {
                Ok(zone) => {
                    self.tz_error.clear();
                    let id = zone.id_str();
                    let changed = self.applied_zone.as_str() != id;
                    self.applied_zone.set(id);
                    if changed || self.proposed_zone.is_empty() {
                        // Keep proposed only if user already has a pending selection.
                        if !selection_is_pending(self.applied_zone.as_str(), self.proposed_zone.as_str())
                        {
                            self.seed_proposed_from_applied();
                        }
                    }
                    true
                }
                Err(e) => {
                    self.tz_error.set(tz_err_label(e));
                    true
                }
            },
            Err(_) => {
                self.tz_error.set("Timezone service unavailable");
                true
            }
        }
    }

    fn refresh_local_time(&mut self) -> bool {
        match TzClient::connect() {
            Ok(client) => match client.get_local_time() {
                Ok(lt) => {
                    self.tz_error.clear();
                    let next = SolarClockSnapshot::new(lt.hour, lt.minute, lt.second);
                    let second_changed = next.second != self.last_snapshot.second
                        || next.minute != self.last_snapshot.minute
                        || next.hour != self.last_snapshot.hour;
                    self.last_snapshot = next;
                    self.local = Some(lt);
                    self.date_line.clear();
                    let _ = write!(
                        &mut self.date_line,
                        "{:04}-{:02}-{:02}",
                        lt.year, lt.month, lt.day
                    );
                    self.offset_line.clear();
                    format_offset_line(&mut self.offset_line, lt.utc_offset_secs, lt.is_dst);
                    // Only request a redraw when something visible changed.
                    second_changed
                }
                Err(e) => {
                    self.tz_error.set(tz_err_label(e));
                    true
                }
            },
            Err(_) => {
                // Soft-fail: keep last local time if any.
                if self.tz_error.is_empty() {
                    self.tz_error.set("Timezone service unavailable");
                    true
                } else {
                    false
                }
            }
        }
    }

    fn refresh_sync_status(&mut self) -> bool {
        match TimeClient::connect() {
            Ok(client) => match client.sync_status() {
                Ok(st) => {
                    self.timed_error.clear();
                    self.sync = Some(st);
                    true
                }
                Err(e) => {
                    self.timed_error.set(time_err_label(e));
                    true
                }
            },
            Err(_) => {
                self.timed_error.set("Time service unavailable");
                self.sync = None;
                true
            }
        }
    }

    fn queue_sync(&mut self) -> bool {
        if !matches!(self.sync_phase, SyncUiPhase::Idle) {
            return false;
        }
        self.sync_phase = SyncUiPhase::Queued;
        self.sync_feedback.set("Syncing…");
        self.sync_error.clear();
        true
    }

    fn perform_sync(&mut self) -> bool {
        self.sync_phase = SyncUiPhase::Active;
        let result = match TimeClient::connect() {
            Ok(client) => client.sync_now(),
            Err(e) => Err(e),
        };
        self.sync_phase = SyncUiPhase::Idle;
        match result {
            Ok(st) => {
                self.sync = Some(st);
                self.timed_error.clear();
                self.sync_error.clear();
                if st.ntp_synced || st.state == sunlight_ipc::NtpSyncState::SYNCHRONIZED {
                    self.sync_feedback.set("Synchronization succeeded");
                } else {
                    self.sync_feedback.set(st.state_label());
                }
                // Local time may have stepped.
                let _ = self.refresh_local_time();
            }
            Err(e) => {
                self.sync_feedback.clear();
                self.sync_error.set(time_err_label(e));
                let _ = self.refresh_sync_status();
            }
        }
        self.next_status_ms = monotonic_millis().saturating_add(STATUS_REFRESH_MS);
        true
    }

    fn run_search(&mut self) {
        let results = search_locations(self.search.value(), SEARCH_VISIBLE);
        self.search_hits_len = 0;
        for (i, hit) in results.iter().enumerate() {
            if i >= MAX_SEARCH_RESULTS {
                break;
            }
            self.search_hits_zones[i].set(hit.location.zone_id);
            self.search_hits_cities[i].set(hit.location.city);
            self.search_hits_countries[i].set(hit.location.country);
            self.search_hits_len = i + 1;
        }
        if self.selected_result >= self.search_hits_len {
            self.selected_result = 0;
        }
    }

    fn propose_location(&mut self, loc: &TzLocation) {
        self.proposed_zone.set(loc.zone_id);
        self.proposed_city.set(loc.city);
        self.proposed_country.set(loc.country);
        self.proposed_coord = Some(GeoCoord::new(md_to_deg(loc.lon_md), md_to_deg(loc.lat_md)));
        self.apply_feedback.clear();
    }

    fn propose_from_result(&mut self, index: usize) {
        if index >= self.search_hits_len {
            return;
        }
        let zone = self.search_hits_zones[index].as_str();
        if let Some(loc) = location_by_zone_id(zone) {
            self.propose_location(loc);
            self.map_feedback.clear();
        } else {
            // Unsupported catalog/zone pairing — keep selection invalid for Apply.
            self.proposed_zone.set(zone);
            self.proposed_city.set(self.search_hits_cities[index].as_str());
            self.proposed_country
                .set(self.search_hits_countries[index].as_str());
            self.proposed_coord = None;
            self.map_feedback.set("Unsupported timezone");
            self.apply_feedback.clear();
        }
        self.selected_result = index;
    }

    fn handle_map_click(&mut self, coord: GeoCoord) {
        let lat_md = deg_to_md(coord.lat);
        let lon_md = deg_to_md(coord.lon);
        let near = nearest_locations(lat_md, lon_md, 3, DEFAULT_MAX_DISTANCE_MD);
        if near.is_empty() {
            // Failed map lookup: feedback only — do not clear a valid proposed selection.
            self.map_feedback.set("No nearby city in catalog");
            return;
        }
        self.propose_location(near.first().unwrap().location);
        // Optional ambiguity note in the dedicated feedback row (not the preview).
        self.map_feedback.clear();
        if near.len() > 1 {
            let a = near.get(0).unwrap();
            let b = near.get(1).unwrap();
            if b.dist_sq.saturating_mul(100) < a.dist_sq.saturating_mul(115).max(1) {
                let _ = write!(
                    &mut self.map_feedback,
                    "Near {} / {}",
                    a.location.city, b.location.city
                );
            }
        }
    }

    fn apply_proposed(&mut self) -> bool {
        let id = self.proposed_zone.as_str();
        if id.is_empty() {
            self.apply_feedback.set("No timezone selected");
            return true;
        }
        if !selection_is_pending(self.applied_zone.as_str(), id) {
            self.apply_feedback.set("Already using this timezone");
            return true;
        }
        // Validate against bundled DB before calling the service.
        if tz_by_id(id).is_none() {
            self.apply_feedback.set("Unsupported timezone");
            return true;
        }
        match TzClient::connect() {
            Ok(client) => match client.set_zone(id) {
                Ok(()) => {
                    self.apply_feedback.set("Timezone updated");
                    self.tz_error.clear();
                    // Refresh from authoritative service state (UTC preserved by service).
                    let _ = self.refresh_zone();
                    let _ = self.refresh_local_time();
                    let _ = self.refresh_sync_status();
                }
                Err(e) => {
                    self.apply_feedback.set(tz_err_label(e));
                }
            },
            Err(_) => {
                self.apply_feedback.set("Timezone service unavailable");
            }
        }
        true
    }

    fn pending(&self) -> bool {
        selection_is_pending(self.applied_zone.as_str(), self.proposed_zone.as_str())
    }

    // -----------------------------------------------------------------------
    // Layout helpers — fixed non-overlapping bands (WIN 500×560)
    //
    //   y 276–302  search field (left) | map top (right)
    //   y 308–380  result list (left)  | map continues
    //   y 276–392  map bounds (right)
    //   y 400–418  proposed preview (full width)
    //   y 420–436  transient map/search feedback
    //   y 438–454  apply action feedback
    //   y 520–548  Back / Apply buttons
    // -----------------------------------------------------------------------

    fn back_rect(win_w: u32, win_h: u32) -> Rect {
        let _ = win_w;
        Rect::new(12, win_h as i32 - 40, 80, 28)
    }

    fn clock_rect() -> Rect {
        Rect::new(16, 52, 132, 132)
    }

    fn sync_card_rect(win_w: u32) -> Rect {
        Rect::new(12, 196, win_w.saturating_sub(24), 72)
    }

    fn search_rect(win_w: u32) -> Rect {
        let _ = win_w;
        Rect::new(12, 276, 220, 26)
    }

    fn results_band_rect() -> Rect {
        // Exactly SEARCH_VISIBLE rows of RESULT_ROW_H.
        Rect::new(
            12,
            308,
            220,
            (SEARCH_VISIBLE as i32 * RESULT_ROW_H) as u32,
        )
    }

    fn result_row_rect(index: usize) -> Rect {
        let band = Self::results_band_rect();
        Rect::new(
            band.x,
            band.y + index as i32 * RESULT_ROW_H,
            band.w,
            RESULT_ROW_H as u32,
        )
    }

    fn map_rect(win_w: u32) -> Rect {
        // Right column; ends at y=392 so it stays above the preview band.
        Rect::new(246, 276, win_w.saturating_sub(258), 116)
    }

    fn preview_rect(win_w: u32) -> Rect {
        Rect::new(12, 400, win_w.saturating_sub(24), 18)
    }

    fn map_feedback_rect(win_w: u32) -> Rect {
        Rect::new(12, 420, win_w.saturating_sub(24), 16)
    }

    fn apply_feedback_rect(win_w: u32) -> Rect {
        Rect::new(12, 438, win_w.saturating_sub(24), 16)
    }

    fn apply_rect(win_w: u32, win_h: u32) -> Rect {
        Rect::new(win_w as i32 - 120, win_h as i32 - 40, 100, 28)
    }

    fn sync_btn_rect(win_w: u32) -> Rect {
        Rect::new(win_w as i32 - 112, 214, 88, 28)
    }

    // -----------------------------------------------------------------------
    // Draw
    // -----------------------------------------------------------------------

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme, win_w: u32, win_h: u32) {
        canvas.clear_transparent(Rect::new(0, 0, win_w, win_h));

        // Header
        let header = Rect::new(0, 0, win_w, 44);
        canvas.fill_material(
            header,
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(0)
                .without_border(),
        );
        canvas.draw_rect(Rect::new(0, 43, win_w, 1), theme.chrome.subtle_border);
        draw_text(
            canvas,
            Rect::new(16, 12, win_w - 32, 20),
            "Date & Time",
            theme,
            FontRole::UiTitle,
            theme.text,
        );

        self.draw_current_time(canvas, theme, win_w);
        self.draw_sync_card(canvas, theme, win_w);
        self.draw_timezone_section(canvas, theme, win_w, win_h);

        // Back
        let back = Button::secondary(Self::back_rect(win_w, win_h), "Back");
        draw_button(canvas, theme, back);
    }

    fn draw_current_time(&self, canvas: &mut Canvas, theme: &Theme, win_w: u32) {
        let card = Rect::new(12, 52, win_w.saturating_sub(24), 136);
        canvas.fill_material(
            card,
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(10)
                .without_border(),
        );

        let snap = self.last_snapshot;
        let clock = SolarClockWidget::new(Self::clock_rect(), snap)
            .with_date(self.date_line.as_str());
        clock.draw(canvas, theme);

        let info_x = 160i32;
        let mut y = 60i32;

        // Digital HH:MM via DigitalNumberWidget (updates only with minute change
        // from the controller-supplied snapshot).
        let mut hhmm = [0u8; 5];
        sunlight_ui::widgets::format_hhmm(snap.hour, snap.minute, &mut hhmm);
        let hhmm_str = core::str::from_utf8(&hhmm).unwrap_or("--:--");
        let mut digital = DigitalNumberWidget::new(Rect::new(info_x, y, 120, 28))
            .with_digit_size(12, 20)
            .with_max_chars(5)
            .with_align(DigitalAlign::Left)
            .with_colors(theme.accent, theme.panel_alt.lighten(12));
        let _ = digital.set_value_str(hhmm_str);
        digital.draw(canvas, theme);
        y += 34;

        draw_text(
            canvas,
            Rect::new(info_x, y, 300, 16),
            self.date_line.as_str(),
            theme,
            FontRole::UiRegular,
            theme.text,
        );
        y += 18;

        let zone = if self.applied_zone.is_empty() {
            "—"
        } else {
            self.applied_zone.as_str()
        };
        draw_text(
            canvas,
            Rect::new(info_x, y, 300, 16),
            zone,
            theme,
            FontRole::UiMedium,
            theme.text,
        );
        y += 18;

        draw_text(
            canvas,
            Rect::new(info_x, y, 300, 16),
            self.offset_line.as_str(),
            theme,
            FontRole::UiSmall,
            theme.text_dim,
        );
        y += 16;

        if !self.tz_error.is_empty() {
            draw_text(
                canvas,
                Rect::new(info_x, y, 300, 14),
                self.tz_error.as_str(),
                theme,
                FontRole::UiSmall,
                theme.warn,
            );
        }
    }

    fn draw_sync_card(&self, canvas: &mut Canvas, theme: &Theme, win_w: u32) {
        let card = Self::sync_card_rect(win_w);
        canvas.fill_material(
            card,
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(10)
                .without_border(),
        );

        draw_text(
            canvas,
            Rect::new(card.x + 12, card.y + 8, 160, 16),
            "Time synchronization",
            theme,
            FontRole::UiMedium,
            theme.text,
        );

        let mut line1 = FixedStr::<96>::empty();
        if let Some(st) = self.sync {
            let _ = write!(&mut line1, "{}", st.state_label());
            if st.server_count > 0 {
                let _ = write!(&mut line1, "  ·  {}", st.region_label());
            }
            if !st.last_server_str().is_empty() {
                let _ = write!(&mut line1, "  ·  {}", st.last_server_str());
            }
        } else if !self.timed_error.is_empty() {
            line1.set(self.timed_error.as_str());
        } else {
            line1.set("Status unavailable");
        }
        draw_text(
            canvas,
            Rect::new(card.x + 12, card.y + 28, card.w.saturating_sub(120), 14),
            line1.as_str(),
            theme,
            FontRole::UiSmall,
            theme.text_dim,
        );

        let mut line2 = FixedStr::<96>::empty();
        if let Some(st) = self.sync {
            if st.last_sync_unix != 0 {
                let _ = write!(&mut line2, "Last sync UTC {}", st.last_sync_unix);
            } else {
                line2.set("No successful sync yet");
            }
            if st.last_offset_ms != 0 || st.last_delay_ms != 0 {
                let _ = write!(
                    &mut line2,
                    "  ·  offset {} ms  delay {} ms",
                    st.last_offset_ms, st.last_delay_ms
                );
            }
        }
        if !self.sync_feedback.is_empty() {
            line2.set(self.sync_feedback.as_str());
        }
        if !self.sync_error.is_empty() {
            line2.set(self.sync_error.as_str());
        }
        let line2_color = if !self.sync_error.is_empty() {
            theme.warn
        } else {
            theme.text_dim
        };
        draw_text(
            canvas,
            Rect::new(card.x + 12, card.y + 46, card.w.saturating_sub(120), 14),
            line2.as_str(),
            theme,
            FontRole::UiSmall,
            line2_color,
        );

        let busy = !matches!(self.sync_phase, SyncUiPhase::Idle);
        let mut btn = Button::new(Self::sync_btn_rect(win_w), "Sync now");
        if busy || self.sync.is_none() && !self.timed_error.is_empty() {
            // Allow retry when timed was briefly unavailable; only disable while active.
            if busy {
                btn.state = ButtonState::Disabled;
            }
        }
        if busy {
            btn = Button::new(Self::sync_btn_rect(win_w), "Syncing…");
            btn.state = ButtonState::Disabled;
        }
        draw_button(canvas, theme, btn);
    }

    fn draw_timezone_section(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        win_w: u32,
        win_h: u32,
    ) {
        // 1) Search field (left)
        self.search.draw(
            canvas,
            theme,
            Self::search_rect(win_w),
            "Search cities…",
            Some(&Typography::UI_REGULAR),
        );

        // 2) Result list (left band — clipped rows)
        let results_band = Self::results_band_rect();
        if self.search_hits_len == 0 {
            draw_text_clipped(
                canvas,
                Rect::new(results_band.x + 4, results_band.y + 2, results_band.w - 8, 16),
                "No matching cities",
                FontRole::UiSmall,
                theme.text_dim,
            );
        } else {
            for i in 0..self.search_hits_len.min(SEARCH_VISIBLE) {
                let row = Self::result_row_rect(i);
                if i == self.selected_result {
                    canvas.fill_rounded_rect(row, 4, theme.panel_alt.lighten(10));
                }
                let mut label = FixedStr::<72>::empty();
                let _ = write!(
                    &mut label,
                    "{}, {}",
                    self.search_hits_cities[i].as_str(),
                    self.search_hits_countries[i].as_str()
                );
                draw_text_clipped(
                    canvas,
                    Rect::new(row.x + 6, row.y, row.w.saturating_sub(10), row.h),
                    label.as_str(),
                    FontRole::UiSmall,
                    if i == self.selected_result {
                        theme.accent
                    } else {
                        theme.text
                    },
                );
            }
        }

        // 3) World map (right column — monochrome orange)
        let map_rect = Self::map_rect(win_w);
        let mut markers: [MapMarker; 1] = [MapMarker {
            coord: GeoCoord::new(0.0, 0.0),
            hit_radius: 6,
        }];
        let marker_slice: &[MapMarker] = if let Some(c) = self.proposed_coord {
            markers[0] = MapMarker {
                coord: c,
                hit_radius: 7,
            };
            &markers
        } else {
            &[]
        };
        let mut map = WorldMapWidget::new(map_rect)
            .with_markers(marker_slice)
            .with_grid(false);
        map.land = Some(theme.accent.darken(30));
        map.ocean = Some(theme.panel_alt);
        map.fill_background = true;
        if let Some(c) = self.proposed_coord {
            map = map.with_selected(c);
        }
        map.draw(canvas, theme);

        // 4) Proposed selection preview (full width, single authoritative state)
        let preview = Self::preview_rect(win_w);
        let mut preview_text = FixedStr::<120>::empty();
        if self.proposed_zone.is_empty() {
            preview_text.set("Select a city or click the map");
        } else {
            // city[, country] · zone → HH:MM
            if !self.proposed_city.is_empty() {
                let _ = write!(&mut preview_text, "{}", self.proposed_city.as_str());
                if !self.proposed_country.is_empty() {
                    let _ = write!(&mut preview_text, ", {}", self.proposed_country.as_str());
                }
                let _ = write!(&mut preview_text, " · ");
            }
            let _ = write!(&mut preview_text, "{}", self.proposed_zone.as_str());
            if let Some(entry) = tz_by_id(self.proposed_zone.as_str()) {
                let utc = if let Some(lt) = self.local {
                    local_to_utc_approx(lt)
                } else {
                    0
                };
                if utc != 0 && utc != u64::MAX {
                    let proposed_local = local_now(utc, entry);
                    let _ = write!(
                        &mut preview_text,
                        "  →  {:02}:{:02}",
                        proposed_local.hour, proposed_local.minute
                    );
                }
            }
        }
        draw_text_clipped(
            canvas,
            preview,
            preview_text.as_str(),
            FontRole::UiSmall,
            theme.text,
        );

        // 5) Transient map/search feedback (never overlaps preview)
        if !self.map_feedback.is_empty() {
            draw_text_clipped(
                canvas,
                Self::map_feedback_rect(win_w),
                self.map_feedback.as_str(),
                FontRole::UiSmall,
                theme.text_dim,
            );
        }

        // 6) Apply action feedback (dedicated band above buttons)
        if !self.apply_feedback.is_empty() {
            draw_text_clipped(
                canvas,
                Self::apply_feedback_rect(win_w),
                self.apply_feedback.as_str(),
                FontRole::UiSmall,
                theme.text_dim,
            );
        }

        // 7) Apply button — only for a pending *valid* proposed zone
        let mut apply = Button::new(Self::apply_rect(win_w, win_h), "Apply");
        if !self.pending() || tz_by_id(self.proposed_zone.as_str()).is_none() {
            apply.state = ButtonState::Disabled;
        }
        draw_button(canvas, theme, apply);
    }

    // -----------------------------------------------------------------------
    // Update
    // -----------------------------------------------------------------------

    pub fn update(&mut self, event: Event, win_w: u32, win_h: u32) -> (bool, DateTimeAction) {
        if !self.page_active {
            return (false, DateTimeAction::None);
        }

        if matches!(event, Event::Tick) {
            return (self.on_tick(), DateTimeAction::None);
        }

        match event {
            Event::Key(ch) => {
                if !self.search.active {
                    return (false, DateTimeAction::None);
                }
                if self.search.handle_char(ch) {
                    self.run_search();
                    return (true, DateTimeAction::None);
                }
                (false, DateTimeAction::None)
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } => {
                if keycode == KEY_ESC {
                    if self.search.active {
                        self.search.active = false;
                        return (true, DateTimeAction::None);
                    }
                    return (true, DateTimeAction::Back);
                }
                if self.search.active {
                    if keycode == KEY_BACKSPACE {
                        if self.search.backspace() {
                            self.run_search();
                            return (true, DateTimeAction::None);
                        }
                    } else if keycode == KEY_UP {
                        if self.selected_result > 0 {
                            self.selected_result -= 1;
                            return (true, DateTimeAction::None);
                        }
                    } else if keycode == KEY_DOWN {
                        if self.selected_result + 1 < self.search_hits_len {
                            self.selected_result += 1;
                            return (true, DateTimeAction::None);
                        }
                    } else if keycode == KEY_ENTER {
                        self.propose_from_result(self.selected_result);
                        return (true, DateTimeAction::None);
                    }
                    return (false, DateTimeAction::None);
                }
                if keycode == KEY_ENTER
                    && self.pending()
                    && tz_by_id(self.proposed_zone.as_str()).is_some()
                {
                    return (self.apply_proposed(), DateTimeAction::None);
                }
                (false, DateTimeAction::None)
            }
            Event::Click { x, y } | Event::MouseDown { x, y, button: 0 } => {
                let pt = Point::new(x, y);
                if Self::back_rect(win_w, win_h).contains(pt) {
                    return (true, DateTimeAction::Back);
                }
                if Self::search_rect(win_w).contains(pt) {
                    self.search.active = true;
                    return (true, DateTimeAction::None);
                } else if self.search.active {
                    self.search.active = false;
                }

                if Self::sync_btn_rect(win_w).contains(pt)
                    && matches!(self.sync_phase, SyncUiPhase::Idle)
                {
                    return (self.queue_sync(), DateTimeAction::None);
                }

                for i in 0..self.search_hits_len.min(SEARCH_VISIBLE) {
                    if Self::result_row_rect(i).contains(pt) {
                        self.propose_from_result(i);
                        return (true, DateTimeAction::None);
                    }
                }

                // Map hit
                let map = WorldMapWidget::new(Self::map_rect(win_w));
                match map.hit_test(pt) {
                    MapHit::Inside { coord, .. } => {
                        self.handle_map_click(coord);
                        return (true, DateTimeAction::None);
                    }
                    MapHit::Outside => {}
                }

                if Self::apply_rect(win_w, win_h).contains(pt)
                    && self.pending()
                    && tz_by_id(self.proposed_zone.as_str()).is_some()
                {
                    return (self.apply_proposed(), DateTimeAction::None);
                }

                (false, DateTimeAction::None)
            }
            _ => (false, DateTimeAction::None),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn draw_button(canvas: &mut Canvas, theme: &Theme, button: Button<'_>) {
    button.with_font(&Typography::UI_MEDIUM).draw(canvas, theme);
}

fn draw_text(
    canvas: &mut Canvas,
    rect: Rect,
    text: &str,
    _theme: &Theme,
    role: FontRole,
    color: Color,
) {
    draw_text_clipped(canvas, rect, text, role, color);
}

/// Draw left-aligned text vertically centred in `rect`, truncating with an
/// ellipsis so glyphs never paint outside the allocated band.
fn draw_text_clipped(
    canvas: &mut Canvas,
    rect: Rect,
    text: &str,
    role: FontRole,
    color: Color,
) {
    if rect.w == 0 || rect.h == 0 || text.is_empty() {
        return;
    }
    let style = TextStyle::new(role, color);
    let max_w = rect.w;
    let fit = fit_text_ellipsis(text, role, max_w);
    sun_font::draw_text_vcenter(canvas, fit.as_str(), rect.x, rect.y, rect.h, &style);
}

/// Longest prefix of `text` that fits `max_w`, with "…" when truncated.
fn fit_text_ellipsis(text: &str, role: FontRole, max_w: u32) -> FixedStr<128> {
    let mut out = FixedStr::<128>::empty();
    if sun_font::measure_text(text, role).w <= max_w {
        out.set(text);
        return out;
    }
    let ell = "…";
    let ell_w = sun_font::measure_text(ell, role).w;
    if ell_w >= max_w {
        return out;
    }
    let budget = max_w.saturating_sub(ell_w);
    let bytes = text.as_bytes();
    let mut best = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        i += 1;
        while i < bytes.len() && !text.is_char_boundary(i) {
            i += 1;
        }
        if sun_font::measure_text(&text[..i], role).w <= budget {
            best = i;
        } else {
            break;
        }
    }
    if best == 0 {
        out.set(ell);
    } else {
        out.set(&text[..best]);
        out.push_str(ell);
    }
    out
}

fn format_offset_line(out: &mut FixedStr<40>, offset_secs: i64, is_dst: bool) {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    let oh = abs / 3600;
    let om = (abs % 3600) / 60;
    let _ = write!(out, "UTC{}{:02}:{:02}", sign, oh, om);
    if is_dst {
        let _ = write!(out, "  ·  DST");
    } else {
        let _ = write!(out, "  ·  Standard");
    }
}

/// Approximate UTC epoch from a local-time snapshot (reverse of applied offset).
fn local_to_utc_approx(lt: LocalTimeSnapshot) -> u64 {
    // Prefer timed UTC when available.
    if let Ok(client) = TimeClient::connect() {
        if let Ok(utc) = client.get_utc() {
            if utc != 0 && utc != u64::MAX {
                return utc;
            }
        }
    }
    // Fallback: reconstruct civil local as if UTC then subtract offset — rough.
    // days from a simple packing is not available; use offset reverse only when
    // we already have seconds-of-day style via IPC words. Without UTC, return 0
    // so local_now yields a still-useful zone-relative display of epoch 0.
    let _ = lt;
    0
}

fn tz_err_label(e: TzClientError) -> &'static str {
    match e {
        TzClientError::ServiceUnavailable => "Timezone service unavailable",
        TzClientError::Timeout => "Timezone service timeout",
        TzClientError::Transport => "Timezone transport error",
        TzClientError::UnsupportedZone => "Unsupported timezone",
        TzClientError::PersistFailed => "Could not save timezone",
        TzClientError::Failed(_) => "Timezone operation failed",
        TzClientError::Unexpected => "Unexpected timezone reply",
    }
}

fn time_err_label(e: TimeClientError) -> &'static str {
    match e {
        TimeClientError::ServiceUnavailable => "Time service unavailable",
        TimeClientError::Timeout => "Sync timed out — check network and try again",
        TimeClientError::Transport => "Time service transport error",
        TimeClientError::Failed(code) => match code {
            sunlight_ipc::TimeMsg::ERR_NETWORK => "No network for time sync",
            sunlight_ipc::TimeMsg::ERR_DNS => "Could not resolve NTP server",
            sunlight_ipc::TimeMsg::ERR_TIMEOUT => "NTP server timed out",
            sunlight_ipc::TimeMsg::ERR_PERMISSION => "Permission denied for forced sync",
            sunlight_ipc::TimeMsg::ERR_BUSY => "Sync already in progress",
            sunlight_ipc::TimeMsg::ERR_CLOCK_UPDATE => "Could not update system clock",
            sunlight_ipc::TimeMsg::ERR_VALIDATION => "NTP response rejected",
            _ => "Synchronization failed",
        },
        TimeClientError::Unexpected => "Unexpected time service reply",
    }
}
