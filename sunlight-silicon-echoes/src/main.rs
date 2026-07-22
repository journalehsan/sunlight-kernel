#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use alloc::{boxed::Box, string::String};
use core::sync::atomic::{AtomicBool, Ordering};

use sun_font::{draw_text, draw_text_vcenter, measure_text, FontRole, TextStyle};
use sunlight_ipc::{
    debug_log, ipc_call_timeout, monotonic_millis, nameserver_lookup_timeout, process_yield,
    shm_alloc, shm_free, shm_map, CapabilityToken, IpcMsg, ProcessExit, SHM_PAGE,
};
use sunlight_libc::{self as libc, rand::getrandom, GRND_NONCRYPTO};
use sunlight_silicon_echoes::{
    chapter_two_consequence_summary, chapter_two_turning_point_layout, choice_row_height,
    decode_save, echo_object_is_active, echo_objects, encode_save, hotspot, layout_choice_rows,
    node, presentation_narration, run_deterministic_stress, validate_graph, ChoiceId,
    EchoLayer, GameState, HotspotId, NarrativePresentation, PresentationConfig, SaveError,
    SaveStage, SceneId, ScenePresentation, ShortcutGate, StoryNodeId, Transition,
    CHOICE_LINE_HEIGHT, CHOICE_TEXT_INSET_LEFT, CHOICE_TEXT_INSET_RIGHT,
};
use sunlight_ui::{
    request_close, set_client_cursor, App, Canvas, Color, CursorShape, Event, Point, Rect, Theme,
    Window, WindowConfig, WindowDecoration,
};

const WIN_W: u32 = 1080;
const WIN_H: u32 = 720;
const KEY_ESC: u8 = 0x01;
const KEY_ENTER: u8 = 0x1c;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_TAB: u8 = 0x0F;

const OBSIDIAN: Color = Color::rgb(0x0A, 0x0A, 0x0C);
const BONE: Color = Color::rgb(0xED, 0xE6, 0xD8);
const SUNLIGHT: Color = Color::rgb(0xFF, 0x98, 0x00);
const NARRATIVE_LINE_HEIGHT: i32 = 24;
const NARRATIVE_TEXT_TOP: i32 = 38;
const SAVE_SLOT_A_KEY: &str = "games/silicon-echoes/save.a";
const SAVE_SLOT_B_KEY: &str = "games/silicon-echoes/save.b";
const KV_REPLY: u64 = 0x4BFF;
const KV_ERROR: u64 = 0x4BEE;
const KV_VALUE: u64 = 0x4B05;
const KV_PUT_SHM2: u64 = 0x4B08;
const KV_GET_SHM2: u64 = 0x4B09;
const KV_LOOKUP_TIMEOUT_MS: u64 = 250;
const KV_TIMEOUT_MS: u64 = 250;

static KV_LOOKED_UP: AtomicBool = AtomicBool::new(false);
static mut KV_CAP: CapabilityToken = CapabilityToken::INVALID;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Title,
    Play,
    Ending,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hover {
    None,
    TitleNew,
    TitleContinue,
    Choice(usize),
    Object(SceneObjectTarget),
    ReturnTitle,
    ContinueChapterTwo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SceneObjectTarget {
    Hotspot(HotspotId),
    Choice(ChoiceId),
    Overlay,
}

#[derive(Clone, Copy)]
struct Layout {
    frame: Rect,
    image: Rect,
    narrative: Rect,
    choices: Rect,
    title_new: Rect,
    title_continue: Rect,
    return_title: Rect,
    continue_chapter_two: Rect,
}

impl Layout {
    fn new(width: u32, height: u32) -> Self {
        let frame = Rect::new(22, 20, width.saturating_sub(44), height.saturating_sub(40));
        let image_h = (frame.h as i32 * 57 / 100).max(260) as u32;
        let image = Rect::new(
            frame.x + 18,
            frame.y + 18,
            frame.w.saturating_sub(36),
            image_h,
        );
        let narrative_y = image.bottom() + 16;
        let narrative = Rect::new(
            image.x,
            narrative_y,
            image.w,
            (frame.bottom() - narrative_y - 18).max(80) as u32,
        );
        let choices = Rect::new(
            narrative.x + narrative.w as i32 * 55 / 100,
            narrative.y + 14,
            narrative.w.saturating_mul(45) / 100 - 14,
            narrative.h.saturating_sub(28),
        );
        let title_new = Rect::new(
            image.x + image.w as i32 / 2 - 100,
            image.y + image.h as i32 - 128,
            200,
            34,
        );
        let title_continue = Rect::new(title_new.x, title_new.bottom() + 10, 200, 34);
        let return_title = Rect::new(
            image.x + image.w as i32 / 2 - 100,
            image.y + image.h as i32 - 56,
            200,
            34,
        );
        let continue_chapter_two = Rect::new(
            image.x + image.w as i32 / 2 - 124,
            image.y + image.h as i32 - 98,
            248,
            34,
        );
        Self {
            frame,
            image,
            narrative,
            choices,
            title_new,
            title_continue,
            return_title,
            continue_chapter_two,
        }
    }

    fn bedroom_hotspot_rect(&self, hotspot: HotspotId) -> Rect {
        match hotspot {
            HotspotId::Clock => Rect::new(
                self.image.x + self.image.w as i32 * 13 / 100,
                self.image.y + self.image.h as i32 * 21 / 100,
                self.image.w.saturating_mul(14) / 100,
                self.image.h.saturating_mul(17) / 100,
            ),
            HotspotId::Workstation => Rect::new(
                self.image.x + self.image.w as i32 * 56 / 100,
                self.image.y + self.image.h as i32 * 38 / 100,
                self.image.w.saturating_mul(24) / 100,
                self.image.h.saturating_mul(34) / 100,
            ),
            HotspotId::Desk => Rect::new(
                self.image.x + self.image.w as i32 * 44 / 100,
                self.image.y + self.image.h as i32 * 64 / 100,
                self.image.w.saturating_mul(38) / 100,
                self.image.h.saturating_mul(17) / 100,
            ),
            HotspotId::Window => Rect::new(
                self.image.x + self.image.w as i32 * 70 / 100,
                self.image.y + self.image.h as i32 * 12 / 100,
                self.image.w.saturating_mul(19) / 100,
                self.image.h.saturating_mul(35) / 100,
            ),
        }
    }
}

struct SiliconEchoesApp {
    mode: Mode,
    game: GameState,
    saved_game: Option<GameState>,
    selected_choice: usize,
    hover: Hover,
    layout: Layout,
    save_notice: &'static str,
    save_stage: Option<SaveStage>,
    started_ms: u64,
    last_tick_ms: u64,
    ambient_seed: u32,
    focused: bool,
    suppress_next_click: bool,
    presentation: NarrativePresentation,
    shortcut_gate: ShortcutGate,
    selected_hotspot: usize,
    key_down: [bool; 256],
    scene_cache: Option<Box<SceneCache>>,
}

struct SceneCache {
    node: StoryNodeId,
    narration: String,
    wrap_width: i32,
    lines: Vec<TextLine>,
}

#[derive(Clone, Copy)]
struct TextLine {
    start: usize,
    end: usize,
}

struct SurfaceStressApp {
    frames_left: u8,
}

impl App for SurfaceStressApp {
    fn view(&mut self, canvas: &mut Canvas, _: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, canvas.width, canvas.height), OBSIDIAN);
        canvas.blend_rounded_rect(
            Rect::new(
                24,
                24,
                canvas.width.saturating_sub(48),
                canvas.height.saturating_sub(48),
            ),
            10,
            Color::rgba(0xED, 0xE6, 0xD8, 24),
        );
        draw_center(
            canvas,
            Rect::new(0, canvas.height as i32 / 2 - 12, canvas.width, 24),
            "SILICON ECHOES SURFACE CHECK",
            FontRole::UiMedium,
            SUNLIGHT,
        );
    }

    fn update(&mut self, event: Event) -> bool {
        if matches!(event, Event::Tick) {
            if self.frames_left == 0 {
                request_close();
            } else {
                self.frames_left -= 1;
            }
            return true;
        }
        false
    }

    fn poll_timeout_ms(&self) -> u64 {
        16
    }
}

impl SiliconEchoesApp {
    fn new() -> Self {
        Self {
            mode: Mode::Title,
            game: GameState::new(),
            saved_game: None,
            selected_choice: 0,
            hover: Hover::None,
            layout: Layout::new(WIN_W, WIN_H),
            save_notice: "",
            save_stage: None,
            started_ms: monotonic_millis(),
            last_tick_ms: 0,
            ambient_seed: 0x1993_0317,
            focused: true,
            suppress_next_click: false,
            presentation: NarrativePresentation::new(
                String::new(),
                monotonic_millis(),
                PresentationConfig::default(),
            ),
            shortcut_gate: ShortcutGate::default(),
            selected_hotspot: 0,
            key_down: [false; 256],
            scene_cache: None,
        }
    }

    fn refresh_scene_cache(&mut self) {
        let narration = node(self.game.current_node)
            .map(|story_node| presentation_narration(&self.game, story_node))
            .unwrap_or_default();
        self.presentation = NarrativePresentation::new(
            narration.clone(),
            monotonic_millis(),
            PresentationConfig::default(),
        );
        self.scene_cache = Some(Box::new(SceneCache {
            node: self.game.current_node,
            narration,
            wrap_width: 0,
            lines: Vec::new(),
        }));
        self.selected_choice = self
            .available_choice_indices()
            .first()
            .copied()
            .unwrap_or(0);
        self.selected_hotspot = 0;
        self.shortcut_gate.block_until_quiet(monotonic_millis());
    }

    fn load_save(&mut self) {
        let slots = [kv_get(SAVE_SLOT_A_KEY), kv_get(SAVE_SLOT_B_KEY)];
        let mut newest = None;
        let mut saw_damaged_record = false;
        let mut service_failed = false;
        for slot in slots {
            match slot {
                Ok(Some(bytes)) => match decode_save(&bytes) {
                    Ok(state) => {
                        if newest
                            .as_ref()
                            .map(|saved: &GameState| saved.save_generation < state.save_generation)
                            .unwrap_or(true)
                        {
                            newest = Some(state);
                        }
                    }
                    Err(
                        SaveError::InvalidRecord
                        | SaveError::InvalidUtf8
                        | SaveError::UnsupportedVersion
                        | SaveError::TooLarge,
                    ) => saw_damaged_record = true,
                },
                Ok(None) => {}
                Err(_) => service_failed = true,
            }
        }
        self.saved_game = newest;
        self.save_notice = if self.saved_game.is_some() {
            ""
        } else if service_failed {
            "Save service is asleep. This session remains playable."
        } else if saw_damaged_record {
            "No valid save could be loaded; the damaged echo was left untouched."
        } else {
            ""
        };
    }

