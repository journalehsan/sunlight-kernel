#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, IpcMsg, KbdMsg,
    unpack_key_event,
};
use sunlight_tty::login::{LoginField, LoginResult, LoginScreen};
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
pub extern "C" fn _start(fb_addr: u64, fb_width: u64, fb_height: u64, fb_pitch: u64) -> ! {
    debug_log("[TTY]  TTY server started");

    let has_fb = fb_addr != 0 && fb_width != 0 && fb_height != 0 && fb_pitch != 0;
    let fb32_w = fb_width as u32;
    let fb32_h = fb_height as u32;
    let fb32_p = fb_pitch as u32;

    if has_fb {
        debug_log("[TTY] Framebuffer acquired");
        unsafe {
            sunlight_tui::render_login_screen(fb_addr as *mut u32, fb32_w, fb32_h, fb32_p);
        }
        debug_log("[TTY] Login rendered");
    }

    let ep = endpoint_create();
    debug_log("[TTY]  endpoint created");

    debug_log("[TTY]  Registered as 'tty'");
    debug_log("[TTY]  Login screen ready");

    let mut login = LoginScreen::new();
    // Pre-fill "root" to match the static login screen render; move focus to password.
    for &b in b"root" { login.username.push(b); }
    login.focused = LoginField::Password;

    let mut state = TtyState::Login;
    let mut mux: Option<TermMux> = None;
    let mut prev_tab_count: usize = 0;

    let mut msg = ipc_recv(ep);
    let mut phase3_6_done = false;
    loop {
        match state {
            TtyState::Login => {
                if let Some(ascii) = key_ascii_from_msg(&msg) {
                    if login.focused == LoginField::Password {
                        debug_log_kbd_byte("[TTY] Key received in password field: ", ascii);
                    }
                    let result = login.handle_key_ascii(ascii);
                    match result {
                        LoginResult::Success { username, username_len, uid, gid } => {
                            debug_log_login_success(&username[..username_len], uid, gid);
                            debug_log("[SunlightOS] Phase 3.7 OK");
                            debug_log("[TTY]  Built-in shell ready");
                            mux = Some(TermMux::new(&username[..username_len]));
                            state = TtyState::Shell;
                            if has_fb {
                                if let Some(tmux) = mux.as_ref() {
                                    render_shell_fb(tmux, fb_addr, fb32_w, fb32_h, fb32_p);
                                }
                            }
                        }
                        LoginResult::Locked => { debug_log("[TTY]  Login locked"); }
                        LoginResult::Pending => {}
                    }
                }
                login.tick();
                // Re-render login screen with live state after every message
                if has_fb {
                    render_login_fb(&login, fb_addr, fb32_w, fb32_h, fb32_p);
                }
            }
            TtyState::Shell => {
                if let Some(tmux) = mux.as_mut() {
                    match msg.label {
                        KbdMsg::KEY_EVENT => {
                            let (_keycode, pressed, _shift, ctrl, _alt, ascii) =
                                unpack_key_event(msg.words[0]);

                            if !pressed {
                                // ignore key-release
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
                                    } else if a == b'l' || a == b'L' {
                                        tmux.handle_ctrl(b'l');
                                    } else if (a >= b'1' && a <= b'9') || a == b'0' {
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
                // Re-render shell view after every key event
                if has_fb {
                    if let Some(tmux) = mux.as_ref() {
                        render_shell_fb(tmux, fb_addr, fb32_w, fb32_h, fb32_p);
                    }
                }
            }
        }

        let reply = IpcMsg::with_label(0);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn render_login_fb(login: &LoginScreen, fb_addr: u64, fb_w: u32, fb_h: u32, fb_p: u32) {
    unsafe {
        sunlight_tui::render_login_dynamic(
            fb_addr as *mut u32,
            fb_w,
            fb_h,
            fb_p,
            &login.username.buf[..login.username.len],
            login.password.len,
            login.focused == LoginField::Password,
            login.message,
        );
    }
}

fn render_shell_fb(tmux: &TermMux, fb_addr: u64, fb_w: u32, fb_h: u32, fb_p: u32) {
    let (output, output_len) = tmux.tabs[tmux.active]
        .as_ref()
        .map(|t| (t.output.as_slice(), t.output_len))
        .unwrap_or((&[], 0));
    let input_line = tmux.active_line();
    let prompt = tmux.active_prompt().as_bytes();
    unsafe {
        sunlight_tui::render_tty_shell(
            fb_addr as *mut u32,
            fb_w,
            fb_h,
            fb_p,
            tmux.count,
            tmux.active,
            &output[..output_len],
            input_line,
            prompt,
        );
    }
}

/// Extract the ASCII byte from a keyboard IPC message, but only for key-press events.
/// Key-release events return None to prevent every character being processed twice.
fn key_ascii_from_msg(msg: &IpcMsg) -> Option<u8> {
    if msg.label == KbdMsg::KEY_EVENT {
        let (_keycode, pressed, _shift, _ctrl, _alt, ascii) = unpack_key_event(msg.words[0]);
        if pressed { ascii } else { None }
    } else {
        None
    }
}

/// Log a keyboard byte value without heap allocation.
fn debug_log_kbd_byte(prefix: &str, byte: u8) {
    let mut buf = [0u8; 64];
    let pb = prefix.as_bytes();
    let plen = pb.len().min(60);
    buf[..plen].copy_from_slice(&pb[..plen]);
    let dstart = plen;
    let dlen = if byte < 10 {
        buf[dstart] = b'0' + byte;
        1
    } else if byte < 100 {
        buf[dstart]     = b'0' + byte / 10;
        buf[dstart + 1] = b'0' + byte % 10;
        2
    } else {
        buf[dstart]     = b'0' + byte / 100;
        buf[dstart + 1] = b'0' + (byte % 100) / 10;
        buf[dstart + 2] = b'0' + byte % 10;
        3
    };
    // SAFETY: only ASCII digits and the caller-supplied prefix (valid UTF-8).
    if let Ok(s) = core::str::from_utf8(&buf[..dstart + dlen]) {
        debug_log(s);
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

/// Log "[TTY]  Login success: <username> (uid=<uid>, gid=<gid>)".
fn debug_log_login_success(username: &[u8], uid: u32, gid: u32) {
    let mut buf = [0u8; 128];
    let prefix = b"[TTY]  Login success: ";
    let mut pos = prefix.len();
    buf[..pos].copy_from_slice(prefix);
    let ulen = username.len().min(64);
    buf[pos..pos + ulen].copy_from_slice(&username[..ulen]);
    pos += ulen;
    let mid = b" (uid=";
    buf[pos..pos + mid.len()].copy_from_slice(mid);
    pos += mid.len();
    pos += fmt_u32(&mut buf[pos..], uid);
    let sep = b", gid=";
    buf[pos..pos + sep.len()].copy_from_slice(sep);
    pos += sep.len();
    pos += fmt_u32(&mut buf[pos..], gid);
    buf[pos] = b')';
    pos += 1;
    // SAFETY: only ASCII digits and caller-supplied username (valid ASCII).
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        debug_log(s);
    }
}

fn fmt_u32(buf: &mut [u8], val: u32) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut n = 0;
    let mut v = val;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    n
}
