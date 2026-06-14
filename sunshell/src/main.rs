#![cfg_attr(feature = "sunlight", no_std)]
#![cfg_attr(feature = "sunlight", no_main)]

#[cfg(feature = "std")]
mod builtins;
#[cfg(feature = "std")]
mod exec;
#[cfg(feature = "std")]
mod input;
#[cfg(feature = "std")]
mod parser;

#[cfg(feature = "std")]
use exec::{Executor, PosixExecutor};
#[cfg(feature = "std")]
use input::ReadLine;
#[cfg(feature = "std")]
use std::env;

#[cfg(feature = "std")]
fn run_command(line: &str, executor: &dyn Executor) -> Option<i32> {
    let tokens = parser::tokenize(line);
    if tokens.is_empty() {
        return Some(0);
    }

    let argv: Vec<&str> = tokens.iter().map(|s| s.as_str()).collect();

    match builtins::run(&argv) {
        builtins::BuiltinResult::Done(code) => Some(code),
        builtins::BuiltinResult::Exit(code) => {
            std::process::exit(code);
        }
        builtins::BuiltinResult::NotBuiltin => match executor.run(&argv) {
            Ok(code) => Some(code),
            Err(e) => {
                eprintln!("sshl: {e}");
                Some(127)
            }
        },
    }
}

#[cfg(feature = "std")]
fn make_prompt() -> String {
    let cwd = env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    format!("user@sunlight:{cwd} $")
}

#[cfg(feature = "std")]
fn repl(executor: &dyn Executor) {
    loop {
        let prompt = make_prompt();
        match input::readline(&prompt) {
            Ok(ReadLine::Eof) => break,
            Ok(ReadLine::Line(line)) => {
                run_command(&line, executor);
            }
            Err(e) => {
                eprintln!("sshl: read error: {e}");
                std::process::exit(1);
            }
        }
    }
}

#[cfg(feature = "std")]
fn main() {
    let executor = PosixExecutor;
    let args: Vec<String> = env::args().collect();

    match args.as_slice() {
        // sshl -c "command args..."
        [_, flag, cmd] if flag == "-c" => {
            let code = run_command(cmd, &executor).unwrap_or(0);
            std::process::exit(code);
        }
        // interactive
        [_] => repl(&executor),
        _ => {
            eprintln!("Usage: sshl [-c command]");
            std::process::exit(1);
        }
    }
}

// ============================================================================
// SunlightOS (no_std) build
// ============================================================================

#[cfg(feature = "sunlight")]
extern crate alloc;

#[cfg(feature = "sunlight")]
struct BumpAllocator;

#[cfg(feature = "sunlight")]
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 1 * 1024 * 1024] = [0; 1 * 1024 * 1024];
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

#[cfg(feature = "sunlight")]
#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[cfg(feature = "sunlight")]
mod sysfetch;

#[cfg(feature = "sunlight")]
mod shellenv;

#[cfg(feature = "sunlight")]
#[no_main]
mod sunlight {
    use core::fmt::Write;

    use sunlight_ipc::{
        debug_log, endpoint_create, get_init_cap, ipc_call, ipc_reply_and_wait, nameserver_lookup,
        nameserver_register, sysinfo, CapabilityToken, InitMsg, IpcMsg, SunlightSyscall, VfsMsg, TzMsg,
    };

    /// CPU brand string via CPUID leaves 0x80000002..=0x80000004 (unprivileged).
    /// Returns the bytes written into `buf` (trimmed of NULs/leading spaces).
    fn cpu_brand(buf: &mut [u8; 48]) -> usize {
        for (i, leaf) in (0x8000_0002u32..=0x8000_0004).enumerate() {
            let r = core::arch::x86_64::__cpuid(leaf);
            for (j, reg) in [r.eax, r.ebx, r.ecx, r.edx].into_iter().enumerate() {
                buf[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&reg.to_le_bytes());
            }
        }
        let start = buf.iter().position(|&b| b != b' ' && b != 0).unwrap_or(0);
        let end = buf.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
        if start < end {
            buf.copy_within(start..end, 0);
            end - start
        } else {
            0
        }
    }

    const KBD_LABEL: u64 = 1;
    const OUTPUT_LABEL: u64 = 2;
    const EXIT_LABEL: u64 = 3;
    const DRAIN_LABEL: u64 = 4;
    const MAX_LINE: usize = 256;
    const MAX_OUT: usize = 64;
    const LONG_OUT_MAX: usize = 16384;
    const IPC_OUTPUT_BYTES: usize = 16;
    const OS_NAME: &str = "SunlightOS";
    const KERNEL_NAME: &str = "SunlightOS/CORE";
    const OS_VERSION: &str = env!("CARGO_PKG_VERSION");
    const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

    enum PasswdState {
        None,
        PromptNew {
            target_user: [u8; 64],
            target_user_len: usize,
        },
        PromptConfirm {
            target_user: [u8; 64],
            target_user_len: usize,
            new_password: [u8; 64],
            new_password_len: usize,
        },
    }

    struct Shell {
        line: [u8; MAX_LINE],
        line_len: usize,
        username: [u8; 64],
        username_len: usize,
        uid: u32,
        gid: u32,
        passwd_state: PasswdState,
        passwd_buffer: [u8; 64],
        passwd_buffer_len: usize,
        env: crate::shellenv::ShellEnv,
        cwd: alloc::string::String,
    }

    impl Shell {
        fn new() -> Self {
            Self {
                line: [0; MAX_LINE],
                line_len: 0,
                username: [
                    b'r', b'o', b'o', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ],
                username_len: 4,
                uid: 0,
                gid: 0,
                passwd_state: PasswdState::None,
                passwd_buffer: [0; 64],
                passwd_buffer_len: 0,
                env: crate::shellenv::ShellEnv::new(),
                cwd: alloc::string::String::from("/"),
            }
        }

        /// Seed PATH/USER/HOME/SHELL once the user identity is known.
        fn init_env(&mut self) {
            let username = alloc::string::String::from(
                core::str::from_utf8(&self.username[..self.username_len]).unwrap_or(""),
            );
            self.env.load_defaults(self.uid, &username);
            self.env.set("PWD", &self.cwd);
        }

        fn handle_byte(&mut self, byte: u8) -> ([u8; MAX_OUT], usize) {
            // Check if we're in password entry mode
            match &self.passwd_state {
                PasswdState::PromptNew {
                    target_user,
                    target_user_len,
                } => {
                    let target = (*target_user, *target_user_len);
                    return self.handle_passwd_input(
                        byte,
                        PasswdState::PromptNew {
                            target_user: target.0,
                            target_user_len: target.1,
                        },
                    );
                }
                PasswdState::PromptConfirm {
                    target_user,
                    target_user_len,
                    new_password,
                    new_password_len,
                } => {
                    let state = PasswdState::PromptConfirm {
                        target_user: *target_user,
                        target_user_len: *target_user_len,
                        new_password: *new_password,
                        new_password_len: *new_password_len,
                    };
                    return self.handle_passwd_input(byte, state);
                }
                PasswdState::None => {}
            }

            // Normal command mode
            match byte {
                b'\n' | b'\r' => {
                    let result = self.run_line();
                    self.line_len = 0;
                    result
                }
                0x08 => {
                    if self.line_len > 0 {
                        self.line_len -= 1;
                    }
                    ([0; MAX_OUT], 0)
                }
                c if c >= 0x20 && c <= 0x7E => {
                    if self.line_len < MAX_LINE {
                        self.line[self.line_len] = c;
                        self.line_len += 1;
                    }
                    ([0; MAX_OUT], 0)
                }
                _ => ([0; MAX_OUT], 0),
            }
        }