    fn next_save_slot(&self) -> &'static str {
        match self
            .saved_game
            .as_ref()
            .map(|state| state.save_generation % 2)
            .unwrap_or(1)
        {
            0 => SAVE_SLOT_A_KEY,
            _ => SAVE_SLOT_B_KEY,
        }
    }

    fn seed_ambient(&mut self) {
        let mut bytes = [0u8; 4];
        if getrandom(&mut bytes, GRND_NONCRYPTO) == bytes.len() as isize {
            self.ambient_seed = u32::from_le_bytes(bytes);
        }
    }

    fn start_new(&mut self) {
        self.game = GameState::new();
        self.mode = Mode::Play;
        self.selected_choice = 0;
        self.refresh_scene_cache();
        self.save_game();
    }

    fn continue_game(&mut self) {
        if let Some(saved) = self.saved_game.clone() {
            self.game = saved;
            self.mode = Mode::Play;
            self.selected_choice = 0;
            self.refresh_scene_cache();
        }
    }

    fn save_game(&mut self) {
        if self.mode != Mode::Play {
            return;
        }
        self.game.play_time_ms = self
            .game
            .play_time_ms
            .saturating_add(monotonic_millis().saturating_sub(self.started_ms));
        self.started_ms = monotonic_millis();
        self.game.save_generation = self
            .saved_game
            .as_ref()
            .map(|state| state.save_generation)
            .unwrap_or(self.game.save_generation)
            .saturating_add(1);
        let bytes = encode_save(&self.game);
        if bytes.len() > SHM_PAGE {
            self.save_stage = Some(SaveStage::RequestTooLarge);
            debug_log("[SILICON-ECHOES] save_game: encoded size exceeds SHM_PAGE\n");
            self.save_notice =
                "Save service did not confirm this echo; the previous save remains intact.";
            return;
        }
        let slot = self.next_save_slot();
        match kv_put(slot, &bytes).and_then(|()| verify_saved_snapshot(slot, &self.game)) {
            Ok(()) => {
                self.saved_game = Some(self.game.clone());
                self.save_notice = "Saved";
                self.save_stage = None;
            }
            Err(stage) => {
                self.save_stage = Some(stage);
                debug_log("[SILICON-ECHOES] save failed at stage\n");
                self.save_notice =
                    "Save service did not confirm this echo; the previous save remains intact.";
            }
        }
    }

    fn return_to_title(&mut self) {
        self.mode = Mode::Title;
        self.hover = Hover::None;
        self.selected_choice = 0;
        self.load_save();
    }

    fn continue_chapter_two(&mut self) {
        if self.game.begin_chapter_two().is_ok() {
            self.mode = Mode::Play;
            self.selected_choice = 0;
            self.refresh_scene_cache();
            self.save_game();
        }
    }

    fn choices(&self) -> &'static [sunlight_silicon_echoes::Choice] {
        node(self.game.current_node)
            .map(|story_node| story_node.choices)
            .unwrap_or(&[])
    }

    fn available_choice_indices(&self) -> Vec<usize> {
        self.choices()
            .iter()
            .enumerate()
            .filter_map(|(index, choice)| {
                choice
                    .condition
                    .map(|condition| sunlight_silicon_echoes::condition_met(condition, &self.game))
                    .unwrap_or(true)
                    .then_some(index)
            })
            .collect()
    }

    fn available_hotspots(&self) -> Vec<HotspotId> {
        [
            HotspotId::Clock,
            HotspotId::Workstation,
            HotspotId::Desk,
            HotspotId::Window,
        ]
        .into_iter()
        .filter(|hotspot| self.can_inspect(*hotspot))
        .collect()
    }

    fn presentation_accepts_input(&self) -> bool {
        self.presentation.state() == ScenePresentation::AwaitingChoice
    }

    fn activate_choice(&mut self, index: usize) {
        if !self.presentation_accepts_input()
            || !self
                .available_choice_indices()
                .iter()
                .any(|visible| *visible == index)
        {
            return;
        }
        let Some(choice) = self.choices().get(index) else {
            return;
        };
        self.presentation.begin_transition();
        let target = self.game.select_choice(choice.id);
        match target {
            Ok(Transition::Ending(_)) => {
                self.save_game();
                self.mode = Mode::Ending;
            }
            Ok(Transition::Node(_)) => {
                self.selected_choice = 0;
                self.refresh_scene_cache();
                self.save_game();
            }
            Err(_) => {}
        }
    }

    fn advance_uncontrolled_event(&mut self) {
        if !self.presentation_accepts_input() {
            return;
        }
        self.presentation.begin_transition();
        if self.game.advance_uncontrolled_event().is_ok() {
            self.selected_choice = 0;
            self.refresh_scene_cache();
            self.save_game();
        }
    }

    fn hit_test(&self, x: i32, y: i32) -> Hover {
        let point = Point::new(x, y);
        match self.mode {
            Mode::Title => {
                if self.layout.title_new.contains(point) {
                    Hover::TitleNew
                } else if self.saved_game.is_some() && self.layout.title_continue.contains(point) {
                    Hover::TitleContinue
                } else {
                    Hover::None
                }
            }
            Mode::Ending => {
                if self.game.chapter == 1 && self.layout.continue_chapter_two.contains(point) {
                    Hover::ContinueChapterTwo
                } else if self.ending_return_rect().contains(point) {
                    Hover::ReturnTitle
                } else {
                    Hover::None
                }
            }
            Mode::Play => {
                if !self.presentation_accepts_input() {
                    return Hover::None;
                }
                for (target, rect) in self.scene_object_bounds() {
                    if self.object_is_available(target) && rect.contains(point) {
                        return Hover::Object(target);
                    }
                }
                for (visible_index, choice_index) in
                    self.available_choice_indices().iter().enumerate()
                {
                    if self.choice_rect(visible_index).contains(point) {
                        return Hover::Choice(*choice_index);
                    }
                }
                if self.current_node_is_uncontrolled() && self.layout.choices.contains(point) {
                    Hover::Choice(0)
                } else {
                    Hover::None
                }
            }
        }
    }

    fn can_inspect(&self, hotspot_id: HotspotId) -> bool {
        let condition_met = hotspot(hotspot_id)
            .condition
            .map(|condition| sunlight_silicon_echoes::condition_met(condition, &self.game))
            .unwrap_or(true);
        condition_met
            && (self.game.current_node == StoryNodeId("bedroom.wake")
                || node(self.game.current_node)
                    .map(|story_node| story_node.uncontrolled_event)
                    .unwrap_or(false)
                || matches!(hotspot_id, HotspotId::Window) && self.game.flags.get("saw_date"))
    }

    fn object_is_available(&self, target: SceneObjectTarget) -> bool {
        match target {
            SceneObjectTarget::Hotspot(hotspot) => self.can_inspect(hotspot),
            SceneObjectTarget::Choice(choice_id) => self
                .game
                .available_actions()
                .iter()
                .any(|action| action.id == choice_id),
            SceneObjectTarget::Overlay => self.game.supports_echo_overlay(),
        }
    }

    fn scene_object_bounds(&self) -> Vec<(SceneObjectTarget, Rect)> {
        let scene_id = node(self.game.current_node)
            .map(|story_node| story_node.scene)
            .unwrap_or(SceneId("bedroom"));
        if scene_id == SceneId("bedroom") {
            return [
                HotspotId::Clock,
                HotspotId::Workstation,
                HotspotId::Desk,
                HotspotId::Window,
            ]
            .into_iter()
            .map(|hotspot| {
                (
                    SceneObjectTarget::Hotspot(hotspot),
                    self.layout.bedroom_hotspot_rect(hotspot),
                )
            })
            .collect();
        }
        let mut bounds = scene_object_bounds(scene_id, self.layout.image);
        if self.game.supports_echo_overlay() {
            bounds.retain(|(target, _)| match target {
                SceneObjectTarget::Choice(choice_id) => echo_objects(scene_id)
                    .iter()
                    .find(|object| object.action == Some(*choice_id))
                    .map(|object| echo_object_is_active(object, self.game.echo_layer))
                    .unwrap_or(true),
                _ => true,
            });
            bounds.push((
                SceneObjectTarget::Overlay,
                Rect::new(
                    self.layout.image.right() - 184,
                    self.layout.image.y + 52,
                    144,
                    30,
                ),
            ));
        }
        bounds
    }

    fn current_node_is_uncontrolled(&self) -> bool {
        node(self.game.current_node)
            .map(|story_node| story_node.uncontrolled_event)
            .unwrap_or(false)
    }

    fn window_size(&self) -> (u32, u32) {
        (
            self.layout.frame.w.saturating_add(44),
            self.layout.frame.h.saturating_add(40),
        )
    }

    fn ending_return_rect(&self) -> Rect {
        if self.game.chapter == 2 {
            let (win_w, win_h) = self.window_size();
            let tp = chapter_two_turning_point_layout(win_w, win_h);
            Rect::new(
                tp.return_button.0,
                tp.return_button.1,
                tp.return_button.2 as u32,
                tp.return_button.3 as u32,
            )
        } else {
            self.layout.return_title
        }
    }

    fn choice_text_width(&self) -> i32 {
        (self.layout.choices.w as i32 - CHOICE_TEXT_INSET_LEFT - CHOICE_TEXT_INSET_RIGHT).max(40)
    }

    fn choice_line_count(&self, text: &str) -> usize {
        let lines = prepare_text_lines(text, self.choice_text_width(), FontRole::UiRegular);
        let count = lines
            .iter()
            .filter(|line| line.end > line.start)
            .count();
        count.max(1)
    }

    fn visible_choice_texts(&self) -> Vec<&'static str> {
        let available = self.available_choice_indices();
        if self.current_node_is_uncontrolled() {
            return vec!["LET THE ROOM MOVE ON"];
        }
        if available.is_empty() {
            return self
                .available_hotspots()
                .iter()
                .map(|hotspot| hotspot_label(*hotspot))
                .collect();
        }
        available
            .iter()
            .filter_map(|index| self.choices().get(*index).map(|choice| choice.text))
            .collect()
    }

    fn choice_rects(&self) -> Vec<Rect> {
        let texts = self.visible_choice_texts();
        let line_counts: Vec<usize> = texts
            .iter()
            .map(|text| self.choice_line_count(text))
            .collect();
        layout_choice_rows(
            &line_counts,
            self.layout.choices.x,
            self.layout.choices.y,
            self.layout.choices.w,
            self.layout.choices.h as i32,
        )
        .into_iter()
        .map(|(x, y, w, h)| Rect::new(x, y, w, h))
        .collect()
    }

    fn choice_rect(&self, visible_index: usize) -> Rect {
        self.choice_rects()
            .get(visible_index)
            .copied()
            .unwrap_or_else(|| {
                Rect::new(
                    self.layout.choices.x,
                    self.layout.choices.y + visible_index as i32 * choice_row_height(1),
                    self.layout.choices.w,
                    choice_row_height(1) as u32,
                )
            })
    }

    fn activate_hover(&mut self, hover: Hover) {
        if self.mode == Mode::Play && !self.presentation_accepts_input() {
            return;
        }
        match hover {
            Hover::TitleNew => self.start_new(),
            Hover::TitleContinue => self.continue_game(),
            Hover::Object(SceneObjectTarget::Hotspot(hotspot)) => {
                self.presentation.begin_transition();
                self.game.enter_hotspot(hotspot);
                self.selected_choice = 0;
                self.refresh_scene_cache();
                self.save_game();
            }
            Hover::Object(SceneObjectTarget::Choice(choice_id)) => {
                if let Some(index) = self
                    .choices()
                    .iter()
                    .position(|choice| choice.id == choice_id)
                {
                    self.selected_choice = index;
                    self.activate_choice(index);
                }
            }
            Hover::Object(SceneObjectTarget::Overlay) => {
                if self.game.toggle_echo_layer() {
                    self.save_game();
                }
            }
            Hover::Choice(_) if self.current_node_is_uncontrolled() => {
                self.advance_uncontrolled_event()
            }
            Hover::Choice(index) => self.activate_choice(index),
            Hover::ReturnTitle => self.return_to_title(),
            Hover::ContinueChapterTwo => self.continue_chapter_two(),
            Hover::None => {}
        }
    }

    fn select_next_choice(&mut self, direction: i32) {
        if !self.presentation_accepts_input() {
            return;
        }
        let available = self.available_choice_indices();
        if available.is_empty() {
            let hotspots = self.available_hotspots();
            if !hotspots.is_empty() {
                self.selected_hotspot = (self.selected_hotspot as i32 + direction)
                    .rem_euclid(hotspots.len() as i32)
                    as usize;
            }
            return;
        }
        let current = available
            .iter()
            .position(|index| *index == self.selected_choice)
            .unwrap_or(0) as i32;
        let next = (current + direction).rem_euclid(available.len() as i32) as usize;
        self.selected_choice = available[next];
    }

    fn keyboard_activate(&mut self, now_ms: u64) {
        match self.mode {
            Mode::Title => {
                if self.saved_game.is_some() {
                    self.continue_game();
                } else {
                    self.start_new();
                }
            }
            Mode::Ending if self.game.chapter == 1 => self.continue_chapter_two(),
            Mode::Ending => self.return_to_title(),
            Mode::Play if self.presentation.is_revealing() => {
                self.presentation.skip_reveal(now_ms);
            }
            Mode::Play if !self.presentation_accepts_input() => {}
            Mode::Play if self.current_node_is_uncontrolled() => self.advance_uncontrolled_event(),
            Mode::Play if self.available_choice_indices().is_empty() => {
                if let Some(hotspot) = self
                    .available_hotspots()
                    .get(self.selected_hotspot)
                    .copied()
                {
                    self.presentation.begin_transition();
                    self.game.enter_hotspot(hotspot);
                    self.refresh_scene_cache();
                    self.save_game();
                }
            }
            Mode::Play => self.activate_choice(self.selected_choice),
        }
    }

    fn activate_shortcut(&mut self, index: usize) {
        if !self.presentation_accepts_input() {
            return;
        }
        let available = self.available_choice_indices();
        if let Some(choice_index) = available.get(index).copied() {
            self.selected_choice = choice_index;
            self.activate_choice(choice_index);
            return;
        }
        if available.is_empty() {
            if let Some(hotspot) = self.available_hotspots().get(index).copied() {
                self.selected_hotspot = index;
                self.presentation.begin_transition();
                self.game.enter_hotspot(hotspot);
                self.refresh_scene_cache();
                self.save_game();
            }
        }
    }

    fn back(&mut self) {
        match self.mode {
            Mode::Title => request_close(),
            Mode::Ending => self.return_to_title(),
            Mode::Play => {
                self.save_game();
                self.return_to_title();
            }
        }
    }

    fn ensure_narrative_layout(&mut self) {
        // Keep prose near 55–80 characters at the default window width by using
        // just over half the narrative band; choices occupy the remaining side.
        let width = self.layout.narrative.w as i32 * 50 / 100;
        let Some(cache) = self.scene_cache.as_mut() else {
            return;
        };
        if cache.node != self.game.current_node {
            return;
        }
        if cache.wrap_width == width && !cache.lines.is_empty() {
            return;
        }
        cache.lines = prepare_text_lines(&cache.narration, width, FontRole::SerifRegular);
        cache.wrap_width = width;
    }

    fn draw_frame(&self, canvas: &mut Canvas) {
        canvas.fill_rect(Rect::new(0, 0, canvas.width, canvas.height), OBSIDIAN);
        canvas.draw_rect(self.layout.frame, Color::rgba(0xED, 0xE6, 0xD8, 112));
        canvas.fill_rect(
            Rect::new(
                self.layout.frame.x + 1,
                self.layout.frame.y + 1,
                self.layout.frame.w - 2,
                3,
            ),
            SUNLIGHT,
        );
    }

    fn draw_room(&self, canvas: &mut Canvas) {
        let scene_id = node(self.game.current_node)
            .map(|story_node| story_node.scene.0)
            .unwrap_or("bedroom");
        if scene_id == "bedroom" {
            self.draw_bedroom(canvas);
        } else {
            self.draw_chapter_scene(canvas, scene_id);
        }
    }

    fn draw_bedroom(&self, canvas: &mut Canvas) {
        let rect = self.layout.image;
        canvas.fill_rect(rect, Color::rgb(0x0A, 0x0A, 0x0C));
        let wall = Rect::new(rect.x + 12, rect.y + 12, rect.w - 24, rect.h * 67 / 100);
        let floor = Rect::new(
            rect.x + 12,
            wall.bottom(),
            rect.w - 24,
            rect.bottom() as u32 - wall.bottom() as u32 - 12,
        );
        canvas.blend_rect(wall, Color::rgba(0xED, 0xE6, 0xD8, 30));
        canvas.blend_rect(floor, Color::rgba(0xED, 0xE6, 0xD8, 18));

        let window = self.layout.bedroom_hotspot_rect(HotspotId::Window);
        canvas.blend_rect(window, Color::rgba(0xED, 0xE6, 0xD8, 30));
        canvas.draw_rect(window, BONE);
        canvas.vline(
            window.x + window.w as i32 / 2,
            window.y,
            window.h,
            Color::rgba(0xED, 0xE6, 0xD8, 150),
        );
        canvas.hline(
            window.x,
            window.y + window.h as i32 / 2,
            window.w,
            Color::rgba(0xED, 0xE6, 0xD8, 150),
        );
        for index in 0..6 {
            let x = window.x
                + 8
                + ((self.ambient_seed.wrapping_add(index as u32 * 71) % window.w.max(12)) as i32);
            let y = window.y
                + 8
                + ((self.ambient_seed.rotate_left(index as u32) % window.h.max(12)) as i32);
            canvas.blend_pixel(x, y, Color::rgba(0xFF, 0x98, 0x00, 90));
        }

        let clock = self.layout.bedroom_hotspot_rect(HotspotId::Clock);
        canvas.blend_rounded_rect(clock, 6, Color::rgba(0xED, 0xE6, 0xD8, 36));
        canvas.stroke_rounded_rect(clock, 6, 1, BONE);
        draw_center(canvas, clock, "03:17", FontRole::MonoMedium, BONE);
        draw_center(
            canvas,
            Rect::new(clock.x, clock.bottom() - 16, clock.w, 13),
            "1993",
            FontRole::UiSmall,
            SUNLIGHT,
        );

        let desk = self.layout.bedroom_hotspot_rect(HotspotId::Desk);
        canvas.blend_rect(desk, Color::rgba(0xED, 0xE6, 0xD8, 52));
        canvas.hbar(desk.x - 5, desk.bottom() - 5, desk.w + 10, 5, BONE);
        canvas.vline(desk.x + 18, desk.bottom(), 48, BONE);
        canvas.vline(desk.right() - 18, desk.bottom(), 48, BONE);
        let letter = Rect::new(
            desk.x + desk.w as i32 / 8,
            desk.y + 10,
            desk.w / 4,
            desk.h / 2,
        );
        canvas.fill_rect(letter, BONE);
        canvas.draw_rect(letter, SUNLIGHT);

        let workstation = self.layout.bedroom_hotspot_rect(HotspotId::Workstation);
        let crt = Rect::new(
            workstation.x,
            workstation.y,
            workstation.w,
            workstation.h * 66 / 100,
        );
        let glow = Rect::new(crt.x - 5, crt.y - 5, crt.w + 10, crt.h + 10);
        canvas.blend_rounded_rect(glow, 14, Color::rgba(0xFF, 0x98, 0x00, 20));
        canvas.blend_rounded_rect(crt, 10, Color::rgba(0xED, 0xE6, 0xD8, 120));
        canvas.stroke_rounded_rect(crt, 10, 2, BONE);
        let screen = crt.inset(10);
        canvas.fill_rounded_rect(screen, 5, OBSIDIAN);
        let flicker = ((monotonic_millis() / 120 + self.ambient_seed as u64) & 3) as u8;
        canvas.blend_rect(
            Rect::new(screen.x + 9, screen.y + 11, screen.w.saturating_sub(18), 2),
            Color::rgba(0xFF, 0x98, 0x00, 42 + flicker * 14),
        );
        draw_text(
            canvas,
            "HELLO, MARA_",
            screen.x + 10,
            screen.y + 24,
            &TextStyle::new(FontRole::MonoRegular, SUNLIGHT),
        );
        canvas.fill_rect(
            Rect::new(crt.x + crt.w as i32 / 2 - 9, crt.bottom(), 18, 18),
            BONE,
        );
        canvas.fill_rounded_rect(
            Rect::new(crt.x + crt.w as i32 / 2 - 42, crt.bottom() + 16, 84, 12),
            5,
            BONE,
        );

        canvas.draw_rect(rect, Color::rgba(0xED, 0xE6, 0xD8, 160));
        self.draw_hotspot_feedback(canvas);
    }

    fn draw_chapter_scene(&self, canvas: &mut Canvas, scene_id: &str) {
        let rect = self.layout.image;
        canvas.fill_rect(rect, OBSIDIAN);
        let wall = Rect::new(rect.x + 12, rect.y + 12, rect.w - 24, rect.h * 70 / 100);
        let floor = Rect::new(
            rect.x + 12,
            wall.bottom(),
            rect.w - 24,
            rect.bottom() as u32 - wall.bottom() as u32 - 12,
        );
        canvas.blend_rect(wall, Color::rgba(0xED, 0xE6, 0xD8, 26));
        canvas.blend_rect(floor, Color::rgba(0xED, 0xE6, 0xD8, 14));
        let label = match scene_id {
            "hallway" => "FOURTH FLOOR",
            "kitchen" => "MORNING PAPER / 1993",
            "landing" => "APPOINTMENT / 04:00",
            "stairwell" => "STAIRS / DOWN",
            "street" => "RAIN STREET",
            "diner" => "CEDAR DINER",
            "phone" => "INCOMING",
            "repair-shop" => "LIO'S REPAIR",
            "transit" => "ROUTE CANCELED",
            "archive-lobby" => "CITY ARCHIVE",
            "archive-stacks" => "RESTRICTED / ECHO",
            "revelation" => "RECORDS DISAGREE",
            "turning-point" => "SUNSET ADDRESS",
            "c2-address" => "REVISION 7 / 2013",
            "c2-contact" => "CONTACT OR SILENCE",
            "c2-frequency" => "88.3 / SECOND CHANNEL",
            "c2-records" => "CITY RECORDS",
            "c2-route" => "RIVER SERVICE",
            "c2-exterior" => "SUNSET LOT 17",
            "c2-caretaker" => "ELIAS / WITNESS",
            "c2-entry" => "SERVICE ENTRANCE",
            "c2-overlay" => "ECHO OVERLAY",
            "c2-disagreement" => "RECORDS DISAGREE",
            "c2-personal-record" => "PERSONAL RECORD",
            "c2-intervention" => "UNASKED ACTION",
            "c2-chamber" => "REVISION CHAMBER",
            "c2-predicted-choice" => "ANNOTATED ACTION",
            "c2-response" => "2013 / REPLY",
            "c2-consequence" => "AFTERIMAGE",
            "c2-displacement" => "SHIFTED THRESHOLD",
            "c2-turning-point" => "DIFFERENT MEMORY",
            _ => "SILICON ECHOES",
        };
        let glow = Color::rgba(0xFF, 0x98, 0x00, 38);
        let soft = Color::rgba(0xED, 0xE6, 0xD8, 92);
        let strong = Color::rgba(0xED, 0xE6, 0xD8, 180);
        match scene_id {
            "c2-address" | "c2-records" => {
                let desk = Rect::new(
                    rect.x + 88,
                    rect.y + rect.h as i32 * 63 / 100,
                    rect.w - 176,
                    46,
                );
                canvas.fill_rect(desk, Color::rgba(0xED, 0xE6, 0xD8, 58));
                canvas.hbar(desk.x - 8, desk.bottom() - 5, desk.w + 16, 5, BONE);
                let card = Rect::new(desk.x + 92, desk.y - 72, 242, 52);
                canvas.fill_rect(card, BONE);
                canvas.draw_rect(card, SUNLIGHT);
                draw_text(
                    canvas,
                    if scene_id == "c2-address" {
                        "REVISION 7 / LOT 17 / 2013"
                    } else {
                        "SUNSET LOT 17 / OMITTED"
                    },
                    card.x + 12,
                    card.y + 29,
                    &TextStyle::new(FontRole::MonoRegular, OBSIDIAN),
                );
                for index in 0..3 {
                    let ledger =
                        Rect::new(desk.x + 410 + index * 74, desk.y - 98 + index * 10, 58, 80);
                    canvas.fill_rect(ledger, Color::rgba(0xED, 0xE6, 0xD8, 78));
                    canvas.draw_rect(ledger, soft);
                    canvas.hline(ledger.x + 9, ledger.y + 22, ledger.w - 18, SUNLIGHT);
                }
                canvas.fill_rect(
                    Rect::new(rect.right() - 194, rect.y + 72, 86, 176),
                    Color::rgba(0xED, 0xE6, 0xD8, 26),
                );
                canvas.draw_rect(Rect::new(rect.right() - 194, rect.y + 72, 86, 176), soft);
            }
            "c2-contact" => {
                let booth = Rect::new(rect.x + rect.w as i32 * 42 / 100, rect.y + 160, 320, 48);
                canvas.blend_rounded_rect(booth, 18, Color::rgba(0xED, 0xE6, 0xD8, 48));
                canvas.hbar(booth.x - 6, booth.bottom() - 4, booth.w + 12, 4, BONE);
                canvas.fill_rounded_rect(Rect::new(booth.x + 44, booth.y - 72, 38, 38), 19, BONE);
                canvas.fill_rounded_rect(Rect::new(booth.x + 38, booth.y - 38, 50, 38), 16, strong);
                let riley_present = self
                    .game
                    .relationship(sunlight_silicon_echoes::ActorId("riley"))
                    >= 0;
                if riley_present {
                    canvas.fill_rounded_rect(
                        Rect::new(booth.right() - 82, booth.y - 76, 42, 42),
                        21,
                        SUNLIGHT,
                    );
                    canvas.fill_rounded_rect(
                        Rect::new(booth.right() - 88, booth.y - 38, 54, 38),
                        17,
                        strong,
                    );
                }
                let pager = Rect::new(rect.right() - 190, rect.y + 90, 94, 42);
                canvas.fill_rounded_rect(pager, 7, Color::rgba(0xED, 0xE6, 0xD8, 70));
                canvas.stroke_rounded_rect(pager, 7, 2, BONE);
                draw_center(canvas, pager, "88.3  II", FontRole::MonoRegular, SUNLIGHT);
            }
            "c2-frequency" => {
                let counter = Rect::new(
                    rect.x + 82,
                    rect.y + rect.h as i32 * 62 / 100,
                    rect.w - 240,
                    44,
                );
                canvas.fill_rect(counter, Color::rgba(0xED, 0xE6, 0xD8, 58));
                canvas.hbar(counter.x - 6, counter.bottom() - 5, counter.w + 12, 5, BONE);
                let pager = Rect::new(counter.x + 230, counter.y - 50, 112, 42);
                canvas.fill_rounded_rect(pager, 7, Color::rgba(0xED, 0xE6, 0xD8, 74));
                canvas.stroke_rounded_rect(pager, 7, 2, BONE);
                draw_center(canvas, pager, "88.3 / 2", FontRole::MonoRegular, SUNLIGHT);
                let manual = Rect::new(counter.right() - 132, counter.y - 80, 96, 62);
                canvas.fill_rect(manual, BONE);
                canvas.draw_rect(manual, SUNLIGHT);
                draw_center(canvas, manual, "SERVICE", FontRole::UiSmall, OBSIDIAN);
                canvas.fill_rounded_rect(Rect::new(rect.x + 148, rect.y + 78, 38, 40), 19, BONE);
                canvas.fill_rounded_rect(Rect::new(rect.x + 142, rect.y + 116, 50, 88), 18, strong);
            }
            "c2-route" | "c2-exterior" | "c2-displacement" => {
                let ground = rect.y + rect.h as i32 * 65 / 100;
                canvas.hbar(rect.x + 12, ground, rect.w - 24, 3, BONE);
                for index in 0..7 {
                    let x = rect.x + 64 + index * 126;
                    let h = 44 + ((self.ambient_seed.rotate_left(index as u32 + 3) % 58) as i32);
                    canvas.fill_rect(
                        Rect::new(x, ground - h, 76, h as u32),
                        Color::rgba(0xED, 0xE6, 0xD8, 30),
                    );
                }
                let facade = Rect::new(
                    rect.x + rect.w as i32 * 42 / 100,
                    rect.y + 72,
                    246,
                    (ground - rect.y - 72) as u32,
                );
                canvas.fill_rect(facade, Color::rgba(0xED, 0xE6, 0xD8, 28));
                canvas.draw_rect(facade, BONE);
                let gate = Rect::new(facade.x + 76, facade.y + 72, 94, facade.h - 72);
                canvas.draw_rect(gate, BONE);
                canvas.vline(gate.x + gate.w as i32 / 2, gate.y, gate.h, soft);
                canvas.stroke_rounded_rect(
                    gate.inset(-10),
                    4,
                    2,
                    Color::rgba(0xFF, 0x98, 0x00, 88),
                );
                if scene_id == "c2-route" {
                    let board = Rect::new(rect.x + 96, rect.y + 72, 210, 66);
                    canvas.fill_rect(board, OBSIDIAN);
                    canvas.draw_rect(board, BONE);
                    draw_text(
                        canvas,
                        "SUNSET SERVICE",
                        board.x + 14,
                        board.y + 22,
                        &TextStyle::new(FontRole::UiSmall, BONE),
                    );
                    draw_text(
                        canvas,
                        "UNLISTED",
                        board.x + 14,
                        board.y + 48,
                        &TextStyle::new(FontRole::MonoRegular, SUNLIGHT),
                    );
                }
                if scene_id != "c2-route" {
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 170, ground - 84, 38, 38),
                        19,
                        BONE,
                    );
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 164, ground - 48, 50, 54),
                        17,
                        strong,
                    );
                    if scene_id == "c2-displacement" {
                        canvas.stroke_rounded_rect(gate.inset(-22), 6, 2, SUNLIGHT);
                        canvas.fill_rounded_rect(
                            Rect::new(rect.right() - 154, ground - 54, 78, 28),
                            5,
                            BONE,
                        );
                        draw_center(
                            canvas,
                            Rect::new(rect.right() - 154, ground - 54, 78, 28),
                            "2013",
                            FontRole::MonoRegular,
                            OBSIDIAN,
                        );
                    }
                }
            }
            "c2-caretaker" | "c2-entry" => {
                let gate = Rect::new(
                    rect.x + rect.w as i32 * 43 / 100,
                    rect.y + 58,
                    202,
                    rect.h * 62 / 100,
                );
                canvas.draw_rect(gate, BONE);
                for index in 0..5 {
                    canvas.vline(gate.x + 24 + index * 38, gate.y + 12, gate.h - 24, soft);
                }
                canvas.fill_rounded_rect(Rect::new(rect.x + 192, rect.y + 104, 42, 42), 21, BONE);
                canvas.fill_rounded_rect(Rect::new(rect.x + 186, rect.y + 142, 54, 98), 18, strong);
                if scene_id == "c2-caretaker" {
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 284, rect.y + 182, 30, 16),
                        4,
                        BONE,
                    );
                    canvas.hline(rect.x + 286, rect.y + 190, 26, SUNLIGHT);
                } else {
                    canvas.stroke_rounded_rect(
                        Rect::new(gate.x + 54, gate.y + 86, 94, 134),
                        4,
                        2,
                        SUNLIGHT,
                    );
                    canvas.stroke_rounded_rect(
                        Rect::new(gate.x + 66, gate.y + 74, 94, 134),
                        4,
                        1,
                        Color::rgba(0xFF, 0x98, 0x00, 110),
                    );
                }
            }
            "c2-overlay"
            | "c2-disagreement"
            | "c2-personal-record"
            | "c2-intervention"
            | "c2-chamber"
            | "c2-predicted-choice"
            | "c2-response"
            | "c2-consequence"
            | "c2-turning-point" => {
                let chamber = Rect::new(
                    rect.x + rect.w as i32 * 38 / 100,
                    rect.y + 64,
                    322,
                    rect.h * 62 / 100,
                );
                canvas.draw_rect(chamber, BONE);
                canvas.hline(chamber.x, chamber.y + 44, chamber.w, soft);
                canvas.hline(chamber.x, chamber.bottom() - 42, chamber.w, soft);
                let projector = Rect::new(chamber.x + 92, chamber.y + 72, 138, 78);
                canvas.fill_rounded_rect(projector, 9, Color::rgba(0xED, 0xE6, 0xD8, 52));
                canvas.stroke_rounded_rect(projector, 9, 2, BONE);
                canvas.fill_rounded_rect(projector.inset(10), 5, OBSIDIAN);
                if self.game.supports_echo_overlay()
                    && self.game.echo_layer == EchoLayer::Revision2013
                {
                    canvas.stroke_rounded_rect(
                        Rect::new(chamber.x + 54, chamber.y + 156, 100, 116),
                        4,
                        2,
                        SUNLIGHT,
                    );
                    canvas.stroke_rounded_rect(
                        Rect::new(chamber.x + 178, chamber.y + 154, 84, 120),
                        4,
                        2,
                        Color::rgba(0xFF, 0x98, 0x00, 110),
                    );
                    canvas.hline(chamber.x + 188, chamber.y + 246, 64, SUNLIGHT);
                } else {
                    canvas.fill_rect(
                        Rect::new(chamber.x + 54, chamber.y + 156, 100, 116),
                        Color::rgba(0xED, 0xE6, 0xD8, 46),
                    );
                    canvas.draw_rect(Rect::new(chamber.x + 54, chamber.y + 156, 100, 116), BONE);
                    canvas.draw_rect(Rect::new(chamber.x + 178, chamber.y + 154, 84, 120), BONE);
                }
                if scene_id == "c2-personal-record" || scene_id == "c2-intervention" {
                    let card = Rect::new(chamber.x + 188, chamber.y + 176, 92, 54);
                    canvas.fill_rect(card, BONE);
                    canvas.draw_rect(card, SUNLIGHT);
                    draw_center(canvas, card, "MARA / 2013", FontRole::UiSmall, OBSIDIAN);
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 178, rect.y + 156, 38, 38),
                        19,
                        SUNLIGHT,
                    );
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 172, rect.y + 192, 50, 64),
                        18,
                        strong,
                    );
                }
                if scene_id == "c2-chamber"
                    || scene_id == "c2-predicted-choice"
                    || scene_id == "c2-response"
                {
                    for index in 0..7 {
                        let x = chamber.x + 22 + index * 40;
                        canvas.fill_rounded_rect(
                            Rect::new(x, chamber.y + 36, 18, 18),
                            9,
                            if index == 3 { SUNLIGHT } else { BONE },
                        );
                        canvas.hline(x + 9, chamber.y + 54, 46, soft);
                    }
                    draw_center(
                        canvas,
                        Rect::new(projector.x, projector.y + 22, projector.w, 22),
                        if scene_id == "c2-predicted-choice" {
                            "SEND THE NAME"
                        } else if scene_id == "c2-response" {
                            "2013 / RECEIVED"
                        } else {
                            "REVISION / 7"
                        },
                        FontRole::MonoRegular,
                        SUNLIGHT,
                    );
                }
                if scene_id == "c2-turning-point" {
                    // On the ending screen the primary artifact carries the
                    // reply; keep only a low-contrast machine whisper here.
                    let color = if self.mode == Mode::Ending {
                        Color::rgba(0xFF, 0x98, 0x00, 48)
                    } else {
                        SUNLIGHT
                    };
                    draw_center(
                        canvas,
                        projector,
                        if self.mode == Mode::Ending {
                            "REVISION / 7"
                        } else {
                            "I REMEMBER YOU\nDIFFERENTLY"
                        },
                        FontRole::MonoRegular,
                        color,
                    );
                }
            }
            "hallway" | "landing" | "stairwell" => {
                let door = Rect::new(
                    rect.x + rect.w as i32 * 41 / 100,
                    rect.y + 40,
                    rect.w * 20 / 100,
                    rect.h * 58 / 100,
                );
                canvas.fill_rect(door, Color::rgba(0xED, 0xE6, 0xD8, 36));
                canvas.draw_rect(door, BONE);
                canvas.fill_rect(
                    Rect::new(door.right() - 18, door.y + door.h as i32 / 2, 6, 6),
                    SUNLIGHT,
                );
                for index in 0..4 {
                    let y = rect.y + 54 + index * 40;
                    canvas.hline(
                        rect.x + 34,
                        y,
                        rect.w - 68,
                        Color::rgba(0xED, 0xE6, 0xD8, 45),
                    );
                }
                if scene_id == "stairwell" {
                    for index in 0..6 {
                        canvas.hline(
                            rect.x + 120 + index * 42,
                            rect.y + rect.h as i32 - 50 - index * 21,
                            150,
                            soft,
                        );
                    }
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 190, rect.y + 146, 32, 22),
                        8,
                        SUNLIGHT,
                    );
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 236, rect.y + 166, 28, 20),
                        8,
                        SUNLIGHT,
                    );
                } else {
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 84, rect.y + 72, 20, 34),
                        8,
                        SUNLIGHT,
                    );
                    canvas.blend_rect(
                        Rect::new(rect.x + 74, rect.y + 107, 40, 8),
                        Color::rgba(0xFF, 0x98, 0x00, 35),
                    );
                    canvas.draw_rect(
                        Rect::new(rect.x + 94, rect.y + 162, 58, 74),
                        Color::rgba(0xED, 0xE6, 0xD8, 122),
                    );
                    draw_center(
                        canvas,
                        Rect::new(rect.x + 94, rect.y + 176, 58, 22),
                        if scene_id == "landing" { "04" } else { "4B" },
                        FontRole::MonoRegular,
                        SUNLIGHT,
                    );
                    canvas.fill_rect(
                        Rect::new(rect.right() - 128, rect.y + 96, 46, 68),
                        Color::rgba(0xED, 0xE6, 0xD8, 32),
                    );
                    canvas.draw_rect(Rect::new(rect.right() - 128, rect.y + 96, 46, 68), soft);
                }
            }
            "kitchen" => {
                let counter = Rect::new(
                    rect.x + 68,
                    rect.y + rect.h as i32 * 58 / 100,
                    rect.w - 136,
                    52,
                );
                canvas.fill_rect(counter, Color::rgba(0xED, 0xE6, 0xD8, 52));
                canvas.hbar(counter.x - 8, counter.bottom() - 6, counter.w + 16, 6, BONE);
                let paper = Rect::new(counter.x + 80, counter.y - 42, 150, 92);
                canvas.fill_rect(paper, BONE);
                canvas.draw_rect(paper, SUNLIGHT);
                draw_text(
                    canvas,
                    "TUESDAY / 1993",
                    paper.x + 10,
                    paper.y + 15,
                    &TextStyle::new(FontRole::UiSmall, OBSIDIAN),
                );
                canvas.fill_rounded_rect(
                    Rect::new(counter.right() - 160, counter.y - 25, 38, 32),
                    8,
                    Color::rgba(0xED, 0xE6, 0xD8, 130),
                );
                canvas.hline(counter.right() - 154, counter.y + 8, 27, SUNLIGHT);
                canvas.fill_rect(
                    Rect::new(rect.right() - 142, rect.y + 82, 92, 156),
                    Color::rgba(0xED, 0xE6, 0xD8, 30),
                );
                canvas.draw_rect(Rect::new(rect.right() - 142, rect.y + 82, 92, 156), soft);
                canvas.hline(rect.right() - 130, rect.y + 121, 68, soft);
                canvas.hline(rect.right() - 130, rect.y + 164, 68, soft);
                canvas.fill_rounded_rect(
                    Rect::new(rect.x + 382, counter.y - 30, 84, 62),
                    6,
                    Color::rgba(0xED, 0xE6, 0xD8, 88),
                );
                canvas.draw_rect(Rect::new(rect.x + 382, counter.y - 30, 84, 62), SUNLIGHT);
            }
            "street" | "transit" => {
                let skyline_y = rect.y + rect.h as i32 * 43 / 100;
                for index in 0..8 {
                    let x = rect.x + 36 + index * 118;
                    let h = 58 + ((self.ambient_seed.rotate_left(index as u32) % 76) as i32);
                    canvas.fill_rect(
                        Rect::new(x, skyline_y - h, 72, h as u32),
                        Color::rgba(0xED, 0xE6, 0xD8, 34),
                    );
                    canvas.blend_rect(Rect::new(x + 12, skyline_y - h + 16, 12, 3), glow);
                }
                canvas.hbar(rect.x + 12, skyline_y, rect.w - 24, 3, BONE);
                canvas.hbar(rect.x + 12, skyline_y + 82, rect.w - 24, 2, soft);
                let booth = Rect::new(rect.x + 74, rect.y + 92, 66, 144);
                canvas.draw_rect(booth, BONE);
                canvas.hline(booth.x, booth.y + 31, booth.w, SUNLIGHT);
                canvas.vline(
                    booth.x + booth.w as i32 / 2,
                    booth.y + 32,
                    booth.h - 32,
                    soft,
                );
                canvas.fill_rounded_rect(
                    Rect::new(rect.x + rect.w as i32 * 67 / 100, rect.y + 100, 22, 62),
                    10,
                    SUNLIGHT,
                );
                canvas.blend_rect(
                    Rect::new(rect.x + rect.w as i32 * 67 / 100 - 8, rect.y + 156, 38, 8),
                    glow,
                );
                if scene_id == "transit" {
                    let board = Rect::new(rect.x + rect.w as i32 * 53 / 100, rect.y + 44, 250, 82);
                    canvas.fill_rect(board, OBSIDIAN);
                    canvas.draw_rect(board, BONE);
                    canvas.hbar(board.x - 2, board.y - 22, board.w + 4, 6, SUNLIGHT);
                    canvas.hline(board.x, board.y - 8, board.w, soft);
                    draw_text(
                        canvas,
                        "ARCHIVE LINE",
                        board.x + 14,
                        board.y + 17,
                        &TextStyle::new(FontRole::UiSmall, BONE),
                    );
                    draw_text(
                        canvas,
                        "CANCELED",
                        board.x + 14,
                        board.y + 45,
                        &TextStyle::new(FontRole::MonoRegular, SUNLIGHT),
                    );
                    for i in 0..2 {
                        let lx = board.x + board.w as i32 - 36 - i * 42;
                        canvas.fill_rect(
                            Rect::new(lx, board.y + 22, 24, 18),
                            Color::rgba(0xED, 0xE6, 0xD8, 52),
                        );
                    }
                }
                for index in 0..5 {
                    let x = rect.x + 110 + index * 150;
                    let h = 38 + ((self.ambient_seed.rotate_left(index as u32 + 7) % 18) as i32);
                    canvas.fill_rounded_rect(Rect::new(x, skyline_y + 20, 16, h as u32), 7, strong);
                    let head_w =
                        22 + ((self.ambient_seed.wrapping_add(index as u32 * 3) % 10) as i32);
                    canvas.fill_rounded_rect(
                        Rect::new(x - (head_w - 16) / 2, skyline_y + 10, head_w as u32, 20),
                        10,
                        BONE,
                    );
                    if self.ambient_seed.wrapping_add(index as u32) % 3 == 0 {
                        canvas.fill_rect(
                            Rect::new(x - 8, skyline_y + 26, 32, 12),
                            Color::rgba(0xED, 0xE6, 0xD8, 72),
                        );
                    }
                }
                if scene_id == "street" {
                    canvas.fill_rect(
                        Rect::new(rect.x + rect.w as i32 * 33 / 100, skyline_y + 12, 38, 42),
                        Color::rgba(0xED, 0xE6, 0xD8, 88),
                    );
                    canvas.fill_rounded_rect(
                        Rect::new(
                            rect.x + rect.w as i32 * 33 / 100 + 6,
                            skyline_y - 14,
                            25,
                            30,
                        ),
                        13,
                        BONE,
                    );
                }
            }
            "diner" | "phone" => {
                let window = Rect::new(
                    rect.x + 78,
                    rect.y + 48,
                    rect.w * 35 / 100,
                    rect.h * 43 / 100,
                );
                canvas.fill_rect(window, OBSIDIAN);
                canvas.draw_rect(window, BONE);
                canvas.hline(window.x, window.y + window.h as i32 / 2, window.w, soft);
                let booth_back = Rect::new(
                    rect.x + rect.w as i32 * 42 / 100,
                    rect.y + rect.h as i32 * 39 / 100,
                    rect.w * 44 / 100,
                    58,
                );
                canvas.blend_rounded_rect(booth_back, 18, Color::rgba(0xED, 0xE6, 0xD8, 45));
                canvas.stroke_rounded_rect(booth_back, 18, 2, soft);
                let table = Rect::new(
                    rect.x + rect.w as i32 * 49 / 100,
                    rect.y + rect.h as i32 * 62 / 100,
                    300,
                    34,
                );
                canvas.fill_rounded_rect(table, 8, Color::rgba(0xED, 0xE6, 0xD8, 64));
                canvas.hbar(table.x - 4, table.bottom() - 4, table.w + 8, 4, BONE);
                // Mara — lighter silhouette, slightly shorter
                let mara_head = Rect::new(table.x + 44, table.y - 88, 32, 32);
                let mara_body = Rect::new(table.x + 32, table.y - 58, 56, 58);
                canvas.fill_rounded_rect(mara_head, 16, BONE);
                canvas.fill_rounded_rect(mara_body, 16, Color::rgba(0xED, 0xE6, 0xD8, 150));
                canvas.hline(
                    mara_body.x + 8,
                    mara_body.bottom() - 5,
                    mara_body.w - 16,
                    soft,
                );
                // Riley — orange head accent, taller
                let riley_head = Rect::new(table.x + 202, table.y - 95, 36, 36);
                let riley_body = Rect::new(table.x + 188, table.y - 62, 64, 62);
                canvas.fill_rounded_rect(riley_head, 18, SUNLIGHT);
                canvas.fill_rounded_rect(riley_body, 18, strong);
                canvas.hline(
                    riley_body.x + 10,
                    riley_body.bottom() - 5,
                    riley_body.w - 20,
                    SUNLIGHT,
                );
                // Table items
                canvas.fill_rounded_rect(Rect::new(table.x + 96, table.y - 15, 22, 18), 5, BONE);
                canvas.hline(table.x + 98, table.y + 4, 18, SUNLIGHT);
                canvas.fill_rounded_rect(Rect::new(table.x + 144, table.y - 20, 12, 22), 4, BONE);
                canvas.hline(table.x + 139, table.y - 20, 22, BONE);
                let clock = Rect::new(rect.right() - 105, rect.y + 42, 54, 54);
                canvas.stroke_rounded_rect(clock, 27, 2, BONE);
                canvas.hline(clock.x + 27, clock.y + 27, 17, SUNLIGHT);
                canvas.vline(clock.x + 27, clock.y + 12, 15, BONE);
                if scene_id == "phone" {
                    let phone = Rect::new(rect.x + rect.w as i32 * 66 / 100, rect.y + 74, 112, 100);
                    canvas.blend_rounded_rect(phone, 12, glow);
                    canvas.stroke_rounded_rect(phone, 12, 2, BONE);
                    // Handset (arched bar above body)
                    canvas.stroke_rounded_rect(
                        Rect::new(phone.x + 10, phone.y + 10, 92, 30),
                        14,
                        3,
                        SUNLIGHT,
                    );
                    canvas.fill_rounded_rect(
                        Rect::new(phone.x + 24, phone.y + 10, 18, 14),
                        6,
                        BONE,
                    );
                    canvas.fill_rounded_rect(
                        Rect::new(phone.x + 70, phone.y + 10, 18, 14),
                        6,
                        BONE,
                    );
                    // Body/keypad area
                    canvas.fill_rounded_rect(
                        Rect::new(phone.x + 16, phone.y + 44, 80, 38),
                        8,
                        BONE,
                    );
                    // Keypad
                    for row in 0..3 {
                        for col in 0..3 {
                            canvas.fill_rounded_rect(
                                Rect::new(phone.x + 28 + col * 17, phone.y + 76 + row * 12, 8, 6),
                                2,
                                Color::rgba(0xED, 0xE6, 0xD8, 150),
                            );
                        }
                    }
                    // Coiled cable
                    canvas.hline(phone.x + 12, phone.bottom(), 18, soft);
                    canvas.hline(phone.x + 16, phone.bottom() + 6, 26, soft);
                    canvas.hline(phone.x + 10, phone.bottom() + 12, 36, soft);
                }
            }
            "repair-shop" => {
                for index in 0..4 {
                    let shelf = Rect::new(rect.x + 74, rect.y + 30 + index * 46, rect.w - 180, 3);
                    canvas.hbar(shelf.x, shelf.y, shelf.w, 3, BONE);
                    let item_kinds = [(48, 26), (32, 32), (62, 24), (36, 28), (52, 20)];
                    for item in 0..6 {
                        let kind_idx = ((index * 3 + item) % 5) as usize;
                        let (iw, ih) = item_kinds[kind_idx];
                        canvas.fill_rect(
                            Rect::new(
                                shelf.x + 22 + item * 102,
                                shelf.y - ih,
                                iw as u32,
                                ih as u32,
                            ),
                            Color::rgba(0xED, 0xE6, 0xD8, 54 + ((index * 7) as u8 & 15)),
                        );
                        if item % 3 == 1 {
                            canvas.hline(
                                shelf.x + 22 + item * 102 + 6,
                                shelf.y - ih / 2,
                                iw as u32 - 12,
                                soft,
                            );
                        }
                    }
                }
                let counter = Rect::new(
                    rect.x + 82,
                    rect.y + rect.h as i32 * 61 / 100,
                    rect.w - 296,
                    46,
                );
                canvas.fill_rect(counter, Color::rgba(0xED, 0xE6, 0xD8, 56));
                canvas.hbar(counter.x - 6, counter.bottom() - 5, counter.w + 12, 5, BONE);
                let pager = Rect::new(
                    counter.x + counter.w as i32 * 55 / 100,
                    counter.y - 38,
                    98,
                    40,
                );
                canvas.blend_rounded_rect(pager, 7, Color::rgba(0xED, 0xE6, 0xD8, 76));
                canvas.stroke_rounded_rect(pager, 7, 2, BONE);
                draw_center(canvas, pager, "88.3", FontRole::MonoMedium, SUNLIGHT);
                // Test instrument on counter
                canvas.fill_rounded_rect(
                    Rect::new(counter.x + 66, counter.y - 42, 76, 44),
                    6,
                    Color::rgba(0xED, 0xE6, 0xD8, 72),
                );
                canvas.hline(counter.x + 72, counter.y - 18, 64, soft);
                let freq = Rect::new(counter.x + 82, counter.y - 24, 44, 14);
                canvas.fill_rect(freq, OBSIDIAN);
                draw_center(canvas, freq, "88.3", FontRole::MonoRegular, SUNLIGHT);
                // Lio
                canvas.fill_rounded_rect(
                    Rect::new(rect.x + 144, rect.y + 96, 46, 102),
                    20,
                    Color::rgba(0xED, 0xE6, 0xD8, 155),
                );
                canvas.fill_rounded_rect(Rect::new(rect.x + 150, rect.y + 62, 34, 38), 18, BONE);
                canvas.hline(rect.x + 134, rect.y + 162, 68, SUNLIGHT);
                // Soldering lamp
                canvas.hline(rect.x + 370, rect.y + 86, 18, SUNLIGHT);
                canvas.fill_rect(
                    Rect::new(rect.x + 382, rect.y + 72, 6, 16),
                    Color::rgba(0xFF, 0x98, 0x00, 60),
                );
                // Service manual
                let manual = Rect::new(rect.x + 435, rect.y + 188, 94, 62);
                canvas.fill_rect(manual, BONE);
                canvas.draw_rect(manual, SUNLIGHT);
                draw_text(
                    canvas,
                    "ECHO",
                    manual.x + 16,
                    manual.y + 30,
                    &TextStyle::new(FontRole::MonoRegular, OBSIDIAN),
                );
                // Tool outline on wall
                canvas.draw_rect(
                    Rect::new(rect.x + rect.w as i32 - 148, rect.y + 32, 64, 64),
                    Color::rgba(0xED, 0xE6, 0xD8, 110),
                );
                canvas.hline(
                    rect.x + rect.w as i32 - 138,
                    rect.y + 64,
                    44,
                    Color::rgba(0xED, 0xE6, 0xD8, 140),
                );
                canvas.vline(
                    rect.x + rect.w as i32 - 116,
                    rect.y + 44,
                    34,
                    Color::rgba(0xED, 0xE6, 0xD8, 140),
                );
            }
            "archive-lobby" | "archive-stacks" | "revelation" | "turning-point" => {
                for index in 0..5 {
                    let x = rect.x + 68 + index * 168;
                    let w = 112;
                    canvas.fill_rect(
                        Rect::new(x, rect.y + 44, w, rect.h - 110),
                        Color::rgba(0xED, 0xE6, 0xD8, 24),
                    );
                    canvas.vline(x, rect.y + 44, rect.h - 110, soft);
                    canvas.vline(x + w as i32, rect.y + 44, rect.h - 110, soft);
                    for shelf in 0..5 {
                        canvas.hline(x + 6, rect.y + 68 + shelf * 44, w - 12, soft);
                        for item in 0..2 {
                            let item_w = 54 - (shelf * 4) as u32;
                            canvas.fill_rect(
                                Rect::new(x + 12 + item * 44, rect.y + 50 + shelf * 44, item_w, 14),
                                Color::rgba(0xED, 0xE6, 0xD8, 38 + (shelf as u8 * 5)),
                            );
                        }
                    }
                }
                let term_x = rect.x + rect.w as i32 * 52 / 100;
                let term_y = rect.y + rect.h as i32 * 50 / 100;
                // Monitor enclosure
                let monitor = Rect::new(term_x, term_y, 210, 110);
                canvas.blend_rounded_rect(monitor, 10, Color::rgba(0xED, 0xE6, 0xD8, 60));
                canvas.stroke_rounded_rect(monitor, 10, 2, BONE);
                // Screen
                let screen = Rect::new(
                    monitor.x + 14,
                    monitor.y + 12,
                    monitor.w - 28,
                    monitor.h - 46,
                );
                canvas.fill_rounded_rect(screen, 6, OBSIDIAN);
                canvas.stroke_rounded_rect(screen, 6, 1, soft);
                draw_text(
                    canvas,
                    "ECHO / REVISION",
                    screen.x + 10,
                    screen.y + 16,
                    &TextStyle::new(FontRole::MonoRegular, SUNLIGHT),
                );
                draw_text(
                    canvas,
                    "1993  <>  FUTURE",
                    screen.x + 10,
                    screen.y + 38,
                    &TextStyle::new(FontRole::UiSmall, BONE),
                );
                // Bezel lights
                for light in 0..3 {
                    canvas.fill_rounded_rect(
                        Rect::new(monitor.x + 16 + light * 14, monitor.y + 4, 6, 6),
                        3,
                        if light == 1 { SUNLIGHT } else { BONE },
                    );
                }
                // Media slot
                let slot = Rect::new(
                    monitor.x + monitor.w as i32 - 54,
                    monitor.y + monitor.h as i32 - 14,
                    44,
                    6,
                );
                canvas.fill_rounded_rect(slot, 3, SUNLIGHT);
                // Stand/neck
                canvas.fill_rect(
                    Rect::new(
                        monitor.x + monitor.w as i32 / 2 - 6,
                        monitor.bottom(),
                        12,
                        16,
                    ),
                    BONE,
                );
                // Base
                canvas.fill_rounded_rect(
                    Rect::new(monitor.x + 26, monitor.bottom() + 12, monitor.w - 52, 12),
                    4,
                    Color::rgba(0xED, 0xE6, 0xD8, 100),
                );
                // Keyboard — separated from monitor
                let kb = Rect::new(monitor.x + 14, monitor.bottom() + 28, monitor.w - 28, 28);
                canvas.fill_rounded_rect(kb, 4, Color::rgba(0xED, 0xE6, 0xD8, 52));
                for row in 0..3 {
                    for col in 0..12 {
                        canvas.fill_rect(
                            Rect::new(kb.x + 4 + col * 15, kb.y + 5 + row * 8, 8, 4),
                            Color::rgba(0xED, 0xE6, 0xD8, 82),
                        );
                    }
                }
                if scene_id == "archive-lobby" {
                    canvas.fill_rect(
                        Rect::new(rect.x + 244, rect.y + 165, 176, 56),
                        Color::rgba(0xED, 0xE6, 0xD8, 60),
                    );
                    canvas.hbar(rect.x + 232, rect.y + 222, 202, 4, BONE);
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 348, rect.y + 111, 34, 52),
                        16,
                        BONE,
                    );
                }
                if scene_id == "archive-stacks" {
                    let ledger = Rect::new(rect.x + 240, rect.y + 140, 106, 92);
                    canvas.fill_rect(ledger, BONE);
                    canvas.draw_rect(ledger, SUNLIGHT);
                    draw_text(
                        canvas,
                        "REV. 7",
                        ledger.x + 14,
                        ledger.y + 27,
                        &TextStyle::new(FontRole::MonoRegular, OBSIDIAN),
                    );
                    canvas.hline(
                        ledger.x + 14,
                        ledger.y + 48,
                        70,
                        Color::rgba(0x0A, 0x0A, 0x0C, 150),
                    );
                    canvas.hline(
                        ledger.x + 14,
                        ledger.y + 65,
                        70,
                        Color::rgba(0x0A, 0x0A, 0x0C, 150),
                    );
                }
                if scene_id == "revelation" {
                    canvas.blend_rect(
                        Rect::new(
                            monitor.x - 16,
                            monitor.y - 16,
                            monitor.w + 32,
                            monitor.h + kb.h as u32 + 52,
                        ),
                        Color::rgba(0xFF, 0x98, 0x00, 14),
                    );
                    canvas.fill_rounded_rect(
                        Rect::new(rect.x + 316, rect.y + 164, 24, 58),
                        10,
                        BONE,
                    );
                    canvas.stroke_rounded_rect(
                        Rect::new(rect.x + 304, rect.y + 139, 48, 36),
                        16,
                        3,
                        SUNLIGHT,
                    );
                }
                if scene_id == "turning-point" {
                    let address = Rect::new(monitor.x - 16, monitor.y - 42, 242, 44);
                    canvas.fill_rect(address, BONE);
                    canvas.draw_rect(address, SUNLIGHT);
                    draw_text(
                        canvas,
                        "SUNSET / LOT 17 / 2013",
                        address.x + 12,
                        address.y + 27,
                        &TextStyle::new(FontRole::MonoRegular, OBSIDIAN),
                    );
                    canvas.blend_rect(
                        Rect::new(rect.x + 44, rect.y + 46, 182, rect.h - 106),
                        Color::rgba(0xFF, 0x98, 0x00, 18),
                    );
                }
            }
            _ => {}
        }
        canvas.blend_rect(
            Rect::new(rect.x + 18, rect.y + 18, rect.w - 36, 30),
            Color::rgba(0x0A, 0x0A, 0x0C, 120),
        );
        let label_color = if self.mode == Mode::Ending {
            Color::rgba(0xFF, 0x98, 0x00, 70)
        } else {
            SUNLIGHT
        };
        draw_text(
            canvas,
            label,
            rect.x + 32,
            rect.y + 38,
            &TextStyle::new(FontRole::MonoRegular, label_color),
        );
        canvas.draw_rect(rect, Color::rgba(0xED, 0xE6, 0xD8, 160));
    }

    fn draw_hotspot_feedback(&self, canvas: &mut Canvas) {
        if let Hover::Object(target) = self.hover {
            let Some((_, rect)) = self
                .scene_object_bounds()
                .into_iter()
                .find(|(candidate, _)| *candidate == target)
            else {
                return;
            };
            canvas.stroke_rounded_rect(rect.inset(-3), 5, 2, SUNLIGHT);
            let label = match target {
                SceneObjectTarget::Hotspot(hotspot) => hotspot_label(hotspot),
                SceneObjectTarget::Choice(choice_id) => choice_id.0,
                SceneObjectTarget::Overlay => "ECHO LAYER",
            };
            draw_text(
                canvas,
                label,
                rect.x,
                rect.y - 17,
                &TextStyle::new(FontRole::UiSmall, SUNLIGHT),
            );
        }
    }

    fn draw_title(&self, canvas: &mut Canvas) {
        self.draw_room(canvas);
        canvas.blend_rect(self.layout.image, Color::rgba(0x0A, 0x0A, 0x0C, 138));
        draw_center(
            canvas,
            Rect::new(
                self.layout.image.x,
                self.layout.image.y + 72,
                self.layout.image.w,
                32,
            ),
            "SILICON ECHOES",
            FontRole::UiTitle,
            BONE,
        );
        draw_center(
            canvas,
            Rect::new(
                self.layout.image.x,
                self.layout.image.y + 108,
                self.layout.image.w,
                22,
            ),
            "1993",
            FontRole::MonoMedium,
            SUNLIGHT,
        );
        draw_center(
            canvas,
            Rect::new(
                self.layout.image.x,
                self.layout.image.y + 142,
                self.layout.image.w,
                20,
            ),
            "A native SunlightOS story",
            FontRole::UiRegular,
            Color::rgba(0xED, 0xE6, 0xD8, 174),
        );
        self.draw_action(
            canvas,
            self.layout.title_new,
            "NEW GAME",
            self.hover == Hover::TitleNew,
        );
        if self.saved_game.is_some() {
            self.draw_action(
                canvas,
                self.layout.title_continue,
                "CONTINUE",
                self.hover == Hover::TitleContinue,
            );
        }
        draw_text(
            canvas,
            "Mouse or Enter to begin  /  Esc to exit",
            self.layout.narrative.x,
            self.layout.narrative.y + 20,
            &TextStyle::new(FontRole::UiSmall, Color::rgba(0xED, 0xE6, 0xD8, 160)),
        );
        if !self.save_notice.is_empty() {
            draw_text(
                canvas,
                self.save_notice,
                self.layout.narrative.x,
                self.layout.narrative.y + 42,
                &TextStyle::new(FontRole::UiSmall, SUNLIGHT),
            );
        }
    }

    fn draw_play(&mut self, canvas: &mut Canvas) {
        self.draw_room(canvas);
        let story_node = node(self.game.current_node)
            .unwrap_or_else(|| node(StoryNodeId("bedroom.wake")).unwrap());
        canvas.blend_rect(self.layout.narrative, Color::rgba(0xED, 0xE6, 0xD8, 18));
        canvas.hbar(
            self.layout.narrative.x,
            self.layout.narrative.y,
            self.layout.narrative.w,
            1,
            Color::rgba(0xED, 0xE6, 0xD8, 110),
        );
        draw_text(
            canvas,
            story_node.scene.0,
            self.layout.narrative.x + 12,
            self.layout.narrative.y + 12,
            &TextStyle::new(FontRole::UiSmall, SUNLIGHT),
        );
        self.ensure_narrative_layout();
        if let Some(cache) = self
            .scene_cache
            .as_ref()
            .filter(|cache| cache.node == self.game.current_node)
        {
            draw_prepared_prefix(
                canvas,
                &cache.narration,
                &cache.lines,
                self.presentation.visible_byte_end(),
                self.layout.narrative.x + 14,
                self.layout.narrative.y + NARRATIVE_TEXT_TOP,
                FontRole::SerifRegular,
                BONE,
                NARRATIVE_LINE_HEIGHT,
            );
        }
        if self.presentation.is_revealing() {
            // Park the caret in the margin rather than over the final glyph.
            draw_text(
                canvas,
                "_",
                self.layout.choices.x - 28,
                self.layout.narrative.y + NARRATIVE_TEXT_TOP,
                &TextStyle::new(FontRole::MonoRegular, Color::rgba(0xFF, 0x98, 0x00, 160)),
            );
        }
        if self.game.flags.get("signal_arrived") {
            draw_text(
                canvas,
                "A pulse answers from the glass.",
                self.layout.narrative.x + 12,
                self.layout.narrative.bottom() - 22,
                &TextStyle::new(FontRole::UiSmall, SUNLIGHT),
            );
        }
        if self.game.supports_echo_overlay() {
            let layer = match self.game.echo_layer {
                EchoLayer::Physical1993 => "E: 1993 PHYSICAL",
                EchoLayer::Revision2013 => "E: REVISION 2013",
            };
            self.draw_action(
                canvas,
                Rect::new(
                    self.layout.image.right() - 184,
                    self.layout.image.y + 52,
                    144,
                    30,
                ),
                layer,
                self.hover == Hover::Object(SceneObjectTarget::Overlay),
            );
            draw_text(
                canvas,
                "Only the active record can be touched.",
                self.layout.narrative.x + 12,
                self.layout.narrative.bottom() - 22,
                &TextStyle::new(FontRole::UiSmall, SUNLIGHT),
            );
        }
        if self.presentation_accepts_input() {
            canvas.vline(
                self.layout.choices.x - 12,
                self.layout.choices.y,
                self.layout.choices.h,
                Color::rgba(0xED, 0xE6, 0xD8, 70),
            );
        }
        if self.presentation_accepts_input() && self.current_node_is_uncontrolled() {
            let rects = self.choice_rects();
            let rect = rects.first().copied().unwrap_or_else(|| self.choice_rect(0));
            self.draw_action(
                canvas,
                rect,
                "LET THE ROOM MOVE ON",
                self.hover == Hover::Choice(0),
            );
            draw_text(
                canvas,
                "There is no choice here.",
                self.layout.choices.x,
                rect.bottom() + 12,
                &TextStyle::new(FontRole::UiSmall, Color::rgba(0xED, 0xE6, 0xD8, 140)),
            );
        } else if self.presentation_accepts_input() {
            let available = self.available_choice_indices();
            let rects = self.choice_rects();
            if available.is_empty() {
                for (visible_index, hotspot) in self.available_hotspots().iter().enumerate() {
                    let rect = rects
                        .get(visible_index)
                        .copied()
                        .unwrap_or_else(|| self.choice_rect(visible_index));
                    self.draw_choice(
                        canvas,
                        rect,
                        hotspot_label(*hotspot),
                        self.hover == Hover::Object(SceneObjectTarget::Hotspot(*hotspot)),
                        self.focused && self.selected_hotspot == visible_index,
                        visible_index,
                    );
                }
            }
            for (visible_index, choice_index) in available.iter().enumerate() {
                let choice = self.choices()[*choice_index];
                let hovered = self.hover == Hover::Choice(*choice_index);
                let focused = self.focused && self.selected_choice == *choice_index;
                let rect = rects
                    .get(visible_index)
                    .copied()
                    .unwrap_or_else(|| self.choice_rect(visible_index));
                self.draw_choice(
                    canvas,
                    rect,
                    choice.text,
                    hovered,
                    focused,
                    visible_index,
                );
            }
        }
        draw_text(
            canvas,
            if self.save_notice.is_empty() {
                "Esc: title"
            } else {
                self.save_notice
            },
            self.layout.frame.x + 12,
            self.layout.frame.bottom() - 16,
            &TextStyle::new(FontRole::UiSmall, Color::rgba(0xED, 0xE6, 0xD8, 130)),
        );
    }

    fn draw_ending(&self, canvas: &mut Canvas) {
        self.draw_room(canvas);
        // Stronger dim so background terminal machinery stays quiet.
        canvas.blend_rect(self.layout.image, Color::rgba(0x0A, 0x0A, 0x0C, 214));
        if self.game.chapter == 2 {
            let (win_w, win_h) = self.window_size();
            let tp = chapter_two_turning_point_layout(win_w, win_h);
            let artifact = Rect::new(
                tp.artifact.0,
                tp.artifact.1,
                tp.artifact.2 as u32,
                tp.artifact.3 as u32,
            );
            // Quiet background terminal silhouette under the primary artifact.
            let terminal = Rect::new(
                self.layout.image.x + self.layout.image.w as i32 * 38 / 100,
                self.layout.image.y + 72,
                280,
                self.layout.image.h * 48 / 100,
            );
            canvas.blend_rect(terminal, Color::rgba(0xED, 0xE6, 0xD8, 18));
            canvas.draw_rect(terminal, Color::rgba(0xED, 0xE6, 0xD8, 40));
            draw_text(
                canvas,
                "ECHO / REVISION",
                terminal.x + 16,
                terminal.y + 28,
                &TextStyle::new(FontRole::MonoRegular, Color::rgba(0xFF, 0x98, 0x00, 55)),
            );
            canvas.fill_rect(artifact, BONE);
            canvas.draw_rect(artifact, SUNLIGHT);
            draw_center(
                canvas,
                artifact,
                "2013 / I REMEMBER YOU DIFFERENTLY",
                FontRole::MonoRegular,
                OBSIDIAN,
            );
            let chapter_title = Rect::new(
                tp.chapter_title.0,
                tp.chapter_title.1,
                tp.chapter_title.2 as u32,
                tp.chapter_title.3 as u32,
            );
            draw_center(
                canvas,
                chapter_title,
                "CHAPTER TWO TURNING POINT",
                FontRole::UiTitle,
                BONE,
            );
            let theme = Rect::new(
                tp.theme_line.0,
                tp.theme_line.1,
                tp.theme_line.2 as u32,
                tp.theme_line.3 as u32,
            );
            draw_center(
                canvas,
                theme,
                "A reply exists. Its author remains uncertain.",
                FontRole::SerifRegular,
                Color::rgba(0xFF, 0x98, 0x00, 210),
            );
            self.draw_action(
                canvas,
                self.ending_return_rect(),
                "RETURN TO TITLE",
                self.hover == Hover::ReturnTitle,
            );
            let summary = chapter_two_consequence_summary();
            draw_wrapped(
                canvas,
                &summary,
                tp.summary.0,
                tp.summary.1,
                tp.summary.2,
                FontRole::SerifRegular,
                BONE,
                NARRATIVE_LINE_HEIGHT,
            );
            return;
        }
        let card_y = self.layout.image.y + 36;
        let artifact = Rect::new(
            self.layout.image.x + self.layout.image.w as i32 / 2 - 154,
            card_y,
            308,
            52,
        );
        canvas.fill_rect(artifact, BONE);
        canvas.draw_rect(artifact, SUNLIGHT);
        draw_center(
            canvas,
            artifact,
            "REVISION 7  /  SUNSET LOT 17",
            FontRole::MonoRegular,
            OBSIDIAN,
        );
        let title_y = artifact.bottom() + 22;
        draw_center(
            canvas,
            Rect::new(self.layout.image.x, title_y, self.layout.image.w, 28),
            "CHAPTER ONE COMPLETE",
            FontRole::UiTitle,
            BONE,
        );
        let theme_y = title_y + 38;
        draw_center(
            canvas,
            Rect::new(self.layout.image.x, theme_y, self.layout.image.w, 20),
            "The address waits beyond the year.",
            FontRole::SerifRegular,
            SUNLIGHT,
        );
        self.draw_action(
            canvas,
            self.layout.continue_chapter_two,
            "CONTINUE TO CHAPTER TWO",
            self.hover == Hover::ContinueChapterTwo,
        );
        self.draw_action(
            canvas,
            self.layout.return_title,
            "RETURN TO TITLE",
            self.hover == Hover::ReturnTitle,
        );
        draw_wrapped(
            canvas,
            "You have not found an answer. You have learned that ECHO can be a record, a prediction, or a witness\u{2014}and that someone already chose to close the door behind you.",
            self.layout.narrative.x + 12,
            self.layout.narrative.y + 18,
            self.layout.narrative.w as i32 - 24,
            FontRole::SerifRegular,
            BONE,
            NARRATIVE_LINE_HEIGHT,
        );
    }

    fn draw_choice(
        &self,
        canvas: &mut Canvas,
        rect: Rect,
        text: &str,
        hovered: bool,
        focused: bool,
        visible_index: usize,
    ) {
        let border = if hovered || focused {
            SUNLIGHT
        } else {
            Color::rgba(0xED, 0xE6, 0xD8, 96)
        };
        let fill = if hovered {
            Color::rgba(0xFF, 0x98, 0x00, 48)
        } else {
            Color::rgba(0xED, 0xE6, 0xD8, 16)
        };
        canvas.blend_rounded_rect(rect, 6, fill);
        canvas.stroke_rounded_rect(rect, 6, if focused { 2 } else { 1 }, border);
        let shortcut = shortcut_label(visible_index);
        // Shortcut stays on the first line; wrapped prose indents past it.
        draw_text(
            canvas,
            shortcut,
            rect.x + 10,
            rect.y + 10,
            &TextStyle::new(FontRole::UiMedium, SUNLIGHT),
        );
        let text_width = (rect.w as i32 - CHOICE_TEXT_INSET_LEFT - CHOICE_TEXT_INSET_RIGHT).max(24);
        draw_wrapped(
            canvas,
            text,
            rect.x + CHOICE_TEXT_INSET_LEFT,
            rect.y + 8,
            text_width,
            FontRole::UiRegular,
            BONE,
            CHOICE_LINE_HEIGHT,
        );
    }

    fn draw_action(&self, canvas: &mut Canvas, rect: Rect, text: &str, hovered: bool) {
        canvas.blend_rounded_rect(
            rect,
            7,
            if hovered {
                Color::rgba(0xFF, 0x98, 0x00, 78)
            } else {
                Color::rgba(0xED, 0xE6, 0xD8, 20)
            },
        );
        canvas.stroke_rounded_rect(
            rect,
            7,
            if hovered { 2 } else { 1 },
            if hovered { SUNLIGHT } else { BONE },
        );
        draw_center(canvas, rect, text, FontRole::UiMedium, BONE);
    }
}

