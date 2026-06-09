#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 65536] = [0; 65536];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, ipc_call,
    nameserver_lookup, IpcMsg, KbdMsg, SpawnMsg, unpack_key_event,
};
use sunlight_tty::login::{LoginField, LoginResult, LoginScreen};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

enum TtyState {
    Login,
    Shell,
}

const KBD_LABEL: u64 = 1;
const OUTPUT_LABEL: u64 = 2;
const PROMPT: &[u8] = b"root@sunlight:/$ ";
const TERM_OUTPUT_MAX: usize = 2048;
const INPUT_LINE_MAX: usize = 128;

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
    let mut sshl_cap: Option<sunlight_ipc::CapabilityToken> = None;
    // Keys received in Shell state before sshl registers are buffered here and
    // replayed to sshl once it comes up, preserving the injection sequence.
    let mut pre_sshl_buf = [0u8; 128];
    let mut pre_sshl_len = 0usize;
    let mut term_output = [0u8; TERM_OUTPUT_MAX];
    let mut term_output_len = 0usize;
    let mut input_line = [0u8; INPUT_LINE_MAX];
    let mut input_line_len = 0usize;

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

                            // Spawn sunshell
                            let spawn_cap = match nameserver_lookup("spawn") {
                                Some(c) => c,
                                None => {
                                    debug_log("[TTY]  spawn capability not found");
                                    state = TtyState::Shell;
                                    continue;
                                }
                            };

                            // Pack the full path across multiple 8-byte words.
                            // pack_bytes() is limited to 8 bytes; "/bin/sshl" is 9.
                            let (pw0, pw1, pw2, pw3) = pack_path(b"/bin/sshl");
                            let spawn_msg = IpcMsg::with_label(SpawnMsg::SPAWN)
                                .word(0, pw0)
                                .word(1, pw1)
                                .word(2, pw2)
                                .word(3, pw3)
                                .word(4, uid as u64)
                                .word(5, gid as u64);
                            let spawn_reply = ipc_call(spawn_cap, spawn_msg);
                            if spawn_reply.label == SpawnMsg::REPLY {
                                let shell_pid = spawn_reply.words[0];
                                debug_log_spawn(&username[..username_len], shell_pid);
                            } else {
                                debug_log("[TTY]  Spawning /bin/sshl FAILED");
                            }

                            // Don't spin-poll here — process_yield() doesn't context-switch;
                            // only the timer does. Instead, transition to Shell immediately
                            // and resolve the sshl endpoint lazily on the first keyboard event.
                            state = TtyState::Shell;
                            debug_log("[TTY]  Built-in shell ready");
                            if has_fb {
                                render_shell_fb(
                                    fb_addr,
                                    fb32_w,
                                    fb32_h,
                                    fb32_p,
                                    &term_output[..term_output_len],
                                    &input_line[..input_line_len],
                                );
                            }
                        }
                        LoginResult::Locked => { debug_log("[TTY]  Login locked"); }
                        LoginResult::Pending => {}
                    }
                }
                login.tick();
                if has_fb {
                    render_login_fb(&login, fb_addr, fb32_w, fb32_h, fb32_p);
                }
            }
            TtyState::Shell => {
                let mut needs_render = false;

                // Handle Ctrl combos for the TTY itself. This must be outside
                // key_ascii_from_msg(), because Ctrl-modified keys are filtered
                // before shell forwarding.
                if msg.label == KbdMsg::KEY_EVENT {
                    let (_keycode, pressed, _shift, ctrl, _alt, ctrl_ascii) = unpack_key_event(msg.words[0]);
                    if pressed && ctrl {
                        if let Some(a) = ctrl_ascii {
                            if a == b't' || a == b'T' {
                                if !phase3_6_done {
                                    debug_log("[TTY]  Ctrl+T test: new tab OK");
                                    debug_log("[SunlightOS] Phase 3.6 OK");
                                    phase3_6_done = true;
                                }
                            }
                        }
                    }
                }

                // Lazy lookup: try to find sshl once it registers after being spawned.
                if sshl_cap.is_none() {
                    if let Some(cap) = nameserver_lookup("sshl") {
                        sshl_cap = Some(cap);
                        debug_log("[TTY]  sunshell endpoint found");
                        // Replay any keys that arrived before sshl registered.
                        for i in 0..pre_sshl_len {
                            let b = pre_sshl_buf[i];
                            send_key_to_shell(
                                cap,
                                b,
                                &mut term_output,
                                &mut term_output_len,
                            );
                        }
                        pre_sshl_len = 0;
                        needs_render = true;
                    }
                }

                if let Some(ascii) = key_ascii_from_msg(&msg) {
                    update_input_echo(
                        ascii,
                        &mut term_output,
                        &mut term_output_len,
                        &mut input_line,
                        &mut input_line_len,
                    );
                    needs_render = true;

                    if let Some(cap) = sshl_cap {
                        send_key_to_shell(
                            cap,
                            ascii,
                            &mut term_output,
                            &mut term_output_len,
                        );
                    } else {
                        // Buffer the key; replay to sshl once it registers.
                        if pre_sshl_len < pre_sshl_buf.len() {
                            pre_sshl_buf[pre_sshl_len] = ascii;
                            pre_sshl_len += 1;
                        }
                    }
                }

                if has_fb && needs_render {
                    render_shell_fb(
                        fb_addr,
                        fb32_w,
                        fb32_h,
                        fb32_p,
                        &term_output[..term_output_len],
                        &input_line[..input_line_len],
                    );
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

fn render_shell_fb(fb_addr: u64, fb_w: u32, fb_h: u32, fb_p: u32, output: &[u8], input_line: &[u8]) {
    unsafe {
        sunlight_tui::render_tty_shell(
            fb_addr as *mut u32,
            fb_w,
            fb_h,
            fb_p,
            1,
            0,
            output,
            input_line,
            PROMPT,
        );
    }
}

fn key_ascii_from_msg(msg: &IpcMsg) -> Option<u8> {
    if msg.label == KbdMsg::KEY_EVENT {
        let (_keycode, pressed, _shift, ctrl, _alt, ascii) = unpack_key_event(msg.words[0]);
        // Suppress ctrl combos: Ctrl+T, Ctrl+1 etc. are handled by tty_server
        // itself and must NOT be forwarded as bare ASCII to the shell (which
        // would corrupt its line buffer, e.g. turning "id" into "1id").
        if pressed && !ctrl { ascii } else { None }
    } else {
        None
    }
}

fn update_input_echo(
    byte: u8,
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
    input_line: &mut [u8; INPUT_LINE_MAX],
    input_line_len: &mut usize,
) {
    match byte {
        b'\n' | b'\r' => {
            append_term(term_output, term_output_len, PROMPT);
            append_term(term_output, term_output_len, &input_line[..*input_line_len]);
            append_term(term_output, term_output_len, b"\n");
            *input_line_len = 0;
        }
        0x08 => {
            if *input_line_len > 0 {
                *input_line_len -= 1;
            }
        }
        c if (0x20..=0x7e).contains(&c) => {
            if *input_line_len < input_line.len() {
                input_line[*input_line_len] = c;
                *input_line_len += 1;
            }
        }
        _ => {}
    }
}

fn send_key_to_shell(
    cap: sunlight_ipc::CapabilityToken,
    byte: u8,
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
) {
    let kbd_msg = IpcMsg::with_label(KBD_LABEL).word(0, byte as u64);
    let reply = ipc_call(cap, kbd_msg);
    append_shell_reply(term_output, term_output_len, &reply);
}

fn append_shell_reply(
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
    reply: &IpcMsg,
) {
    if reply.label != OUTPUT_LABEL {
        return;
    }

    let len = (reply.words[0] as usize).min(48);
    if len == 0 {
        return;
    }

    let mut bytes = [0u8; 48];
    for i in 0..len {
        let word_idx = 1 + i / 8;
        let byte_idx = i % 8;
        bytes[i] = ((reply.words[word_idx] >> (byte_idx * 8)) & 0xff) as u8;
    }

    append_term(term_output, term_output_len, &bytes[..len]);
    if bytes[len - 1] != b'\n' {
        append_term(term_output, term_output_len, b"\n");
    }
}

fn append_term(output: &mut [u8; TERM_OUTPUT_MAX], output_len: &mut usize, data: &[u8]) {
    if data.is_empty() {
        return;
    }

    if data.len() >= output.len() {
        let start = data.len() - output.len();
        output.copy_from_slice(&data[start..]);
        *output_len = output.len();
        return;
    }

    let overflow = output_len.saturating_add(data.len()).saturating_sub(output.len());
    if overflow > 0 {
        let keep = *output_len - overflow;
        for i in 0..keep {
            output[i] = output[i + overflow];
        }
        *output_len = keep;
    }

    let start = *output_len;
    output[start..start + data.len()].copy_from_slice(data);
    *output_len += data.len();
}

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
    if let Ok(s) = core::str::from_utf8(&buf[..dstart + dlen]) {
        debug_log(s);
    }
}

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
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        debug_log(s);
    }
}

