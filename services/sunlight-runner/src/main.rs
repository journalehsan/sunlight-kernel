#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, show_notification, NotificationKind, ProcessExit,
};
use sunlight_libc as libc;
use sunlight_ui::{
    request_close,
    widgets::{Button, ButtonState, Checkbox, Label, Panel, TextInput},
    App, Event, HBox, Rect, VBox, Window, WindowConfig,
};

const WIN_W: u32 = 420;
const WIN_H: u32 = 180;
const KEY_ENTER: u8 = 0x1C;
const KEY_Q: u8 = 0x10;
const MAX_ARGS: usize = 8;
const MAX_ARG_LEN: usize = 64;
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

        let mut command_buf = [0u8; MAX_PATH_LEN];
        let command_len = input.len().min(command_buf.len());
        command_buf[..command_len].copy_from_slice(&input[..command_len]);
        let trace = self.next_trace();
        let command = core::str::from_utf8(&command_buf[..command_len]).unwrap_or("command");
        launch_trace::log_phase_now(trace, command, "launch_request_received", None);
        launch_trace::log_phase_now(trace, command, "duplicate_launch_check_done", None);
        launch_trace::log_phase_now(trace, command, "parse_args_start", None);

        let mut args_buf = [[0u8; MAX_ARG_LEN]; MAX_ARGS];
        let mut arg_lens = [0usize; MAX_ARGS];
        let arg_count = match parse_args(input, &mut args_buf, &mut arg_lens) {
            Ok(0) => return self.fail("Enter an application name", "Enter an application name"),
            Ok(count) => count,
            Err(LaunchParseError::TooManyArgs) => {
                return self.fail("Too many arguments", "Too many arguments")
            }
            Err(LaunchParseError::ArgTooLong) => {
                return self.fail("Argument is too long", "Argument is too long")
            }
        };
        launch_trace::log_phase_now(trace, command, "parse_args_done", None);

        launch_trace::log_phase_now(trace, command, "resolve_path_start", None);
        let mut path_buf = [0u8; MAX_PATH_LEN];
        let path_len = match resolve_path(&args_buf[0][..arg_lens[0]], &mut path_buf) {
            Some(len) => len,
            None => return self.fail("Executable path is too long", "Executable path is too long"),
        };
        launch_trace::log_phase_now(trace, command, "resolve_path_done", None);

        let mut argv: [&[u8]; MAX_ARGS] = [&[]; MAX_ARGS];
        for idx in 0..arg_count {
            argv[idx] = &args_buf[idx][..arg_lens[idx]];
        }
        let gui_target = is_trace_launchable_gui_app(&path_buf[..path_len]);
        let mut trace_arg = [0u8; 64];
        let mut argv_len = arg_count;
        if gui_target && argv_len < MAX_ARGS {
            if let Some(trace_arg_len) = launch_trace::format_launch_arg(trace, &mut trace_arg) {
                argv[argv_len] = &trace_arg[..trace_arg_len];
                argv_len += 1;
            }
        }

        launch_trace::log_phase_now(trace, command, "spawn_start", None);
        match libc::spawn(&path_buf[..path_len], &argv[..argv_len], None) {
            Ok(pid) => {
                launch_trace::log_phase_now(trace, command, "spawn_returned", Some(pid));
                self.register_launch_trace(trace, pid);
                launch_trace::log_phase_now(trace, command, "process_created", Some(pid));
                self.finish();
                true
            }
            Err(_) => {
                launch_trace::log_phase_now(trace, command, "spawn_failed", None);
                let mut body = [0u8; 64];
                let prefix = b"Could not start ";
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

        Label::new(label_rect, "Open:").draw(canvas, theme);
        self.open.draw(canvas, theme);
        self.elevated.draw(canvas, theme);

        let mut browse = Button::secondary(browse_rect, "Browse");
        browse.state = ButtonState::Normal;
        browse.draw(canvas, theme);

        let mut run = Button::new(run_rect, "Run");
        run.state = ButtonState::Normal;
        run.draw(canvas, theme);

        let mut cancel = Button::secondary(cancel_rect, "Cancel");
        cancel.state = ButtonState::Normal;
        cancel.draw(canvas, theme);

        Label::new(
            Rect::new(root.x + 12, root.bottom() - 22, 220, 14),
            self.status,
        )
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

fn is_trace_launchable_gui_app(path: &[u8]) -> bool {
    matches!(
        path,
        b"/bin/calculator"
            | b"/usr/bin/calculator"
            | b"/bin/sunlight-files"
            | b"/usr/bin/sunlight-files"
            | b"/bin/control-panel"
            | b"/usr/bin/control-panel"
            | b"/bin/sunlight-tasks"
            | b"/usr/bin/sunlight-tasks"
            | b"/bin/sunlight-terminal"
            | b"/usr/bin/sunlight-terminal"
    )
}

enum LaunchParseError {
    TooManyArgs,
    ArgTooLong,
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

fn parse_args(
    input: &[u8],
    args_buf: &mut [[u8; MAX_ARG_LEN]; MAX_ARGS],
    arg_lens: &mut [usize; MAX_ARGS],
) -> Result<usize, LaunchParseError> {
    let mut count = 0usize;
    let mut pos = 0usize;

    while pos < input.len() {
        while pos < input.len() && input[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= input.len() {
            break;
        }
        if count >= MAX_ARGS {
            return Err(LaunchParseError::TooManyArgs);
        }

        let mut len = 0usize;
        while pos < input.len() && !input[pos].is_ascii_whitespace() {
            if len >= MAX_ARG_LEN {
                return Err(LaunchParseError::ArgTooLong);
            }
            args_buf[count][len] = input[pos];
            len += 1;
            pos += 1;
        }
        arg_lens[count] = len;
        count += 1;
    }

    Ok(count)
}

fn resolve_path(command: &[u8], out: &mut [u8; MAX_PATH_LEN]) -> Option<usize> {
    if command.first() == Some(&b'/') {
        let len = command.len().min(out.len());
        if len != command.len() {
            return None;
        }
        out[..len].copy_from_slice(command);
        return Some(len);
    }

    const PREFIX: &[u8] = b"/bin/";
    let total = PREFIX.len() + command.len();
    if total > out.len() {
        return None;
    }
    out[..PREFIX.len()].copy_from_slice(PREFIX);
    out[PREFIX.len()..total].copy_from_slice(command);
    Some(total)
}