impl App for SiliconEchoesApp {
    fn view(&mut self, canvas: &mut Canvas, _: &Theme) {
        self.layout = Layout::new(canvas.width, canvas.height);
        self.draw_frame(canvas);
        match self.mode {
            Mode::Title => self.draw_title(canvas),
            Mode::Play => self.draw_play(canvas),
            Mode::Ending => self.draw_ending(canvas),
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::MouseMove { x, y } => {
                let next = self.hit_test(x, y);
                if next != self.hover {
                    self.hover = next;
                    set_client_cursor(if next == Hover::None {
                        CursorShape::Pointer
                    } else {
                        CursorShape::Hand
                    });
                    return true;
                }
                false
            }
            Event::Click { x, y } => {
                if self.suppress_next_click || !self.focused {
                    self.suppress_next_click = false;
                    self.hover = Hover::None;
                    return true;
                }
                if self.mode == Mode::Play
                    && self.presentation.is_revealing()
                    && self.layout.narrative.contains(Point::new(x, y))
                {
                    return self.presentation.skip_reveal(monotonic_millis());
                }
                self.activate_hover(self.hit_test(x, y));
                true
            }
            Event::Key('\n') | Event::Key('\r') | Event::Key(' ') => {
                self.keyboard_activate(monotonic_millis());
                true
            }
            Event::Key(ch) if self.mode == Mode::Play => {
                let now = monotonic_millis();
                let visible_count = self.available_choice_indices().len();
                if matches!(ch, 'e' | 'E')
                    && self.presentation_accepts_input()
                    && self.game.supports_echo_overlay()
                    && visible_count < 5
                {
                    if self.game.toggle_echo_layer() {
                        self.save_game();
                        return true;
                    }
                }
                if let Some(index) =
                    self.shortcut_gate
                        .shortcut_index(ch, self.presentation_accepts_input(), now)
                {
                    self.activate_shortcut(index);
                }
                true
            }
            Event::KeyPress {
                keycode,
                pressed,
                shift,
                ..
            } => {
                let was_down = self.key_down[keycode as usize];
                self.key_down[keycode as usize] = pressed;
                if !pressed || was_down {
                    return false;
                }
                match keycode {
                    KEY_ESC => {
                        self.back();
                        true
                    }
                    KEY_ENTER => {
                        self.keyboard_activate(monotonic_millis());
                        true
                    }
                    KEY_UP | KEY_LEFT if self.mode == Mode::Play => {
                        self.select_next_choice(-1);
                        true
                    }
                    KEY_DOWN | KEY_RIGHT if self.mode == Mode::Play => {
                        self.select_next_choice(1);
                        true
                    }
                    KEY_TAB if self.mode == Mode::Play => {
                        self.select_next_choice(if shift { -1 } else { 1 });
                        true
                    }
                    _ => false,
                }
            }
            Event::FocusChanged { focused } => {
                self.focused = focused;
                self.hover = Hover::None;
                self.suppress_next_click = focused;
                self.key_down = [false; 256];
                self.shortcut_gate.block_until_quiet(monotonic_millis());
                true
            }
            Event::MouseDown { .. } | Event::MouseUp { .. } => !self.focused,
            Event::Tick => {
                let now = monotonic_millis();
                let presentation_changed = self.mode == Mode::Play && self.presentation.tick(now);
                if now.saturating_sub(self.last_tick_ms) >= 90 {
                    self.last_tick_ms = now;
                    return true;
                }
                presentation_changed
            }
            _ => false,
        }
    }

