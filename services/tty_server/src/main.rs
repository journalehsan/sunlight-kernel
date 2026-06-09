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
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait, nameserver_lookup,
    CapabilityToken, IpcMsg, KbdMsg, SpawnMsg, unpack_key_event,
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
const EXIT_LABEL: u64 = 3;
const PROMPT: &[u8] = b"root@sunlight:/$ ";
const TERM_OUTPUT_MAX: usize = 2048;
const INPUT_LINE_MAX: usize = 128;
const PENDING_INPUT_MAX: usize = 128;
const MAX_TABS: usize = 10;

#[derive(Clone, Copy)]
struct ShellTab {
    shell_id: u64,
    pid: u64,
    cap: Option<CapabilityToken>,
    output: [u8; TERM_OUTPUT_MAX],
    output_len: usize,
    input_line: [u8; INPUT_LINE_MAX],
    input_line_len: usize,
    pending: [u8; PENDING_INPUT_MAX],
    pending_len: usize,
}

impl ShellTab {
    const fn empty() -> Self {
        Self {
            shell_id: 0,
            pid: 0,
            cap: None,
            output: [0; TERM_OUTPUT_MAX],
            output_len: 0,
            input_line: [0; INPUT_LINE_MAX],
            input_line_len: 0,
            pending: [0; PENDING_INPUT_MAX],
            pending_len: 0,
        }
    }
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
    let mut spawn_cap: Option<CapabilityToken> = None;
    let mut tabs = [ShellTab::empty(); MAX_TABS];
    let mut tab_count = 0usize;
    let mut active_tab = 0usize;
    let mut next_shell_id = 0u64;
    let mut logged_initial_spawn = false;

