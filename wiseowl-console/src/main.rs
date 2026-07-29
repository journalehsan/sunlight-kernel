#![no_std]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

#[cfg(not(test))]
use sunlight_ipc::{debug_log, process_yield, ProcessExit};
use sunlight_ui::{request_close, App, Canvas, Event, Theme};
#[cfg(not(test))]
use sunlight_ui::{Window, WindowConfig, WindowDecoration, WindowMaterial};

mod activity;
mod conversation;
mod health;
mod privacy;
mod transport;
mod ui;

const WIN_W: u32 = 900;
const WIN_H: u32 = 640;
const KEY_ESC: u8 = 0x01;

#[cfg(not(test))]
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

#[cfg(feature = "conversation-v1-test")]
fn run_conversation_v1_gate() {
    ui::run_conversation_v1_gate();
}

#[cfg(all(not(test), feature = "delegated-session-lifecycle-ipc-v1-test"))]
fn run_delegated_session_lifecycle_gate() {
    use crate::transport::{
        ConversationTransport, NativeConversationTransport, WiseOwlConversationUiRequest,
        WiseOwlConversationUiResponse,
    };
    let mut transport = NativeConversationTransport::new();
    let response = transport.submit(WiseOwlConversationUiRequest::QueryConversationState {
        conversation_id: transport::ConversationId(1),
        session_id: transport::SessionId(1),
    });
    if matches!(
        response,
        WiseOwlConversationUiResponse::Rejected { .. } | WiseOwlConversationUiResponse::Unavailable
    ) {
        debug_log("[WISEOWL-DELEGATION] CONSOLE_FIXTURE_FAILED\n");
    }
}

impl App for WiseOwlApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.ui.draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } if keycode == KEY_ESC => {
                request_close();
                false
            }
            _ => self.ui.update(event),
        }
    }
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    debug_log("[WISEOWL-GUI] Starting Wise Owl Console\n");
    #[cfg(feature = "conversation-v1-test")]
    run_conversation_v1_gate();
    #[cfg(feature = "delegated-session-lifecycle-ipc-v1-test")]
    run_delegated_session_lifecycle_gate();

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
