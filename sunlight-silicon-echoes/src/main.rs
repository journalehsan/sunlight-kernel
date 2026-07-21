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
    decode_save, encode_save, hotspot, node, presentation_narration, run_deterministic_stress,
    validate_graph, ChoiceId, GameState, HotspotId, SaveError, SceneId, StoryNodeId, Transition,
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SceneObjectTarget {
    Hotspot(HotspotId),
    Choice(ChoiceId),
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
        Self {
            frame,
            image,
            narrative,
            choices,
            title_new,
            title_continue,
            return_title,
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
    started_ms: u64,
    last_tick_ms: u64,
    ambient_seed: u32,
    focused: bool,
    suppress_next_click: bool,
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
            started_ms: monotonic_millis(),
            last_tick_ms: 0,
            ambient_seed: 0x1993_0317,
            focused: true,
            suppress_next_click: false,
            scene_cache: None,
        }
    }

    fn refresh_scene_cache(&mut self) {
        let narration = node(self.game.current_node)
            .map(|story_node| presentation_narration(&self.game, story_node))
            .unwrap_or_default();
        self.scene_cache = Some(Box::new(SceneCache {
            node: self.game.current_node,
            narration,
        }));
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
                Err(()) => service_failed = true,
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
        let slot = self.next_save_slot();
        match kv_put(slot, &bytes).and_then(|()| verify_saved_snapshot(slot, &self.game)) {
            Ok(()) => {
                self.saved_game = Some(self.game.clone());
                self.save_notice = "Saved";
            }
            Err(()) => {
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
        scene_object_bounds(scene_id, self.layout.image)
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
            Hover::Object(SceneObjectTarget::Hotspot(hotspot)) => {
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
            Mode::Play => {
                self.save_game();
                self.return_to_title();
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
            _ => "SILICON ECHOES",
        };
        let glow = Color::rgba(0xFF, 0x98, 0x00, 38);
        let soft = Color::rgba(0xED, 0xE6, 0xD8, 92);
        let strong = Color::rgba(0xED, 0xE6, 0xD8, 180);
        match scene_id {
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
                    let board = Rect::new(rect.x + rect.w as i32 * 58 / 100, rect.y + 54, 210, 72);
                    canvas.fill_rect(board, OBSIDIAN);
                    canvas.draw_rect(board, BONE);
                    draw_text(
                        canvas,
                        "ARCHIVE LINE",
                        board.x + 14,
                        board.y + 19,
                        &TextStyle::new(FontRole::UiSmall, BONE),
                    );
                    draw_text(
                        canvas,
                        "CANCELED",
                        board.x + 14,
                        board.y + 45,
                        &TextStyle::new(FontRole::MonoRegular, SUNLIGHT),
                    );
                }
                for index in 0..5 {
                    let x = rect.x + 116 + index * 150;
                    canvas.fill_rounded_rect(Rect::new(x, skyline_y + 20, 16, 40), 7, strong);
                    canvas.fill_rounded_rect(Rect::new(x - 5, skyline_y + 10, 26, 20), 10, BONE);
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
                let mara_head = Rect::new(table.x + 39, table.y - 93, 34, 34);
                let mara_body = Rect::new(table.x + 29, table.y - 62, 56, 62);
                canvas.fill_rounded_rect(mara_head, 17, BONE);
                canvas.fill_rounded_rect(mara_body, 17, Color::rgba(0xED, 0xE6, 0xD8, 165));
                canvas.hline(
                    mara_body.x + 8,
                    mara_body.bottom() - 5,
                    mara_body.w - 16,
                    soft,
                );
                let riley_head = Rect::new(table.x + 207, table.y - 99, 38, 38);
                let riley_body = Rect::new(table.x + 195, table.y - 64, 62, 64);
                canvas.fill_rounded_rect(riley_head, 18, SUNLIGHT);
                canvas.fill_rounded_rect(riley_body, 18, strong);
                canvas.hline(
                    riley_body.x + 9,
                    riley_body.bottom() - 5,
                    riley_body.w - 18,
                    SUNLIGHT,
                );
                canvas.fill_rounded_rect(Rect::new(table.x + 96, table.y - 15, 22, 18), 5, BONE);
                canvas.hline(table.x + 98, table.y + 4, 18, SUNLIGHT);
                canvas.fill_rect(Rect::new(table.x + 151, table.y - 22, 12, 25), BONE);
                canvas.hline(table.x + 146, table.y - 22, 22, BONE);
                let clock = Rect::new(rect.right() - 105, rect.y + 42, 54, 54);
                canvas.stroke_rounded_rect(clock, 27, 2, BONE);
                canvas.hline(clock.x + 27, clock.y + 27, 17, SUNLIGHT);
                canvas.vline(clock.x + 27, clock.y + 12, 15, BONE);
                if scene_id == "phone" {
                    let phone = Rect::new(rect.x + rect.w as i32 * 68 / 100, rect.y + 80, 104, 88);
                    canvas.blend_rounded_rect(phone, 12, glow);
                    canvas.stroke_rounded_rect(phone, 12, 2, BONE);
                    canvas.fill_rounded_rect(
                        Rect::new(phone.x + 18, phone.y + 39, 68, 27),
                        8,
                        BONE,
                    );
                    canvas.stroke_rounded_rect(
                        Rect::new(phone.x + 12, phone.y + 12, 80, 28),
                        12,
                        3,
                        SUNLIGHT,
                    );
                    for row in 0..3 {
                        for col in 0..3 {
                            canvas.fill_rounded_rect(
                                Rect::new(phone.x + 25 + col * 17, phone.y + 70 + row * 11, 8, 6),
                                2,
                                Color::rgba(0xED, 0xE6, 0xD8, 150),
                            );
                        }
                    }
                }
            }
            "repair-shop" => {
                for index in 0..5 {
                    let shelf = Rect::new(rect.x + 74, rect.y + 42 + index * 41, rect.w - 148, 3);
                    canvas.hbar(shelf.x, shelf.y, shelf.w, 3, BONE);
                    for item in 0..6 {
                        canvas.fill_rect(
                            Rect::new(shelf.x + 24 + item * 118, shelf.y - 28, 50, 27),
                            Color::rgba(0xED, 0xE6, 0xD8, 54),
                        );
                    }
                }
                let pager = Rect::new(
                    rect.x + rect.w as i32 * 61 / 100,
                    rect.y + rect.h as i32 * 61 / 100,
                    120,
                    50,
                );
                canvas.blend_rounded_rect(pager, 7, Color::rgba(0xED, 0xE6, 0xD8, 76));
                canvas.stroke_rounded_rect(pager, 7, 2, BONE);
                draw_center(canvas, pager, "88.3", FontRole::MonoMedium, SUNLIGHT);
                canvas.fill_rounded_rect(
                    Rect::new(rect.x + 154, rect.y + 108, 48, 104),
                    20,
                    Color::rgba(0xED, 0xE6, 0xD8, 155),
                );
                canvas.fill_rounded_rect(Rect::new(rect.x + 160, rect.y + 75, 36, 38), 18, BONE);
                canvas.hline(rect.x + 144, rect.y + 172, 68, SUNLIGHT);
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
            }
            "archive-lobby" | "archive-stacks" | "revelation" | "turning-point" => {
                for index in 0..6 {
                    let x = rect.x + 62 + index * 148;
                    canvas.fill_rect(
                        Rect::new(x, rect.y + 44, 102, rect.h - 112),
                        Color::rgba(0xED, 0xE6, 0xD8, 28),
                    );
                    for shelf in 0..5 {
                        canvas.hline(x + 8, rect.y + 72 + shelf * 42, 86, soft);
                    }
                }
                let terminal = Rect::new(
                    rect.x + rect.w as i32 * 61 / 100,
                    rect.y + rect.h as i32 * 55 / 100,
                    220,
                    92,
                );
                canvas.blend_rounded_rect(terminal, 9, Color::rgba(0xED, 0xE6, 0xD8, 76));
                canvas.stroke_rounded_rect(terminal, 9, 2, BONE);
                canvas.fill_rect(terminal.inset(12), OBSIDIAN);
                draw_text(
                    canvas,
                    "ECHO / REVISION",
                    terminal.x + 22,
                    terminal.y + 31,
                    &TextStyle::new(FontRole::MonoRegular, SUNLIGHT),
                );
                draw_text(
                    canvas,
                    "1993  <>  FUTURE",
                    terminal.x + 22,
                    terminal.y + 57,
                    &TextStyle::new(FontRole::UiSmall, BONE),
                );
                let slot = Rect::new(terminal.x + 142, terminal.y + 69, 48, 7);
                canvas.fill_rounded_rect(slot, 3, SUNLIGHT);
                for light in 0..3 {
                    canvas.fill_rounded_rect(
                        Rect::new(terminal.x + 18 + light * 12, terminal.y + 72, 6, 6),
                        3,
                        if light == 1 { SUNLIGHT } else { BONE },
                    );
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
                    let ledger = Rect::new(rect.x + 300, rect.y + 160, 106, 92);
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
                            terminal.x - 16,
                            terminal.y - 16,
                            terminal.w + 32,
                            terminal.h + 32,
                        ),
                        Color::rgba(0xFF, 0x98, 0x00, 16),
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
                    let address = Rect::new(terminal.x - 22, terminal.y - 36, 264, 44);
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
        draw_text(
            canvas,
            label,
            rect.x + 32,
            rect.y + 38,
            &TextStyle::new(FontRole::MonoRegular, SUNLIGHT),
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

    fn draw_play(&self, canvas: &mut Canvas) {
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
        canvas.blend_rect(self.layout.image, Color::rgba(0x0A, 0x0A, 0x0C, 168));
        let artifact = Rect::new(
            self.layout.image.x + self.layout.image.w as i32 / 2 - 154,
            self.layout.image.y + 44,
            308,
            56,
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
        draw_center(
            canvas,
            Rect::new(
                self.layout.image.x,
                self.layout.image.y + 122,
                self.layout.image.w,
                24,
            ),
            "CHAPTER ONE COMPLETE",
            FontRole::UiTitle,
            BONE,
        );
        draw_center(
            canvas,
            Rect::new(
                self.layout.image.x,
                self.layout.image.y + 158,
                self.layout.image.w,
                18,
            ),
            "The address waits beyond the year.",
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
            "You have not found an answer. You have learned that ECHO can be a record, a prediction, or a witness—and that someone already chose to close the door behind you.",
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
        canvas.blend_rounded_rect(rect, 6, fill);
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
                self.hover = Hover::None;
                self.suppress_next_click = focused;
                true
            }
            Event::MouseDown { .. } | Event::MouseUp { .. } => !self.focused,
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
                Rect::new(image.x + image.w as i32 * 24 / 100, image.y + 122, 78, 148),
            ),
            (
                choice("repair.borrow-manual"),
                Rect::new(
                    image.x + image.w as i32 * 61 / 100,
                    image.y + image.h as i32 * 61 / 100,
                    120,
                    50,
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
                    image.x + image.w as i32 * 61 / 100,
                    image.y + image.h as i32 * 55 / 100,
                    96,
                    66,
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
                Rect::new(image.x + image.w as i32 * 30 / 100, image.y + 138, 106, 92),
            ),
            (
                choice("stacks.search-terminal"),
                Rect::new(
                    image.x + image.w as i32 * 61 / 100,
                    image.y + image.h as i32 * 55 / 100,
                    220,
                    92,
                ),
            ),
        ],
        "revelation" => vec![
            (
                choice("revelation.call-riley"),
                Rect::new(
                    image.x + image.w as i32 * 61 / 100,
                    image.y + image.h as i32 * 55 / 100,
                    220,
                    92,
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
                image.x + image.w as i32 * 60 / 100,
                image.y + image.h as i32 * 52 / 100,
                232,
                112,
            ),
        )],
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

fn verify_saved_snapshot(key: &str, expected: &GameState) -> Result<(), ()> {
    let Some(bytes) = kv_get(key)? else {
        return Err(());
    };
    let loaded = decode_save(&bytes).map_err(|_| ())?;
    if loaded == *expected {
        Ok(())
    } else {
        Err(())
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
