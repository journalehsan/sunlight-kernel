// Guest workloads for the existing serial-output QEMU gate. Only compiled
// with native-tests; normal parser, expansion and native executor are used.
use super::*;
use sunlight_libc as libc;
use alloc::{format, string::String, vec::Vec};

fn raw_args(raw: &[*const u8]) -> Vec<String> {
    raw.iter().map(|ptr| unsafe {
        let len = libc::crt0::cstr_len(*ptr, libc::MAX_PATH);
        String::from(core::str::from_utf8(core::slice::from_raw_parts(*ptr, len)).unwrap())
    }).collect()
}

pub(super) fn probe(raw: &[*const u8]) -> ! {
    let args = raw_args(raw);
    let mut buf = [b'x'; 4096];
    let number = args.get(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    let result = match args.first().map(String::as_str) {
        Some("emit") => {
            let mut left = number;
            let mut error = None;
            while left > 0 {
                let n = left.min(buf.len());
                if let Err(e) = libc::write_all(libc::STDOUT, &buf[..n]) { error = Some(e); break; }
                left -= n;
            }
            if let Some(error) = error {
                debug_log(&format!("[SSHL-TEST] producer error={error:?}"));
                libc::exit(if error == libc::Errno::BrokenPipe { 32 } else { 1 });
            }
            Ok(())
        }
        Some("copy" | "count" | "take") => {
            let mut total = 0;
            loop {
                let requested = if args[0] == "take" { number.saturating_sub(total).min(buf.len()) } else { buf.len() };
                if requested == 0 { break; }
                match libc::read(libc::STDIN, &mut buf[..requested]) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        if args[0] != "count" && libc::write_all(libc::STDOUT, &buf[..n]).is_err() { libc::exit(1); }
                    }
                    Err(_) => libc::exit(1),
                }
            }
            if args[0] == "count" { libc::write_all(libc::STDOUT, format!("{total}\n").as_bytes()) } else { Ok(()) }
        }
        Some("args") => {
            for arg in &args[1..] { if libc::write_all(libc::STDOUT, format!("<{arg}>\n").as_bytes()).is_err() { libc::exit(1); } }
            Ok(())
        }
        Some("stdio") => libc::write_all(libc::STDOUT, b"out\n").and_then(|_| libc::write_all(libc::STDERR, b"err\n")),
        Some("exit") => libc::exit(number as u64),
        Some("pause") => {
            let until = monotonic_millis() + number as u64;
            while monotonic_millis() < until { process_yield(); }
            Ok(())
        }
        _ => libc::exit(2),
    };
    libc::exit(if result.is_ok() { 0 } else { 1 });
}

fn fail(message: &str) -> ! {
    debug_log(&format!("[SSHL-TEST] FAIL {message}"));
    libc::exit(1)
}
fn file(path: &str) -> Vec<u8> {
    let fd = libc::open(path.as_bytes()).unwrap_or_else(|_| fail(path));
    let mut result = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = libc::read(fd, &mut buf).unwrap_or_else(|_| fail("file read"));
        if n == 0 { break; }
        result.extend_from_slice(&buf[..n]);
        if result.len() > 2 * 1024 * 1024 { fail("unbounded test output"); }
    }
    libc::close(fd).unwrap();
    result
}
fn expect(path: &str, bytes: &[u8]) {
    let actual = file(path);
    if actual != bytes { fail(&format!("{path}: expected {:?}, got {:?}", bytes, actual)); }
}

