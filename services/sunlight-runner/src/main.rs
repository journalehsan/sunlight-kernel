#![no_std]
#![no_main]

use sun_font::{FontRole, VecFont};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, show_notification, NotificationKind, ProcessExit,
};
use sunlight_libc::sun_exec;
use sunlight_ui::{
    request_close,
    widgets::{Button, ButtonState, Checkbox, Label, Panel, TextInput},
    App, Event, HBox, Rect, VBox, Window, WindowConfig,
};

static F_UI:    VecFont = VecFont(FontRole::UiRegular);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);

const WIN_W: u32 = 420;
const WIN_H: u32 = 180;
const KEY_ENTER: u8 = 0x1C;
const KEY_Q: u8 = 0x10;
const MAX_ARGS: usize = 8;
const MAX_PATH_LEN: usize = 96;

struct NoAlloc;

unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[RUNNER] panic\n");
    loop {
        process_yield();
    }
}

struct RunnerApp {
    open: TextInput<128>,
    elevated: Checkbox<'static>,
    status: &'static str,
    next_launch_id: u64,
}

impl RunnerApp {
    fn new() -> Self {
        let mut open = TextInput::new(Rect::default());
        open.active = true;
        Self {
            open,
            elevated: Checkbox::new(Rect::default(), "Run with elevated privileges"),
            status: "Ready",
            next_launch_id: 1,
        }
    }

    fn finish(&mut self) {
        self.open.active = false;
        self.elevated.active = false;
        request_close();
    }

    fn next_trace(&mut self) -> LaunchTrace {
        let trace = LaunchTrace::new(
            self.next_launch_id,
            LaunchSource::Runner,
            sunlight_ipc::monotonic_millis(),
        );
        self.next_launch_id = self.next_launch_id.saturating_add(1);
        trace
    }

    fn register_launch_trace(&self, trace: LaunchTrace, pid: u64) {
        let Some(display_ep) = sunlight_ipc::nameserver_lookup("display_server") else {
            return;
        };
        let _ = sunlight_ipc::ipc_call_timeout(
            display_ep,
            sunlight_ipc::IpcMsg::with_label(sunlight_ipc::SgpMsg::LAUNCH_TRACE)
                .word(0, trace.launch_id)
                .word(1, trace.source as u64)
                .word(2, pid)
                .word(3, trace.requested_at_ms),
            50,
        );
    }

    fn fail(&mut self, status: &'static str, body: &str) -> bool {
        self.status = status;
        let _ = show_notification(NotificationKind::Error, "Launch failed", body, 30_000);
        self.finish();
        true
    }

    fn layout(&self) -> (Rect, Rect, Rect, Rect, Rect, Rect, Rect) {
        let root = Rect::new(12, 12, WIN_W - 24, WIN_H - 24);
        let row_heights = [18, 28, 18, 28];
        let mut rows = VBox::new(root.inset(12))
            .with_spacing(10)
            .layout(&row_heights);
        let label = rows.next().unwrap_or_default();
        let input_row = rows.next().unwrap_or_default();
        let checkbox_row = rows.next().unwrap_or_default();
        let actions_row = rows.next().unwrap_or_default();

        let input_widths = [input_row.w.saturating_sub(88), 80];
        let mut input_cells = HBox::new(input_row).with_spacing(8).layout(&input_widths);
        let input = input_cells.next().unwrap_or_default();
        let browse = input_cells.next().unwrap_or_default();

        let action_widths = [72, 72];
        let mut actions = HBox::new(actions_row)
            .with_spacing(8)
            .layout(&action_widths);
        let run = actions
            .next()
            .unwrap_or_default()
            .translate((actions_row.w as i32 - 152).max(0), 0);
        let cancel = actions
            .next()
            .unwrap_or_default()
            .translate((actions_row.w as i32 - 152).max(0), 0);

        (root, label, input, browse, checkbox_row, run, cancel)
    }

