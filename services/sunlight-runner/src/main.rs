#![no_std]
#![no_main]

use sunlight_ipc::{debug_log, process_yield};
use sunlight_ui::{
    App, Event, HBox, Rect, VBox, Window, WindowConfig,
    widgets::{Button, ButtonState, Checkbox, Label, Panel, TextInput},
};

const WIN_W: u32 = 420;
const WIN_H: u32 = 180;

struct NoAlloc;

unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 { core::ptr::null_mut() }
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
        Self {
            open: TextInput::new(Rect::default()),
            elevated: Checkbox::new(Rect::default(), "Run with elevated privileges"),
            status: "Ready",
        }
    }

    fn layout(&self) -> (Rect, Rect, Rect, Rect, Rect, Rect, Rect) {
        let root = Rect::new(12, 12, WIN_W - 24, WIN_H - 24);
        let row_heights = [18, 28, 18, 28];
        let mut rows = VBox::new(root.inset(12)).with_spacing(10).layout(&row_heights);
        let label = rows.next().unwrap_or_default();
        let input_row = rows.next().unwrap_or_default();
        let checkbox_row = rows.next().unwrap_or_default();
        let actions_row = rows.next().unwrap_or_default();

        let input_widths = [input_row.w.saturating_sub(88), 80];
        let mut input_cells = HBox::new(input_row).with_spacing(8).layout(&input_widths);
        let input = input_cells.next().unwrap_or_default();
        let browse = input_cells.next().unwrap_or_default();

        let action_widths = [72, 72];
        let mut actions = HBox::new(actions_row).with_spacing(8).layout(&action_widths);
        let run = actions.next().unwrap_or_default().translate((actions_row.w as i32 - 152).max(0), 0);
        let cancel = actions.next().unwrap_or_default().translate((actions_row.w as i32 - 152).max(0), 0);

        (root, label, input, browse, checkbox_row, run, cancel)
    }
}

impl App for RunnerApp {
    fn view(&mut self, canvas: &mut sunlight_ui::Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        let (root, label_rect, input_rect, browse_rect, checkbox_rect, run_rect, cancel_rect) = self.layout();
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

        Label::new(Rect::new(root.x + 12, root.bottom() - 22, 220, 14), self.status).draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        let (_, _, _, browse_rect, _, run_rect, cancel_rect) = self.layout();
        if self.open.update(event) || self.elevated.update(event) {
            return true;
        }

        if let Event::Click { x, y } = event {
            let point = sunlight_ui::Point::new(x, y);
            if browse_rect.contains(point) {
                self.status = "Browse is not implemented";
                return true;
            }
            if run_rect.contains(point) {
                if self.elevated.checked {
                    // TODO: Send IPC request to capabilityctl for elevated tokens.
                    self.status = "Elevated launch pending";
                } else {
                    // TODO: Spawn process with standard /tmp and /home VFS tokens.
                    self.status = "Standard launch pending";
                }
                return true;
            }
            if cancel_rect.contains(point) {
                self.status = "Cancelled";
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
    loop {
        process_yield();
    }
}