    fn on_ready(&mut self) -> bool {
        self.seed_ambient();
        self.load_save();
        true
    }

    fn poll_timeout_ms(&self) -> u64 {
        if self.mode == Mode::Play
            && matches!(
                self.presentation.state(),
                ScenePresentation::Entering
                    | ScenePresentation::Revealing
                    | ScenePresentation::PostRevealPause
            )
        {
            16
        } else {
            90
        }
    }
}

fn hotspot_label(hotspot_id: HotspotId) -> &'static str {
    hotspot(hotspot_id).label
}

fn scene_object_bounds(scene_id: SceneId, image: Rect) -> Vec<(SceneObjectTarget, Rect)> {
    let choice = |id| SceneObjectTarget::Choice(ChoiceId(id));
    match scene_id.0 {
        "hallway" => vec![
            (
                choice("hallway.inspect-note"),
                Rect::new(image.x + 120, image.y + 94, 72, 38),
            ),
            (
                choice("hallway.leave-note"),
                Rect::new(
                    image.x + image.w as i32 * 41 / 100,
                    image.y + 40,
                    image.w * 20 / 100,
                    image.h * 58 / 100,
                ),
            ),
        ],
        "kitchen" => vec![
            (
                choice("kitchen.read-newspaper"),
                Rect::new(
                    image.x + 148,
                    image.y + image.h as i32 * 58 / 100 - 42,
                    150,
                    92,
                ),
            ),
            (
                choice("kitchen.study-photo"),
                Rect::new(
                    image.x + 366,
                    image.y + image.h as i32 * 58 / 100 - 28,
                    86,
                    62,
                ),
            ),
        ],
        "landing" => vec![
            (
                choice("landing.take-card"),
                Rect::new(image.x + image.w as i32 * 42 / 100, image.y + 148, 72, 44),
            ),
            (
                choice("landing.leave-card"),
                Rect::new(
                    image.x + image.w as i32 * 41 / 100,
                    image.y + 40,
                    image.w * 20 / 100,
                    image.h * 58 / 100,
                ),
            ),
        ],
        "stairwell" => vec![
            (
                choice("stairwell.help-vale"),
                Rect::new(image.x + 174, image.y + 126, 112, 82),
            ),
            (
                choice("stairwell.take-stairs"),
                Rect::new(image.x + 126, image.bottom() - 178, 258, 128),
            ),
        ],
        "street" => vec![
            (
                choice("street.follow-pager"),
                Rect::new(image.x + image.w as i32 * 68 / 100, image.y + 92, 62, 102),
            ),
            (
                choice("street.ask-vendor"),
                Rect::new(image.x + image.w as i32 * 34 / 100, image.y + 190, 72, 112),
            ),
        ],
        "diner" => vec![
            (
                choice("diner.tell-riley"),
                Rect::new(image.x + image.w as i32 * 66 / 100, image.y + 114, 76, 150),
            ),
            (
                choice("diner.test-riley"),
                Rect::new(image.x + image.w as i32 * 48 / 100, image.y + 154, 104, 70),
            ),
        ],
        "phone" => vec![
            (
                choice("phone.record-message"),
                Rect::new(image.x + image.w as i32 * 68 / 100, image.y + 80, 104, 88),
            ),
            (
                choice("phone.hang-up"),
                Rect::new(
                    image.x + image.w as i32 * 49 / 100,
                    image.y + image.h as i32 * 62 / 100,
                    108,
                    54,
                ),
            ),
        ],
        "repair-shop" => vec![
            (
                choice("repair.ask-lio"),
                Rect::new(image.x + image.w as i32 * 22 / 100, image.y + 96, 80, 148),
            ),
            (
                choice("repair.borrow-manual"),
                Rect::new(
                    image.x + image.w as i32 * 45 / 100,
                    image.y + image.h as i32 * 50 / 100,
                    100,
                    72,
                ),
            ),
        ],
        "transit" => vec![
            (
                choice("transit.wait"),
                Rect::new(image.x + image.w as i32 * 58 / 100, image.y + 54, 210, 72),
            ),
            (
                choice("transit.walk"),
                Rect::new(image.x + 108, image.bottom() - 130, 160, 74),
            ),
        ],
        "archive-lobby" => vec![
            (
                choice("archive.use-card"),
                Rect::new(
                    image.x + image.w as i32 * 52 / 100,
                    image.y + image.h as i32 * 50 / 100,
                    210,
                    140,
                ),
            ),
            (
                choice("archive.ask-public"),
                Rect::new(image.x + image.w as i32 * 36 / 100, image.y + 142, 76, 146),
            ),
        ],
        "archive-stacks" => vec![
            (
                choice("stacks.read-ledger"),
                Rect::new(image.x + image.w as i32 * 25 / 100, image.y + 120, 106, 92),
            ),
            (
                choice("stacks.search-terminal"),
                Rect::new(
                    image.x + image.w as i32 * 52 / 100,
                    image.y + image.h as i32 * 50 / 100,
                    210,
                    170,
                ),
            ),
        ],
        "revelation" => vec![
            (
                choice("revelation.call-riley"),
                Rect::new(
                    image.x + image.w as i32 * 52 / 100,
                    image.y + image.h as i32 * 50 / 100,
                    210,
                    170,
                ),
            ),
            (
                choice("revelation.carry-alone"),
                Rect::new(image.x + image.w as i32 * 35 / 100, image.y + 136, 112, 112),
            ),
        ],
        "turning-point" => vec![(
            choice("turning-point.keep-address"),
            Rect::new(
                image.x + image.w as i32 * 52 / 100 - 16,
                image.y + image.h as i32 * 50 / 100 - 42,
                242,
                170,
            ),
        )],
        "c2-address" => vec![
            (
                choice("c2.address.call-riley"),
                Rect::new(
                    image.x + 180,
                    image.y + image.h as i32 * 63 / 100 - 72,
                    242,
                    52,
                ),
            ),
            (
                choice("c2.address.keep-quiet"),
                Rect::new(
                    image.x + image.w as i32 * 62 / 100,
                    image.y + image.h as i32 * 55 / 100,
                    170,
                    100,
                ),
            ),
        ],
        "c2-contact" => vec![
            (
                choice("c2.contact.wait-riley"),
                Rect::new(image.x + image.w as i32 * 42 / 100, image.y + 118, 110, 120),
            ),
            (
                choice("c2.contact.follow-pager"),
                Rect::new(image.right() - 210, image.y + 78, 124, 80),
            ),
        ],
        "c2-frequency" => vec![
            (
                choice("c2.frequency.ask-lio"),
                Rect::new(image.x + 132, image.y + 66, 84, 154),
            ),
            (
                choice("c2.frequency.read-manual"),
                Rect::new(
                    image.right() - 230,
                    image.y + image.h as i32 * 62 / 100 - 80,
                    104,
                    70,
                ),
            ),
            (
                choice("c2.frequency.trace-caller"),
                Rect::new(
                    image.x + image.w as i32 * 45 / 100,
                    image.y + image.h as i32 * 62 / 100 - 60,
                    126,
                    56,
                ),
            ),
        ],
        "c2-records" => vec![
            (
                choice("c2.records.directory"),
                Rect::new(
                    image.x + 180,
                    image.y + image.h as i32 * 63 / 100 - 72,
                    242,
                    52,
                ),
            ),
            (
                choice("c2.records.permit"),
                Rect::new(
                    image.x + image.w as i32 * 62 / 100,
                    image.y + image.h as i32 * 55 / 100,
                    170,
                    100,
                ),
            ),
        ],
        "c2-route" => vec![
            (
                choice("c2.route.wait-service"),
                Rect::new(image.x + 86, image.y + 62, 226, 84),
            ),
            (
                choice("c2.route.walk-cut"),
                Rect::new(
                    image.x + image.w as i32 * 42 / 100,
                    image.y + 72,
                    246,
                    image.h * 62 / 100,
                ),
            ),
        ],
        "c2-exterior" => vec![
            (
                choice("c2.exterior.ask-caretaker"),
                Rect::new(
                    image.x + 146,
                    image.y + image.h as i32 * 65 / 100 - 100,
                    82,
                    106,
                ),
            ),
            (
                choice("c2.exterior.inspect-facade"),
                Rect::new(
                    image.x + image.w as i32 * 42 / 100,
                    image.y + 72,
                    246,
                    image.h * 62 / 100,
                ),
            ),
        ],
        "c2-caretaker" => vec![
            (
                choice("c2.caretaker.accept-key"),
                Rect::new(image.x + 270, image.y + 174, 54, 34),
            ),
            (
                choice("c2.caretaker.refuse-key"),
                Rect::new(image.x + 178, image.y + 94, 74, 150),
            ),
        ],
        "c2-entry" => vec![
            (
                choice("c2.entry.service-door"),
                Rect::new(
                    image.x + image.w as i32 * 43 / 100 + 54,
                    image.y + 144,
                    94,
                    134,
                ),
            ),
            (
                choice("c2.entry.projector-door"),
                Rect::new(
                    image.x + image.w as i32 * 43 / 100 + 66,
                    image.y + 132,
                    94,
                    134,
                ),
            ),
        ],
        "c2-overlay" => vec![
            (
                choice("c2.overlay.inspect-physical-door"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 54,
                    image.y + 64 + 156,
                    100,
                    116,
                ),
            ),
            (
                choice("c2.overlay.inspect-revision-door"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 178,
                    image.y + 64 + 154,
                    84,
                    120,
                ),
            ),
        ],
        "c2-disagreement" => vec![
            (
                choice("c2.disagreement.keep-physical"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 54,
                    image.y + 64 + 156,
                    100,
                    116,
                ),
            ),
            (
                choice("c2.disagreement.keep-revision"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 178,
                    image.y + 64 + 154,
                    84,
                    120,
                ),
            ),
        ],
        "c2-personal-record" => vec![
            (
                choice("c2.personal.open-card"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 188,
                    image.y + 64 + 176,
                    92,
                    54,
                ),
            ),
            (
                choice("c2.personal.leave-card"),
                Rect::new(image.x + 166, image.y + 148, 64, 116),
            ),
        ],
        "c2-intervention" => vec![(
            choice("c2.intervention.follow"),
            Rect::new(
                image.x + image.w as i32 * 38 / 100 + 92,
                image.y + 64 + 72,
                138,
                78,
            ),
        )],
        "c2-chamber" => vec![
            (
                choice("c2.chamber.read-revisions"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 20,
                    image.y + 64 + 30,
                    286,
                    36,
                ),
            ),
            (
                choice("c2.chamber.seal-port"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 232,
                    image.y + 64 + 220,
                    64,
                    64,
                ),
            ),
        ],
        "c2-predicted-choice" => vec![
            (
                choice("c2.predicted.refuse"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 92,
                    image.y + 64 + 72,
                    138,
                    78,
                ),
            ),
            (
                choice("c2.predicted.preserve"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 54,
                    image.y + 64 + 156,
                    100,
                    116,
                ),
            ),
            (
                choice("c2.predicted.reinterpret"),
                Rect::new(image.x + 166, image.y + 148, 64, 116),
            ),
        ],
        "c2-response" => vec![
            (
                choice("c2.response.disconnect"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 232,
                    image.y + 64 + 220,
                    64,
                    64,
                ),
            ),
            (
                choice("c2.response.send-name"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 92,
                    image.y + 64 + 72,
                    138,
                    78,
                ),
            ),
        ],
        "c2-consequence" => vec![(
            choice("c2.consequence.leave"),
            Rect::new(
                image.x + image.w as i32 * 38 / 100 + 178,
                image.y + 64 + 154,
                84,
                120,
            ),
        )],
        "c2-displacement" => vec![
            (
                choice("c2.displacement.take-cartridge"),
                Rect::new(
                    image.right() - 164,
                    image.y + image.h as i32 * 65 / 100 - 60,
                    96,
                    40,
                ),
            ),
            (
                choice("c2.displacement.leave-cartridge"),
                Rect::new(
                    image.x + image.w as i32 * 42 / 100 + 54,
                    image.y + 144,
                    94,
                    134,
                ),
            ),
        ],
        "c2-turning-point" => vec![
            (
                choice("c2.turning.keep-channel"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 92,
                    image.y + 64 + 72,
                    138,
                    78,
                ),
            ),
            (
                choice("c2.turning.close-notebook"),
                Rect::new(
                    image.x + image.w as i32 * 38 / 100 + 54,
                    image.y + 64 + 156,
                    100,
                    116,
                ),
            ),
        ],
        _ => Vec::new(),
    }
}