        fn handle_passwd_input(&mut self, byte: u8, state: PasswdState) -> ([u8; MAX_OUT], usize) {
            match byte {
                b'\n' | b'\r' => {
                    // Password entry complete
                    let password = self.passwd_buffer[..self.passwd_buffer_len].to_vec();

                    match state {
                        PasswdState::PromptNew {
                            target_user,
                            target_user_len,
                        } => {
                            // Transition to confirm password prompt
                            self.passwd_state = PasswdState::PromptConfirm {
                                target_user,
                                target_user_len,
                                new_password: self.passwd_buffer,
                                new_password_len: self.passwd_buffer_len,
                            };
                            self.passwd_buffer_len = 0;
                            return copy_out(b"Retype new password: ");
                        }
                        PasswdState::PromptConfirm {
                            target_user,
                            target_user_len,
                            new_password,
                            new_password_len,
                        } => {
                            // Verify passwords match
                            if self.passwd_buffer_len != new_password_len
                                || &self.passwd_buffer[..self.passwd_buffer_len]
                                    != &new_password[..new_password_len]
                            {
                                self.passwd_state = PasswdState::None;
                                self.passwd_buffer_len = 0;
                                return copy_out(b"passwd: passwords do not match\n");
                            }
                            // Passwords match, update shadow
                            let result = self.update_shadow(
                                &target_user[..target_user_len],
                                &new_password[..new_password_len],
                            );
                            self.passwd_state = PasswdState::None;
                            self.passwd_buffer_len = 0;
                            return result;
                        }
                        PasswdState::None => {}
                    }
                    ([0; MAX_OUT], 0)
                }
                0x08 => {
                    // Backspace - delete from password buffer silently
                    if self.passwd_buffer_len > 0 {
                        self.passwd_buffer_len -= 1;
                    }
                    ([0; MAX_OUT], 0)
                }
                c if c >= 0x20 && c <= 0x7E => {
                    // Accumulate password without echoing
                    if self.passwd_buffer_len < 64 {
                        self.passwd_buffer[self.passwd_buffer_len] = c;
                        self.passwd_buffer_len += 1;
                    }
                    ([0; MAX_OUT], 0)
                }
                _ => ([0; MAX_OUT], 0),
            }
        }

        fn run_line(&mut self) -> ([u8; MAX_OUT], usize) {
            // Make owned copies of cmd and args first
            let cmd_owned: alloc::string::String;
            let target_user_owned: Option<alloc::string::String>;

            {
                let line = &self.line[..self.line_len];
                let line_str = match core::str::from_utf8(line) {
                    Ok(s) => s.trim(),
                    Err(_) => return copy_out(b"Invalid UTF-8\n"),
                };

                if line_str.is_empty() {
                    return ([0; MAX_OUT], 0);
                }

                let mut parts = line_str.split_ascii_whitespace();
                let cmd = parts.next().unwrap_or("");
                cmd_owned = alloc::string::String::from(cmd);

                // Check if this is passwd and get target arg
                if cmd == "passwd" {
                    let args: alloc::vec::Vec<&str> = parts.collect();
                    target_user_owned = if args.is_empty() {
                        None
                    } else {
                        Some(alloc::string::String::from(args[0]))
                    };
                } else {
                    target_user_owned = None;
                }
            }

            // Now self is not borrowed anymore, safe to call mutable methods
            if cmd_owned == "passwd" {
                return self.cmd_passwd(target_user_owned.as_deref());
            }

            // Re-parse for normal commands, expanding $VAR tokens in arguments.
            // cmd and args are owned so &mut self builtins (export/unset) can
            // run without keeping self.line borrowed.
            let cmd: alloc::string::String;
            let expanded: alloc::vec::Vec<alloc::string::String>;
            {
                let line = &self.line[..self.line_len];
                let line_str = core::str::from_utf8(line).unwrap_or("").trim();
                let mut parts = line_str.split_ascii_whitespace();
                cmd = alloc::string::String::from(parts.next().unwrap_or(""));
                expanded = parts.map(|t| self.env.expand_token(t)).collect();
            }
            let cmd = cmd.as_str();
            let args: alloc::vec::Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();

            // Handle shutdown and reboot specially (they don't return)
            match cmd {
                "shutdown" => {
                    self.cmd_shutdown();
                    unreachable!();
                }
                "reboot" => {
                    self.cmd_reboot();
                    unreachable!();
                }
                _ => {}
            }

            let out: &[u8] = match cmd {
                "cd" => self.cmd_cd(&args),
                "pwd" => self.cmd_pwd(),
                "ls" => self.run_ls_external(&args),
                "useradd" => self.cmd_useradd(&args),
                "userdel" => self.cmd_userdel(&args),
                "su" => b"su: not implemented\n",
                "groups" => self.cmd_groups(&args),
                "chmod" => self.cmd_chmod(&args),
                "chown" => self.cmd_chown(&args),
                "env" => self.cmd_env(),
                "export" => self.cmd_export(&args),
                "unset" => self.cmd_unset(&args),
                "sysfetch" => self.cmd_sysfetch(),
                "hostnamectl" => self.cmd_hostnamectl(),
                "uptime" => self.cmd_uptime(),
                "tzctl" => self.cmd_tzctl(&args),
                "help" => b"Builtins: cd, pwd, useradd, userdel, passwd, groups, chmod, chown, env, export, unset, sysfetch, hostnamectl, uptime, tzctl, help, shutdown, reboot\n",
                "clear" => b"\x1B[2J\x1B[H",  // Clear screen + home cursor (0,0)
                "exit" => b"exit\n",
                // Not a builtin: resolve through $PATH and run it (Step 3)
                _ => self.run_external(cmd, &args),
            };

            // Backward compatibility logging for phase 3.6 tests
            let trimmed = trim_newline(out);
            if trimmed == b"root" {
                debug_log("[TTY]  Output: root");
            } else if trimmed.starts_with(b"Welcome to SunlightOS") {
                debug_log("[TTY]  Output: Welcome to SunlightOS");
            } else if trimmed == b"hello" {
                debug_log("[TTY]  Output: hello");
            }

            copy_out(out)
        }

        /// Load user information from VFS by username
        fn load_user_from_vfs(&mut self, username: &[u8]) -> bool {
            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return false,
            };

            // Build GETPWNAM request
            let mut msg = IpcMsg::with_label(VfsMsg::GETPWNAM);
            let mut word_idx = 0;
            for i in 0..(username.len() / 8 + 1) {
                let start = i * 8;
                let end = (start + 8).min(username.len());
                if start < username.len() {
                    let mut word = 0u64;
                    for (j, &b) in username[start..end].iter().enumerate() {
                        word |= (b as u64) << (j * 8);
                    }
                    msg = msg.word(word_idx, word);
                }
                word_idx += 1;
            }

            let reply = ipc_call(vfs_cap, msg);
            if reply.label == VfsMsg::REPLY && reply.word_count >= 3 {
                self.uid = reply.words[1] as u32;
                self.gid = reply.words[2] as u32;
                // Copy username to shell
                let len = username.len().min(self.username.len() - 1);
                self.username[..len].copy_from_slice(&username[..len]);
                self.username_len = len;
                true
            } else {
                false
            }
        }

        fn load_user_by_uid(&mut self, uid: u32) -> bool {
            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return false,
            };

