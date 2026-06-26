#![no_std]
#![no_main]

use sunlight_ipc::{debug_log, process_yield, ProcessExit};
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
}

impl RunnerApp {
    fn new() -> Self {
        let mut open = TextInput::new(Rect::default());
        open.active = true;
        Self {
            open,
            elevated: Checkbox::new(Rect::default(), "Run with elevated privileges"),
            status: "Ready",
        }
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
            self.status = "Elevated launch is not available here";
            return true;
        }

        let input = trim_ascii(self.open.value().as_bytes());
        if input.is_empty() {
            self.status = "Enter an application name";
            return true;
        }

        let mut args_buf = [[0u8; MAX_ARG_LEN]; MAX_ARGS];
        let mut arg_lens = [0usize; MAX_ARGS];
        let arg_count = match parse_args(input, &mut args_buf, &mut arg_lens) {
            Ok(0) => {
                self.status = "Enter an application name";
                return true;
            }
            Ok(count) => count,
            Err(LaunchParseError::TooManyArgs) => {
                self.status = "Too many arguments";
                return true;
            }
            Err(LaunchParseError::ArgTooLong) => {
                self.status = "Argument is too long";
                return true;
            }
        };

        let mut path_buf = [0u8; MAX_PATH_LEN];
        let path_len = match resolve_path(&args_buf[0][..arg_lens[0]], &mut path_buf) {
            Some(len) => len,
            None => {
                self.status = "Executable path is too long";
                return true;
            }
        };

        let mut argv: [&[u8]; MAX_ARGS] = [&[]; MAX_ARGS];
        for idx in 0..arg_count {
            argv[idx] = &args_buf[idx][..arg_lens[idx]];
        }

        match libc::spawn(&path_buf[..path_len], &argv[..arg_count], None) {
            Ok(_) => {
                request_close();
                true
            }
            Err(_) => {
                self.status = "Launch failed";
                true
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