fn draw_center(canvas: &mut Canvas, rect: Rect, text: &str, role: FontRole, color: Color) {
    let width = measure_text(text, role).w as i32;
    draw_text_vcenter(
        canvas,
        text,
        rect.x + (rect.w as i32 - width) / 2,
        rect.y,
        rect.h,
        &TextStyle::new(role, color),
    );
}

fn shortcut_label(index: usize) -> &'static str {
    const LABELS: [&str; 26] = [
        "[A]", "[B]", "[C]", "[D]", "[E]", "[F]", "[G]", "[H]", "[I]", "[J]", "[K]", "[L]", "[M]",
        "[N]", "[O]", "[P]", "[Q]", "[R]", "[S]", "[T]", "[U]", "[V]", "[W]", "[X]", "[Y]", "[Z]",
    ];
    LABELS.get(index).copied().unwrap_or("[?]")
}

fn prepare_text_lines(text: &str, max_width: i32, role: FontRole) -> Vec<TextLine> {
    let mut lines = Vec::new();
    let mut paragraph_start = 0;
    for (index, ch) in text
        .char_indices()
        .chain(core::iter::once((text.len(), '\n')))
    {
        if ch != '\n' {
            continue;
        }
        prepare_paragraph_lines(text, paragraph_start, index, max_width, role, &mut lines);
        if index < text.len() {
            lines.push(TextLine {
                start: index,
                end: index,
            });
        }
        paragraph_start = index.saturating_add(ch.len_utf8());
    }
    lines
}