    fn launch(&mut self) -> bool {
        if self.elevated.checked {
            return self.fail(
                "Elevated launch is not available here",
                "Elevated launch is not available here",
            );
        }

        let mut input_buf = [0u8; MAX_PATH_LEN];
        let input_len = {
            let input = trim_ascii(self.open.value().as_bytes());
            if input.is_empty() {
                return self.fail("Enter an application name", "Enter an application name");
            }
            let len = input.len().min(input_buf.len());
            input_buf[..len].copy_from_slice(&input[..len]);
            len
        };
        let input = &input_buf[..input_len];

        let trace = self.next_trace();
        let command = core::str::from_utf8(input).unwrap_or("command");
        launch_trace::log_phase_now(trace, command, "launch_request_received", None);
        launch_trace::log_phase_now(trace, command, "duplicate_launch_check_done", None);
        launch_trace::log_phase_now(trace, command, "parse_args_start", None);

        let mut words: [&[u8]; MAX_ARGS] = [&[]; MAX_ARGS];
        let word_count = match sun_exec::split_words(input, &mut words) {
            Ok(0) => return self.fail("Enter an application name", "Enter an application name"),
            Ok(count) => count,
            Err(sun_exec::LaunchError::TooManyArgs) => {
                return self.fail("Too many arguments", "Too many arguments")
            }
            Err(sun_exec::LaunchError::ArgTooLong) => {
                return self.fail("Argument is too long", "Argument is too long")
            }
            Err(_) => return self.fail("Invalid command", "Invalid command"),
        };
        launch_trace::log_phase_now(trace, command, "parse_args_done", None);

        match sun_exec::launch_from_words(trace, LaunchSource::Runner, &words[..word_count], true) {
            Ok(result) => {
                self.register_launch_trace(trace, result.pid);
                self.finish();
                true
            }
            Err(err) => {
                let mut body = [0u8; 64];
                let prefix = launch_error_prefix(err);
                let mut len = 0usize;
                body[..prefix.len()].copy_from_slice(prefix);
                len += prefix.len();
                let command_bytes = command.as_bytes();
                let take = command_bytes.len().min(body.len().saturating_sub(len));
                body[len..len + take].copy_from_slice(&command_bytes[..take]);
                len += take;
                let body = core::str::from_utf8(&body[..len]).unwrap_or("Could not start app");
                self.fail("Launch failed", body)
            }
        }
    }
}

impl App for RunnerApp {
    fn view(&mut self, canvas: &mut sunlight_ui::Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        let (root, label_rect, input_rect, browse_rect, checkbox_rect, run_rect, cancel_rect) =
            self.layout();
        Panel::with_title(root, "Run").draw(canvas, theme);

        self.open.rect = input_rect;
        self.elevated.rect = checkbox_rect;

        Label::new(label_rect, "Open:").with_font(&F_UI).draw(canvas, theme);
        self.open.draw(canvas, theme);
        self.elevated.draw(canvas, theme);

        let mut browse = Button::secondary(browse_rect, "Browse").with_font(&F_UI);
        browse.state = ButtonState::Normal;
        browse.draw(canvas, theme);

        let mut run = Button::new(run_rect, "Run").with_font(&F_UI);
        run.state = ButtonState::Normal;
        run.draw(canvas, theme);

        let mut cancel = Button::secondary(cancel_rect, "Cancel").with_font(&F_UI);
        cancel.state = ButtonState::Normal;
        cancel.draw(canvas, theme);

        Label::new(
            Rect::new(root.x + 12, root.bottom() - 22, 220, 14),
            self.status,
        )
        .with_font(&F_SMALL)
        .draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        let (_, _, _, browse_rect, _, run_rect, cancel_rect) = self.layout();
        if self.open.update(event) || self.elevated.update(event) {
            return true;
        }

        if let Event::Key('\n') = event {
            return self.launch();
        }

        if let Event::KeyPress {
            keycode: KEY_ENTER,
            pressed: true,
            ..
        } = event
        {
            return self.launch();
        }

        if let Event::KeyPress {
            keycode: KEY_Q,
            pressed: true,
            ctrl: true,
            ..
        } = event
        {
            request_close();
            return true;
        }

        if let Event::Click { x, y } = event {
            let point = sunlight_ui::Point::new(x, y);
            if browse_rect.contains(point) {
                self.status = "Browse is not implemented";
                return true;
            }
            if run_rect.contains(point) {
                return self.launch();
            }
            if cancel_rect.contains(point) {
                request_close();
                return true;
            }
        }
        false
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut app = RunnerApp::new();
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Run",
    }) {
        Some(window) => window,
        None => loop {
            process_yield();
        },
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &bytes[start..end]
}

fn launch_error_prefix(err: sun_exec::LaunchError) -> &'static [u8] {
    match err {
        sun_exec::LaunchError::AppNotFound => b"App not found: ",
        sun_exec::LaunchError::InvalidCommand => b"Invalid command: ",
        sun_exec::LaunchError::SpawnFailed(_) => b"Spawn failed: ",
        sun_exec::LaunchError::PermissionDenied => b"Permission denied: ",
        sun_exec::LaunchError::DisplayUnavailable => b"Display unavailable: ",
        sun_exec::LaunchError::TooManyArgs => b"Too many args: ",
        sun_exec::LaunchError::ArgTooLong => b"Argument too long: ",
    }
}
