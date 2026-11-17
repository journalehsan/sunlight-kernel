#![no_std]
#![no_main]
extern crate alloc;
mod audio;
mod input;
mod render;
use input::{Input, TOP};
use sunlight_empires::{BuildingKind, Civilization, Command, Envelope, World};
use sunlight_game::FixedStep;
use sunlight_ipc::{debug_log, monotonic_millis, ProcessExit};
use sunlight_ui::{
    request_close, App, Canvas, Event, Theme, Window, WindowConfig, WindowDecoration, WindowEvent,
};
struct Empires {
    world: World,
    input: Input,
    clock: FixedStep,
    sequence: u32,
    width: u32,
    height: u32,
    notice: &'static str,
    focused: bool,
    close_after: Option<u64>,
    last_rejected: u32,
    last_complete: u32,
    last_accepted: u32,
    last_wood: u32,
    last_stone: u32,
}
impl Empires {
    fn new() -> Self {
        Self {
            world: World::new(Civilization::Persia),
            input: Input::new(),
            clock: FixedStep::new(monotonic_millis(), 50),
            sequence: 0,
            width: 960,
            height: 680,
            notice: "Sunlight Empires | Offline M1 | Select a worker to begin",
            focused: true,
            close_after: None,
            last_rejected: 0,
            last_complete: 0,
            last_accepted: 0,
            last_wood: 240,
            last_stone: 100,
        }
    }
    fn submit(&mut self, c: Command) {
        let e = Envelope {
            tick: self.world.tick,
            player: 0,
            sequence: self.sequence,
            command: c,
        };
        self.sequence += 1;
        debug_log(&alloc::format!(
            "[EMPIRES] submit tick={} command={:?}\n",
            self.world.tick,
            e.command
        ));
        if self.world.submit(e).is_err() {
            self.notice = "Order queue full or invalid";
            audio::cue(audio::Cue::Invalid);
        } else {
            self.notice = "Order queued";
        }
    }
    fn action(&mut self, i: usize) {
        match i {
            0..=2 => {
                if self.input.worker(&self.world).is_some() {
                    self.input.placing = Some(
                        [
                            BuildingKind::House,
                            BuildingKind::Storage,
                            BuildingKind::University,
                        ][i],
                    );
                    self.notice = "Left-click open ground to build; Esc cancels placement";
                } else {
                    self.notice = "Select a worker first";
                }
            }
            3 => {
                if let Some(id) = self.input.building {
                    self.submit(Command::CancelConstruction { building: id });
                }
            }
            4 => {
                self.input.rally = true;
                self.notice = "Select a completed building, then right-click a rally tile";
            }
            _ => {}
        }
    }
    fn new_game(&mut self, civ: Civilization) {
        self.world = World::new(civ);
        self.input = Input::new();
        self.sequence = 0;
        self.last_rejected = 0;
        self.last_complete = 0;
        self.last_accepted = 0;
        self.last_wood = self.world.resources.wood;
        self.last_stone = self.world.resources.stone;
        self.clock.reset(monotonic_millis());
        self.notice = "New settlement | Select a worker";
    }
}
impl App for Empires {
    fn view(&mut self, c: &mut Canvas, t: &Theme) {
        self.width = c.width;
        self.height = c.height;
        render::draw(c, t, &self.world, &mut self.input, self.notice);
    }
    fn poll_timeout_ms(&self) -> u64 {
        16
    }
    fn window_event(&mut self, _: WindowEvent) -> bool {
        true
    }
    fn update(&mut self, e: Event) -> bool {
        // Advance from every event, so continuous pointer motion cannot starve
        // simulation. Clock exists only in this native adapter.
        let now = monotonic_millis();
        let steps = self.clock.advance(now);
        let mut changed = false;
        {
            for _ in 0..steps {
                let active = self
                    .world
                    .units
                    .iter()
                    .any(|u| u.task != sunlight_empires::Task::Idle || !u.path.is_empty());
                self.world.step();
                changed |= active;
            }
        }
        if self.close_after.is_some_and(|tick| self.world.tick >= tick) {
            request_close();
        }
        if self.world.rejected != self.last_rejected {
            changed = true;
            self.last_rejected = self.world.rejected;
            self.notice = "Invalid order: check funds, occupancy, worker and reachability";
            audio::cue(audio::Cue::Invalid);
            debug_log("[EMPIRES] order rejected\n");
        }
        if self.world.accepted != self.last_accepted {
            changed = true;
            self.last_accepted = self.world.accepted;
            self.notice = "Order accepted";
            audio::cue(audio::Cue::Started);
            debug_log("[EMPIRES] order accepted\n");
        }
        if self.world.completed != self.last_complete {
            self.last_complete = self.world.completed;
            self.notice = "Construction completed";
            audio::cue(audio::Cue::Completed);
            debug_log("[EMPIRES] construction completed\n");
        }
        if self.world.resources.wood != self.last_wood
            || self.world.resources.stone != self.last_stone
        {
            debug_log(&alloc::format!(
                "[EMPIRES] balance tick={} wood={} stone={}\n",
                self.world.tick,
                self.world.resources.wood,
                self.world.resources.stone
            ));
            self.last_wood = self.world.resources.wood;
            self.last_stone = self.world.resources.stone;
            changed = true;
        }
        match e {
            Event::FocusChanged { focused } => {
                self.focused = focused;
                self.input.clear_transient();
                true
            }
            Event::MouseMove { x, y } if self.focused => {
                self.input.pointer = (x, y);
                changed || self.input.drag.is_some() || self.input.placing.is_some()
            }
            Event::MouseDown { x, y, button: 0 } if self.focused => {
                if y >= self.height as i32 - 83 && y < self.height as i32 - 56 {
                    self.action(((x - 8) / ((self.width as i32 - 16) / 5).max(1)).max(0) as usize);
                    return true;
                }
                if y < TOP || y >= self.height as i32 - 116 {
                    return changed;
                }
                if self.input.placing.is_some() {
                    if let Some(c) = self.input.order(&self.world, self.input.tile(x, y)) {
                        self.submit(c);
                    }
                    self.input.placing = None;
                } else {
                    self.input.drag = Some((x, y));
                    self.input.pointer = (x, y);
                }
                true
            }
            Event::Click { x, y } | Event::MouseUp { x, y, button: 0 } if self.focused => {
                self.input.select(&self.world, x, y);
                debug_log(&alloc::format!(
                    "[EMPIRES] selection {:?} building {:?}\n",
                    self.input.selected,
                    self.input.building
                ));
                audio::cue(audio::Cue::Selection);
                true
            }
            Event::MouseDown { x, y, button: 1 }
                if self.focused && y >= TOP && y < self.height as i32 - 116 =>
            {
                if let Some(c) = self.input.order(&self.world, self.input.tile(x, y)) {
                    self.submit(c);
                }
                self.input.rally = false;
                true
            }
            Event::MouseWheel { delta, .. } if self.focused => {
                self.input.camera.tile_px =
                    (self.input.camera.tile_px + if delta < 0 { 4 } else { -4 }).clamp(16, 40);
                true
            }
            Event::Key(ch) if self.focused => {
                match ch {
                    'h' | 'H' => self.action(0),
                    's' | 'S' => self.action(1),
                    'u' | 'U' => self.action(2),
                    'x' | 'X' => self.action(3),
                    'r' | 'R' => self.action(4),
                    'p' | 'P' => self.new_game(Civilization::Persia),
                    'g' | 'G' => self.new_game(Civilization::Greece),
                    _ => return changed,
                }
                true
            }
            Event::KeyPress {
                keycode,
                pressed,
                shift,
                ..
            } if self.focused => {
                self.input.shift = shift;
                let previous = self.input.keys[keycode as usize];
                self.input.keys[keycode as usize] = pressed;
                if pressed && !previous {
                    match keycode {
                        1 => {
                            if self.input.placing.take().is_none() {
                                request_close();
                            }
                        }
                        _ => {}
                    }
                }
                true
            }
            Event::Tick => {
                if self.focused && steps > 0 {
                    let cam = &mut self.input.camera;
                    let keys = &self.input.keys;
                    if keys[0x4b] {
                        cam.x -= 1;
                    }
                    if keys[0x4d] {
                        cam.x += 1;
                    }
                    if keys[0x48] {
                        cam.y -= 1;
                    }
                    if keys[0x50] {
                        cam.y += 1;
                    }
                    cam.clamp(64, 64, self.width as i32, self.height as i32 - 116 - TOP);
                }
                changed
            }
            _ => changed,
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[EMPIRES] panic\n");
    ProcessExit::exit(1)
}
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    if argument_present(argc, argv, b"--stress") {
        ProcessExit::exit(if simulation_stress() { 0 } else { 1 });
    }
    if argument_present(argc, argv, b"--display-stress") {
        ProcessExit::exit(if display_stress() { 0 } else { 1 });
    }
    if argument_present(argc, argv, b"--native-suite") {
        // Run without a registered window, so no close-grace timer is active.
        if !simulation_stress() {
            ProcessExit::exit(1);
        }
        for round in 0..2 {
            debug_log(&alloc::format!(
                "[EMPIRES] native fixture interactive round={}\n",
                round + 1
            ));
            if !interactive_window() {
                ProcessExit::exit(1);
            }
        }
        let pass = display_stress();
        debug_log(if pass {
            "[EMPIRES] native fixture suite PASS\n"
        } else {
            "[EMPIRES] native fixture suite FAIL\n"
        });
        ProcessExit::exit(if pass { 0 } else { 1 });
    }
    ProcessExit::exit(if interactive_window() { 0 } else { 1 })
}
fn interactive_window() -> bool {
    let mut app = Empires::new();
    let Some(mut window) = Window::connect(WindowConfig {
        width: 960,
        height: 680,
        title: "Sunlight Empires",
        decoration: WindowDecoration::Normal,
    }) else {
        debug_log("[EMPIRES] display unavailable\n");
        return false;
    };
    if !window.set_close_grace(6_000) {
        debug_log("[EMPIRES] close grace unavailable\n");
        return false;
    }
    debug_log("[EMPIRES] native window connected\n");
    window.run(&mut app);
    drop(window);
    debug_log(&alloc::format!(
        "[EMPIRES] exit tick={} checksum={:016x}\n",
        app.world.tick,
        app.world.checksum()
    ));
    drop(app);
    debug_log("[EMPIRES] clean exit\n");
    true
}

fn argument_present(argc: u64, argv: *const *const u8, wanted: &[u8]) -> bool {
    let mut raw = [core::ptr::null(); 4];
    let n = unsafe { sunlight_libc::crt0::collect_raw_args(argc, argv, &mut raw) };
    raw[..n].iter().any(|p| {
        let len = unsafe { sunlight_libc::crt0::cstr_len(*p, 32) };
        (unsafe { core::slice::from_raw_parts(*p, len) }) == wanted
    })
}
fn simulation_stress() -> bool {
    let before = sunlight_libc::alloc::heap_stats();
    let start = monotonic_millis();
    for _ in 0..64 {
        let mut w = World::new(Civilization::Persia);
        if w.submit(Envelope {
            tick: 0,
            player: 0,
            sequence: 0,
            command: Command::AssignWorker {
                worker: 2,
                node: sunlight_empires::Tile::new(14, 8),
            },
        })
        .is_err()
        {
            return false;
        }
        for _ in 0..400 {
            w.step();
        }
        let bytes = sunlight_empires::replay::save(&w);
        let Ok(recovered) = sunlight_empires::replay::load(&bytes) else {
            return false;
        };
        if recovered.checksum() != w.checksum() {
            return false;
        }
    }
    let elapsed = monotonic_millis().saturating_sub(start);
    let after = sunlight_libc::alloc::heap_stats();
    let pass = after.live_allocation_count == before.live_allocation_count
        && after.requested_user_bytes == before.requested_user_bytes
        && after.failed_allocation_count == before.failed_allocation_count;
    debug_log(&alloc::format!("[EMPIRES] native simulation stress {} scenes=64 ticks=51200 elapsed_ms={} live_before={} live_after={} bytes_before={} bytes_after={} high_water={}\n",if pass{"PASS"}else{"FAIL"},elapsed,before.live_allocation_count,after.live_allocation_count,before.requested_user_bytes,after.requested_user_bytes,after.high_water_allocated_bytes));
    pass
}
fn display_stress() -> bool {
    let mut baseline = None;
    let start = monotonic_millis();
    for _ in 0..12 {
        {
            let mut app = Empires::new();
            app.close_after = Some(3);
            let Some(mut window) = Window::connect(WindowConfig {
                width: 640,
                height: 480,
                title: "Sunlight Empires Surface Check",
                decoration: WindowDecoration::Normal,
            }) else {
                return false;
            };
            if !window.set_close_grace(6_000) {
                return false;
            }
            window.run(&mut app);
            drop(window);
            drop(app);
        }
        let current = sunlight_libc::alloc::heap_stats();
        if let Some((count, bytes, failed)) = baseline {
            if current.live_allocation_count != count
                || current.requested_user_bytes != bytes
                || current.failed_allocation_count != failed
            {
                debug_log("[EMPIRES] native display stress FAIL: heap growth\n");
                return false;
            }
        } else {
            baseline = Some((
                current.live_allocation_count,
                current.requested_user_bytes,
                current.failed_allocation_count,
            ));
        }
    }
    let s = sunlight_libc::alloc::heap_stats();
    debug_log(&alloc::format!("[EMPIRES] native display stress PASS windows=12 elapsed_ms={} live={} bytes={} high_water={}\n",monotonic_millis().saturating_sub(start),s.live_allocation_count,s.requested_user_bytes,s.high_water_allocated_bytes));
    true
}