fn prepare_paragraph_lines(
    text: &str,
    start: usize,
    end: usize,
    max_width: i32,
    role: FontRole,
    lines: &mut Vec<TextLine>,
) {
    let mut line_start = start;
    while line_start < end {
        let Some(ch) = text[line_start..].chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        line_start += ch.len_utf8();
    }
    let mut word_start = line_start;
    let mut index = line_start;
    while index < end {
        let ch = text[index..].chars().next().unwrap();
        let next = index + ch.len_utf8();
        if ch.is_whitespace() {
            if word_start < index
                && measure_text(&text[line_start..index], role).w as i32 > max_width
                && line_start < word_start
            {
                lines.push(TextLine {
                    start: line_start,
                    end: word_start.saturating_sub(1),
                });
                line_start = word_start;
            }
            word_start = next;
        }
        index = next;
    }
    if line_start < end
        && measure_text(&text[line_start..end], role).w as i32 > max_width
        && line_start < word_start
    {
        lines.push(TextLine {
            start: line_start,
            end: word_start.saturating_sub(1),
        });
        line_start = word_start;
    }
    if line_start < end {
        lines.push(TextLine {
            start: line_start,
            end,
        });
    }
}

fn draw_prepared_prefix(
    canvas: &mut Canvas,
    text: &str,
    lines: &[TextLine],
    visible_end: usize,
    x: i32,
    y: i32,
    role: FontRole,
    color: Color,
    line_height: i32,
) {
    for (index, line) in lines.iter().enumerate() {
        if visible_end <= line.start {
            break;
        }
        let end = line.end.min(visible_end);
        if end > line.start {
            draw_text(
                canvas,
                &text[line.start..end],
                x,
                y + index as i32 * line_height,
                &TextStyle::new(role, color),
            );
        }
    }
}

