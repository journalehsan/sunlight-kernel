//! Login screen state machine.
//!
//! Renders a modern grid-based login form with selectable user avatars,
//! a password field, and an environment dropdown. Authenticates against
//! /etc/passwd + /etc/shadow via VFS IPC.

use sunlight_ipc::{IpcMsg, VfsMsg, nameserver_lookup, ipc_call};
use sunlight_fs::{parse_passwd, parse_shadow, lookup_by_name};

pub const MAX_FIELD_LEN: usize = 64;
pub const MAX_USERS: usize = 6;

#[derive(Clone, Copy)]
pub struct InputField {
    pub buf: [u8; MAX_FIELD_LEN],
    pub len: usize,
}

impl InputField {
    pub const fn new() -> Self {
        Self {
            buf: [0; MAX_FIELD_LEN],
            len: 0,
        }
    }

    pub fn push(&mut self, c: u8) {
        if self.len < MAX_FIELD_LEN {
            self.buf[self.len] = c;
            self.len += 1;
        }
    }

    pub fn backspace(&mut self) {
        if self.len > 0 {
            self.len -= 1;
        }
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: we only store valid ASCII bytes from keyboard input.
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusArea {
    UserSlot(usize), // 0 up to active_count - 1
    Password,
    Dropdown,
    Reboot,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionType {
    Tty,
    Desktop,
}

impl SessionType {
    pub const fn toggle(self) -> Self {
        match self {
            Self::Tty => Self::Desktop,
            Self::Desktop => Self::Tty,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Tty => "TTY",
            Self::Desktop => "Desktop",
        }
    }
}

pub enum LoginResult {
    Pending,
    Success {
        username: [u8; 64],
        username_len: usize,
        uid: u32,
        gid: u32,
        session: SessionType,
    },
    Locked,
    Reboot,
    Shutdown,
}

pub struct LoginScreen {
    pub users: [InputField; MAX_USERS],
    pub is_custom_slot: [bool; MAX_USERS],
    pub active_count: usize,      // How many slots are currently displayed
    pub focus: FocusArea,         // Where the keyboard input is currently directed
    pub selected_user_idx: usize, // The user locked in for the login attempt
    pub session: SessionType,
    pub password: InputField,
    pub message: &'static str,
    pub attempts: u8,
    pub locked_ticks: u32,
}

impl LoginScreen {
    pub fn new() -> Self {
        let mut users = [InputField::new(); MAX_USERS];
        let mut is_custom_slot = [false; MAX_USERS];

        // Slot 0: root
        let root_str = b"root";
        users[0].buf[..root_str.len()].copy_from_slice(root_str);
        users[0].len = root_str.len();

        // Slot 1: Ehsan Tork
        let ehsan_str = b"Ehsan Tork";
        users[1].buf[..ehsan_str.len()].copy_from_slice(ehsan_str);
        users[1].len = ehsan_str.len();

        // Slot 2: The dynamic "More / Custom" slot
        is_custom_slot[2] = true;

        Self {
            users,
            is_custom_slot,
            active_count: 3,
            focus: FocusArea::UserSlot(0),
            selected_user_idx: 0,
            session: SessionType::Tty,
            password: InputField::new(),
            message: "Welcome. Please log in.",
            attempts: 0,
            locked_ticks: 0,
        }
    }

    /// Handle a key event. Returns the login result.
    pub fn handle_key_ascii(&mut self, ascii: u8) -> LoginResult {
        self.handle_key_event(0, true, Some(ascii))
    }

    /// Handle a raw key event. Arrow keys are routed here so the dropdown can
    /// be toggled without smuggling non-ASCII values through the ASCII path.
    pub fn handle_key_event(
        &mut self,
        keycode: u8,
        pressed: bool,
        ascii: Option<u8>,
    ) -> LoginResult {
        if self.locked_ticks > 0 {
            return LoginResult::Locked;
        }

        if !pressed {
            return LoginResult::Pending;
        }

        match ascii {
            Some(b'\n') | Some(b'\r') => {
                match self.focus {
                    FocusArea::UserSlot(idx) => {
                        // Enter on a user box commits the selection and moves to password
                        if self.users[idx].len > 0 {
                            self.selected_user_idx = idx;
                            self.focus = FocusArea::Password;
                        }
                        LoginResult::Pending
                    }
                    FocusArea::Password | FocusArea::Dropdown => {
                        // Enter on password or dropdown triggers the login attempt
                        if self.password.len > 0 {
                            self.attempt_login()
                        } else {
                            LoginResult::Pending
                        }
                    }
                    FocusArea::Reboot => LoginResult::Reboot,
                    FocusArea::Shutdown => LoginResult::Shutdown,
                }
            }
            Some(b'\t') => {
                self.cycle_focus_forward();
                LoginResult::Pending
            }
            Some(b' ') => {
                if self.focus == FocusArea::Dropdown {
                    self.session = self.session.toggle();
                }
                LoginResult::Pending
            }
            Some(0x08) | Some(0x7F) => {
                // Handle Backspace or DEL
                match self.focus {
                    FocusArea::UserSlot(idx) if self.is_custom_slot[idx] => {
                        self.users[idx].backspace()
                    }
                    FocusArea::Password => self.password.backspace(),
                    _ => {} // Read-only user boxes ignore backspace
                }
                LoginResult::Pending
            }
            Some(c) if c >= 0x20 && c <= 0x7E => {
                // Printable characters
                match self.focus {
                    FocusArea::UserSlot(idx) if self.is_custom_slot[idx] => self.users[idx].push(c),
                    FocusArea::Password => self.password.push(c),
                    _ => {} // Read-only user boxes ignore typing
                }
                LoginResult::Pending
            }
            None => {
                if self.focus == FocusArea::Dropdown {
                    match keycode {
                        0x48 | 0x50 => {
                            self.session = self.session.toggle();
                        }
                        _ => {}
                    }
                }
                LoginResult::Pending
            }
            _ => LoginResult::Pending,
        }
    }

    fn cycle_focus_forward(&mut self) {
        match self.focus {
            FocusArea::UserSlot(idx) => {
                if idx + 1 < self.active_count {
                    self.focus = FocusArea::UserSlot(idx + 1);
                } else {
                    self.focus = FocusArea::Password;
                }
            }
            FocusArea::Password => self.focus = FocusArea::Dropdown,
            FocusArea::Dropdown => self.focus = FocusArea::Reboot,
            FocusArea::Reboot => self.focus = FocusArea::Shutdown,
            FocusArea::Shutdown => self.focus = FocusArea::UserSlot(0),
        }
    }

    fn attempt_login(&mut self) -> LoginResult {
        let u_idx = self.selected_user_idx;
        let user = &self.users[u_idx].buf[..self.users[u_idx].len];
        let pass = &self.password.buf[..self.password.len];

        let cred = verify_login(user, pass);

        if let Some((uid, gid)) = cred {
            let ulen = self.users[u_idx].len.min(63);
            let mut uname = [0u8; 64];
            uname[..ulen].copy_from_slice(&self.users[u_idx].buf[..ulen]);
            self.message = "Login successful.";
            self.attempts = 0;
            LoginResult::Success {
                username: uname,
                username_len: ulen,
                uid,
                gid,
                session: self.session,
            }
        } else {
            self.attempts += 1;
            self.password.clear();
            if self.attempts >= 3 {
                self.locked_ticks = 30; // 30 second lockout
                self.message = "Too many failed attempts. Locked for 30s.";
                LoginResult::Locked
            } else {
                self.message = "Invalid username or password.";
                // Kick focus back to the password field so they can try again quickly
                self.focus = FocusArea::Password;
                LoginResult::Pending
            }
        }
    }

    pub fn tick(&mut self) {
        if self.locked_ticks > 0 {
            self.locked_ticks -= 1;
            if self.locked_ticks == 0 {
                self.message = "Welcome. Please log in.";
                self.attempts = 0;
            }
        }
    }
}

/// Verify login credentials via /etc/passwd + /etc/shadow.
/// Returns Some((uid, gid)) on success, None on failure.
/// Falls back to hardcoded credentials if VFS is unavailable.
fn verify_login(username: &[u8], password: &[u8]) -> Option<(u32, u32)> {
    use sunlight_ipc::debug_log;

    let vfs_cap = match nameserver_lookup("vfs") {
        Some(c) => c,
        None => {
            return fallback_auth(username, password);
        }
    };

    debug_log("[TTY]  Login: reading /etc/passwd via VFS");

    let (passwd_data, passwd_len) = match read_vfs_bytes(vfs_cap, "/etc/passwd") {
        Some(pair) => pair,
        None => return fallback_auth(username, password),
    };

    let (passwd_entries, passwd_count) = parse_passwd(&passwd_data[..passwd_len]);
    let entry = lookup_by_name(&passwd_entries[..passwd_count], username)?;
    let uid = entry.uid;
    let gid = entry.gid;

    debug_log("[TTY]  Login: auth from /etc/shadow");

    let (shadow_data, shadow_len) = match read_vfs_bytes(vfs_cap, "/etc/shadow") {
        Some(pair) => pair,
        None => return fallback_auth(username, password),
    };

    let (shadow_entries, shadow_count) = parse_shadow(&shadow_data[..shadow_len]);

    for i in 0..shadow_count {
        let s = &shadow_entries[i];
        let slen = s.username.iter().position(|&b| b == 0).unwrap_or(64);
        if slen != username.len() || &s.username[..slen] != username {
            continue;
        }
        let plen = s.password.iter().position(|&b| b == 0).unwrap_or(128);
        if plen == password.len() && &s.password[..plen] == password {
            return Some((uid, gid));
        }
        return None;
    }

    None
}

/// Hardcoded fallback used when VFS is unavailable.
fn fallback_auth(username: &[u8], password: &[u8]) -> Option<(u32, u32)> {
    if username == b"root" && password == b"root" {
        return Some((0, 0));
    }
    if username == b"user" && password == b"user" {
        return Some((1000, 1000));
    }
    None
}

/// Read a file from VFS into a fixed 512-byte buffer.
/// Returns (buffer, bytes_read) on success.
fn read_vfs_bytes(
    vfs_cap: sunlight_ipc::CapabilityToken,
    path: &str,
) -> Option<([u8; 512], usize)> {
    let open_msg = path_msg(VfsMsg::OPEN, path);
    let reply = ipc_call(vfs_cap, open_msg);
    if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
        return None;
    }
    let handle = reply.words[1] as u32;

    let mut data = [0u8; 512];
    let mut total = 0usize;

    loop {
        if total >= data.len() {
            break;
        }
        let read_msg = IpcMsg::with_label(VfsMsg::READ)
            .word(0, handle as u64)
            .word(1, total as u64)
            .word(2, 16);
        let reply = ipc_call(vfs_cap, read_msg);
        if reply.label != VfsMsg::REPLY {
            break;
        }
        let n = reply.words[1] as usize;
        if n == 0 {
            break;
        }
        let src = &reply.words[2..4];
        for i in 0..n.min(data.len() - total) {
            let word_idx = i / 8;
            let byte_idx = i % 8;
            data[total + i] = ((src[word_idx] >> (byte_idx * 8)) & 0xFF) as u8;
        }
        total += n;
    }

    let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));

    Some((data, total))
}

fn path_msg(label: u64, path: &str) -> IpcMsg {
    let bytes = path.as_bytes();
    let mut msg = IpcMsg::with_label(label);
    for word_idx in 0..4 {
        let start = word_idx * 8;
        let end = (start + 8).min(bytes.len());
        if start < bytes.len() {
            msg = msg.word(word_idx, pack_bytes(&bytes[start..end]));
        }
    }
    msg
}

fn pack_bytes(bytes: &[u8]) -> u64 {
    let mut out = 0u64;
    for (i, &b) in bytes.iter().enumerate().take(8) {
        out |= (b as u64) << (i * 8);
    }
    out
}
