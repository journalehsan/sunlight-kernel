//! Login screen state machine.
//!
//! Renders a modern grid-based login form with selectable user avatars,
//! a password field, and an environment dropdown. Authenticates through the
//! central UAC broker.

use sunlight_ipc::CapabilityToken;
use sunlight_uac::auth::{authenticate_password_for_session, AuthSuccess};

pub const MAX_FIELD_LEN: usize = 64;
pub const MAX_USERS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginUserIcon {
    User,
    Luggage,
}

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
            self.buf[self.len - 1] = 0;
            self.len -= 1;
        }
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: we only store valid ASCII bytes from keyboard input.
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }

    pub fn clear(&mut self) {
        for byte in &mut self.buf[..self.len] {
            *byte = 0;
        }
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

pub fn login_display_name(username: &str) -> &str {
    if username == "user" {
        "Guest"
    } else {
        username
    }
}

pub fn login_user_icon(username: &str) -> LoginUserIcon {
    if username == "user" {
        LoginUserIcon::Luggage
    } else {
        LoginUserIcon::User
    }
}

pub enum LoginResult {
    Pending,
    Success {
        username: [u8; 64],
        username_len: usize,
        uid: u32,
        gid: u32,
        session_grant: CapabilityToken,
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

        // Slot 1: bootstrap regular user
        let user_str = b"user";
        users[1].buf[..user_str.len()].copy_from_slice(user_str);
        users[1].len = user_str.len();

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
        self.attempt_login_with(verify_login)
    }

    fn attempt_login_with<F>(&mut self, verify: F) -> LoginResult
    where
        F: FnOnce(&[u8], &[u8]) -> Option<AuthSuccess>,
    {
        let u_idx = self.selected_user_idx;
        let user = &self.users[u_idx].buf[..self.users[u_idx].len];
        let pass = &self.password.buf[..self.password.len];

        let cred = verify(user, pass);

        if let Some(success) = cred {
            let ulen = self.users[u_idx].len.min(63);
            let mut uname = [0u8; 64];
            uname[..ulen].copy_from_slice(&self.users[u_idx].buf[..ulen]);
            self.message = "Login successful.";
            self.attempts = 0;
            LoginResult::Success {
                username: uname,
                username_len: ulen,
                uid: success.uid,
                gid: success.gid,
                session_grant: success.session_grant,
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

fn verify_login(username: &[u8], password: &[u8]) -> Option<AuthSuccess> {
    authenticate_password_for_session(username, password)
}

#[cfg(test)]
mod tests {
    use super::{
        login_display_name, login_user_icon, InputField, LoginResult, LoginScreen, LoginUserIcon,
    };

    #[test]
    fn guest_display_name_is_presentation_only() {
        assert_eq!(login_display_name("user"), "Guest");
        assert_eq!(login_display_name("root"), "root");
    }

    #[test]
    fn guest_uses_luggage_icon_only_for_user_account() {
        assert_eq!(login_user_icon("user"), LoginUserIcon::Luggage);
        assert_eq!(login_user_icon("root"), LoginUserIcon::User);
    }

    #[test]
    fn bootstrap_user_slot_keeps_canonical_username() {
        let login = LoginScreen::new();
        assert_eq!(login.users[1].as_str(), "user");
        assert_eq!(login_display_name(login.users[1].as_str()), "Guest");
    }

    #[test]
    fn guest_card_authenticates_with_canonical_username() {
        let mut login = LoginScreen::new();
        login.selected_user_idx = 1;
        login.password = InputField::new();
        login.password.push(b's');
        login.password.push(b'e');
        login.password.push(b'c');
        login.password.push(b'r');
        login.password.push(b'e');
        login.password.push(b't');

        let mut seen_username = [0u8; 64];
        let mut seen_username_len = 0usize;
        let result = login.attempt_login_with(|username, password| {
            seen_username[..username.len()].copy_from_slice(username);
            seen_username_len = username.len();
            assert_eq!(password, b"secret");
            Some(AuthSuccess {
                uid: 1000,
                gid: 1000,
                session_grant: CapabilityToken::INVALID,
            })
        });

        assert_eq!(&seen_username[..seen_username_len], b"user");
        match result {
            LoginResult::Success {
                username,
                username_len,
                uid,
                gid,
                ..
            } => {
                assert_eq!(&username[..username_len], b"user");
                assert_eq!(uid, 1000);
                assert_eq!(gid, 1000);
            }
            _ => panic!("expected login success"),
        }
    }

    #[test]
    fn incorrect_password_fails_normally_for_guest_card() {
        let mut login = LoginScreen::new();
        login.selected_user_idx = 1;
        login.password.push(b'x');

        let result = login.attempt_login_with(|username, password| {
            assert_eq!(username, b"user");
            assert_eq!(password, b"x");
            None
        });

        match result {
            LoginResult::Pending => {
                assert_eq!(login.attempts, 1);
                assert_eq!(login.password.len, 0);
                assert_eq!(login.message, "Invalid username or password.");
            }
            _ => panic!("expected failed login to remain pending"),
        }
    }
}