fn draw_wrapped(
    canvas: &mut Canvas,
    text: &str,
    x: i32,
    y: i32,
    max_width: i32,
    role: FontRole,
    color: Color,
    line_height: i32,
) {
    let mut line = String::new();
    let mut current_y = y;
    for word in text.split_whitespace() {
        let separator = if line.is_empty() { "" } else { " " };
        let candidate_len =
            measure_text(word, role).w as i32 + measure_text(separator, role).w as i32;
        if !line.is_empty() && measure_text(&line, role).w as i32 + candidate_len > max_width {
            draw_text(canvas, &line, x, current_y, &TextStyle::new(role, color));
            line.clear();
            current_y += line_height;
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        draw_text(canvas, &line, x, current_y, &TextStyle::new(role, color));
    }
}

fn kv_cap() -> Result<CapabilityToken, ()> {
    if KV_LOOKED_UP.load(Ordering::Relaxed) {
        let cap = unsafe { KV_CAP };
        return if cap == CapabilityToken::INVALID {
            Err(())
        } else {
            Ok(cap)
        };
    }
    let cap = nameserver_lookup_timeout("sunlight-kv", KV_LOOKUP_TIMEOUT_MS).ok_or(())?;
    unsafe {
        KV_CAP = cap;
    }
    KV_LOOKED_UP.store(true, Ordering::Relaxed);
    Ok(cap)
}

fn kv_put(key: &str, value: &[u8]) -> Result<(), SaveStage> {
    if key.len() > SHM_PAGE || value.len() > SHM_PAGE {
        debug_log("[SILICON-ECHOES] kv_put: request too large\n");
        return Err(SaveStage::RequestTooLarge);
    }
    let cap = kv_cap().map_err(|()| SaveStage::TransportSend)?;
    let (key_ptr, key_token) = shm_alloc().map_err(|_| {
        debug_log("[SILICON-ECHOES] kv_put: shm_alloc key failed\n");
        SaveStage::TransportSend
    })?;
    let (value_ptr, value_token) = shm_alloc().map_err(|_| {
        let _ = shm_free(key_token);
        debug_log("[SILICON-ECHOES] kv_put: shm_alloc value failed\n");
        SaveStage::TransportSend
    })?;
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
        core::ptr::copy_nonoverlapping(value.as_ptr(), value_ptr, value.len());
    }
    let result = ipc_call_timeout(
        cap,
        IpcMsg::with_label(KV_PUT_SHM2)
            .word(0, key.len() as u64)
            .word(1, value.len() as u64)
            .with_cap(0, key_token)
            .with_cap(1, value_token),
        KV_TIMEOUT_MS,
    );
    let _ = shm_free(key_token);
    let _ = shm_free(value_token);
    match result {
        Ok(reply) if reply.label == KV_REPLY && reply.words[0] == 0 => Ok(()),
        Ok(reply) => {
            debug_log("[SILICON-ECHOES] kv_put: unexpected reply\n");
            if reply.label == KV_ERROR {
                Err(SaveStage::ServiceValidate)
            } else {
                Err(SaveStage::ServiceDecode)
            }
        }
        Err(e) => {
            let _ = e;
            debug_log("[SILICON-ECHOES] kv_put: ipc_call_timeout failed\n");
            Err(SaveStage::ReplyTimeout)
        }
    }
}

