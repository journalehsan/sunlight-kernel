#![no_std]
#![no_main]

extern crate alloc;

use sunlight_ipc::{debug_log, process_yield, ProcessExit};
use sunlight_ui::{
    request_close, App, Canvas, Event, Theme, Window, WindowConfig, WindowDecoration,
    WindowMaterial,
};

mod activity;
mod character;
mod conversation;
mod health;
mod privacy;
mod ui;

const WIN_W: u32 = 900;
const WIN_H: u32 = 640;
const KEY_ESC: u8 = 0x01;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[WISEOWL-GUI] panic\n");
    loop {
        process_yield();
    }
}

struct WiseOwlApp {
    ui: ui::UiState,
}

impl WiseOwlApp {
    fn new() -> Self {
        Self {
            ui: ui::UiState::new(WIN_W, WIN_H),
        }
    }
}

impl App for WiseOwlApp {
    fn view(&mut self, canvas: &mut Canvas, _theme: &Theme) {
        self.ui.draw(canvas);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => self.ui.handle_click(x, y),
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } if keycode == KEY_ESC => {
                request_close();
                false
            }
            Event::Tick => true,
            _ => false,
        }
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    debug_log("[WISEOWL-GUI] Starting Wise Owl Console\n");

    let mut app = WiseOwlApp::new();
    let mut window = match Window::connect_with_material(
        WindowConfig {
            width: WIN_W,
            height: WIN_H,
            title: "Wise Owl",
            decoration: WindowDecoration::CompactCloseMinimize,
        },
        WindowMaterial::Opaque,
    ) {
        Some(window) => window,
        None => {
            debug_log("[WISEOWL-GUI] window connect failed\n");
            ProcessExit::exit(1);
        }
    };

    debug_log("[WISEOWL-GUI] WINDOW_CREATED PASS\n");
    window.run(&mut app);
    debug_log("[WISEOWL-GUI] exit\n");
    ProcessExit::exit(0);
}
