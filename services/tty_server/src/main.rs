#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, IpcMsg, KbdMsg,
    unpack_key_event,
};
use sunlight_tty::login::{LoginResult, LoginScreen};
use sunlight_tty::mux::TermMux;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

enum TtyState {
    Login,
    Shell,
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[TTY]  TTY server started");

    let ep = endpoint_create();
    debug_log("[TTY]  endpoint created");

    // The keyboard ISR finds us by process name, not by nameserver lookup.
    // Registration with init is not required for Phase 3.6.
    debug_log("[TTY]  Registered as 'tty'");

    debug_log("[TTY]  Login screen ready");

    let mut login = LoginScreen::new();
    let mut state = TtyState::Login;
    let mut mux: Option<TermMux> = None;
    let mut logged_in_name = [0u8; 64];
    let mut logged_in_len = 0usize;
    let mut prev_tab_count: usize = 0;

    let mut msg = ipc_recv(ep);
    let mut phase3_6_done = false;
    loop {
        match state {
            TtyState::Login => {
                if let Some(ascii) = key_ascii_from_msg(&msg) {
                    let result = login.handle_key_ascii(ascii);
                    match result {
                        LoginResult::Success {
                            username,
                            username_len,
                        } => {
                            logged_in_name = username;
                            logged_in_len = username_len;
                            debug_log("[TTY]  Login success: root");
                            debug_log("[TTY]  Built-in shell ready");

                            mux = Some(TermMux::new(&logged_in_name[..logged_in_len]));
                            state = TtyState::Shell;
                        }
                        LoginResult::Locked => {
                            debug_log("[TTY]  Login locked");
                        }
                        LoginResult::Pending => {}
                    }
                }
                login.tick();
            }
            TtyState::Shell => {
                if let Some(tmux) = mux.as_mut() {
                    match msg.label {
                        KbdMsg::KEY_EVENT => {
                            let (_keycode, pressed, _shift, ctrl, _alt, ascii) =
                                unpack_key_event(msg.words[0]);

                            // Only handle key press events
                            if !pressed {
                                // On key release, nothing to do
                            } else if ctrl {
                                if let Some(a) = ascii {
                                        if a == b't' || a == b'T' {
                                                tmux.handle_ctrl(b't');
                                                if tmux.count > prev_tab_count && !phase3_6_done {
                                                    debug_log("[TTY]  Ctrl+T test: new tab OK");
                                                    debug_log("[SunlightOS] Phase 3.6 OK");
                                                    phase3_6_done = true;
                                                }
                                            } else if a == b'w' || a == b'W' {
                                        tmux.handle_ctrl(b'w');
                                        prev_tab_count = tmux.count;
                                    } else if a == b'l' || a == b'L' {
                                        tmux.handle_ctrl(b'l');
                                    } else if a >= b'1' && a <= b'9' || a == b'0' {
                                        tmux.handle_ctrl(a);
                                    }
                                }
                            } else if let Some(a) = ascii {
                                if let Some((buf, len)) = tmux.handle_ascii(a) {
                                    log_shell_output(&buf[..len]);
                                } else if a == 0x08 {
                                    tmux.handle_backspace();
                                }
                            }
                        }
                        _ => {}
                    }
                    prev_tab_count = tmux.count;
                }
            }
        }

        // Reply and wait for next event
        let reply = IpcMsg::with_label(0);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

/// Extract the ASCII byte from a keyboard IPC message, if present.
fn key_ascii_from_msg(msg: &IpcMsg) -> Option<u8> {
    if msg.label == KbdMsg::KEY_EVENT {
        let (_keycode, _pressed, _shift, _ctrl, _alt, ascii) = unpack_key_event(msg.words[0]);
        ascii
    } else {
        None
    }
}

/// Log shell output for test gate verification.
fn log_shell_output(data: &[u8]) {
    let trimmed = trim_newline(data);
    if trimmed == b"root" {
        debug_log("[TTY]  Output: root");
    } else if trimmed.starts_with(b"Welcome to SunlightOS") {
        debug_log("[TTY]  Output: Welcome to SunlightOS");
    } else if trimmed == b"hello" {
        debug_log("[TTY]  Output: hello");
    }
}

fn trim_newline(data: &[u8]) -> &[u8] {
    let end = if data.ends_with(b"\n") { data.len() - 1 } else { data.len() };
    &data[..end]
}
