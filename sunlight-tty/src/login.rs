//! Login screen state machine.
//!
//! Renders a simple login form with username/password fields, authenticates
//! against /etc/sunlight/users via VFS IPC.

use sunlight_ipc::{IpcMsg, VfsMsg, nameserver_lookup, ipc_call};

pub const MAX_FIELD_LEN: usize = 64;

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
pub enum LoginField {
    Username,
    Password,
}

pub enum LoginResult {
    Pending,
    Success { username: [u8; 64], username_len: usize },
    Locked,
}

pub struct LoginScreen {
    pub username: InputField,
    pub password: InputField,
    pub focused: LoginField,
    pub message: &'static str,
    pub attempts: u8,
    pub locked_ticks: u32,
}

impl LoginScreen {
    pub fn new() -> Self {
        Self {
            username: InputField::new(),
            password: InputField::new(),
            focused: LoginField::Username,
            message: "Welcome. Please log in.",
            attempts: 0,
            locked_ticks: 0,
        }
    }

    /// Handle a key event. Returns the login result.
    /// For printable chars (ascii Some), route to the focused field.
    /// For Enter, attempt login.
    /// For Backspace, delete from focused field.
    /// For Tab, switch focus.
    pub fn handle_key_ascii(&mut self, ascii: u8) -> LoginResult {
        if self.locked_ticks > 0 {
            return LoginResult::Locked;
        }

        match ascii {
            b'\n' | b'\r' => {
                if self.focused == LoginField::Username && self.username.len > 0 {
                    self.focused = LoginField::Password;
                    LoginResult::Pending
                } else if self.focused == LoginField::Password && self.password.len > 0 {
                    self.attempt_login()
                } else {
                    LoginResult::Pending
                }
            }
            b'\t' => {
                if self.focused == LoginField::Username {
                    self.focused = LoginField::Password;
                } else {
                    self.focused = LoginField::Username;
                }
                LoginResult::Pending
            }
            0x08 => {
                match self.focused {
                    LoginField::Username => self.username.backspace(),
                    LoginField::Password => self.password.backspace(),
                }
                LoginResult::Pending
            }
            c if c >= 0x20 && c <= 0x7E => {
                match self.focused {
                    LoginField::Username => self.username.push(c),
                    LoginField::Password => self.password.push(c),
                }
                LoginResult::Pending
            }
            _ => LoginResult::Pending,
        }
    }

    fn attempt_login(&mut self) -> LoginResult {
        let user = self.username.as_str();
        let pass = self.password.as_str();

        let valid = verify_login(user, pass);

        if valid {
            let ulen = self.username.len.min(63);
            let mut uname = [0u8; 64];
            uname[..ulen].copy_from_slice(&self.username.buf[..ulen]);
            self.message = "Login successful.";
            self.attempts = 0;
            LoginResult::Success {
                username: uname,
                username_len: ulen,
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
                LoginResult::Pending
            }
        }
    }

    /// Decrement lockout timer. Called on each timer tick.
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

/// Verify login credentials against /etc/sunlight/users via VFS IPC.
/// Falls back to hardcoded root:root and user:user if VFS is unavailable.
fn verify_login(username: &str, password: &str) -> bool {
    let users_data = match read_vfs_file("/etc/sunlight/users") {
        Some(data) => data,
        None => {
            // Fallback: hardcoded users
            return (username == "root" && password == "root")
                || (username == "user" && password == "user");
        }
    };

    let text = match core::str::from_utf8(&users_data) {
        Ok(s) => s,
        Err(_) => return false,
    };

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let u = &line[..colon];
            let p = &line[colon + 1..];
            if u == username && p == password {
                return true;
            }
        }
    }

    false
}

/// Read a file from VFS, returning the data in a fixed-size buffer.
fn read_vfs_file(path: &str) -> Option<[u8; 256]> {
    let vfs_cap = nameserver_lookup("vfs")?;

    let open_msg = path_msg(VfsMsg::OPEN, path);
    let reply = ipc_call(vfs_cap, open_msg);
    if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
        return None;
    }
    let handle = reply.words[1] as u32;

    let mut data = [0u8; 256];
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

    // Close
    let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));

    Some(data)
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
