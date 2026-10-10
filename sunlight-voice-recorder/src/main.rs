#![no_std]
#![no_main]

extern crate alloc;
use alloc::{format, string::String, vec};
use sun_font::{draw_text, draw_text_centered, FontRole, TextStyle, Typography};
use sunlight_dialogs::{
    ConfirmRequest, ConfirmStyle, DialogButton, DialogClient, DialogCommonOptions, DialogRequest,
    DialogResult, SaveFileRequest,
};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    ProcessExit,
};
use sunlight_ui::{
    request_close, set_client_cursor,
    widgets::{Button, ButtonState},
    App, Canvas, CursorShape, Event, Point, Rect, Theme, Window, WindowConfig, WindowDecoration,
    WindowEvent,
};
use sunlight_voice_recorder::{
    state::State,
    worker::{recording_directory, Command, Snapshot, Worker},
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[VOICE-RECORDER] panic");
    ProcessExit::exit(1);
}

struct RecorderApp {
    worker: Option<Worker>,
    snapshot: Option<Snapshot>,
    width: u32,
    height: u32,
    hover: Option<usize>,
    pressed: Option<usize>,
    message: &'static str,
}
impl RecorderApp {
    fn new() -> Self {
        Self {
            worker: None,
            snapshot: None,
            width: 480,
            height: 350,
            hover: None,
            pressed: None,
            message: "Ready to record",
        }
    }
    fn buttons(&self) -> [(Option<Command>, &'static str); 4] {
        let state = self.snapshot.map_or(State::Idle, |s| s.state);
        match state {
            State::Idle => [
                (Some(Command::Record), "Record"),
                (None, "Pause"),
                (None, "Save As"),
                (None, "New"),
            ],
            State::Recording => [
                (Some(Command::Pause), "Pause"),
                (Some(Command::Stop), "Stop"),
                (None, "Save As"),
                (None, "New"),
            ],
            State::Paused => [
                (Some(Command::Resume), "Resume"),
                (Some(Command::Stop), "Stop"),
                (None, "Save As"),
                (None, "New"),
            ],
            State::Recorded => [
                (Some(Command::Play), "Play"),
                (None, "Stop"),
                (Some(Command::Save), "Save As"),
                (Some(Command::New), "New"),
            ],
            State::Playing => [
                (Some(Command::PausePlayback), "Pause"),
                (Some(Command::Stop), "Stop"),
                (None, "Save As"),
                (None, "New"),
            ],
            State::PlaybackPaused => [
                (Some(Command::ResumePlayback), "Resume"),
                (Some(Command::Stop), "Stop"),
                (None, "Save As"),
                (None, "New"),
            ],
            State::Finalizing => [
                (None, "Finalizing"),
                (None, "Stop"),
                (None, "Save As"),
                (None, "New"),
            ],
            State::Error => [
                (None, "Record"),
                (None, "Stop"),
                (None, "Save As"),
                (Some(Command::New), "New"),
            ],
        }
    }
    fn rect(&self, index: usize) -> Rect {
        let width = self.width.saturating_sub(56) / 2;
        Rect::new(
            24 + (index % 2) as i32 * (width + 8) as i32,
            196 + (index / 2) as i32 * 44,
            width,
            36,
        )
    }
    fn enabled(&self, index: usize) -> bool {
        self.worker.is_some()
            && !self.snapshot.is_some_and(|s| s.busy)
            && self.buttons()[index].0.is_some()
    }
    fn hit(&self, x: i32, y: i32) -> Option<usize> {
        (0..4).find(|i| self.enabled(*i) && self.rect(*i).contains(Point::new(x, y)))
    }
    fn confirm(&self, title: &str, message: String) -> bool {
        matches!(
            DialogClient::new().show(&DialogRequest::Confirm(ConfirmRequest {
                common: DialogCommonOptions::new(title, message).silent(),
                style: ConfirmStyle::YesNo,
                default_button: DialogButton::No,
            })),
            Ok(DialogResult::Yes)
        )
    }
    fn save(&mut self) {
        let timestamp = sunlight_ipc::get_time_utc();
        // Unix seconds are an unambiguous filesystem-safe timestamp even when
        // no local timezone/date service is available.
        let request = DialogRequest::SaveFile(SaveFileRequest {
            title: String::from("Save Recording As"),
            initial_dir: Some(recording_directory()),
            suggested_name: Some(format!("Recording {}.wav", timestamp)),
            default_extension: Some(String::from("wav")),
            allowed_extensions: vec![String::from("wav")],
            overwrite_confirm: true,
            confirm_button_label: Some(String::from("Save")),
        });
        match DialogClient::new().show(&request) {
            Ok(DialogResult::SavePathSelected(path)) => {
                let replace = sunlight_libc::stat(path.as_bytes()).is_ok();
                // Explicit approval for the target observed here; no silent
                // replacement of a file that appeared after the dialog checked it.
                if replace && !self.confirm("Replace Recording?", format!("Replace {}?", path)) {
                    self.message = "Save cancelled";
                    return;
                }
                match self.worker.as_ref().unwrap().save(&path, replace) {
                    Ok(()) => self.message = "Saving…",
                    Err(e) => self.message = e.message(),
                }
            }
            Ok(DialogResult::Cancelled | DialogResult::Cancel | DialogResult::Dismissed) => {
                self.message = "Save cancelled"
            }
            _ => self.message = "Save As dialog unavailable",
        }
    }
    fn activate(&mut self, index: usize) {
        if !self.enabled(index) {
            return;
        }
        let command = self.buttons()[index].0.unwrap();
        if command == Command::Save {
            self.save();
            return;
        }
        if command == Command::New
            && self.snapshot.is_some_and(|s| s.frames != 0 && !s.saved)
            && !self.confirm(
                "Discard Recording?",
                String::from("This recording has not been saved. Discard it?"),
            )
        {
            return;
        }
        match self.worker.as_ref().unwrap().send(command) {
            Ok(()) => self.message = "Working…",
            Err(e) => self.message = e.message(),
        }
        self.poll();
    }
    fn poll(&mut self) -> bool {
        let Some(worker) = self.worker.as_ref() else {
            return false;
        };
        let next = worker.snapshot();
        let changed = self.snapshot != Some(next);
        if changed {
            self.message = next.error.map_or_else(
                || {
                    if next.busy {
                        "Working…"
                    } else {
                        match next.state {
                            State::Idle => "Ready to record",
                            State::Recording => "Recording",
                            State::Paused => "Recording paused",
                            State::Finalizing => "Finalizing recording…",
                            State::Recorded if next.saved => "Recording saved",
                            State::Recorded => "Recording ready — listen or Save As",
                            State::Playing => "Playing",
                            State::PlaybackPaused => "Playback paused",
                            State::Error => "Select New to try again",
                        }
                    }
                },
                |e| e.message(),
            );
            self.snapshot = Some(next);
        }
        changed
    }
}
impl App for RecorderApp {
    fn on_ready(&mut self) -> bool {
        self.worker = Some(Worker::new());
        self.poll();
        true
    }
    fn poll_timeout_ms(&self) -> u64 {
        if self
            .snapshot
            .is_some_and(|s| s.busy || matches!(s.state, State::Recording | State::Playing))
        {
            40
        } else {
            200
        }
    }
    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.width = width;
        self.height = height;
        true
    }
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, self.width, self.height), theme.bg);
        draw_text(
            canvas,
            "Voice Recorder",
            24,
            16,
            &TextStyle::new(FontRole::UiTitle, theme.text),
        );
        let ms = self.snapshot.map_or(0, |s| s.duration_ms());
        let seconds = ms / 1000;
        let timer = format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        );
        draw_text_centered(
            canvas,
            Rect::new(24, 66, self.width.saturating_sub(48), 54),
            &timer,
            &TextStyle::new(FontRole::UiLarge, theme.text),
        );
        let meter = Rect::new(24, 145, self.width.saturating_sub(48), 14);
        canvas.fill_rounded_rect(meter, 7, theme.panel_alt);
        let peak = self.snapshot.map_or(0, |s| s.peak.min(32768));
        let filled = u64::from(meter.w) * u64::from(peak) / 32768;
        if filled != 0 {
            canvas.fill_rounded_rect(
                Rect::new(meter.x, meter.y, filled as u32, meter.h),
                7,
                theme.accent,
            );
        }
        draw_text(
            canvas,
            "Microphone level",
            24,
            166,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );
        for (i, (_, label)) in self.buttons().iter().enumerate() {
            let mut button = if i == 0 {
                Button::new(self.rect(i), label)
            } else {
                Button::secondary(self.rect(i), label)
            }
            .with_font(&Typography::UI_MEDIUM);
            button.state = if !self.enabled(i) {
                ButtonState::Disabled
            } else if self.pressed == Some(i) {
                ButtonState::Pressed
            } else if self.hover == Some(i) {
                ButtonState::Hovered
            } else {
                ButtonState::Normal
            };
            button.draw(canvas, theme);
        }
        let color = if self.snapshot.is_some_and(|s| s.error.is_some()) {
            theme.danger_text
        } else {
            theme.text_muted
        };
        draw_text(
            canvas,
            self.message,
            24,
            290,
            &TextStyle::new(FontRole::UiSmall, color),
        );
        draw_text(
            canvas,
            "R Record · P Pause · S Stop · Space Play · A Save · N New",
            24,
            318,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }
    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => self.poll(),
            Event::MouseMove { x, y, .. } => {
                let old = self.hover;
                self.hover = self.hit(x, y);
                set_client_cursor(if self.hover.is_some() {
                    CursorShape::Hand
                } else {
                    CursorShape::Pointer
                });
                old != self.hover
            }
            Event::MouseDown { x, y, button: 0 } => {
                self.pressed = self.hit(x, y);
                true
            }
            Event::Click { x, y } | Event::MouseUp { x, y, button: 0 } => {
                let pressed = self.pressed.take();
                if let Some(i) = pressed.filter(|i| self.hit(x, y) == Some(*i)) {
                    self.activate(i);
                }
                true
            }
            Event::Key(ch) => {
                self.poll();
                match ch.to_ascii_lowercase() {
                    'r' if self.snapshot.is_some_and(|s| s.state == State::Idle) => {
                        self.activate(0)
                    }
                    'p' | ' ' => self.activate(0),
                    's' => self.activate(1),
                    'a' => self.activate(2),
                    'n' => self.activate(3),
                    _ => return false,
                }
                true
            }
            Event::KeyPress {
                keycode: 1,
                pressed: true,
                ..
            } => {
                request_close();
                true
            }
            _ => false,
        }
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, envp: *const *const u8) -> ! {
    sunlight_libc::env::init(envp);
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=voice-recorder",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );
    let mut app = RecorderApp::new();
    let Some(mut window) = Window::connect(WindowConfig {
        width: 480,
        height: 350,
        title: "Sunlight Voice Recorder",
        decoration: WindowDecoration::Normal,
    }) else {
        ProcessExit::exit(1);
    };
    if !window.set_close_grace(6000) {
        debug_log("[VOICE-RECORDER] close grace unavailable");
        ProcessExit::exit(1);
    }
    window.run(&mut app);
    drop(app);
    ProcessExit::exit(0);
}