            // Build GETPWUID request
            let msg = IpcMsg::with_label(VfsMsg::GETPWUID).word(0, uid as u64);
            let reply = ipc_call(vfs_cap, msg);
            if reply.label == VfsMsg::REPLY && reply.word_count >= 3 {
                self.uid = reply.words[1] as u32;
                self.gid = reply.words[2] as u32;

                // Extract username from words[3:7] if available (new GETPWUID enhancement)
                if reply.word_count >= 7 {
                    let mut username_bytes = [0u8; 64];
                    for i in 0..4 {
                        let word = reply.words[3 + i];
                        for j in 0..8 {
                            let b = ((word >> (j * 8)) & 0xFF) as u8;
                            if b == 0 {
                                break;
                            }
                            username_bytes[i * 8 + j] = b;
                        }
                    }
                    let username_len = username_bytes
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(64)
                        .min(64);
                    self.username[..username_len].copy_from_slice(&username_bytes[..username_len]);
                    self.username_len = username_len;
                }
                true
            } else {
                false
            }
        }

        fn cmd_whoami(&self) -> &[u8] {
            &self.username[..self.username_len]
        }

        fn cmd_id(&self, args: &[&str]) -> &[u8] {
            if args.is_empty() {
                // Return current user's info
                unsafe {
                    static mut BUF: [u8; 64] = [0u8; 64];
                    let buf = &mut BUF;
                    let username_str =
                        core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("root");
                    let prefix = alloc::format!(
                        "uid={}({}) gid={}(root) groups={}(root)",
                        self.uid,
                        username_str,
                        self.gid,
                        self.gid
                    );
                    let bytes = prefix.as_bytes();
                    let len = bytes.len().min(buf.len());
                    buf[..len].copy_from_slice(&bytes[..len]);
                    return &buf[..len];
                }
            }
            // lookup specific user
            let username = args[0].as_bytes();
            if let Some((uid, gid)) = lookup_user(username) {
                unsafe {
                    static mut BUF: [u8; 64] = [0u8; 64];
                    let buf = &mut BUF;
                    let prefix =
                        alloc::format!("uid={}({}) gid={}({})", uid, args[0], gid, "users");
                    let bytes = prefix.as_bytes();
                    let len = bytes.len().min(buf.len());
                    buf[..len].copy_from_slice(&bytes[..len]);
                    return &buf[..len];
                }
            }
            b"id: no such user\n"
        }

        fn cmd_useradd(&self, args: &[&str]) -> &[u8] {
            if self.uid != 0 {
                return b"useradd: permission denied\n";
            }
            let username = match args.get(0) {
                Some(u) => u,
                None => return b"useradd: missing username\n",
            };

            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return b"useradd: VFS not available\n",
            };

            // Read /etc/passwd to find next uid
            let passwd_data = read_file(vfs_cap, "/etc/passwd");
            let max_uid = find_max_uid(&passwd_data);
            let new_uid = if max_uid < 1000 { 1000 } else { max_uid + 1 };

            let mut new_passwd = alloc::string::String::from_utf8_lossy(&passwd_data).into_owned();
            new_passwd.push_str(&alloc::format!(
                "{}:x:{}:100::/home/{}:/bin/sh\n",
                username,
                new_uid,
                username
            ));

            let shadow_data = read_file(vfs_cap, "/etc/shadow");
            let mut new_shadow = alloc::string::String::from_utf8_lossy(&shadow_data).into_owned();
            new_shadow.push_str(&alloc::format!("{}:!:0:0:99999:7:::\n", username));

            let group_data = read_file(vfs_cap, "/etc/group");
            let mut new_group = alloc::string::String::from_utf8_lossy(&group_data).into_owned();
            // Append user to users group (gid=100)
            if let Some(pos) = new_group.find("users:x:100:") {
                let insert_pos = pos + b"users:x:100:".len();
                new_group.insert_str(insert_pos, &alloc::format!("{},", username));
            }

            // Write back
            if let Err(_) = write_file(vfs_cap, "/etc/passwd", new_passwd.as_bytes()) {
                return b"useradd: failed to write passwd\n";
            }
            if let Err(_) = write_file(vfs_cap, "/etc/shadow", new_shadow.as_bytes()) {
                return b"useradd: failed to write shadow\n";
            }
            if let Err(_) = write_file(vfs_cap, "/etc/group", new_group.as_bytes()) {
                return b"useradd: failed to write group\n";
            }

            // Create home directory
            let home = alloc::format!("/home/{}", username);
            let _ = mkdir(vfs_cap, &home, new_uid, 100, 0o755);

            b"OK\n"
        }

        fn cmd_userdel(&self, args: &[&str]) -> &[u8] {
            if self.uid != 0 {
                return b"userdel: permission denied\n";
            }
            let username = match args.get(0) {
                Some(u) => u,
                None => return b"userdel: missing username\n",
            };

            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return b"userdel: VFS not available\n",
            };

            let passwd_data = read_file(vfs_cap, "/etc/passwd");
            let shadow_data = read_file(vfs_cap, "/etc/shadow");
            let group_data = read_file(vfs_cap, "/etc/group");

            let mut new_passwd = alloc::string::String::new();
            for line in alloc::string::String::from_utf8_lossy(&passwd_data).lines() {
                if !line.starts_with(username)
                    || !line
                        .as_bytes()
                        .get(username.len())
                        .map(|&b| b == b':')
                        .unwrap_or(false)
                {
                    new_passwd.push_str(line);
                    new_passwd.push('\n');
                }
            }

            let mut new_shadow = alloc::string::String::new();
            for line in alloc::string::String::from_utf8_lossy(&shadow_data).lines() {
                if !line.starts_with(username)
                    || !line
                        .as_bytes()
                        .get(username.len())
                        .map(|&b| b == b':')
                        .unwrap_or(false)
                {
                    new_shadow.push_str(line);
                    new_shadow.push('\n');
                }
            }

            let mut new_group = alloc::string::String::new();
            for line in alloc::string::String::from_utf8_lossy(&group_data).lines() {
                let mut modified = alloc::string::String::from(line);
                if modified.contains("users:x:100:") {
                    modified = modified.replace(&alloc::format!("{},", username), "");
                    modified = modified.replace(&alloc::format!(",{}", username), "");
                    modified = modified.replace(username, "");
                }
                new_group.push_str(&modified);
                new_group.push('\n');
            }

            if let Err(_) = write_file(vfs_cap, "/etc/passwd", new_passwd.as_bytes()) {
                return b"userdel: failed to write passwd\n";
            }
            if let Err(_) = write_file(vfs_cap, "/etc/shadow", new_shadow.as_bytes()) {
                return b"userdel: failed to write shadow\n";
            }
            if let Err(_) = write_file(vfs_cap, "/etc/group", new_group.as_bytes()) {
                return b"userdel: failed to write group\n";
            }

            debug_log("[SunlightOS] Phase 3.8 OK");
            b"OK\n"
        }

        fn cmd_passwd(&mut self, target_arg: Option<&str>) -> ([u8; MAX_OUT], usize) {
            // Determine target user
            let target_user_bytes = if let Some(arg) = target_arg {
                // Arg provided: change specified user's password (root only)
                if self.uid != 0 {
                    return copy_out(b"passwd: permission denied\n");
                }
                arg.as_bytes()
            } else {
                // No args: change current user's password
                &self.username[..self.username_len]
            };
            let target_user = target_user_bytes;

            // Verify target user exists
            if !self.user_exists(target_user) {
                return copy_out(b"passwd: user not found\n");
            }

            // Enter password prompt mode
            let mut target_user_buf = [0u8; 64];
            let target_user_len = target_user.len().min(64);
            target_user_buf[..target_user_len].copy_from_slice(&target_user[..target_user_len]);

            self.passwd_state = PasswdState::PromptNew {
                target_user: target_user_buf,
                target_user_len,
            };
            self.passwd_buffer_len = 0;

            copy_out(b"New password: ")
        }

        fn user_exists(&self, username: &[u8]) -> bool {
            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return false,
            };

            let mut msg = IpcMsg::with_label(VfsMsg::GETPWNAM);
            let mut word_idx = 0;
            for i in 0..(username.len() / 8 + 1) {
                let start = i * 8;
                let end = (start + 8).min(username.len());
                if start < username.len() {
                    let mut word = 0u64;
                    for (j, &b) in username[start..end].iter().enumerate() {
                        word |= (b as u64) << (j * 8);
                    }
                    msg = msg.word(word_idx, word);
                }
                word_idx += 1;
            }

            let reply = ipc_call(vfs_cap, msg);
            reply.label == VfsMsg::REPLY && reply.words[0] == 0
        }

        fn update_shadow(&mut self, username: &[u8], password: &[u8]) -> ([u8; MAX_OUT], usize) {
            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return copy_out(b"passwd: VFS not available\n"),
            };

            // Read current shadow file
            let shadow_data = read_file(vfs_cap, "/etc/shadow");
            let shadow_str = core::str::from_utf8(&shadow_data).unwrap_or("");

            // Find and update the target user's shadow entry
            let mut new_shadow = alloc::string::String::new();
            let target_username = core::str::from_utf8(username).unwrap_or("");
            let new_password_str = core::str::from_utf8(password).unwrap_or("?");
            let mut found = false;

            for line in shadow_str.lines() {
                let parts: alloc::vec::Vec<&str> = line.split(':').collect();
                if !parts.is_empty() && parts[0] == target_username {
                    // Replace password in this entry
                    new_shadow.push_str(&alloc::format!(
                        "{}:{}:0:0:99999:7:::\n",
                        target_username,
                        new_password_str
                    ));
                    found = true;
                } else {
                    new_shadow.push_str(line);
                    new_shadow.push('\n');
                }
            }

            // If user not found in shadow, add new entry
            if !found {
                new_shadow.push_str(&alloc::format!(
                    "{}:{}:0:0:99999:7:::\n",
                    target_username,
                    new_password_str
                ));
            }

            // Write updated shadow file
            match write_file(vfs_cap, "/etc/shadow", new_shadow.as_bytes()) {
                Ok(()) => {
                    debug_log("[PASSWD] Password updated");
                    copy_out(b"passwd: password updated\n")
                }
                Err(()) => copy_out(b"passwd: failed to update shadow\n"),
            }
        }

        fn cmd_groups(&self, _args: &[&str]) -> &[u8] {
            b"root wheel\n"
        }

        /// List the environment, one KEY=VALUE per line (long output path).
        fn cmd_env(&self) -> &[u8] {
            unsafe {
                LONG_OUT_ACTIVE = true;
            }
            for (key, value) in self.env.iter() {
                let line = alloc::format!("{}={}", key, value);
                push_line(&line);
            }
            b""
        }

        fn cmd_export(&mut self, args: &[&str]) -> &[u8] {
            if args.is_empty() {
                return b"export: usage: export KEY=VALUE\n";
            }
            for arg in args {
                match arg.split_once('=') {
                    Some((key, value)) if !key.is_empty() => self.env.set(key, value),
                    _ => return b"export: usage: export KEY=VALUE\n",
                }
            }
            b""
        }

        fn cmd_cd(&mut self, args: &[&str]) -> &[u8] {
            let target = if let Some(path) = args.first().copied() {
                path
            } else {
                self.env.get("HOME").unwrap_or("/")
            };
            let Some(next) = normalize_path(&self.cwd, target) else {
                return b"cd: invalid path\n";
            };

            let Some(vfs_cap) = nameserver_lookup("vfs") else {
                return b"cd: VFS not available\n";
            };
            if !stat_is_dir(vfs_cap, &next) {
                return b"cd: no such directory\n";
            }

            self.env.set("OLDPWD", &self.cwd);
            self.cwd = next;
            self.env.set("PWD", &self.cwd);
            b""
        }

        fn cmd_pwd(&self) -> &[u8] {
            static mut BUF: [u8; 256] = [0; 256];
            unsafe {
                let bytes = self.cwd.as_bytes();
                let n = bytes.len().min(BUF.len().saturating_sub(1));
                BUF[..n].copy_from_slice(&bytes[..n]);
                BUF[n] = b'\n';
                &BUF[..n + 1]
            }
        }

        fn run_ls_external(&mut self, args: &[&str]) -> &'static [u8] {
            let has_path = args.iter().any(|a| !a.starts_with('-') || *a == "-");
            if has_path {
                return self.run_external("ls", args);
            }
            let mut forwarded: alloc::vec::Vec<&str> = alloc::vec::Vec::with_capacity(args.len() + 1);
            forwarded.extend_from_slice(args);
            let cwd_owned = self.cwd.clone();
            forwarded.push(cwd_owned.as_str());
            self.run_external("ls", &forwarded)
        }

        fn cmd_unset(&mut self, args: &[&str]) -> &[u8] {
            if args.is_empty() {
                return b"unset: usage: unset KEY\n";
            }
            for arg in args {
                self.env.unset(arg);
            }
            b""
        }

        /// Resolve a non-builtin command via $PATH, spawn it with its stdout
        /// on a pipe, stream the output into the long-output buffer, and
        /// record the exit code in `$?` (Phase 6.5 Step 3).
        fn run_external(&mut self, cmd: &str, args: &[&str]) -> &'static [u8] {
            use sunlight_libc as ulibc;

            let path = match self.resolve_in_path(cmd) {
                Some(p) => p,
                None => {
                    self.env.set("?", "127");
                    unsafe {
                        LONG_OUT_ACTIVE = true;
                    }
                    push_line(&alloc::format!("sshl: command not found: {}", cmd));
                    return b"";
                }
            };

            let (read_end, write_end) = match ulibc::pipe() {
                Ok(p) => p,
                Err(_) => {
                    self.env.set("?", "126");
                    unsafe {
                        LONG_OUT_ACTIVE = true;
                    }
                    push_line("sshl: pipe failed");
                    return b"";
                }
            };

            // argv[0] is the applet name (multi-call binaries dispatch on it).
            let mut argv: alloc::vec::Vec<&[u8]> = alloc::vec::Vec::new();
            argv.push(cmd.as_bytes());
            for a in args {
                argv.push(a.as_bytes());
            }

            let pid = match ulibc::spawn(path.as_bytes(), &argv, Some(write_end)) {
                Ok(pid) => pid,
                Err(_) => {
                    let _ = ulibc::close(read_end);
                    let _ = ulibc::close(write_end);
                    self.env.set("?", "126");
                    unsafe {
                        LONG_OUT_ACTIVE = true;
                    }
                    push_line(&alloc::format!("sshl: cannot execute {}", path));
                    return b"";
                }
            };

            unsafe {
                LONG_OUT_ACTIVE = true;
            }

            // Drain the pipe until the child exits. We keep our copy of the
            // write end open, so an empty pipe reads as EAGAIN (never EOF)
            // while the child is alive.
            let mut chunk = [0u8; 256];
            let mut exit_code: u64 = 1;
            let mut spins: u32 = 0;
            loop {
                if let Ok(n) = ulibc::read(read_end, &mut chunk) {
                    if n > 0 {
                        push_bytes(&chunk[..n]);
                        continue;
                    }
                }
                match ulibc::try_waitpid(pid) {
                    Ok(Some(code)) => {
                        // Final drain after the child finished.
                        while let Ok(n) = ulibc::read(read_end, &mut chunk) {
                            if n == 0 {
                                break;
                            }
                            push_bytes(&chunk[..n]);
                        }
                        exit_code = code;
                        break;
                    }
                    Ok(None) => {
                        spins += 1;
                        if spins > 5_000_000 {
                            push_line("sshl: timeout waiting for child");
                            exit_code = 124;
                            break;
                        }
                        ulibc::yield_now();
                    }
                    Err(_) => break,
                }
            }
            let _ = ulibc::close(read_end);
            let _ = ulibc::close(write_end);

            self.env.set("?", &alloc::format!("{}", exit_code));
            debug_log(&alloc::format!("[EXEC] {} exit={}", cmd, exit_code));
            b""
        }

        /// Search the colon-separated $PATH for `cmd`, probing each candidate
        /// with a VFS STAT. First match wins. Paths containing '/' bypass the
        /// search and are probed directly.
        fn resolve_in_path(&self, cmd: &str) -> Option<alloc::string::String> {
            let vfs_cap = nameserver_lookup("vfs")?;
            if cmd.contains('/') {
                return if stat_is_file(vfs_cap, cmd) {
                    Some(alloc::string::String::from(cmd))
                } else {
                    None
                };
            }
            for dir in self.env.path_entries() {
                let candidate = if dir.ends_with('/') {
                    alloc::format!("{}{}", dir, cmd)
                } else {
                    alloc::format!("{}/{}", dir, cmd)
                };
                if stat_is_file(vfs_cap, &candidate) {
                    return Some(candidate);
                }
            }
            None
        }

        fn cmd_echo(&self, args: &[&str]) -> &[u8] {
            unsafe {
                static mut BUF: [u8; 64] = [0u8; 64];
                let buf = &mut BUF;
                let mut len = 0usize;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 && len < buf.len() {
                        buf[len] = b' ';
                        len += 1;
                    }
                    let bytes = arg.as_bytes();
                    let to_copy = bytes.len().min(buf.len() - len);
                    buf[len..len + to_copy].copy_from_slice(&bytes[..to_copy]);
                    len += to_copy;
                }
                if len < buf.len() {
                    buf[len] = b'\n';
                    len += 1;
                }
                &buf[..len]
            }
        }

        fn cmd_cat(&self, args: &[&str]) -> &[u8] {
            if args.is_empty() {
                return b"cat: missing file\n";
            }
            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return b"cat: VFS not available\n",
            };
            let path = args[0];
            let data = read_file(vfs_cap, path);
            if data.is_empty() {
                return b"cat: file not found\n";
            }
            unsafe {
                static mut BUF: [u8; 64] = [0u8; 64];
                let buf = &mut BUF;
                let len = data.len().min(buf.len() - 1);
                buf[..len].copy_from_slice(&data[..len]);
                buf[len] = b'\n';
                &buf[..len + 1]
            }
        }

        fn cmd_chmod(&self, args: &[&str]) -> &[u8] {
            if args.len() < 2 {
                return b"chmod: missing operand\n";
            }
            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return b"chmod: VFS not available\n",
            };
            let mode_str = args[0];
            let path = args[1];
            let mode = parse_mode(mode_str).unwrap_or(0);
            let msg = path_msg(VfsMsg::CHMOD, path).word(4, mode as u64);
            let reply = ipc_call(vfs_cap, msg);
            if reply.label == VfsMsg::REPLY && reply.words[0] == 0 {
                b""
            } else {
                b"chmod: failed\n"
            }
        }

        fn cmd_chown(&self, args: &[&str]) -> &[u8] {
            if self.uid != 0 {
                return b"chown: permission denied\n";
            }
            if args.len() < 2 {
                return b"chown: missing operand\n";
            }
            let vfs_cap = match nameserver_lookup("vfs") {
                Some(c) => c,
                None => return b"chown: VFS not available\n",
            };
            let owner_group = args[0];
            let path = args[1];
            let mut uid = 0u32;
            let mut gid = 0u32;
            if let Some(colon) = owner_group.find(':') {
                uid = parse_u32(&owner_group[..colon]).unwrap_or(0);
                gid = parse_u32(&owner_group[colon + 1..]).unwrap_or(0);
            } else {
                uid = parse_u32(owner_group).unwrap_or(0);
            }
            let msg = path_msg(VfsMsg::CHOWN, path)
                .word(4, uid as u64)
                .word(5, gid as u64);
            let reply = ipc_call(vfs_cap, msg);
            if reply.label == VfsMsg::REPLY && reply.words[0] == 0 {
                b""
            } else {
                b"chown: failed\n"
            }
        }

        fn cmd_sysfetch(&self) -> &[u8] {
            debug_log("[TTY]  sysfetch invoked");
            unsafe {
                LONG_OUT_ACTIVE = true;
            }

            let info = sysinfo();
            let mut cpu_buf = [0u8; 48];
            let cpu_len = cpu_brand(&mut cpu_buf);
            let cpu = core::str::from_utf8(&cpu_buf[..cpu_len]).unwrap_or("");

            // Phase 5: fetch IP from net service for display in sysfetch (and TUI)
            let net_ip: Option<[u8; 4]> = nameserver_lookup("net").and_then(|net_cap| {
                let reply = ipc_call(net_cap, IpcMsg::with_label(10 /* NetOp::GETIP */));
                if reply.label == 10 && reply.word_count >= 1 {
                    let w = reply.words[0];
                    Some([
                        (w & 0xff) as u8,
                        ((w >> 8) & 0xff) as u8,
                        ((w >> 16) & 0xff) as u8,
                        ((w >> 24) & 0xff) as u8,
                    ])
                } else {
                    None
                }
            });

            let mut buf = [0u8; 640];
            let username = core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("root");
            let mut hostname_buf = [0u8; 64];
            let hostname_len = read_hostname_from_vfs(&mut hostname_buf);
            let hostname = core::str::from_utf8(&hostname_buf[..hostname_len]).unwrap_or("sunlight");
            let len = crate::sysfetch::render_sysfetch_to_buffer(
                username,
                hostname,
                OS_NAME,
                OS_VERSION,
                KERNEL_NAME,
                KERNEL_VERSION,
                machine_name(),
                option_env!("COOKBOOK_SOURCE_IDENT"),
                cpu,
                info.uptime_secs,
                (info.used_ram_kb / 1024) as u32,
                (info.total_ram_kb / 1024) as u32,
                (info.swap_used_kb / 1024) as u32,
                (info.swap_total_kb / 1024) as u32,
                net_ip,
                &mut buf,
            );

            // Copy into long output buffer for chunked transmission
            unsafe {
                let bytes = &buf[..len];
                let space = LONG_OUT_MAX - LONG_OUT_LEN;
                let to_copy = bytes.len().min(space);
                LONG_OUT_BUF[LONG_OUT_LEN..LONG_OUT_LEN + to_copy]
                    .copy_from_slice(&bytes[..to_copy]);
                LONG_OUT_LEN += to_copy;
            }

            b""
        }

        fn cmd_tzctl(&mut self, args: &[&str]) -> &[u8] {
            // tzctl list | get | set <id> | info <id>
            // Uses "tz" service directly.
            unsafe {
                static mut OUT: [u8; 512] = [0u8; 512];
                let out = &mut OUT;
                let mut pos = 0usize;

                fn copy_bytes(dst: &mut [u8], pos: &mut usize, src: &[u8]) -> usize {
                    let n = src.len().min(dst.len().saturating_sub(*pos));
                    dst[*pos..*pos + n].copy_from_slice(&src[..n]);
                    *pos += n; n
                }

                let Some(tz_cap) = nameserver_lookup("tz") else {
                    let msg = b"tzctl: tz service not available\n";
                    out[..msg.len()].copy_from_slice(msg);
                    return &out[..msg.len()];
                };

                if args.is_empty() || args[0] == "get" {
                    // GET_ZONE + local time for display
                    let r = ipc_call(tz_cap, IpcMsg::with_label(TzMsg::GET_ZONE));
                    if r.label == TzMsg::REPLY {
                        let _ = copy_bytes(out, &mut pos, b"Active: ");
                        // simplistic id from first word bytes
                        for wi in 2..5 {
                            for b in 0..8 {
                                let ch = ((r.words[wi] >> (b*8)) & 0xff) as u8;
                                if ch == 0 || pos >= 500 { break; }
                                out[pos] = ch; pos += 1;
                            }
                        }
                    }
                    let r2 = ipc_call(tz_cap, IpcMsg::with_label(TzMsg::GET_LOCAL_TIME));
                    if r2.label == TzMsg::REPLY {
                        let w0 = r2.words[0];
                        let h = ((w0>>24)&0xff) as u8;
                        let m = ((w0>>16)&0xff) as u8;
                        let _ = copy_bytes(out, &mut pos, b"\nLocal: ");
                        out[pos] = b'0'+h/10; pos+=1; out[pos]=b'0'+h%10; pos+=1;
                        out[pos]=b':'; pos+=1;
                        out[pos]=b'0'+m/10; pos+=1; out[pos]=b'0'+m%10; pos+=1;
                    }
                    out[pos] = b'\n'; pos+=1;
                    return &out[..pos];
                } else if args[0] == "list" {
                    let filter = args.get(1).map(|s| s.to_lowercase());
                    let _ = copy_bytes(out, &mut pos, b"ID\tDISPLAY\tOFFSET\n");
                    if filter.is_none() {
                        let _ = copy_bytes(out, &mut pos, b"(showing first matches; use 'tzctl list <filter>' to search by id/name)\n");
                    }
                    let mut shown = 0u32;
                    for i in 0..600u64 {
                        let req = IpcMsg::with_label(TzMsg::LIST_ZONES).word(0, i);
                        let r = ipc_call(tz_cap, req);
                        if r.label != TzMsg::REPLY { break; }
                        if r.words[0] == 0xFFFF_FFFFu64 { break; }

                        // id at words 2..6 (up to 32 bytes)
                        let mut idbuf = [0u8; 32];
                        let mut idlen = 0usize;
                        'idloop: for wi in 2..6 {
                            for b in 0..8 {
                                let ch = ((r.words[wi] >> (b*8)) & 0xff) as u8;
                                if ch == 0 { break 'idloop; }
                                idbuf[idlen] = ch; idlen += 1;
                            }
                        }

                        // display name at words 6..8 (up to 16 bytes)
                        let mut dispbuf = [0u8; 16];
                        let mut displen = 0usize;
                        'disploop: for wi in 6..8 {
                            for b in 0..8 {
                                let ch = ((r.words[wi] >> (b*8)) & 0xff) as u8;
                                if ch == 0 { break 'disploop; }
                                dispbuf[displen] = ch; displen += 1;
                            }
                        }

                        if let Some(ref f) = filter {
                            let id_str = core::str::from_utf8(&idbuf[..idlen]).unwrap_or("");
                            let disp_str = core::str::from_utf8(&dispbuf[..displen]).unwrap_or("");
                            if !id_str.to_lowercase().contains(f.as_str())
                                && !disp_str.to_lowercase().contains(f.as_str())
                            {
                                continue;
                            }
                        }

                        // bail out before overflowing the output buffer
                        if pos + idlen + displen + 10 > out.len() {
                            break;
                        }

                        let _ = copy_bytes(out, &mut pos, &idbuf[..idlen]);
                        out[pos]=b'\t'; pos+=1;
                        let _ = copy_bytes(out, &mut pos, &dispbuf[..displen]);
                        out[pos]=b'\t'; pos+=1;

                        // offset from word(1): hours in low byte (signed), minutes in next byte
                        let w1 = r.words[1];
                        let oh = (w1 & 0xff) as i8;
                        let om = ((w1 >> 8) & 0xff) as u8;
                        out[pos] = if oh < 0 { b'-' } else { b'+' }; pos+=1;
                        let ah = oh.unsigned_abs();
                        out[pos]=b'0'+ah/10; pos+=1; out[pos]=b'0'+ah%10; pos+=1;
                        out[pos]=b':'; pos+=1;
                        out[pos]=b'0'+om/10; pos+=1; out[pos]=b'0'+om%10; pos+=1;
                        out[pos]=b'\n'; pos+=1;

                        shown += 1;
                        if filter.is_none() && shown >= 32 { break; }
                    }
                    if filter.is_some() && shown == 0 {
                        let _ = copy_bytes(out, &mut pos, b"(no matching zones)\n");
                    }
                    return &out[..pos];
                } else if args[0] == "set" && args.len() > 1 {
                    let id = args[1].as_bytes();
                    let mut req = IpcMsg::with_label(TzMsg::SET_ZONE);
                    // pack id into words[0..]
                    let mut wi=0; let mut bi=0; let mut w=0u64;
                    for &bb in id.iter().take(32) {
                        w |= (bb as u64) << (bi*8); bi+=1;
                        if bi==8 { req = req.word(wi, w); w=0; bi=0; wi+=1; }
                    }
                    if bi>0 { req = req.word(wi, w); }
                    let r = ipc_call(tz_cap, req);
                    if r.label == TzMsg::REPLY && r.words[0]==0 {
                        let _ = copy_bytes(out, &mut pos, b"Timezone changed to ");
                        let _ = copy_bytes(out, &mut pos, args[1].as_bytes());
                        out[pos]=b'\n'; pos+=1;
                    } else {
                        let _ = copy_bytes(out, &mut pos, b"tzctl: set failed\n");
                    }
                    return &out[..pos];
                }
                let _ = copy_bytes(out, &mut pos, b"tzctl: list | get | set <id>\n");
                &out[..pos]
            }
        }

        fn cmd_uptime(&self) -> &[u8] {
            debug_log("[TTY]  uptime invoked");

            let info = sysinfo();
            let uptime_secs = info.uptime_secs;
            let days = uptime_secs / 86400;
            let hours = (uptime_secs % 86400) / 3600;
            let mins = (uptime_secs % 3600) / 60;
            let user_count = 1;

            // Wall clock (UTC) for the leading HH:MM:SS field
            let clock = info.unix_time % 86400;

            unsafe {
                static mut BUF: [u8; 128] = [0u8; 128];
                let buf = &mut BUF;
                let uptime_str = alloc::format!(
                    " {}:{:02}:{:02} up {} day, {}:{:02}, {} user",
                    clock / 3600,
                    (clock / 60) % 60,
                    clock % 60,
                    days,
                    hours,
                    mins,
                    user_count
                );
                let bytes = uptime_str.as_bytes();
                let len = bytes.len().min(buf.len());
                buf[..len].copy_from_slice(&bytes[..len]);
                &buf[..len]
            }
        }

        fn cmd_hostnamectl(&self) -> &[u8] {
            debug_log("[TTY]  hostnamectl invoked");
            unsafe {
                LONG_OUT_ACTIVE = true;
            }
            let mut host_buf = [0u8; 64];
            let host_len = read_hostname_from_vfs(&mut host_buf);
            let host = core::str::from_utf8(&host_buf[..host_len]).unwrap_or("sunlight");

            let line1 = alloc::format!(
                "Static hostname: {} | OS: {}/{} | Kernel: {}/{} | Arch: {}",
                host,
                OS_NAME,
                OS_VERSION,
                KERNEL_NAME,
                KERNEL_VERSION,
                machine_name()
            );
            push_line(&line1);

            let line2 = "Chassis: vm | Hardware Vendor: QEMU | Hardware Model: Standard PC (i440FX + PIIX, 1996) | Firmware: Limine BIOS (1.17)";
            push_line(line2);
            b""
        }

        fn cmd_shutdown(&self) -> ! {
            debug_log("[TTY]  cmd: shutdown -> Broadcasting system shutdown loop...");
            unsafe {
                core::arch::asm!(
                    "mov rax, 80", // PowerCtl syscall number
                    "mov rdi, 0",  // 0 = shutdown
                    "syscall",
                    options(noreturn),
                );
            }
        }

        fn cmd_reboot(&self) -> ! {
            debug_log("[TTY]  cmd: reboot -> Broadcasting system reboot loop...");
            unsafe {
                core::arch::asm!(
                    "mov rax, 80", // PowerCtl syscall number
                    "mov rdi, 1",  // 1 = reboot
                    "syscall",
                    options(noreturn),
                );
            }
        }
    }

    fn copy_out(data: &[u8]) -> ([u8; MAX_OUT], usize) {
        let mut buf = [0u8; MAX_OUT];
        let len = data.len().min(buf.len());
        buf[..len].copy_from_slice(&data[..len]);
        (buf, len)
    }

    fn push_art_line(line: &[u8]) {
        long_out_push_byte(b' ');
        long_out_push_byte(b' ');
        for &b in line.iter() {
            if b != b' ' {
                long_out_push_str("\x1b[33m");
                long_out_push_byte(b);
                long_out_push_str("\x1b[0m");
            } else {
                long_out_push_byte(b' ');
            }
        }
        long_out_push_byte(b'\n');
    }

    fn push_label_value(label: &str, value: &str) {
        long_out_push_str("  \x1b[1m");
        long_out_push_str(label);
        long_out_push_str("\x1b[0m ");
        long_out_push_str(value);
        long_out_push_byte(b'\n');
    }

    fn push_label_value_bytes(label: &[u8], value: &[u8]) {
        long_out_push_str("  \x1b[1m");
        for &b in label.iter() {
            long_out_push_byte(b);
        }
        long_out_push_str("\x1b[0m ");
        for &b in value.iter() {
            long_out_push_byte(b);
        }
        long_out_push_byte(b'\n');
    }

    fn push_section_header(title: &str) {
        long_out_push_str("\x1b[33m\x1b[1m[");
        long_out_push_str(title);
        long_out_push_str("]\x1b[0m\n");
    }

    fn push_blank() {
        long_out_push_byte(b'\n');
    }

    fn push_line(s: &str) {
        long_out_push_str(s);
        long_out_push_byte(b'\n');
    }

    fn push_bytes(data: &[u8]) {
        for &b in data {
            long_out_push_byte(b);
        }
    }

    fn format_uptime(total_s: u64) -> alloc::string::String {
        let h = total_s / 3600;
        let m = (total_s % 3600) / 60;
        let s = total_s % 60;
        alloc::format!("{}h {}m {}s", h, m, s)
    }

    /// STAT a path on the VFS and check it is a regular file.
    /// The IPC path encoding carries at most 32 bytes (4 words).
    fn stat_is_file(vfs_cap: CapabilityToken, path: &str) -> bool {
        const FILE_TYPE_FILE: u64 = 1; // vfs_server file_type_code(FileType::File)
        if path.len() > 32 {
            return false;
        }
        let reply = ipc_call(vfs_cap, path_msg(VfsMsg::STAT, path));
        reply.label == VfsMsg::REPLY && reply.words[0] == 0 && reply.words[2] == FILE_TYPE_FILE
    }

    fn stat_is_dir(vfs_cap: CapabilityToken, path: &str) -> bool {
        const FILE_TYPE_DIR: u64 = 2; // vfs_server file_type_code(FileType::Directory)
        if path.len() > 32 {
            return false;
        }
        let reply = ipc_call(vfs_cap, path_msg(VfsMsg::STAT, path));
        reply.label == VfsMsg::REPLY && reply.words[0] == 0 && reply.words[2] == FILE_TYPE_DIR
    }

    fn normalize_path(cwd: &str, path: &str) -> Option<alloc::string::String> {
        let mut parts: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
        let full = if path.starts_with('/') {
            alloc::string::String::from(path)
        } else if cwd == "/" {
            alloc::format!("/{}", path)
        } else {
            alloc::format!("{}/{}", cwd, path)
        };

        for part in full.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                if !parts.is_empty() {
                    parts.pop();
                }
                continue;
            }
            if part.len() > 64 {
                return None;
            }
            parts.push(part);
        }

        if parts.is_empty() {
            return Some(alloc::string::String::from("/"));
        }
        let mut out = alloc::string::String::from("/");
        for (idx, part) in parts.iter().enumerate() {
            if idx > 0 {
                out.push('/');
            }
            out.push_str(part);
        }
        Some(out)
    }

    fn read_file(vfs_cap: CapabilityToken, path: &str) -> alloc::vec::Vec<u8> {
        let mut out = alloc::vec::Vec::new();
        let open_msg = path_msg(VfsMsg::OPEN, path);
        let reply = ipc_call(vfs_cap, open_msg);
        if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
            return out;
        }
        let handle = reply.words[1] as u32;
        let mut offset = 0usize;
        loop {
            let read_msg = IpcMsg::with_label(VfsMsg::READ)
                .word(0, handle as u64)
                .word(1, offset as u64)
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
            for i in 0..n {
                let word_idx = i / 8;
                let byte_idx = i % 8;
                out.push(((src[word_idx] >> (byte_idx * 8)) & 0xFF) as u8);
            }
            offset += n;
        }
        let _ = ipc_call(
            vfs_cap,
            IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64),
        );
        out
    }

    fn read_hostname_from_vfs(out: &mut [u8]) -> usize {
        let Some(vfs_cap) = nameserver_lookup("vfs") else {
            return copy_into(out, b"sunlight");
        };
        let data = read_file(vfs_cap, "/etc/hostname");
        if data.is_empty() {
            return copy_into(out, b"sunlight");
        }
        let end = data
            .iter()
            .position(|b| *b == b'\n' || *b == b'\r')
            .unwrap_or(data.len());
        if end == 0 {
            copy_into(out, b"sunlight")
        } else {
            copy_into(out, &data[..end])
        }
    }

    fn copy_into(dst: &mut [u8], src: &[u8]) -> usize {
        let n = dst.len().min(src.len());
        dst[..n].copy_from_slice(&src[..n]);
        n
    }

    fn machine_name() -> &'static str {
        option_env!("TARGET")
            .and_then(|t| t.split('-').next())
            .unwrap_or("x86_64")
    }

    fn write_file(vfs_cap: CapabilityToken, path: &str, data: &[u8]) -> Result<(), ()> {
        let open_msg = path_msg(VfsMsg::OPEN, path);
        let reply = ipc_call(vfs_cap, open_msg);
        if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
            return Err(());
        }
        let handle = reply.words[1] as u32;
        let mut offset = 0usize;
        while offset < data.len() {
            let chunk = &data[offset..(offset + 16).min(data.len())];
            let mut msg = IpcMsg::with_label(VfsMsg::WRITE)
                .word(0, handle as u64)
                .word(1, offset as u64);
            let mut word_idx = 2;
            let mut byte_idx = 0;
            let mut word = 0u64;
            for &b in chunk {
                word |= (b as u64) << (byte_idx * 8);
                byte_idx += 1;
                if byte_idx == 8 {
                    msg = msg.word(word_idx, word);
                    word = 0;
                    byte_idx = 0;
                    word_idx += 1;
                }
            }
            if byte_idx > 0 {
                msg = msg.word(word_idx, word);
            }
            let reply = ipc_call(vfs_cap, msg);
            if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
                let _ = ipc_call(
                    vfs_cap,
                    IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64),
                );
                return Err(());
            }
            let n = reply.words[1] as usize;
            offset += n;
        }
        let _ = ipc_call(
            vfs_cap,
            IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64),
        );
        Ok(())
    }

    fn mkdir(
        vfs_cap: CapabilityToken,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<(), ()> {
        let msg = path_msg(VfsMsg::MKDIR, path)
            .word(4, uid as u64)
            .word(5, gid as u64)
            .word(6, mode as u64);
        let reply = ipc_call(vfs_cap, msg);
        if reply.label == VfsMsg::REPLY && reply.words[0] == 0 {
            Ok(())
        } else {
            Err(())
        }
    }

    fn path_msg(label: u64, path: &str) -> IpcMsg {
        let bytes = path.as_bytes();
        let mut msg = IpcMsg::with_label(label);
        for word_idx in 0..4 {
            let start = word_idx * 8;
            let end = (start + 8).min(bytes.len());
            if start < bytes.len() {
                let mut word = 0u64;
                for (i, &b) in bytes[start..end].iter().enumerate() {
                    word |= (b as u64) << (i * 8);
                }
                msg = msg.word(word_idx, word);
            }
        }
        msg
    }

    fn find_max_uid(passwd_data: &[u8]) -> u32 {
        let mut max_uid = 0u32;
        for line in alloc::string::String::from_utf8_lossy(passwd_data).lines() {
            let parts: alloc::vec::Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                if let Some(uid) = parse_u32(parts[2]) {
                    if uid > max_uid {
                        max_uid = uid;
                    }
                }
            }
        }
        max_uid
    }

    fn lookup_user(username: &[u8]) -> Option<(u32, u32)> {
        let vfs_cap = nameserver_lookup("vfs")?;
        let passwd_data = read_file(vfs_cap, "/etc/passwd");
        for line in alloc::string::String::from_utf8_lossy(&passwd_data).lines() {
            let parts: alloc::vec::Vec<&str> = line.split(':').collect();
            if parts.len() >= 4 && parts[0].as_bytes() == username {
                let uid = parse_u32(parts[2])?;
                let gid = parse_u32(parts[3])?;
                return Some((uid, gid));
            }
        }
        None
    }

    fn parse_u32(s: &str) -> Option<u32> {
        let mut result = 0u32;
        for &b in s.as_bytes() {
            if b < b'0' || b > b'9' {
                return None;
            }
            result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
        }
        Some(result)
    }

    fn parse_mode(s: &str) -> Option<u16> {
        let mut result = 0u16;
        for &b in s.as_bytes() {
            if b < b'0' || b > b'7' {
                return None;
            }
            result = result * 8 + (b - b'0') as u16;
        }
        Some(result)
    }

    fn trim_newline(data: &[u8]) -> &[u8] {
        if data.ends_with(b"\n") {
            &data[..data.len() - 1]
        } else {
            data
        }
    }

    fn pack_output(data: &[u8]) -> IpcMsg {
        let len = data.len().min(IPC_OUTPUT_BYTES);
        let mut msg = IpcMsg::with_label(OUTPUT_LABEL)
            .word(0, len as u64)
            .word(1, 0);
        let mut word = 0u64;
        let mut byte_idx = 0;
        let mut word_idx = 2;
        for &b in data.iter().take(len) {
            word |= (b as u64) << (byte_idx * 8);
            byte_idx += 1;
            if byte_idx == 8 {
                msg = msg.word(word_idx, word);
                word = 0;
                byte_idx = 0;
                word_idx += 1;
                if word_idx >= 4 {
                    break;
                }
            }
        }
        if byte_idx > 0 && word_idx < 4 {
            msg = msg.word(word_idx, word);
        }
        msg
    }

    fn pack_long_output(data: &[u8], more_remaining: usize) -> IpcMsg {
        let len = data.len().min(IPC_OUTPUT_BYTES);
        let mut msg = IpcMsg::with_label(OUTPUT_LABEL)
            .word(0, len as u64)
            .word(1, more_remaining as u64);
        let mut word = 0u64;
        let mut byte_idx = 0;
        let mut word_idx = 2;
        for &b in data.iter().take(len) {
            word |= (b as u64) << (byte_idx * 8);
            byte_idx += 1;
            if byte_idx == 8 {
                msg = msg.word(word_idx, word);
                word = 0;
                byte_idx = 0;
                word_idx += 1;
                if word_idx >= 4 {
                    break;
                }
            }
        }
        if byte_idx > 0 && word_idx < 4 {
            msg = msg.word(word_idx, word);
        }
        msg
    }

    fn shell_name(shell_id: u64, buf: &mut [u8; 16]) -> &str {
        let prefix = b"sshl";
        buf[..prefix.len()].copy_from_slice(prefix);
        let mut pos = prefix.len();
        pos += fmt_u64_into(&mut buf[pos..], shell_id);
        core::str::from_utf8(&buf[..pos]).unwrap_or("sshl0")
    }

    // Retained for reference: the old shell-startup CPU/RAM banner. No longer
    // called — these stats now render in the TUI title bar (tty_server).
    #[allow(dead_code)]
    fn send_system_stats_header() {
        // System stats banner: CPU % and RAM % display
        let cpu_percent = 15; // Placeholder: needs scheduler accounting (Phase 5.12)
        let info = sysinfo();
        let ram_total = (info.total_ram_kb / 1024) as u32;
        let ram_used = (info.used_ram_kb / 1024) as u32;
        let ram_percent = (ram_used as u64 * 100) / (ram_total as u64).max(1);

        // ANSI colors
        let bold = "\x1B[1m";
        let cyan = "\x1B[36m";
        let green = "\x1B[32m";
        let yellow = "\x1B[33m";
        let reset = "\x1B[0m";

        // Color code CPU usage
        let cpu_color = if cpu_percent < 50 {
            green
        } else if cpu_percent < 80 {
            yellow
        } else {
            "\x1B[31m" // red
        };

        // Color code RAM usage
        let ram_color = if ram_percent < 50 {
            green
        } else if ram_percent < 80 {
            yellow
        } else {
            "\x1B[31m" // red
        };

        // Build header string
        let header = alloc::format!(
            "{}╔═══════════════════════════════════╗{}{}╝{}\n\
             {}║{}  CPU: {}{}%{} │ RAM: {}{}%{} ({}MB)  {}║{}\n\
             {}╚═══════════════════════════════════╝{}\n",
            cyan,
            reset,
            bold,
            reset,
            cyan,
            reset,
            cpu_color,
            cpu_percent,
            reset,
            ram_color,
            ram_percent as u32,
            reset,
            ram_used,
            reset,
            reset,
            cyan,
            reset
        );

        // Push header into long output buffer
        long_out_push_str(&header);
        long_out_push_byte(b'\n');
    }

    fn fmt_u64_into(buf: &mut [u8], val: u64) -> usize {
        if val == 0 {
            buf[0] = b'0';
            return 1;
        }
        let mut tmp = [0u8; 20];
        let mut n = 0usize;
        let mut v = val;
        while v > 0 && n < tmp.len() {
            tmp[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
        for i in 0..n {
            buf[i] = tmp[n - 1 - i];
        }
        n
    }

    static mut LONG_OUT_BUF: [u8; LONG_OUT_MAX] = [0; LONG_OUT_MAX];
    static mut LONG_OUT_LEN: usize = 0;
    static mut LONG_OUT_ACTIVE: bool = false;

    fn long_out_reset() {
        unsafe {
            LONG_OUT_LEN = 0;
            LONG_OUT_ACTIVE = false;
        }
    }

    fn long_out_replace(data: &[u8]) {
        unsafe {
            LONG_OUT_ACTIVE = true;
            LONG_OUT_LEN = 0;
            let to_copy = data.len().min(LONG_OUT_MAX);
            LONG_OUT_BUF[..to_copy].copy_from_slice(&data[..to_copy]);
            LONG_OUT_LEN = to_copy;
        }
    }

    fn long_out_push_str(s: &str) {
        unsafe {
            let bytes = s.as_bytes();
            let space = LONG_OUT_MAX - LONG_OUT_LEN;
            let to_copy = bytes.len().min(space);
            LONG_OUT_BUF[LONG_OUT_LEN..LONG_OUT_LEN + to_copy].copy_from_slice(&bytes[..to_copy]);
            LONG_OUT_LEN += to_copy;
        }
    }

    fn long_out_push_byte(b: u8) {
        unsafe {
            if LONG_OUT_LEN < LONG_OUT_MAX {
                LONG_OUT_BUF[LONG_OUT_LEN] = b;
                LONG_OUT_LEN += 1;
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn _start(shell_id: u64, uid: u64, gid: u64) -> ! {
        debug_log("[TTY]  Shell: sshl v0.1.0 running");

        let ep = endpoint_create();
        let mut name_buf = [0u8; 16];
        let name = shell_name(shell_id, &mut name_buf);
        nameserver_register(name, ep);
        if shell_id == 0 {
            nameserver_register("sshl", ep);
        }
        debug_log("[TTY]  sunshell registered as 'sshl'");

        let mut shell = Shell::new();
        // Load real user info from VFS by uid (GETPWUID returns uid, gid, AND username)
        // This is more robust than hardcoded uid→username mapping
        shell.load_user_by_uid(uid as u32);
        // Seed PATH/USER/HOME/SHELL now that the user identity is resolved
        shell.init_env();

        // Send welcome banner with system stats (clear screen first)
        unsafe {
            LONG_OUT_ACTIVE = true;
            LONG_OUT_LEN = 0;
        }

        // Write clear screen + home cursor to LONG_OUT_BUF
        unsafe {
            LONG_OUT_BUF[0] = b'\x1B';
            LONG_OUT_BUF[1] = b'[';
            LONG_OUT_BUF[2] = b'2';
            LONG_OUT_BUF[3] = b'J';
            LONG_OUT_BUF[4] = b'\x1B';
            LONG_OUT_BUF[5] = b'[';
            LONG_OUT_BUF[6] = b'H';
            LONG_OUT_LEN = 7;
        }

        // Simple, fast greeting. The CPU/RAM stats banner was removed from the
        // shell startup: every tab spawned its own shell, and each did a sysinfo
        // syscall on launch — extra latency/IPC churn per tab. Live CPU/RAM now
        // lives in the TUI title bar (rendered once by tty_server, see
        // build_titlebar in services/tty_server). Keep this greeting minimal so
        // a new tab paints instantly.
        long_out_push_str("\x1b[36m"); // cyan
        long_out_push_str("Welcome to SunlightOS\n");
        long_out_push_str("\x1b[0m"); // reset
        long_out_push_str("Type commands at the prompt below.\n");
        long_out_push_str("\n");

        let mut msg = ipc_reply_and_wait(ep, IpcMsg::with_label(0));
        loop {
            // Drain request from tty_server: send the next chunk of long output
            if msg.label == DRAIN_LABEL {
                let total = unsafe { LONG_OUT_LEN };
                let offset = msg.words[0] as usize * IPC_OUTPUT_BYTES;
                if offset >= total {
                    // Nothing more — return empty marker
                    msg = ipc_reply_and_wait(
                        ep,
                        IpcMsg::with_label(OUTPUT_LABEL).word(0, 0).word(1, 0),
                    );
                    continue;
                }
                let remaining = total - offset;
                let chunk_size = remaining.min(IPC_OUTPUT_BYTES);
                let mut tmp = [0u8; IPC_OUTPUT_BYTES];
                unsafe {
                    tmp[..chunk_size].copy_from_slice(&LONG_OUT_BUF[offset..offset + chunk_size]);
                }
                let more_remaining = remaining.saturating_sub(chunk_size);
                let chunk_reply = pack_long_output(&tmp[..chunk_size], more_remaining);
                msg = ipc_reply_and_wait(ep, chunk_reply);
                continue;
            }

            // Kbd event
            let reply = if msg.label == KBD_LABEL {
                let byte = msg.words[0] as u8;
                let is_enter = byte == b'\n' || byte == b'\r';
                let is_poll = byte == 0;
                // Snapshot the full command (with args) BEFORE handle_byte
                // resets line_len to 0 on Enter. This is what gets logged.
                let cmd_snap_len = if is_enter { shell.line_len } else { 0 };
                let mut cmd_snap = [0u8; MAX_LINE];
                if is_enter {
                    cmd_snap[..cmd_snap_len].copy_from_slice(&shell.line[..cmd_snap_len]);
                }
                let mut out = [0u8; MAX_OUT];
                let mut out_len = 0usize;
                if !is_poll {
                    long_out_reset();
                    let handled = shell.handle_byte(byte);
                    out = handled.0;
                    out_len = handled.1;
                }
                if out_len > 0 {
                    debug_log_cmd_output(&cmd_snap[..cmd_snap_len], &out[..out_len]);
                }
                if cmd_snap_len == 4 && &cmd_snap[..cmd_snap_len] == b"exit" {
                    IpcMsg::with_label(EXIT_LABEL)
                } else if unsafe { LONG_OUT_ACTIVE } {
                    let total = unsafe { LONG_OUT_LEN };
                    let chunk_size = total.min(IPC_OUTPUT_BYTES);
                    let mut tmp = [0u8; IPC_OUTPUT_BYTES];
                    unsafe {
                        tmp[..chunk_size].copy_from_slice(&LONG_OUT_BUF[..chunk_size]);
                    }
                    let more_remaining = total.saturating_sub(chunk_size);
                    pack_long_output(&tmp[..chunk_size], more_remaining)
                } else if out_len > IPC_OUTPUT_BYTES {
                    long_out_replace(&out[..out_len]);
                    let chunk_size = out_len.min(IPC_OUTPUT_BYTES);
                    let more_remaining = out_len.saturating_sub(chunk_size);
                    pack_long_output(&out[..chunk_size], more_remaining)
                } else {
                    pack_output(&out[..out_len])
                }
            } else {
                IpcMsg::with_label(0)
            };
            msg = ipc_reply_and_wait(ep, reply);
        }
    }

    fn debug_log_cmd_output(cmd: &[u8], output: &[u8]) {
        let mut buf = [0u8; 128];
        let prefix = b"[TTY]  cmd: ";
        let mut pos = prefix.len();
        buf[..pos].copy_from_slice(prefix);
        let clen = cmd.len().min(64);
        buf[pos..pos + clen].copy_from_slice(&cmd[..clen]);
        pos += clen;
        let arrow = b" -> ";
        buf[pos..pos + arrow.len()].copy_from_slice(arrow);
        pos += arrow.len();
        let olen = output.len().min(64);
        // Remove trailing newline for log
        let olen = if olen > 0 && output[olen - 1] == b'\n' {
            olen - 1
        } else {
            olen
        };
        buf[pos..pos + olen].copy_from_slice(&output[..olen]);
        pos += olen;
        if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
            debug_log(s);
        }
    }
}

#[cfg(feature = "sunlight")]
use sunlight::*;

#[cfg(feature = "sunlight")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
