#![no_std]
#![no_main]

extern crate alloc;

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
    decode_save, encode_save, hotspot, node, run_deterministic_stress, validate_graph, GameState,
    HotspotId, SaveError, StoryNodeId, Transition,
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

const OBSIDIAN: Color = Color::rgb(0x0A, 0x0A, 0x0C);
const BONE: Color = Color::rgb(0xED, 0xE6, 0xD8);
const SUNLIGHT: Color = Color::rgb(0xFF, 0x98, 0x00);
const SAVE_KEY: &str = "games/silicon-echoes/save.v1";
const KV_REPLY: u64 = 0x4BFF;
const KV_ERROR: u64 = 0x4BEE;
const KV_VALUE: u64 = 0x4B05;
const KV_PUT_SHM2: u64 = 0x4B08;
const KV_GET_SHM2: u64 = 0x4B09;
const KV_DELETE_SHM2: u64 = 0x4B0A;
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
    Hotspot(HotspotId),
    ReturnTitle,
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
    hotspots: [(HotspotId, Rect); 4],
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
            image.y + image.h as i32 - 92,
            200,
            34,
        );
        let workstation = Rect::new(
            image.x + image.w as i32 * 56 / 100,
            image.y + image.h as i32 * 38 / 100,
            image.w.saturating_mul(24) / 100,
            image.h.saturating_mul(34) / 100,
        );
        let clock = Rect::new(
            image.x + image.w as i32 * 13 / 100,
            image.y + image.h as i32 * 21 / 100,
            image.w.saturating_mul(14) / 100,
            image.h.saturating_mul(17) / 100,
        );
        let desk = Rect::new(
            image.x + image.w as i32 * 44 / 100,
            image.y + image.h as i32 * 64 / 100,
            image.w.saturating_mul(38) / 100,
            image.h.saturating_mul(17) / 100,
        );
        let window = Rect::new(
            image.x + image.w as i32 * 70 / 100,
            image.y + image.h as i32 * 12 / 100,
            image.w.saturating_mul(19) / 100,
            image.h.saturating_mul(35) / 100,
        );
        Self {
            frame,
            image,
            narrative,
            choices,
            title_new,
            title_continue,
            return_title,
            hotspots: [
                (HotspotId::Clock, clock),
                (HotspotId::Workstation, workstation),
                (HotspotId::Desk, desk),
                (HotspotId::Window, window),
            ],
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
    started_ms: u64,
    last_tick_ms: u64,
    ambient_seed: u32,
    focused: bool,
    scene_cache: Option<Box<SceneCache>>,
}

struct SceneCache {
    node: StoryNodeId,
    narration: String,
}

struct SurfaceStressApp {
    frames_left: u8,
}

