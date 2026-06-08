//! Built-in shell with no fork/exec.
//!
//! Provides a command line interface with basic commands. All output goes
//! through a fixed-size text buffer. Uses VFS IPC for `cat`.

use sunlight_ipc::{CapabilityToken, IpcMsg, VfsMsg, nameserver_lookup, ipc_call};

pub struct BuiltinShell {
    pub cwd: [u8; 256],
    pub cwd_len: usize,
    pub username: [u8; 64],
    pub username_len: usize,
    pub line_buf: [u8; 512],
    pub line_len: usize,
}

pub enum ShellOutput {
    Text([u8; 512], usize),
    Clear,
    Exit,
}

impl BuiltinShell {
    pub fn new(username: &[u8]) -> Self {
        let ulen = username.len().min(63);
        let mut uname = [0u8; 64];
        uname[..ulen].copy_from_slice(&username[..ulen]);

        let cwd = b"/";
        let mut cwdbuf = [0u8; 256];
        cwdbuf[..cwd.len()].copy_from_slice(cwd);

        Self {
            cwd: cwdbuf,
            cwd_len: cwd.len(),
            username: uname,
            username_len: ulen,
            line_buf: [0; 512],
            line_len: 0,
        }
    }

    /// Format the command prompt.
    pub fn prompt(&self) -> &str {
        // We return a static-ish string. For simplicity, we pre-build a short prompt.
        // In a real shell this would be dynamic.
        "root@sunlight:/ $ "
    }

    /// Feed one character for echo-as-you-type.
    pub fn feed_char(&mut self, c: u8) {
        match c {
            b'\n' | b'\r' => {} // handled by run_line
            0x08 => {
                if self.line_len > 0 {
                    self.line_len -= 1;
                }
            }
            _ => {
                if self.line_len < self.line_buf.len() {
                    self.line_buf[self.line_len] = c;
                    self.line_len += 1;
                }
            }
        }
    }

    /// Delete last character (backspace).
    pub fn backspace(&mut self) {
        if self.line_len > 0 {
            self.line_len -= 1;
        }
    }

    /// Execute the current line buffer as a command.
    pub fn run_line(&mut self, _vfs_cap: CapabilityToken) -> ShellOutput {
        let line = &self.line_buf[..self.line_len];
        let line_str = match core::str::from_utf8(line) {
            Ok(s) => s.trim(),
            Err(_) => {
                let mut buf = [0u8; 512];
                let msg = b"Invalid UTF-8 in command\n";
                let len = msg.len().min(buf.len());
                buf[..len].copy_from_slice(&msg[..len]);
                return ShellOutput::Text(buf, len);
            }
        };

        let result = if line_str.is_empty() {
            None
        } else if line_str == "help" {
            Some(cmd_help())
        } else if line_str.starts_with("echo ") {
            Some(cmd_echo(&line_str[5..]))
        } else if line_str == "clear" {
            return ShellOutput::Clear;
        } else if line_str == "pwd" {
            Some(cmd_pwd(&self.cwd[..self.cwd_len]))
        } else if line_str.starts_with("cat ") {
            Some(cmd_cat(&line_str[4..]))
        } else if line_str == "whoami" {
            Some(cmd_whoami(&self.username[..self.username_len]))
        } else if line_str == "uname" {
            Some(cmd_uname())
        } else if line_str == "exit" {
            return ShellOutput::Exit;
        } else {
            Some(error_msg("unknown command, type 'help'"))
        };

        if let Some((ref buf, len)) = result {
            ShellOutput::Text(*buf, len)
        } else {
            ShellOutput::Text([0; 512], 0)
        }
    }
}

fn make_output(data: &[u8]) -> ([u8; 512], usize) {
    let mut buf = [0u8; 512];
    let len = data.len().min(buf.len());
    buf[..len].copy_from_slice(&data[..len]);
    (buf, len)
}

fn error_msg(msg: &str) -> ([u8; 512], usize) {
    let mut buf = [0u8; 512];
    let text = msg.as_bytes();
    let len = text.len().min(buf.len() - 1);
    buf[..len].copy_from_slice(text);
    buf[len] = b'\n';
    (buf, len + 1)
}

fn cmd_help() -> ([u8; 512], usize) {
    make_output(
        b"Available commands:\n\
          help    Show this message\n\
          echo    Print arguments\n\
          clear   Clear the screen\n\
          pwd     Print working directory\n\
          cat     Read file via VFS\n\
          whoami  Print username\n\
          uname   Print system info\n\
          exit    Return to login\n",
    )
}

fn cmd_echo(args: &str) -> ([u8; 512], usize) {
    let mut buf = [0u8; 512];
    let mut len = 0;
    let bytes = args.as_bytes();
    while len < bytes.len().min(buf.len() - 1) {
        buf[len] = bytes[len];
        len += 1;
    }
    buf[len] = b'\n';
    len += 1;
    (buf, len)
}

fn cmd_pwd(cwd: &[u8]) -> ([u8; 512], usize) {
    let mut buf = [0u8; 512];
    let mut len = 0;
    while len < cwd.len().min(buf.len() - 1) {
        buf[len] = cwd[len];
        len += 1;
    }
    buf[len] = b'\n';
    len += 1;
    (buf, len)
}

fn cmd_whoami(username: &[u8]) -> ([u8; 512], usize) {
    let mut buf = [0u8; 512];
    let mut len = 0;
    while len < username.len().min(buf.len() - 1) {
        buf[len] = username[len];
        len += 1;
    }
    buf[len] = b'\n';
    len += 1;
    (buf, len)
}

fn cmd_uname() -> ([u8; 512], usize) {
    make_output(b"SunlightOS v0.1.0 x86_64\n")
}

/// Read a file via VFS IPC. Uses nameserver_lookup("vfs") → ipc_call.
fn cmd_cat(path: &str) -> ([u8; 512], usize) {
    let vfs_cap = match nameserver_lookup("vfs") {
        Some(c) => c,
        None => return error_msg("cat: VFS not available"),
    };

    let open_msg = path_msg(VfsMsg::OPEN, path);
    let reply = ipc_call(vfs_cap, open_msg);
    if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
        return error_msg("cat: file not found");
    }
    let handle = reply.words[1] as u32;

    let mut out = [0u8; 512];
    let mut total = 0usize;

    loop {
        if total >= out.len() - 1 {
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
        // Decode bytes from words[2] and words[3]
        let src = &reply.words[2..4];
        for i in 0..n.min(out.len() - total) {
            let word_idx = i / 8;
            let byte_idx = i % 8;
            out[total + i] = ((src[word_idx] >> (byte_idx * 8)) & 0xFF) as u8;
        }
        total += n;
    }

    // Close
    let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));

    out[total] = b'\n';
    total += 1;
    (out, total)
}

fn path_msg(label: u64, path: &str) -> IpcMsg {
    let bytes = path.as_bytes();
    let mut msg = IpcMsg::with_label(label);
    let mut word_idx = 0;
    while word_idx < 4 {
        let start = word_idx * 8;
        let end = (start + 8).min(bytes.len());
        if start < bytes.len() {
            msg = msg.word(word_idx, pack_bytes(&bytes[start..end]));
        }
        word_idx += 1;
    }
    msg
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