fn kv_get(key: &str) -> Result<Option<Vec<u8>>, SaveStage> {
    if key.len() > SHM_PAGE {
        return Err(SaveStage::RequestTooLarge);
    }
    let cap = kv_cap().map_err(|()| SaveStage::TransportSend)?;
    let (key_ptr, key_token) = shm_alloc().map_err(|_| SaveStage::TransportSend)?;
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
    }
    let result = ipc_call_timeout(
        cap,
        IpcMsg::with_label(KV_GET_SHM2)
            .word(0, key.len() as u64)
            .with_cap(0, key_token),
        KV_TIMEOUT_MS,
    );
    let _ = shm_free(key_token);
    let reply = result.map_err(|_| SaveStage::ReplyTimeout)?;
    if reply.label == KV_ERROR && reply.words[0] == 2 {
        return Ok(None);
    }
    if reply.label != KV_VALUE || reply.caps[0] == CapabilityToken::INVALID {
        return Err(SaveStage::ReplyDecode);
    }
    let length = (reply.words[0] as usize).min(SHM_PAGE);
    let token = reply.caps[0];
    let pointer = shm_map(token).map_err(|_| {
        let _ = shm_free(token);
        SaveStage::ReplyDecode
    })?;
    let value = unsafe { core::slice::from_raw_parts(pointer, length) }.to_vec();
    let _ = shm_free(token);
    Ok(Some(value))
}

fn verify_saved_snapshot(key: &str, expected: &GameState) -> Result<(), SaveStage> {
    let Some(bytes) = kv_get(key)? else {
        return Err(SaveStage::ServiceValidate);
    };
    let loaded = decode_save(&bytes).map_err(|_| SaveStage::ReplyDecode)?;
    if loaded == *expected {
        Ok(())
    } else {
        Err(SaveStage::ReplyMismatch)
    }
}

fn argument_present(argc: u64, argv: *const *const u8, wanted: &[u8]) -> bool {
    let mut raw = [core::ptr::null(); 4];
    let count = unsafe { libc::crt0::collect_raw_args(argc, argv, &mut raw) };
    raw[..count].iter().any(|pointer| {
        let length = unsafe { libc::crt0::cstr_len(*pointer, 32) };
        let bytes = unsafe { core::slice::from_raw_parts(*pointer, length) };
        bytes == wanted
    })
}

fn run_display_stress() -> bool {
    for _ in 0..16 {
        let Some(mut window) = Window::connect(WindowConfig {
            width: 480,
            height: 320,
            title: "Silicon Echoes Surface Check",
            decoration: WindowDecoration::CompactClose,
        }) else {
            return false;
        };
        let mut app = SurfaceStressApp { frames_left: 3 };
        window.run(&mut app);
    }
    true
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[SILICON-ECHOES] panic\n");
    loop {
        process_yield();
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    if argument_present(argc, argv, b"--stress") {
        let baseline = sunlight_libc::alloc::heap_stats();
        let result = run_deterministic_stress(4096);
        let recovered = sunlight_libc::alloc::heap_stats();
        if result.is_err()
            || recovered.failed_allocation_count != baseline.failed_allocation_count
            || recovered.live_allocation_count > baseline.live_allocation_count.saturating_add(8)
        {
            debug_log("[SILICON-ECHOES] stress failed\n");
            ProcessExit::exit(1);
        }
        debug_log("[SILICON-ECHOES] stress passed\n");
        ProcessExit::exit(0);
    }
    if argument_present(argc, argv, b"--display-stress") {
        if run_display_stress() {
            debug_log("[SILICON-ECHOES] display stress passed\n");
            ProcessExit::exit(0);
        }
        debug_log("[SILICON-ECHOES] display stress failed\n");
        ProcessExit::exit(1);
    }
    if validate_graph().is_err() {
        debug_log("[SILICON-ECHOES] invalid story graph\n");
        ProcessExit::exit(1);
    }
    let mut app = SiliconEchoesApp::new();
    let Some(mut window) = Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Silicon Echoes: 1993",
        decoration: WindowDecoration::Normal,
    }) else {
        debug_log("[SILICON-ECHOES] display unavailable\n");
        ProcessExit::exit(1);
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}