// Same request/reply protocol used by tty_server, in an isolated sibling
// tab so the root tab continues to own the test driver's foreground job.
struct TtyRunner { cap: CapabilityToken, pid: u64 }
impl TtyRunner {
    fn new() -> Self {
        let pid = libc::spawn(b"/bin/sshl6", &[b"sshl6"], None).unwrap_or_else(|_| fail("TTY shell spawn"));
        let start = monotonic_millis();
        let cap = loop {
            if let Some(cap) = nameserver_lookup("sshl6") { break cap; }
            if monotonic_millis() - start > 5000 { fail("TTY shell registration"); }
            process_yield();
        };
        let runner = Self { cap, pid };
        runner.drain();
        runner
    }
    fn call(&self, msg: IpcMsg) -> IpcMsg {
        ipc_call_timeout(self.cap, msg, 15000).unwrap_or_else(|_| fail("TTY shell IPC timeout"))
    }
    fn drain(&self) {
        for _ in 0..2048 {
            let reply = self.call(IpcMsg::with_label(DRAIN_LABEL));
            if reply.word_count == 0 || reply.words[0] == 0 { return; }
        }
        fail("TTY output drain did not finish");
    }
    fn command(&self, line: &str) -> u64 {
        for byte in line.bytes() { self.call(IpcMsg::with_label(KBD_LABEL).word(0, byte as u64)); }
        let reply = self.call(IpcMsg::with_label(KBD_LABEL).word(0, b'\n' as u64));
        if reply.label == FG_STARTED_LABEL {
            let pid = reply.words[0];
            let start = monotonic_millis();
            let mut bytes = 0;
            let mut buf = [0; 4096];
            while sunlight_ipc::process_is_alive(pid) {
                bytes += tty_stdout_pull(6, &mut buf);
                if monotonic_millis() - start > 15000 { fail("TTY foreground timeout"); }
                process_yield();
            }
            loop { let n = tty_stdout_pull(6, &mut buf); bytes += n; if n == 0 { break; } }
            let done = self.call(IpcMsg::with_label(FG_DONE_LABEL));
            if done.label != FG_DONE_LABEL { fail("TTY foreground completion protocol"); }
            if line == "sshl --probe emit 65536" && bytes != 65536 { fail("TTY live output lost"); }
            done.words[0]
        } else { self.drain(); 0 }
    }
    fn run(&self, line: &str) -> u8 {
        self.command(line);
        self.command("echo $? > /tmp/sshl-status");
        core::str::from_utf8(&file("/tmp/sshl-status")).unwrap().trim().parse().unwrap()
    }
}
impl Drop for TtyRunner {
    fn drop(&mut self) { let _ = libc::kill(self.pid, 9); let _ = libc::waitpid(self.pid); }
}