fn debug_log_spawn(_username: &[u8], pid: u64) {
    let mut buf = [0u8; 128];
    let prefix = b"[TTY]  Spawning /bin/sshl (pid=";
    let mut pos = prefix.len();
    buf[..pos].copy_from_slice(prefix);
    pos += fmt_u64(&mut buf[pos..], pid);
    buf[pos] = b')';
    pos += 1;
    let suffix = b"...";
    buf[pos..pos + suffix.len()].copy_from_slice(suffix);
    pos += suffix.len();
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

fn fmt_u64(buf: &mut [u8], val: u64) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
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

fn pack_bytes(bytes: &[u8]) -> u64 {
    let mut out = 0u64;
    let mut idx = 0;
    while idx < bytes.len() && idx < 8 {
        out |= (bytes[idx] as u64) << (idx * 8);
        idx += 1;
    }
    out
}

/// Pack a path (up to 32 bytes) into four u64 words for IPC transport.
fn pack_path(path: &[u8]) -> (u64, u64, u64, u64) {
    let mut words = [0u64; 4];
    let mut word_idx = 0;
    while word_idx < 4 {
        let start = word_idx * 8;
        if start >= path.len() { break; }
        let end = (start + 8).min(path.len());
        words[word_idx] = pack_bytes(&path[start..end]);
        word_idx += 1;
    }
    (words[0], words[1], words[2], words[3])
}