impl App for SurfaceStressApp {
    fn view(&mut self, canvas: &mut Canvas, _: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, canvas.width, canvas.height), OBSIDIAN);
        canvas.fill_rounded_rect(
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
            started_ms: monotonic_millis(),
            last_tick_ms: 0,
            ambient_seed: 0x1993_0317,
            focused: true,
            scene_cache: None,
        }
    }

    fn refresh_scene_cache(&mut self) {
        let narration = node(self.game.current_node)
            .map(|story_node| String::from(story_node.narration))
            .unwrap_or_default();
        self.scene_cache = Some(Box::new(SceneCache {
            node: self.game.current_node,
            narration,
        }));
    }

    fn load_save(&mut self) {
        match kv_get(SAVE_KEY) {
            Ok(Some(bytes)) => match decode_save(&bytes) {
                Ok(state) => {
                    self.saved_game = Some(state);
                    self.save_notice = "";
                }
                Err(
                    SaveError::InvalidRecord
                    | SaveError::InvalidUtf8
                    | SaveError::UnsupportedVersion,
                ) => {
                    self.saved_game = None;
                    self.save_notice = "An older or damaged echo was left untouched.";
                }
            },
            Ok(None) => {
                self.saved_game = None;
                self.save_notice = "";
            }
            Err(()) => {
                self.saved_game = None;
                self.save_notice = "Save service is asleep. This session remains playable.";
            }
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
        let bytes = encode_save(&self.game);
        match kv_put(SAVE_KEY, &bytes) {
            Ok(()) => {
                self.saved_game = Some(self.game.clone());
                self.save_notice = "Saved";
            }
            Err(()) => self.save_notice = "Save unavailable",
        }
    }

    fn return_to_title(&mut self) {
        self.mode = Mode::Title;
        self.hover = Hover::None;
        self.selected_choice = 0;
        self.load_save();
    }

    fn clear_save_after_ending(&mut self) {
        let _ = kv_delete(SAVE_KEY);
        self.saved_game = None;
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

    fn activate_choice(&mut self, index: usize) {
        let Some(choice) = self.choices().get(index) else {
            return;
        };
        let target = self.game.select_choice(choice.id);
        match target {
            Ok(Transition::Ending(_)) => {
                self.save_game();
                self.mode = Mode::Ending;
                self.clear_save_after_ending();
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
                if self.layout.return_title.contains(point) {
                    Hover::ReturnTitle
                } else {
                    Hover::None
                }
            }
            Mode::Play => {
                for (hotspot, rect) in self.layout.hotspots {
                    if self.can_inspect(hotspot) && rect.contains(point) {
                        return Hover::Hotspot(hotspot);
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

    fn current_node_is_uncontrolled(&self) -> bool {
        node(self.game.current_node)
            .map(|story_node| story_node.uncontrolled_event)
            .unwrap_or(false)
    }

    fn choice_rect(&self, visible_index: usize) -> Rect {
        let height = 38u32;
        Rect::new(
            self.layout.choices.x,
            self.layout.choices.y + visible_index as i32 * 46,
            self.layout.choices.w,
            height,
        )
    }

    fn activate_hover(&mut self, hover: Hover) {
        match hover {
            Hover::TitleNew => self.start_new(),
            Hover::TitleContinue => self.continue_game(),
            Hover::Hotspot(hotspot) => {
                self.game.enter_hotspot(hotspot);
                self.selected_choice = 0;
                self.refresh_scene_cache();
                self.save_game();
            }
            Hover::Choice(_) if self.current_node_is_uncontrolled() => {
                self.advance_uncontrolled_event()
            }
            Hover::Choice(index) => self.activate_choice(index),
            Hover::ReturnTitle => self.return_to_title(),
            Hover::None => {}
        }
    }

    fn select_next_choice(&mut self, direction: i32) {
        let available = self.available_choice_indices();
        if available.is_empty() {
            return;
        }
        let current = available
            .iter()
            .position(|index| *index == self.selected_choice)
            .unwrap_or(0) as i32;
        let next = (current + direction).rem_euclid(available.len() as i32) as usize;
        self.selected_choice = available[next];
    }

    fn keyboard_activate(&mut self) {
        match self.mode {
            Mode::Title => {
                if self.saved_game.is_some() {
                    self.continue_game();
                } else {
                    self.start_new();
                }
            }
            Mode::Ending => self.return_to_title(),
            Mode::Play if self.current_node_is_uncontrolled() => self.advance_uncontrolled_event(),
            Mode::Play => self.activate_choice(self.selected_choice),
        }
    }

    fn back(&mut self) {
        match self.mode {
            Mode::Title => request_close(),
            Mode::Ending => self.return_to_title(),
            Mode::Play if self.game.current_node == StoryNodeId("bedroom.wake") => {
                self.return_to_title()
            }
            Mode::Play => {
                self.game
                    .apply_transition(Transition::Node(StoryNodeId("bedroom.wake")));
                self.selected_choice = 0;
                self.refresh_scene_cache();
            }
        }
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
        let rect = self.layout.image;
        canvas.fill_rect(rect, Color::rgb(0x0A, 0x0A, 0x0C));
        let wall = Rect::new(rect.x + 12, rect.y + 12, rect.w - 24, rect.h * 67 / 100);
        let floor = Rect::new(
            rect.x + 12,
            wall.bottom(),
            rect.w - 24,
            rect.bottom() as u32 - wall.bottom() as u32 - 12,
        );
        canvas.fill_rect(wall, Color::rgba(0xED, 0xE6, 0xD8, 30));
        canvas.fill_rect(floor, Color::rgba(0xED, 0xE6, 0xD8, 18));

        let window = self.layout.hotspots[3].1;
        canvas.fill_rect(window, Color::rgba(0xED, 0xE6, 0xD8, 30));
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

        let clock = self.layout.hotspots[0].1;
        canvas.fill_rounded_rect(clock, 6, Color::rgba(0xED, 0xE6, 0xD8, 36));
        canvas.stroke_rounded_rect(clock, 6, 1, BONE);
        draw_center(canvas, clock, "03:17", FontRole::MonoMedium, BONE);
        draw_center(
            canvas,
            Rect::new(clock.x, clock.bottom() - 16, clock.w, 13),
            "1993",
            FontRole::UiSmall,
            SUNLIGHT,
        );

        let desk = self.layout.hotspots[2].1;
        canvas.fill_rect(desk, Color::rgba(0xED, 0xE6, 0xD8, 52));
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

        let workstation = self.layout.hotspots[1].1;
        let crt = Rect::new(
            workstation.x,
            workstation.y,
            workstation.w,
            workstation.h * 66 / 100,
        );
        let glow = Rect::new(crt.x - 5, crt.y - 5, crt.w + 10, crt.h + 10);
        canvas.fill_rounded_rect(glow, 14, Color::rgba(0xFF, 0x98, 0x00, 20));
        canvas.fill_rounded_rect(crt, 10, Color::rgba(0xED, 0xE6, 0xD8, 120));
        canvas.stroke_rounded_rect(crt, 10, 2, BONE);
        let screen = crt.inset(10);
        canvas.fill_rounded_rect(screen, 5, OBSIDIAN);
        let flicker = ((monotonic_millis() / 120 + self.ambient_seed as u64) & 3) as u8;
        canvas.fill_rect(
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

    fn draw_hotspot_feedback(&self, canvas: &mut Canvas) {
        if let Hover::Hotspot(hotspot) = self.hover {
            let rect = self
                .layout
                .hotspots
                .iter()
                .find(|(candidate, _)| *candidate == hotspot)
                .map(|(_, rect)| *rect)
                .unwrap_or(self.layout.image);
            canvas.stroke_rounded_rect(rect.inset(-3), 5, 2, SUNLIGHT);
            draw_text(
                canvas,
                hotspot_label(hotspot),
                rect.x,
                rect.y - 17,
                &TextStyle::new(FontRole::UiSmall, SUNLIGHT),
            );
        }
    }

    fn draw_title(&self, canvas: &mut Canvas) {
        self.draw_room(canvas);
        canvas.fill_rect(self.layout.image, Color::rgba(0x0A, 0x0A, 0x0C, 138));
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

    fn draw_play(&self, canvas: &mut Canvas) {
        self.draw_room(canvas);
        let story_node = node(self.game.current_node)
            .unwrap_or_else(|| node(StoryNodeId("bedroom.wake")).unwrap());
        canvas.fill_rect(self.layout.narrative, Color::rgba(0xED, 0xE6, 0xD8, 18));
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
        let narration = self
            .scene_cache
            .as_ref()
            .filter(|cache| cache.node == self.game.current_node)
            .map(|cache| cache.narration.as_str())
            .unwrap_or(story_node.narration);
        draw_wrapped(
            canvas,
            narration,
            self.layout.narrative.x + 12,
            self.layout.narrative.y + 34,
            self.layout.narrative.w as i32 * 51 / 100,
            FontRole::SerifRegular,
            BONE,
            22,
        );
        if self.game.flags.get("signal_arrived") {
            draw_text(
                canvas,
                "A pulse answers from the glass.",
                self.layout.narrative.x + 12,
                self.layout.narrative.bottom() - 22,
                &TextStyle::new(FontRole::UiSmall, SUNLIGHT),
            );
        }
        canvas.vline(
            self.layout.choices.x - 12,
            self.layout.choices.y,
            self.layout.choices.h,
            Color::rgba(0xED, 0xE6, 0xD8, 70),
        );
        if self.current_node_is_uncontrolled() {
            self.draw_action(
                canvas,
                self.choice_rect(0),
                "LET THE ROOM MOVE ON",
                self.hover == Hover::Choice(0),
            );
            draw_text(
                canvas,
                "There is no choice here.",
                self.layout.choices.x,
                self.choice_rect(0).bottom() + 12,
                &TextStyle::new(FontRole::UiSmall, Color::rgba(0xED, 0xE6, 0xD8, 140)),
            );
        } else {
            let available = self.available_choice_indices();
            if available.is_empty() {
                draw_text(
                    canvas,
                    "Inspect the room.",
                    self.layout.choices.x,
                    self.layout.choices.y + 12,
                    &TextStyle::new(FontRole::UiRegular, BONE),
                );
            }
            for (visible_index, choice_index) in available.iter().enumerate() {
                let choice = self.choices()[*choice_index];
                let hovered = self.hover == Hover::Choice(*choice_index);
                let focused = self.focused && self.selected_choice == *choice_index;
                self.draw_choice(
                    canvas,
                    self.choice_rect(visible_index),
                    choice.text,
                    hovered,
                    focused,
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
        canvas.fill_rect(self.layout.image, Color::rgba(0x0A, 0x0A, 0x0C, 168));
        draw_center(
            canvas,
            Rect::new(
                self.layout.image.x,
                self.layout.image.y + 90,
                self.layout.image.w,
                24,
            ),
            "VERTICAL SLICE END",
            FontRole::UiTitle,
            BONE,
        );
        draw_center(
            canvas,
            Rect::new(
                self.layout.image.x,
                self.layout.image.y + 126,
                self.layout.image.w,
                18,
            ),
            "The voice waits behind the year.",
            FontRole::SerifRegular,
            SUNLIGHT,
        );
        self.draw_action(
            canvas,
            self.layout.return_title,
            "RETURN TO TITLE",
            self.hover == Hover::ReturnTitle,
        );
        draw_wrapped(
            canvas,
            "You have not found an answer. You have only learned that an answer has been looking for you.",
            self.layout.narrative.x + 12,
            self.layout.narrative.y + 18,
            self.layout.narrative.w as i32 - 24,
            FontRole::SerifRegular,
            BONE,
            22,
        );
    }

    fn draw_choice(
        &self,
        canvas: &mut Canvas,
        rect: Rect,
        text: &str,
        hovered: bool,
        focused: bool,
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
        canvas.fill_rounded_rect(rect, 6, fill);
        canvas.stroke_rounded_rect(rect, 6, if focused { 2 } else { 1 }, border);
        draw_wrapped(
            canvas,
            text,
            rect.x + 10,
            rect.y + 9,
            rect.w as i32 - 18,
            FontRole::UiRegular,
            BONE,
            16,
        );
    }

    fn draw_action(&self, canvas: &mut Canvas, rect: Rect, text: &str, hovered: bool) {
        canvas.fill_rounded_rect(
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
                self.activate_hover(self.hit_test(x, y));
                true
            }
            Event::Key('\n') | Event::Key('\r') | Event::Key(' ') => {
                self.keyboard_activate();
                true
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } => match keycode {
                KEY_ESC => {
                    self.back();
                    true
                }
                KEY_ENTER => {
                    self.keyboard_activate();
                    true
                }
                KEY_UP if self.mode == Mode::Play => {
                    self.select_next_choice(-1);
                    true
                }
                KEY_DOWN if self.mode == Mode::Play => {
                    self.select_next_choice(1);
                    true
                }
                _ => false,
            },
            Event::FocusChanged { focused } => {
                self.focused = focused;
                true
            }
            Event::Tick => {
                let now = monotonic_millis();
                if now.saturating_sub(self.last_tick_ms) >= 90 {
                    self.last_tick_ms = now;
                    return true;
                }
                false
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
        90
    }
}

fn hotspot_label(hotspot_id: HotspotId) -> &'static str {
    hotspot(hotspot_id).label
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

fn kv_put(key: &str, value: &[u8]) -> Result<(), ()> {
    if key.len() > SHM_PAGE || value.len() > SHM_PAGE {
        return Err(());
    }
    let cap = kv_cap()?;
    let (key_ptr, key_token) = shm_alloc().map_err(|_| ())?;
    let (value_ptr, value_token) = shm_alloc().map_err(|_| {
        let _ = shm_free(key_token);
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
        _ => Err(()),
    }
}

fn kv_get(key: &str) -> Result<Option<Vec<u8>>, ()> {
    if key.len() > SHM_PAGE {
        return Err(());
    }
    let cap = kv_cap()?;
    let (key_ptr, key_token) = shm_alloc().map_err(|_| ())?;
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
    let reply = result.map_err(|_| ())?;
    if reply.label == KV_ERROR && reply.words[0] == 2 {
        return Ok(None);
    }
    if reply.label != KV_VALUE || reply.caps[0] == CapabilityToken::INVALID {
        return Err(());
    }
    let length = (reply.words[0] as usize).min(SHM_PAGE);
    let token = reply.caps[0];
    let pointer = shm_map(token).map_err(|_| {
        let _ = shm_free(token);
    })?;
    let value = unsafe { core::slice::from_raw_parts(pointer, length) }.to_vec();
    let _ = shm_free(token);
    Ok(Some(value))
}

fn kv_delete(key: &str) -> Result<(), ()> {
    if key.len() > SHM_PAGE {
        return Err(());
    }
    let cap = kv_cap()?;
    let (key_ptr, key_token) = shm_alloc().map_err(|_| ())?;
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
    }
    let result = ipc_call_timeout(
        cap,
        IpcMsg::with_label(KV_DELETE_SHM2)
            .word(0, key.len() as u64)
            .with_cap(0, key_token),
        KV_TIMEOUT_MS,
    );
    let _ = shm_free(key_token);
    match result {
        Ok(reply)
            if (reply.label == KV_REPLY && reply.words[0] == 0)
                || (reply.label == KV_ERROR && reply.words[0] == 2) =>
        {
            Ok(())
        }
        _ => Err(()),
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