struct PtyRunner {
    service: CapabilityToken, master: CapabilityToken, control: CapabilityToken,
    id: u64, generation: u64, pid: u64, buffer: sunlight_ipc::pty::PtyBuffer,
}
impl PtyRunner {
    fn new() -> Self {
        let service = nameserver_lookup("pty").unwrap_or_else(|| fail("pty lookup"));
        let reply = ipc_call_timeout(service, IpcMsg::with_label(PtyMsg::CREATE).word(0, 0), 2000).unwrap();
        if reply.label != PtyMsg::REPLY || reply.cap_count < 2 { fail("pty create"); }
        let id = reply.words[0]; let generation = reply.words[1];
        let master = reply.caps[0]; let control = reply.caps[1];
        let reply = ipc_call_timeout(service, IpcMsg::with_label(PtyMsg::ATTACH_SLAVE)
            .word(0, id).word(1, generation).with_cap(0, control), 2000).unwrap();
        if reply.label != PtyMsg::REPLY || reply.cap_count < 1 { fail("pty attach"); }
        let args = [String::from("sshl7"), format!("--pty-session={id}"), format!("--pty-generation={generation}"),
            format!("--pty-service-cap={}", service.0), format!("--pty-slave-cap={}", reply.caps[0].0)];
        let argv: Vec<&[u8]> = args.iter().map(|s| s.as_bytes()).collect();
        let pid = libc::spawn(b"/bin/sshl7", &argv, None).unwrap_or_else(|_| fail("pty shell spawn"));
        let runner = Self { service, master, control, id, generation, pid, buffer: sunlight_ipc::pty::PtyBuffer::new().unwrap() };
        runner.prompt();
        runner
    }
    fn prompt(&self) -> Vec<u8> {
        let start = monotonic_millis();
        let mut output = Vec::new();
        loop {
            let request = self.buffer.request(PtyMsg::READ_MASTER_BULK, self.id, self.generation, self.master, PtyMsg::BULK_BYTES);
            let reply = ipc_call_timeout(self.service, request, 2000).unwrap_or_else(|_| fail("pty read timeout"));
            if reply.label == PtyMsg::REPLY {
                let mut buf = [0u8; PtyMsg::BULK_BYTES];
                let n = self.buffer.copy_to(&mut buf, reply.words[2] as usize);
                output.extend_from_slice(&buf[..n]);
                if output.windows(b"\x1b]9001;prompt;".len()).any(|s| s == b"\x1b]9001;prompt;") && output.last() == Some(&7) { return output; }
            }
            if output.len() > 2 * 1024 * 1024 || monotonic_millis() - start > 15000 { fail(&format!("pty prompt timeout, bytes={}", output.len())); }
            process_yield();
        }
    }
    fn send(&self, line: &str) {
        for chunk in line.as_bytes().chunks(8) {
            let mut data = [0u8; 8]; data[..chunk.len()].copy_from_slice(chunk);
            let reply = ipc_call_timeout(self.service, IpcMsg::with_label(PtyMsg::WRITE_MASTER)
                .word(0, self.id).word(1, self.generation).word(2, chunk.len() as u64)
                .word(3, u64::from_le_bytes(data)).with_cap(0, self.master), 2000).unwrap();
            if reply.label != PtyMsg::REPLY || reply.words[2] != chunk.len() as u64 { fail("pty write acceptance"); }
        }
    }
    fn run(&self, line: &str) -> u8 {
        self.send(&format!("{line}\n")); self.prompt();
        self.send("echo $? > /tmp/sshl-status\n"); self.prompt();
        core::str::from_utf8(&file("/tmp/sshl-status")).unwrap().trim().parse().unwrap()
    }
}
impl Drop for PtyRunner {
    fn drop(&mut self) {
        let _ = libc::kill(self.pid, 9);
        let _ = libc::waitpid(self.pid);
        let _ = ipc_call_timeout(self.service, IpcMsg::with_label(PtyMsg::CLOSE_SESSION)
            .word(0, self.id).word(1, self.generation).with_cap(0, self.control), 2000);
    }
}

pub(super) fn suite() -> ! {
    debug_log("[SSHL-TEST] native executor suite starting");
    {
        let tty = TtyRunner::new();
        exercise("tty-ipc", &mut |line| tty.run(line));
        if tty.run("sshl --probe emit 65536") != 0 { fail("TTY live output status"); }
        debug_log("[SSHL-TEST] TTY live output 65536 PASS");
    }
    {
        let pty = PtyRunner::new();
        exercise("pty", &mut |line| pty.run(line));
        // Exercise draining a live child before it exits: larger than TTY ring.
        pty.send("sshl --probe emit 65536\n");
        let output = pty.prompt();
        if output.iter().filter(|b| **b == b'x').count() != 65536 { fail("pty streamed output lost"); }
        debug_log("[SSHL-TEST] PTY live output 65536 PASS");
    }
    debug_log("[SSHL-TEST] native gate PASS");
    libc::exit(0)
}

