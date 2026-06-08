//! Terminal multiplexer with tab support.
//!
//! Manages up to 10 tabs, each with a built-in shell. Handles keybindings
//! for tab creation/switching/close and routing printable characters to the
//! active shell.

use crate::shell::{BuiltinShell, ShellOutput};
use sunlight_ipc::CapabilityToken;

const MAX_TABS: usize = 10;

pub struct Tab {
    pub title: [u8; 32],
    pub title_len: usize,
    pub shell: BuiltinShell,
    /// Last output from this tab's shell (for display)
    pub output: [u8; 512],
    pub output_len: usize,
}

impl Tab {
    fn new(title: &[u8], username: &[u8]) -> Self {
        let mut t = [0u8; 32];
        let tlen = title.len().min(32);
        t[..tlen].copy_from_slice(&title[..tlen]);
        Self {
            title: t,
            title_len: tlen,
            shell: BuiltinShell::new(username),
            output: [0; 512],
            output_len: 0,
        }
    }
}

pub struct TermMux {
    pub tabs: [Option<Tab>; MAX_TABS],
    pub active: usize,
    pub count: usize,
}

pub enum MuxAction {
    /// Pass character to active shell for echo
    Echo(u8),
    /// Run active shell's line buffer as a command
    Submit,
    /// Clear the active tab's output
    ClearPane,
    /// Create a new tab
    NewTab,
    /// Close the active tab (decrement tabs)
    CloseTab,
    /// Switch to tab N (1-indexed: 1..=10)
    SwitchTab(usize),
    /// Switch to next tab
    NextTab,
    /// Switch to previous tab
    PrevTab,
    /// Backspace in active tab
    Backspace,
}

impl TermMux {
    pub fn new(username: &[u8]) -> Self {
        let mut tabs: [Option<Tab>; MAX_TABS] = [
            None, None, None, None, None, None, None, None, None, None,
        ];
        tabs[0] = Some(Tab::new(b"shell", username));
        Self {
            tabs,
            active: 0,
            count: 1,
        }
    }

    /// Handle a printable ASCII byte. Return the output produced (if any).
    pub fn handle_ascii(&mut self, ascii: u8) -> Option<([u8; 512], usize)> {
        let tab = self.tabs[self.active].as_mut()?;
        match ascii {
            b'\n' | b'\r' => {
                let vfs_cap = nameserver_lookup_vfs();
                let result = tab.shell.run_line(vfs_cap);
                tab.shell.line_len = 0;
                match result {
                    ShellOutput::Clear => {
                        tab.output_len = 0;
                        None
                    }
                    ShellOutput::Exit => None, // handled at higher level
                    ShellOutput::Text(buf, len) => {
                        tab.output = buf;
                        tab.output_len = len;
                        Some((buf, len))
                    }
                }
            }
            0x08 => {
                tab.shell.backspace();
                None
            }
            c if c >= 0x20 && c <= 0x7E => {
                tab.shell.feed_char(c);
                None
            }
            _ => None,
        }
    }

    /// Handle a control key (Ctrl+letter).
    /// Returns the resulting shell output if a command was submitted.
    pub fn handle_ctrl(&mut self, ascii: u8) -> Option<([u8; 512], usize)> {
        let ctrl_char = if ascii.is_ascii_lowercase() {
            ascii - b'a' + 1
        } else if ascii.is_ascii_uppercase() {
            ascii - b'A' + 1
        } else {
            return None;
        };

        match ctrl_char {
            20 => {
                // Ctrl+T: new tab
                self.new_tab();
                None
            }
            23 => {
                // Ctrl+W: close tab
                self.close_tab();
                None
            }
            12 => {
                // Ctrl+L: clear
                if let Some(tab) = self.tabs[self.active].as_mut() {
                    tab.output_len = 0;
                }
                None
            }
            // Ctrl+1..9,0 → switch to tab N
            n @ 1..=10 => {
                let idx = if n == 10 { 9 } else { (n - 1) as usize };
                if idx < self.count {
                    self.active = idx;
                }
                None
            }
            _ => None,
        }
    }

    /// Handle a backspace key in the active tab.
    pub fn handle_backspace(&mut self) {
        if let Some(tab) = self.tabs[self.active].as_mut() {
            tab.shell.backspace();
        }
    }

    fn new_tab(&mut self) {
        if self.count < MAX_TABS {
            let uname_bytes = &self.tabs[0]
                .as_ref()
                .map(|t| &t.shell.username[..t.shell.username_len])
                .unwrap_or(&[]);
            self.tabs[self.count] = Some(Tab::new(b"shell", uname_bytes));
            self.active = self.count;
            self.count += 1;
        }
    }

    fn close_tab(&mut self) {
        if self.count <= 1 {
            return; // must have at least one tab
        }
        // Shift tabs down
        for i in self.active..self.count - 1 {
            self.tabs[i] = self.tabs[i + 1].take();
        }
        self.tabs[self.count - 1] = None;
        self.count -= 1;
        if self.active >= self.count {
            self.active = self.count - 1;
        }
    }

    /// Get the active tab's prompt string (for display).
    pub fn active_prompt(&self) -> &str {
        if let Some(tab) = &self.tabs[self.active] {
            tab.shell.prompt()
        } else {
            "$ "
        }
    }

    /// Get the active tab's current line buffer for echo display.
    pub fn active_line(&self) -> &[u8] {
        if let Some(tab) = &self.tabs[self.active] {
            &tab.shell.line_buf[..tab.shell.line_len]
        } else {
            &[]
        }
    }
}

fn nameserver_lookup_vfs() -> CapabilityToken {
    sunlight_ipc::nameserver_lookup("vfs").unwrap_or(CapabilityToken(0))
}