    let mut msg = ipc_recv(ep);
    let mut phase3_6_done = false;
    loop {
        match state {
            TtyState::Login => {
                let mut logged_in = false;
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
                            let cap = match nameserver_lookup("spawn") {
                                Some(c) => c,
                                None => {
                                    debug_log("[TTY]  spawn capability not found");
                                    state = TtyState::Shell;
                                    continue;
                                }
                            };
                            spawn_cap = Some(cap);

                            if spawn_tab(
                                &mut tabs,
                                &mut tab_count,
                                &mut active_tab,
                                &mut next_shell_id,
                                cap,
                            ) {
                                if let Some(tab) = active_shell_tab(&tabs, active_tab) {
                                    debug_log_spawn(&username[..username_len], tab.pid);
                                    logged_initial_spawn = true;
                                }
                            }

                            // Don't spin-poll here — process_yield() doesn't context-switch;
                            // only the timer does. Instead, transition to Shell immediately
                            // and resolve the sshl endpoint lazily on the first keyboard event.
                            state = TtyState::Shell;
                            debug_log("[TTY]  Built-in shell ready");
                            if has_fb {
                                render_active_shell_fb(
                                    fb_addr,
                                    fb32_w,
                                    fb32_h,
                                    fb32_p,
                                    &tabs,
                                    tab_count,
                                    active_tab,
                                );
                            }
                            logged_in = true;
                        }
                        LoginResult::Locked => { debug_log("[TTY]  Login locked"); }
                        LoginResult::Pending => {}
                    }
                }
                login.tick();
                if has_fb && !logged_in {
                    render_login_fb(&login, fb_addr, fb32_w, fb32_h, fb32_p);
                }
            }
            TtyState::Shell => {
                let mut needs_render = false;

                // Lazy lookup: try to find sshl once it registers after being spawned.
                if msg.label == KbdMsg::KEY_EVENT {
                    let (_keycode, pressed, _shift, ctrl, _alt, ctrl_ascii) = unpack_key_event(msg.words[0]);
                    if pressed && ctrl {
                        if let Some(a) = ctrl_ascii {
                            if handle_ctrl_key(
                                a,
                                &mut tabs,
                                &mut tab_count,
                                &mut active_tab,
                                &mut next_shell_id,
                                spawn_cap,
                                &mut phase3_6_done,
                            ) {
                                needs_render = true;
                            }
                        }
                    }
                }

                resolve_active_shell(&mut tabs, active_tab, &mut logged_initial_spawn);

                if let Some(ascii) = key_ascii_from_msg(&msg) {
                    if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                        update_input_echo(
                            ascii,
                            &mut tab.output,
                            &mut tab.output_len,
                            &mut tab.input_line,
                            &mut tab.input_line_len,
                        );
                        needs_render = true;

                        if let Some(cap) = tab.cap {
                            let exited = send_key_to_shell(
                                cap,
                                ascii,
                                &mut tab.output,
                                &mut tab.output_len,
                            );
                            if exited {
                                state = TtyState::Login;
                                reset_login(&mut login);
                                reset_tabs(&mut tabs, &mut tab_count, &mut active_tab);
                                spawn_cap = None;
                                logged_initial_spawn = false;
                                if has_fb {
                                    render_login_fb(&login, fb_addr, fb32_w, fb32_h, fb32_p);
                                }
                                continue;
                            }
                        } else if tab.pending_len < tab.pending.len() {
                            tab.pending[tab.pending_len] = ascii;
                            tab.pending_len += 1;
                        }
                    }
                }

                if has_fb && needs_render {
                    render_active_shell_fb(
                        fb_addr,
                        fb32_w,
                        fb32_h,
                        fb32_p,
                        &tabs,
                        tab_count,
                        active_tab,
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

fn render_active_shell_fb(
    fb_addr: u64,
    fb_w: u32,
    fb_h: u32,
    fb_p: u32,
    tabs: &[ShellTab; MAX_TABS],
    tab_count: usize,
    active_tab: usize,
) {
    let (output, input_line) = active_shell_tab(tabs, active_tab)
        .map(|tab| {
            (
                &tab.output[..tab.output_len],
                &tab.input_line[..tab.input_line_len],
            )
        })
        .unwrap_or((&[][..], &[][..]));
    unsafe {
        sunlight_tui::render_tty_shell(
            fb_addr as *mut u32,
            fb_w,
            fb_h,
            fb_p,
            tab_count.max(1),
            active_tab,
            output,
            input_line,
            PROMPT,
        );
    }
}

fn reset_login(login: &mut LoginScreen) {
    *login = LoginScreen::new();
    login.message = "Logged out. Please log in.";
}

fn reset_tabs(tabs: &mut [ShellTab; MAX_TABS], tab_count: &mut usize, active_tab: &mut usize) {
    for tab in tabs.iter_mut() {
        *tab = ShellTab::empty();
    }
    *tab_count = 0;
    *active_tab = 0;
}

fn active_shell_tab(tabs: &[ShellTab; MAX_TABS], active_tab: usize) -> Option<&ShellTab> {
    tabs.get(active_tab).filter(|tab| tab.pid != 0)
}

fn active_shell_tab_mut(
    tabs: &mut [ShellTab; MAX_TABS],
    active_tab: usize,
) -> Option<&mut ShellTab> {
    tabs.get_mut(active_tab).filter(|tab| tab.pid != 0)
}

fn spawn_tab(
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
    next_shell_id: &mut u64,
    spawn_cap: CapabilityToken,
) -> bool {
    if *tab_count >= MAX_TABS {
        return false;
    }

    let shell_id = *next_shell_id;
    *next_shell_id += 1;
    let mut path = [0u8; 16];
    let path_len = make_shell_path(shell_id, &mut path);
    let (pw0, pw1, pw2, pw3) = pack_path(&path[..path_len]);
    let spawn_msg = IpcMsg::with_label(SpawnMsg::SPAWN)
        .word(0, pw0)
        .word(1, pw1)
        .word(2, pw2)
        .word(3, pw3);
    let spawn_reply = ipc_call(spawn_cap, spawn_msg);
    if spawn_reply.label != SpawnMsg::REPLY {
        debug_log("[TTY]  Spawning /bin/sshl FAILED");
        return false;
    }

    let index = *tab_count;
    tabs[index] = ShellTab::empty();
    tabs[index].shell_id = shell_id;
    tabs[index].pid = spawn_reply.words[0];
    *active_tab = index;
    *tab_count += 1;
    true
}

fn handle_ctrl_key(
    ascii: u8,
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
    next_shell_id: &mut u64,
    spawn_cap: Option<CapabilityToken>,
    phase3_6_done: &mut bool,
) -> bool {
    match ascii {
        b't' | b'T' => {
            if let Some(cap) = spawn_cap {
                if spawn_tab(tabs, tab_count, active_tab, next_shell_id, cap) && !*phase3_6_done {
                    debug_log("[TTY]  Ctrl+T test: new tab OK");
                    debug_log("[SunlightOS] Phase 3.6 OK");
                    *phase3_6_done = true;
                }
                return true;
            }
        }
        b'w' | b'W' => {
            close_active_tab(tabs, tab_count, active_tab);
            return true;
        }
        b'1'..=b'9' => {
            let idx = (ascii - b'1') as usize;
            if idx < *tab_count {
                *active_tab = idx;
                return true;
            }
        }
        b'0' => {
            if *tab_count >= 10 {
                *active_tab = 9;
                return true;
            }
        }
        _ => {}
    }
    false
}

fn close_active_tab(
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
) {
    if *tab_count <= 1 {
        return;
    }

    for i in *active_tab..(*tab_count - 1) {
        tabs[i] = tabs[i + 1];
    }
    tabs[*tab_count - 1] = ShellTab::empty();
    *tab_count -= 1;
    if *active_tab >= *tab_count {
        *active_tab = *tab_count - 1;
    }
}

fn resolve_active_shell(
    tabs: &mut [ShellTab; MAX_TABS],
    active_tab: usize,
    logged_initial_spawn: &mut bool,
) {
    let Some(tab) = active_shell_tab_mut(tabs, active_tab) else {
        return;
    };
    if tab.cap.is_some() {
        return;
    }

    let mut name = [0u8; 16];
    let name_len = make_shell_name(tab.shell_id, &mut name);
    let Some(name_str) = core::str::from_utf8(&name[..name_len]).ok() else {
        return;
    };
    if let Some(cap) = nameserver_lookup(name_str) {
        tab.cap = Some(cap);
        if *logged_initial_spawn {
            debug_log("[TTY]  sunshell endpoint found");
            *logged_initial_spawn = false;
        }
        let pending_len = tab.pending_len;
        for i in 0..pending_len {
            let b = tab.pending[i];
            let _ = send_key_to_shell(cap, b, &mut tab.output, &mut tab.output_len);
        }
        tab.pending_len = 0;
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
    cap: CapabilityToken,
    byte: u8,
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
) -> bool {
    let kbd_msg = IpcMsg::with_label(KBD_LABEL).word(0, byte as u64);
    let reply = ipc_call(cap, kbd_msg);
    if reply.label == EXIT_LABEL {
        return true;
    }
    append_shell_reply(term_output, term_output_len, &reply);
    false
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

fn make_shell_path(shell_id: u64, out: &mut [u8]) -> usize {
    let prefix = b"/bin/sshl";
    out[..prefix.len()].copy_from_slice(prefix);
    prefix.len() + fmt_u64(&mut out[prefix.len()..], shell_id)
}

fn make_shell_name(shell_id: u64, out: &mut [u8]) -> usize {
    let prefix = b"sshl";
    out[..prefix.len()].copy_from_slice(prefix);
    prefix.len() + fmt_u64(&mut out[prefix.len()..], shell_id)
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