fn exercise(path: &str, run: &mut dyn FnMut(&str) -> u8) {
    let mut check = |line: &str, status: u8| {
        debug_log(&format!("[SSHL-TEST] {path} command: {line}"));
        let actual = run(line);
        if actual != status { fail(&format!("{path} {line}: expected status {status}, got {actual}")); }
    };
    check("sshl --probe emit 10 | sshl --probe count > /tmp/sshl-count", 0); expect("/tmp/sshl-count", b"10\n");
    check("sshl --probe emit 1048576 | sshl --probe copy | sshl --probe count > /tmp/sshl-count", 0); expect("/tmp/sshl-count", b"1048576\n");
    check("sshl --probe stdio > /tmp/sshl-out 2> /tmp/sshl-err", 0); expect("/tmp/sshl-out", b"out\n"); expect("/tmp/sshl-err", b"err\n");
    check("sshl --probe copy < /tmp/sshl-out >> /tmp/sshl-err", 0); expect("/tmp/sshl-err", b"err\nout\n");
    check("sshl --probe stdio > /tmp/sshl-out 2>&1", 0); expect("/tmp/sshl-out", b"out\nerr\n");
    check("sshl --probe stdio 2>&1 > /tmp/sshl-out | sshl --probe copy > /tmp/sshl-err", 0); expect("/tmp/sshl-out", b"out\n"); expect("/tmp/sshl-err", b"err\n");
    check("cd /tmp | sshl --probe count > /tmp/sshl-count", 0); expect("/tmp/sshl-count", b"0\n");
    check("pwd > /tmp/sshl-out", 0); expect("/tmp/sshl-out", b"/\n");
    check("export TEST=' one two '", 0);
    check("sshl --probe args '$HOME' \"$HOME\" $HOME \"hello world\" \"\" \\$HOME > /tmp/sshl-out", 0);
    expect("/tmp/sshl-out", b"<$HOME>\n</root>\n</root>\n<hello world>\n<>\n<$HOME>\n");
    check("sshl --probe args $TEST \"$TEST\" > /tmp/sshl-out", 0); expect("/tmp/sshl-out", b"<one>\n<two>\n< one two >\n");
    check("sshl --probe exit 9 | sshl --probe exit 3", 3);
    check("sshl --probe args $? > /tmp/sshl-out", 0); expect("/tmp/sshl-out", b"<3>\n");
    check("definitely-missing-sshl", 127);
    check("sshl --probe args $? > /tmp/sshl-out", 0); expect("/tmp/sshl-out", b"<127>\n");
    check("/tmp/sshl-out", 126);
    check("/tmp", 126);
    check("sshl --probe count < /absent/sshl-file", 1);
    check("pwd > /absent/sshl-file", 1);
    check("cd /absent 2> /tmp/sshl-err", 1); expect("/tmp/sshl-err", b"cd: no such directory\n");
    check("export 2INVALID=x", 1);
    check("unset", 1);
    check("cd /absent | sshl --probe count > /tmp/sshl-count", 0); expect("/tmp/sshl-count", b"0\n");
    check("pwd > /tmp/sshl-out < /absent/file", 1);
    check("pwd > /tmp/sshl-out", 0); expect("/tmp/sshl-out", b"/\n");
    check("sshl --probe emit 1048576 | sshl --probe take 1 > /tmp/sshl-out", 0); expect("/tmp/sshl-out", b"x");
    check("sshl --probe emit 1048576 | definitely-missing-sshl | sshl --probe count", 127);
    check("sshl --probe emit 1048576 | /tmp/sshl-out | sshl --probe count", 126);
    check("sshl --probe emit 1048576 | sshl --probe copy > /absent/file", 1);
    // Warm up persistent command storage, then compare the reclaiming heap.
    check("pwd > /tmp/sshl-out", 0);
    let before = libc::alloc::heap_stats();
    let fd_before = libc::dup(libc::STDOUT).unwrap(); libc::close(fd_before).unwrap();
    let start = monotonic_millis();
    for _ in 0..100 { check("sshl --probe emit 16 | sshl --probe copy | sshl --probe count > /tmp/sshl-count", 0); }
    let after = libc::alloc::heap_stats();
    let fd_after = libc::dup(libc::STDOUT).unwrap(); libc::close(fd_after).unwrap();
    if fd_before != fd_after || after.requested_user_bytes > before.requested_user_bytes + 1024 || after.failed_allocation_count != before.failed_allocation_count { fail("descriptor or shell heap growth"); }
    debug_log(&format!("[SSHL-TEST] {path} repeat=100 elapsed_ms={} heap_before={} heap_after={} fd_before={} fd_after={} PASS", monotonic_millis()-start, before.requested_user_bytes, after.requested_user_bytes, fd_before.0, fd_after.0));
    let start = monotonic_millis(); check("sshl --probe pause 200", 0);
    debug_log(&format!("[SSHL-TEST] {path} wait elapsed_ms={} PASS", monotonic_millis()-start));
    debug_log(&format!("[SSHL-TEST] {path} coverage PASS"));
}
