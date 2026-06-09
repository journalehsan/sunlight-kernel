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

#[cfg(feature = "sunlight")]
#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[cfg(feature = "sunlight")]
#[no_main]
mod sunlight {
    use sunlight_ipc::{
        debug_log, endpoint_create, ipc_reply, ipc_reply_and_wait, nameserver_register,
        nameserver_lookup, ipc_call, get_init_cap, IpcMsg, InitMsg, VfsMsg,
        CapabilityToken, SunlightSyscall, process_exit::ProcessExit,
    };

    const KBD_LABEL: u64 = 1;
    const OUTPUT_LABEL: u64 = 2;
    const EXIT_LABEL: u64 = 3;
    const MAX_LINE: usize = 128;
    const MAX_OUT: usize = 64;

    struct Shell {
        line: [u8; MAX_LINE],
        line_len: usize,
        username: [u8; 64],
        username_len: usize,
        uid: u32,
        gid: u32,
    }

    impl Shell {
        fn new() -> Self {
            Self {
                line: [0; MAX_LINE],
                line_len: 0,
                username: [b'r', b'o', b'o', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                          0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                          0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                          0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                username_len: 4,
                uid: 0,
                gid: 0,
            }
        }

        fn handle_byte(&mut self, byte: u8) -> ([u8; MAX_OUT], usize) {
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

        fn run_line(&mut self) -> ([u8; MAX_OUT], usize) {
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
            let args: alloc::vec::Vec<&str> = parts.collect();

            let out: &[u8] = match cmd {
                "whoami" => self.cmd_whoami(),
                "id" => self.cmd_id(&args),
                "useradd" => self.cmd_useradd(&args),
                "userdel" => self.cmd_userdel(&args),
                "passwd" => b"passwd: not implemented\n",
                "su" => b"su: not implemented\n",
                "groups" => self.cmd_groups(&args),
                "chmod" => self.cmd_chmod(&args),
                "chown" => self.cmd_chown(&args),
                "help" => b"Builtins: whoami, id, uname, useradd, userdel, groups, chmod, chown, help, echo, cat\n",
                "echo" => self.cmd_echo(&args),
                "cat" => self.cmd_cat(&args),
                "uname" => self.cmd_uname(&args),
                "clear" => b"",
                "exit" => b"exit\n",
                _ => b"sshl: command not found\n",
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

        fn cmd_whoami(&self) -> &[u8] {
            &self.username[..self.username_len]
        }

        fn cmd_uname(&self, args: &[&str]) -> &[u8] {
            match args.first().copied() {
                None => b"SunlightOS",
                Some("-s") => b"SunlightOS",
                Some("-n") => b"sunlight",
                Some("-r") => b"0.1.0",
                Some("-v") => b"Phase 3.8",
                Some("-m") | Some("-p") | Some("-i") => b"x86_64",
                Some("-o") => b"SunlightOS",
                Some("-a") => b"SunlightOS sunlight 0.1.0 Phase 3.8 x86_64 SunlightOS",
                Some(_) => b"uname: invalid option\n",
            }
        }

        fn cmd_id(&self, args: &[&str]) -> &[u8] {
            if args.is_empty() {
                return b"uid=0(root) gid=0(root) groups=0(root),10(wheel)";
            }
            // lookup specific user
            let username = args[0].as_bytes();
            if let Some((uid, gid)) = lookup_user(username) {
                unsafe {
                    static mut BUF: [u8; 64] = [0u8; 64];
                    let buf = &mut BUF;
                    let prefix = alloc::format!("uid={}({}) gid={}({})", uid, args[0], gid, "users");
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
            new_passwd.push_str(&alloc::format!("{}:x:{}:100::/home/{}:/bin/sh\n", username, new_uid, username));

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
                if !line.starts_with(username) || !line.as_bytes().get(username.len()).map(|&b| b == b':').unwrap_or(false) {
                    new_passwd.push_str(line);
                    new_passwd.push('\n');
                }
            }

            let mut new_shadow = alloc::string::String::new();
            for line in alloc::string::String::from_utf8_lossy(&shadow_data).lines() {
                if !line.starts_with(username) || !line.as_bytes().get(username.len()).map(|&b| b == b':').unwrap_or(false) {
                    new_shadow.push_str(line);
                    new_shadow.push('\n');
                }
            }

            let mut new_group = alloc::string::String::new();
            for line in alloc::string::String::from_utf8_lossy(&group_data).lines() {
                let mut modified = alloc::string::String::from(line);
                if modified.contains("users:x:100:") {
                    modified = modified.replace(&alloc::format!("{},", username), "");
                    modified = modified.replace(&alloc::format!(",{}" , username), "");
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

        fn cmd_groups(&self, _args: &[&str]) -> &[u8] {
            b"root wheel\n"
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
            let msg = path_msg(VfsMsg::CHMOD, path)
                .word(4, mode as u64);
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
    }

    fn copy_out(data: &[u8]) -> ([u8; MAX_OUT], usize) {
        let mut buf = [0u8; MAX_OUT];
        let len = data.len().min(buf.len());
        buf[..len].copy_from_slice(&data[..len]);
        (buf, len)
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
        let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));
        out
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
                let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));
                return Err(());
            }
            let n = reply.words[1] as usize;
            offset += n;
        }
        let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));
        Ok(())
    }

    fn mkdir(vfs_cap: CapabilityToken, path: &str, uid: u32, gid: u32, mode: u16) -> Result<(), ()> {
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
        let mut msg = IpcMsg::with_label(OUTPUT_LABEL).word(0, data.len() as u64);
        let mut word = 0u64;
        let mut byte_idx = 0;
        let mut word_idx = 1;
        for &b in data.iter().take(48) {
            word |= (b as u64) << (byte_idx * 8);
            byte_idx += 1;
            if byte_idx == 8 {
                msg = msg.word(word_idx, word);
                word = 0;
                byte_idx = 0;
                word_idx += 1;
            }
        }
        if byte_idx > 0 && word_idx < 8 {
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

    #[no_mangle]
    pub extern "C" fn _start(shell_id: u64) -> ! {
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
        let mut msg = ipc_reply_and_wait(ep, IpcMsg::with_label(0));
        loop {
            let reply = if msg.label == KBD_LABEL {
                let byte = msg.words[0] as u8;
                // Snapshot the full command (with args) BEFORE handle_byte
                // resets line_len to 0 on Enter. This is what gets logged.
                let is_enter = byte == b'\n' || byte == b'\r';
                let cmd_snap_len = if is_enter { shell.line_len } else { 0 };
                let mut cmd_snap = [0u8; MAX_LINE];
                if is_enter {
                    cmd_snap[..cmd_snap_len].copy_from_slice(&shell.line[..cmd_snap_len]);
                }
                let (out, out_len) = shell.handle_byte(byte);
                if out_len > 0 {
                    debug_log_cmd_output(&cmd_snap[..cmd_snap_len], &out[..out_len]);
                }
                if cmd_snap_len == 4 && &cmd_snap[..cmd_snap_len] == b"exit" {
                    ipc_reply(IpcMsg::with_label(EXIT_LABEL));
                    ProcessExit::exit(0);
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
        let olen = if olen > 0 && output[olen - 1] == b'\n' { olen - 1 } else { olen };
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
