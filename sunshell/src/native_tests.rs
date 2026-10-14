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
            if number == 65536 {
                debug_log(&format!(
                    "[SSHL-LIVE] producer pid={} begin",
                    libc::getpid()
                ));
            }
            let mut left = number;
            let mut error = None;
            while left > 0 {
                let n = left.min(buf.len());
                if let Err(e) = libc::write_all(libc::STDOUT, &buf[..n]) {
                    error = Some(e);
                    break;
                }
                if number == 65536 && left == number {
                    debug_log("[SSHL-LIVE] producer first write complete");
                }
                left -= n;
            }
            if number == 65536 {
                debug_log("[SSHL-LIVE] producer writes complete");
            }
            if let Some(error) = error {
                debug_log(&format!("[SSHL-TEST] producer error={error:?}"));
                libc::exit(if error == libc::Errno::BrokenPipe { 32 } else { 1 });
            }
            Ok(())
        }
        Some("pattern" | "tag" | "nbpattern" | "pidpattern") => {
            if args[0] == "pidpattern" {
                let fd = libc::open_with_flags(b"/tmp/sshl-pty-close-pid", libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC).unwrap();
                libc::write_all(fd, format!("{}\n", libc::getpid()).as_bytes()).unwrap();
                libc::close(fd).unwrap();
            }
            if args[0] == "nbpattern" {
                // Native fcntl(F_SETFL, O_NONBLOCK) must keep EAGAIN semantics.
                let ret = unsafe { libc::sys::syscall3(49, 1, 4, 0x800) };
                libc::sys::check(ret).unwrap();
            }
            let chunk = args.get(2).and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(4096).clamp(1, buf.len());
            let mut total = 0;
            let mut partial = 0;
            let mut again = 0;
            while total < number {
                let n = chunk.min(number - total);
                for (i, byte) in buf[..n].iter_mut().enumerate() {
                    *byte = if args[0] == "tag" {
                        args.get(3).and_then(|s| s.as_bytes().first()).copied().unwrap_or(b'Z')
                    } else { b'A' + ((total + i) % 23) as u8 };
                }
                match libc::write(libc::STDOUT, &buf[..n]) {
                    Ok(0) => libc::exit(1),
                    Ok(written) => {
                        partial += usize::from(written < n);
                        total += written;
                    }
                    Err(libc::Errno::Again) => { again += 1; process_yield(); }
                    Err(error) => libc::exit(if error == libc::Errno::BrokenPipe { 32 } else { 1 }),
                }
            }
            debug_log(&format!("[SSHL-PTY] producer pid={} bytes={total} partial={partial} eagain={again}", libc::getpid()));
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
        Some("stdio") => libc::write_all(libc::STDOUT, b"out\n")
            .and_then(|_| libc::write_all(libc::STDERR, b"err\n")),
        Some("env") => {
            let value = args
                .get(1)
                .and_then(|key| libc::env::getenv(key.as_bytes()));
            libc::write_all(
                libc::STDOUT,
                format!("<{}>\n", value.unwrap_or("unset")).as_bytes(),
            )
        }
        Some("env-length") => {
            let length = args
                .get(1)
                .and_then(|key| libc::env::getenv(key.as_bytes()))
                .map(str::len);
            libc::write_all(
                libc::STDOUT,
                format!("{}\n", length.unwrap_or(0)).as_bytes(),
            )
        }
        Some("identity") => libc::write_all(
            libc::STDOUT,
            format!(
                "{}:{}:{}\n",
                libc::getuid(),
                libc::getgid(),
                nameserver_lookup("sshl6").is_some()
            )
            .as_bytes(),
        ),
        Some("exit") => libc::exit(number as u64),
        Some("pause") => {
            park_ms(number as u64);
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
            park_ms(100);
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
        let status = core::str::from_utf8(&file("/tmp/sshl-status")).unwrap().trim().parse().unwrap();
        // The status-query echo itself succeeds. Restore the observed status
        // so the next workload can test $? from the original command.
        self.command(&format!("sshl --probe exit {status}"));
        status
    }
}
impl Drop for TtyRunner {
    fn drop(&mut self) { let _ = libc::kill(self.pid, 9); let _ = libc::waitpid(self.pid); }
}

struct PtyRunner {
    service: CapabilityToken, master: CapabilityToken, control: CapabilityToken,
    id: u64, generation: u64, pid: u64, buffer: sunlight_ipc::pty::PtyBuffer,
    session_closed: bool,
}
impl PtyRunner {
    fn new() -> Self {
        Self::new_for_tab(7)
    }
    fn new_for_tab(tab: u8) -> Self {
        let service = nameserver_lookup("pty").unwrap_or_else(|| fail("pty lookup"));
        let reply = ipc_call_timeout(service, IpcMsg::with_label(PtyMsg::CREATE).word(0, 0), 2000).unwrap();
        if reply.label != PtyMsg::REPLY || reply.cap_count < 2 { fail("pty create"); }
        let id = reply.words[0]; let generation = reply.words[1];
        let master = reply.caps[0]; let control = reply.caps[1];
        let reply = ipc_call_timeout(service, IpcMsg::with_label(PtyMsg::ATTACH_SLAVE)
            .word(0, id).word(1, generation).with_cap(0, control), 2000).unwrap();
        if reply.label != PtyMsg::REPLY || reply.cap_count < 1 { fail("pty attach"); }
        let args = [format!("sshl{tab}"), format!("--pty-session={id}"), format!("--pty-generation={generation}"),
            format!("--pty-service-cap={}", service.0), format!("--pty-slave-cap={}", reply.caps[0].0)];
        let argv: Vec<&[u8]> = args.iter().map(|s| s.as_bytes()).collect();
        let pid = libc::spawn(format!("/bin/sshl{tab}").as_bytes(), &argv, None).unwrap_or_else(|_| fail("pty shell spawn"));
        let runner = Self { service, master, control, id, generation, pid, buffer: sunlight_ipc::pty::PtyBuffer::new().unwrap(), session_closed: false };
        runner.prompt();
        runner
    }
    fn prompt(&self) -> Vec<u8> {
        let start = monotonic_millis();
        let mut output = Vec::new();
        loop {
            let request = self.buffer.request(PtyMsg::READ_MASTER_BULK, self.id, self.generation, self.master, PtyMsg::BULK_BYTES);
            let reply = ipc_call_timeout(self.service, request, 2000).unwrap_or_else(|error| {
                trace_processes();
                fail(&format!(
                    "pty read IPC {error:?} session={} generation={} bytes={} elapsed_ms={}",
                    self.id, self.generation, output.len(), monotonic_millis() - start
                ));
            });
            if reply.label == PtyMsg::REPLY {
                let mut buf = [0u8; PtyMsg::BULK_BYTES];
                let n = self.buffer.copy_to(&mut buf, reply.words[2] as usize);
                output.extend_from_slice(&buf[..n]);
                if output.windows(b"\x1b]9001;prompt;".len()).any(|s| s == b"\x1b]9001;prompt;") && output.last() == Some(&7) { return output; }
            }
            if output.len() > 2 * 1024 * 1024 || monotonic_millis() - start > 15000 {
                trace_processes();
                fail(&format!("pty prompt timeout, bytes={}", output.len()));
            }
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
        let status = core::str::from_utf8(&file("/tmp/sshl-status")).unwrap().trim().parse().unwrap();
        self.send(&format!("sshl --probe exit {status}\n")); self.prompt();
        status
    }

    // Incrementally validate content with one shared page and bounded OSC
    // metadata. Never retain the stream, including the 1 MiB cases.
    fn stream(&self, expected: usize, read_size: usize, slow: bool) {
        self.stream_content(expected, read_size, slow, None);
    }

    fn stream_content(&self, expected: usize, read_size: usize, slow: bool, tag: Option<u8>) {
        let start = monotonic_millis();
        let mut total = 0;
        let mut osc = Vec::with_capacity(192);
        let mut in_osc = false;
        let mut started = false;
        let mut done = false;
        let mut reads = 0;
        let mut empty = 0;
        let mut sleep_ms = 0;
        let mut sleep_count = 0;
        let mut ipc_ms = 0;
        let mut ipc_max_ms = 0;
        loop {
            let request = self.buffer.request(
                PtyMsg::READ_MASTER_BULK,
                self.id,
                self.generation,
                self.master,
                read_size,
            );
            let call_start = monotonic_millis();
            let reply = ipc_call_timeout(self.service, request, 2000)
                .unwrap_or_else(|e| {
                    trace_processes();
                    fail(&format!("stream IPC {e:?} bytes={total}"))
                });
            let elapsed = monotonic_millis() - call_start;
            ipc_ms += elapsed;
            ipc_max_ms = ipc_max_ms.max(elapsed);
            if reply.label == PtyMsg::REPLY {
                let n = reply.words[2] as usize;
                if n > read_size {
                    fail("stream read count");
                }
                let mut buf = [0u8; PtyMsg::BULK_BYTES];
                if self.buffer.copy_to(&mut buf, n) != n {
                    fail("stream copy count");
                }
                reads += 1;
                for &byte in &buf[..n] {
                    if byte == 27 {
                        in_osc = true;
                        osc.clear();
                    }
                    if in_osc {
                        if osc.len() >= 192 {
                            fail("stream metadata bound");
                        }
                        osc.push(byte);
                        if byte == 7 {
                            in_osc = false;
                            if osc.starts_with(b"\x1b]9001;app_start;") {
                                started = true;
                            }
                            if osc.starts_with(b"\x1b]9001;app_done") {
                                if !started || done || total != expected {
                                    fail(&format!(
                                        "stream completion bytes={total} expected={expected}"
                                    ));
                                }
                                done = true;
                            }
                            if osc.starts_with(b"\x1b]9001;prompt;") {
                                if !done {
                                    fail("stream premature prompt");
                                }
                                debug_log(&format!("[SSHL-PTY] stream bytes={total} read_size={read_size} slow={slow} reads={reads} empty={empty} elapsed_ms={} sleep_ms={sleep_ms} sleeps={sleep_count} ipc_ms={ipc_ms} ipc_max_ms={ipc_max_ms} PASS", monotonic_millis()-start));
                                return;
                            }
                        }
                    } else if started && !done {
                        if total >= expected || byte != tag.unwrap_or(b'A' + (total % 23) as u8) {
                            fail(&format!("stream integrity offset={total} byte={byte}"));
                        }
                        total += 1;
                    }
                }
                if slow && reads % 8 == 0 {
                    let sleep_start = monotonic_millis();
                    park_ms(10);
                    sleep_ms += monotonic_millis() - sleep_start;
                    sleep_count += 1;
                }
            } else if reply.label == PtyMsg::ERROR && reply.words[0] == PtyMsg::ERR_WOULD_BLOCK {
                empty += 1;
            } else {
                fail("stream semantic error");
            }
            if monotonic_millis() - start > 15000 {
                trace_processes();
                fail(&format!(
                    "stream deadline bytes={total} expected={expected} reads={reads} empty={empty} sleep_ms={sleep_ms} sleeps={sleep_count} ipc_ms={ipc_ms} ipc_max_ms={ipc_max_ms}"
                ));
            }
            process_yield();
        }
    }
}

fn park_ms(ms: u64) {
    let word = 0u32;
    let deadline = (monotonic_millis() + ms) * 1_000_000;
    // Private futex with no wake source: timer parking, not a spin delay.
    // Native ABI 155 is the same private-address-space wait used by native std.
    unsafe {
        libc::sys::syscall4(155, &word as *const u32 as u64, 0, 0, deadline);
    }
}

pub(super) fn pty_liveness_suite() -> ! {
    debug_log("[SSHL-PTY] native suite starting");
    let pty = PtyRunner::new();
    pty.send("sshl --probe emit 65536\n");
    let output = pty.prompt();
    if output.iter().filter(|b| **b == b'x').count() != 65536 {
        fail("PTY 64 KiB reproduction count");
    }
    debug_log("[SSHL-PTY] original 65536 PASS");
    let _ = libc::unlink(b"/tmp/sshl-pty-close-pid");
    pty.send("sshl --probe pidpattern 65536 997\n");
    let started = monotonic_millis();
    while libc::stat(b"/tmp/sshl-pty-close-pid").is_err() {
        if monotonic_millis() - started > 2000 {
            fail("delayed TTY producer startup");
        }
        park_ms(10);
    }
    park_ms(30);
    let child: u64 = core::str::from_utf8(&file("/tmp/sshl-pty-close-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // A nonfatal signal interrupts the parked write. The retry must recheck
    // space and park again, without retaining a stale wait subscription.
    libc::kill(child, 17).unwrap();
    park_ms(30);
    trace_processes();
    pty.stream(65536, 257, true);
    debug_log("[SSHL-PTY] blocking TTY delayed drain PASS");
    pty.send("sshl --probe nbpattern 65536 997\n");
    pty.stream(65536, 257, true);
    debug_log("[SSHL-PTY] nonblocking TTY partial EAGAIN PASS");
    let other = PtyRunner::new_for_tab(8);
    pty.send("sshl --probe pattern 262144 997\n");
    other.send("sshl --probe tag 262144 4096 Z\n");
    // While the first session is consumed, the second must remain bounded
    // and cannot prevent the shared broker from servicing the first.
    pty.stream(262144, 257, true);
    other.stream_content(262144, 4096, false, Some(b'Z'));
    trace_processes();
    drop(other);
    debug_log("[SSHL-PTY] concurrent sessions delayed consumer PASS");
    foreground_closure(false);
    foreground_closure(true);
    pipe_liveness();
    broker_closure();
    for run in 0..2 {
        for size in [1024, 4096, 16384, 65536, 262144, 1048576] {
            debug_log(&format!("[SSHL-PTY] begin run={run} bytes={size}"));
            pty.send(&format!(
                "sshl --probe pattern {size} {}\n",
                if run == 0 { 4096 } else { 997 }
            ));
            pty.stream(size, if run == 0 { 4096 } else { 257 }, run == 1);
        }
    }
    debug_log("[SSHL-PTY] sizes repeat=2 integrity PASS");
    drop(pty);
    debug_log("[SSHL-PTY] native gate PASS");
    libc::exit(0)
}

fn foreground_closure(close_session: bool) {
    let _ = libc::unlink(b"/tmp/sshl-pty-close-pid");
    let mut pty = PtyRunner::new();
    pty.send("sshl --probe pidpattern 1048576 4096\n");
    let start = monotonic_millis();
    let child = loop {
        if libc::stat(b"/tmp/sshl-pty-close-pid").is_ok() {
            if let Ok(pid) = core::str::from_utf8(&file("/tmp/sshl-pty-close-pid"))
                .unwrap()
                .trim()
                .parse::<u64>()
            {
                break pid;
            }
        }
        if monotonic_millis() - start > 2000 {
            fail("close producer startup deadline");
        }
        park_ms(10);
    };
    park_ms(30);
    trace_processes();
    if !sunlight_ipc::process_is_alive(child) {
        fail("close producer exited before master close");
    }
    let reply = ipc_call_timeout(
        pty.service,
        IpcMsg::with_label(if close_session {
            PtyMsg::CLOSE_SESSION
        } else {
            PtyMsg::CLOSE_MASTER
        })
        .word(0, pty.id)
        .word(1, pty.generation)
        .with_cap(
            0,
            if close_session {
                pty.control
            } else {
                pty.master
            },
        ),
        2000,
    )
    .unwrap_or_else(|error| {
        trace_processes();
        fail(&format!("foreground close IPC error={error:?}"));
    });
    if reply.label != PtyMsg::REPLY {
        fail("foreground master close");
    }
    pty.session_closed = close_session;
    let start = monotonic_millis();
    while sunlight_ipc::process_is_alive(child) || sunlight_ipc::process_is_alive(pty.pid) {
        if monotonic_millis() - start > 2000 {
            trace_processes();
            fail("foreground hangup deadline");
        }
        park_ms(10);
    }
    if libc::waitpid(pty.pid).unwrap() != 129 {
        fail("PTY shell hangup exit status");
    }
    drop(pty);
    // The closed producer's fixed TTY ring is separate from the broker ring.
    // Drain its retained bytes before reusing this tab in the next scenario.
    let mut stale = [0u8; 4096];
    for _ in 0..2 {
        tty_stdout_pull(7, &mut stale);
    }
    if tty_stdout_pull(7, &mut stale) != 0 {
        fail("closed TTY drain bound");
    }
    debug_log(if close_session {
        "[SSHL-PTY] foreground session closure wakes and exits PASS"
    } else {
        "[SSHL-PTY] foreground master closure wakes and exits PASS"
    });
}

fn pipe_liveness() {
    // A child that cannot exit until we drain its pipe makes a parked
    // try_waitpid deterministically wrong, independently of PTY polling.
    let (read, write) = libc::pipe().unwrap();
    let child = libc::spawn_with_stdio(
        b"/bin/sshl",
        &[b"sshl", b"--probe", b"pattern", b"65536", b"997"],
        None,
        Some(write),
        None,
    )
    .unwrap();
    libc::close(write).unwrap();
    if libc::try_waitpid(child).unwrap().is_some() || sunlight_ipc::try_waitpid(child).is_some() {
        fail("nonblocking child probe exited early");
    }
    park_ms(20);
    trace_processes();
    let start = monotonic_millis();
    let mut buf = [0u8; 257];
    let mut total = 0;
    loop {
        let n = libc::read(read, &mut buf).unwrap();
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if b != b'A' + (total % 23) as u8 {
                fail("pipe pattern integrity");
            }
            total += 1;
        }
        if monotonic_millis() - start > 15000 {
            fail("pipe drain deadline");
        }
    }
    libc::close(read).unwrap();
    if total != 65536 || libc::waitpid(child).unwrap() != 0 {
        fail("pipe EOF/count/status");
    }
    debug_log("[SSHL-PTY] nonblocking wait pipe integrity EOF PASS");

    let (read, write) = libc::pipe().unwrap();
    let mut children = Vec::new();
    for tag in [b"A".as_slice(), b"B".as_slice()] {
        children.push(
            libc::spawn_with_stdio(
                b"/bin/sshl",
                &[b"sshl", b"--probe", b"tag", b"65536", b"997", tag],
                None,
                Some(write),
                None,
            )
            .unwrap(),
        );
    }
    libc::close(write).unwrap();
    let mut counts = [0usize; 2];
    let mut reads = 0;
    let start = monotonic_millis();
    loop {
        let n = libc::read(read, &mut buf).unwrap();
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            match b {
                b'A' => counts[0] += 1,
                b'B' => counts[1] += 1,
                _ => fail("concurrent pipe content"),
            }
        }
        reads += 1;
        if reads % 32 == 0 {
            park_ms(10);
        }
        if monotonic_millis() - start > 15000 {
            fail("concurrent pipe deadline");
        }
    }
    libc::close(read).unwrap();
    for child in children {
        if libc::waitpid(child).unwrap() != 0 {
            fail("concurrent pipe status");
        }
    }
    if counts != [65536, 65536] {
        fail("concurrent pipe byte counts");
    }
    debug_log("[SSHL-PTY] concurrent producers slow pipe consumer PASS");

    let (read, write) = libc::pipe().unwrap();
    let child = libc::spawn_with_stdio(
        b"/bin/sshl",
        &[b"sshl", b"--probe", b"pattern", b"1048576", b"4096"],
        None,
        Some(write),
        None,
    )
    .unwrap();
    libc::close(write).unwrap();
    if libc::read(read, &mut buf).unwrap() == 0 {
        fail("early close first read");
    }
    libc::close(read).unwrap();
    if libc::waitpid(child).unwrap() != 32 {
        fail("early close producer EPIPE");
    }
    // Legacy scalar calls (including unrecognized rsi) must retain parking.
    let child = libc::spawn(b"/bin/sshl", &[b"sshl", b"--probe", b"pause", b"100"], None).unwrap();
    let ret = unsafe { libc::sys::syscall2(libc::sys::SYS_WAITPID, child, 0x1234) };
    if ret != 0 && ret != libc::EAGAIN_RAW {
        fail("legacy child wait result");
    }
    if sunlight_ipc::waitpid(child) != 0 {
        fail("legacy IPC blocking wait");
    }
    debug_log("[SSHL-PTY] early close EPIPE legacy blocking wait PASS");
}

fn broker_closure() {
    let service = nameserver_lookup("pty").unwrap();
    let call = |msg| ipc_call_timeout(service, msg, 2000).unwrap();
    let reply = call(IpcMsg::with_label(PtyMsg::CREATE).word(0, 0));
    if reply.label != PtyMsg::REPLY || reply.cap_count < 2 {
        fail("close fixture create");
    }
    let (id, generation, master, control) =
        (reply.words[0], reply.words[1], reply.caps[0], reply.caps[1]);
    let reply = call(
        IpcMsg::with_label(PtyMsg::ATTACH_SLAVE)
            .word(0, id)
            .word(1, generation)
            .with_cap(0, control),
    );
    if reply.label != PtyMsg::REPLY || reply.cap_count < 1 {
        fail("close fixture attach");
    }
    let slave = reply.caps[0];
    let mut buffer = sunlight_ipc::pty::PtyBuffer::new().unwrap();
    buffer.copy_from(&[b'Q'; 4096]);
    for _ in 0..2 {
        let reply = call(buffer.request(PtyMsg::WRITE_SLAVE_BULK, id, generation, slave, 4096));
        if reply.label != PtyMsg::REPLY || reply.words[2] != 4096 {
            fail("broker fill count");
        }
    }
    let full = call(buffer.request(PtyMsg::WRITE_SLAVE_BULK, id, generation, slave, 4096));
    if full.label != PtyMsg::ERROR || full.words[0] != PtyMsg::ERR_WOULD_BLOCK {
        fail("broker bounded full");
    }
    let reply = call(buffer.request(PtyMsg::READ_MASTER_BULK, id, generation, master, 1));
    if reply.label != PtyMsg::REPLY || reply.words[2] != 1 {
        fail("broker short read");
    }
    let reply = call(buffer.request(PtyMsg::WRITE_SLAVE_BULK, id, generation, slave, 4096));
    if reply.label != PtyMsg::REPLY || reply.words[2] != 1 {
        fail("broker partial write");
    }
    let closed = call(
        IpcMsg::with_label(PtyMsg::CLOSE_SLAVE)
            .word(0, id)
            .word(1, generation)
            .with_cap(0, slave),
    );
    if closed.label != PtyMsg::REPLY {
        fail("broker close slave");
    }
    let mut drained = 0;
    loop {
        let reply = call(buffer.request(PtyMsg::READ_MASTER_BULK, id, generation, master, 4096));
        // The established PTY wire protocol reports EOF as PEER_CLOSED after
        // all queued bytes drain; terminal clients translate this to read(0).
        if reply.label == PtyMsg::ERROR && reply.words[0] == PtyMsg::ERR_PEER_CLOSED {
            break;
        }
        if reply.label != PtyMsg::REPLY {
            fail(&format!(
                "broker drain after close bytes={drained} label={} error={}",
                reply.label, reply.words[0]
            ));
        }
        let n = reply.words[2] as usize;
        if n == 0 {
            fail("broker unexpected zero before wire EOF");
        }
        let mut bytes = [0u8; 4096];
        if buffer.copy_to(&mut bytes, n) != n || bytes[..n].iter().any(|b| *b != b'Q') {
            fail("broker close integrity");
        }
        drained += n;
        if drained > 8192 {
            fail("broker close drain bound");
        }
    }
    if drained != 8192 {
        fail("broker drain before EOF count");
    }
    call(
        IpcMsg::with_label(PtyMsg::CLOSE_SESSION)
            .word(0, id)
            .word(1, generation)
            .with_cap(0, control),
    );

    let reply = call(IpcMsg::with_label(PtyMsg::CREATE).word(0, 0));
    let (id, generation, master, control) =
        (reply.words[0], reply.words[1], reply.caps[0], reply.caps[1]);
    let reply = call(
        IpcMsg::with_label(PtyMsg::ATTACH_SLAVE)
            .word(0, id)
            .word(1, generation)
            .with_cap(0, control),
    );
    let slave = reply.caps[0];
    call(
        IpcMsg::with_label(PtyMsg::CLOSE_MASTER)
            .word(0, id)
            .word(1, generation)
            .with_cap(0, master),
    );
    let reply = call(buffer.request(PtyMsg::WRITE_SLAVE_BULK, id, generation, slave, 4096));
    if reply.label != PtyMsg::ERROR || reply.words[0] != PtyMsg::ERR_PEER_CLOSED {
        fail("broker early master closure");
    }
    call(
        IpcMsg::with_label(PtyMsg::CLOSE_SESSION)
            .word(0, id)
            .word(1, generation)
            .with_cap(0, control),
    );
    debug_log("[SSHL-PTY] broker partial full EOF early closure PASS");
}

fn trace_processes() {
    use sunlight_telemetry::{TelemetryPage, MAX_PROCESSES, TELEMETRY_MAGIC};
    let Ok(address) = libc::map_telemetry() else {
        return;
    };
    let page = address as *const TelemetryPage;
    if unsafe { core::ptr::addr_of!((*page).magic).read_volatile() } != TELEMETRY_MAGIC {
        return;
    }
    // Bounded seqlock retries; this diagnostic must never prevent gate failure.
    for _ in 0..3 {
        let seq = unsafe { core::ptr::addr_of!((*page).sequence).read_volatile() };
        if seq & 1 != 0 {
            continue;
        }
        let count = unsafe { core::ptr::addr_of!((*page).proc_count).read_volatile() } as usize;
        let mut samples = [sunlight_telemetry::ProcessStat::default(); MAX_PROCESSES];
        for (i, sample) in samples
            .iter_mut()
            .enumerate()
            .take(count.min(MAX_PROCESSES))
        {
            *sample = unsafe { core::ptr::addr_of!((*page).procs[i]).read_volatile() };
        }
        if seq != unsafe { core::ptr::addr_of!((*page).sequence).read_volatile() } {
            continue;
        }
        for sample in samples.iter().take(count.min(MAX_PROCESSES)) {
            let name = core::str::from_utf8(&sample.name)
                .unwrap_or("")
                .trim_end_matches('\0');
            if name.starts_with("sshl") || name == "pty_server" {
                let (pid, state, ticks) = (sample.pid, sample.state, sample.cpu_ticks);
                debug_log(&format!(
                    "[SSHL-LIVE] snapshot pid={pid} name={name} state={state} runtime_ns={ticks}"
                ));
            }
        }
        break;
    }
}
impl Drop for PtyRunner {
    fn drop(&mut self) {
        debug_log(&format!("[SSHL-PTY] teardown pid={} kill begin", self.pid));
        let killed = libc::kill(self.pid, 9);
        debug_log(&format!(
            "[SSHL-PTY] teardown pid={} kill returned",
            self.pid
        ));
        let code = libc::waitpid(self.pid).unwrap_or_else(|error| {
            trace_processes();
            fail(&format!(
                "PTY teardown wait pid={} error={error:?}",
                self.pid
            ));
        });
        if killed.is_ok() && code != 137 {
            fail("PTY teardown SIGKILL exit status");
        }
        debug_log(&format!(
            "[SSHL-PTY] teardown pid={} child waited status={code}",
            self.pid
        ));
        if self.session_closed {
            debug_log(&format!(
                "[SSHL-PTY] teardown pid={} session already closed",
                self.pid
            ));
            return;
        }
        let reply = ipc_call_timeout(
            self.service,
            IpcMsg::with_label(PtyMsg::CLOSE_SESSION)
                .word(0, self.id)
                .word(1, self.generation)
                .with_cap(0, self.control),
            2000,
        )
        .unwrap_or_else(|error| {
            trace_processes();
            fail(&format!("PTY teardown close error={error:?}"));
        });
        if reply.label != PtyMsg::REPLY {
            fail("PTY teardown session close reply");
        }
        debug_log(&format!(
            "[SSHL-PTY] teardown pid={} session closed",
            self.pid
        ));
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
    if path == "script-pty" {
        trace_processes();
    }
    let start = monotonic_millis(); check("sshl --probe pause 200", 0);
    debug_log(&format!("[SSHL-TEST] {path} wait elapsed_ms={} PASS", monotonic_millis()-start));
    debug_log(&format!("[SSHL-TEST] {path} coverage PASS"));
}

/// Real /bin/sh invocations; stdout is descriptor-backed, with no terminal input.
pub(super) fn script_suite() -> ! {
    let fixture = b"#!/bin/sh\nf() { echo \"$0:$#:$1\"; return 7; }\nf \"$1\"\necho status:$?\nfor x in \"$@\"; do echo \"<$x>\"; done\nif false; then echo bad; elif ! false; then echo yes; fi\nready=no; until test \"$ready\" = yes; do ready=yes; done\nwhile true; do for x in one two; do echo $x; break 2; done; done\nvalue=parent; (value=child; cd /tmp; echo $value); echo $value\ncase abc in 'a*') echo bad;; a?c) echo case;; esac\n{ echo group; echo two; } > /tmp/sshl-group\nsshl --probe copy < /tmp/sshl-group | sshl --probe copy\nsshl --probe exit 9 | sshl --probe exit 3\necho final:$?\nexit 23\n";
    let fd = libc::open_with_flags_mode(
        b"/tmp/sshl-phase2.sh",
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
        0o644,
    )
    .unwrap_or_else(|_| fail("script create"));
    libc::write_all(fd, fixture).unwrap();
    libc::close(fd).unwrap();
    let run = |argv: &[&[u8]], expected: u64, output: &[u8]| {
        let fd = libc::open_with_flags_mode(
            b"/tmp/sshl-script-output",
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o644,
        )
        .unwrap();
        let pid = libc::spawn_with_stdio(b"/bin/sh", argv, None, Some(fd), None)
            .unwrap_or_else(|_| fail("/bin/sh spawn"));
        libc::close(fd).unwrap();
        let status = libc::waitpid(pid).unwrap_or_else(|_| fail("script wait"));
        if status != expected {
            fail(&format!("script status: {status}, expected {expected}"));
        }
        expect("/tmp/sshl-script-output", output);
    };
    run(&[b"sh", b"/tmp/sshl-phase2.sh", b"hello world", b""], 23,
        b"/tmp/sshl-phase2.sh:1:hello world\nstatus:7\n<hello world>\n<>\nyes\none\nchild\nparent\ncase\ngroup\ntwo\nfinal:3\n");
    debug_log("[SSHL-SCRIPT] file arguments control-flow pipeline PASS");
    run(
        &[
            b"sh",
            b"-c",
            b"echo \"$0:$#:$1\"; false || true; exit 9",
            b"named",
            b"argument",
        ],
        9,
        b"named:1:argument\n",
    );
    run(&[b"sh", b"-c", b"if true; then"], 2, b"");
    run(&[b"sh", b"-c", b"f() { f; }; f"], 2, b"");
    run(
        &[
            b"sh",
            b"-c",
            b"export SCRIPT_EXPORT=changed; sshl --probe exit 0",
        ],
        0,
        b"",
    );
    run(
        &[
            b"sh",
            b"-c",
            b"cd /absent || echo recovered; test bad -eq 2; echo status:$?",
        ],
        0,
        b"recovered\nstatus:2\n",
    );
    if libc::spawn(b"/tmp/sshl-phase2.sh", &[b"/tmp/sshl-phase2.sh"], None).is_ok() {
        fail("unexpected kernel shebang support");
    }
    debug_log("[SSHL-SCRIPT] command syntax status PASS");
    run(&[b"sh", b"-c", b"NAME=Sunlight; export NAME; sh -c 'echo \"$NAME\"'; NAME=temporary sh -c 'echo \"$NAME\"'; echo \"$NAME\"; unset NAME; sh -c 'echo \"<$NAME>\"'"],
        0, b"Sunlight\ntemporary\nSunlight\n<>\n");
    run(&[b"sh", b"-c", b"LOCAL=hidden; sshl --probe env LOCAL; NAME=temporary sshl --probe env NAME; echo \"<$NAME>\"; export NAME; sshl --probe env NAME; NAME=permanent :; echo $NAME"],
        0, b"<unset>\n<temporary>\n<>\n<unset>\npermanent\n");
    run(&[b"sh", b"-c", b"NAME=parent; export NAME; NAME=temporary sshl --probe exit 7; echo \"$?:$NAME\"; (NAME=child; sshl --probe env NAME); sshl --probe env NAME; NAME=special export NAME; sshl --probe env NAME"],
        0, b"7:parent\n<child>\n<parent>\n<special>\n");
    run(&[b"sh", b"-c", b"NAME=parent; export NAME; NAME=temporary missing-sshl-command || :; sshl --probe env NAME; readonly LOCK=fixed; LOCK=bad sh -c 'echo unreachable'"],
        2, b"<parent>\n");
    run(&[b"sh", b"-c", b"export NAME=pipeline; sshl --probe env NAME | sshl --probe copy; unset HOME; sh -c 'echo \"<$HOME>\"'"],
        0, b"<pipeline>\n<>\n");
    run(&[b"sh", b"-c", b"NAME=parent; export NAME; NAME=left sh -c 'echo \"$NAME\"' | NAME=right sh -c 'echo \"$NAME\"; sshl --probe copy'; echo \"$NAME\""],
        0, b"right\nleft\nparent\n");
    environment_transport_suite();
    debug_log("[SSHL-ENV] export assignment isolation transport security PASS");
    run(&[b"sh", b"-c", b"echo \"$(echo hello)\"; echo \"$(echo \"$(echo nested)\")\"; echo '$(echo literal)'; v=$(printf 'a\\nb\\n\\n'); echo \"<$v>\"; v=$(true); echo \"<$v>\"; set -- $(echo a b); echo $#"],
        0, b"hello\nnested\n$(echo literal)\n<a\nb>\n<>\n2\n");
    run(&[b"sh", b"-c", b"f() { NAME=child; cd /tmp; echo \"$NAME\"; return 7; }; NAME=parent; v=$(f); echo \"$?:$v:$NAME\"; pwd; echo \"$(if true; then echo yes; fi)\"; echo \"$(case x in x) echo ')';; *) :;; esac)\""],
        0, b"7:child:parent\n/\nyes\n)\n");
    run(&[b"sh", b"-c", b"v=$(false); echo $?; echo \"$(false)\"; echo $?; v=$(sshl --probe exit 9 | sshl --probe exit 3); echo $?; v=$(missing-sshl-substitution); echo $?; v=$(echo bad < /absent/file); echo $?; v=$(exit 17); echo $?"],
        0, b"1\n\n0\n3\n127\n1\n17\n");
    run(&[b"sh", b"-c", b"echo \"$(echo hello | grep hello | wc -c)\"; v=$(echo out; echo err >&2) 2> /tmp/sshl-sub-error; echo \"$v\"; cat /tmp/sshl-sub-error; echo out > \"$(echo /tmp/sshl-sub-path)\"; cat /tmp/sshl-sub-path"],
        0, b"       6\nout\nerr\nout\n");
    run(&[b"sh", b"-c", b"v=$(sshl --probe pattern 8192 257 | sshl --probe copy); echo \"$v\" > /tmp/sshl-sub-large"], 0, b"");
    let mut large: Vec<_> = (0..8192).map(|i| b'A' + (i % 23) as u8).collect();
    large.push(b'\n');
    expect("/tmp/sshl-sub-large", &large);
    run(
        &[
            b"sh",
            b"-c",
            b"v=$(sshl --probe emit 1048576 | sshl --probe take 17); echo \"$v\"",
        ],
        0,
        b"xxxxxxxxxxxxxxxxx\n",
    );
    run(&[b"sh", b"-c", b"v=$({ echo \"$(sh -c 'sshl --probe tag 8192 257 Y >&2; sshl --probe tag 8192 257 X')\"; } 2>&1); echo \"$v\" > /tmp/sshl-sub-dual"], 0, b"");
    let mut dual = alloc::vec![b'Y'; 8192];
    dual.extend_from_slice(&alloc::vec![b'X'; 8192]);
    dual.push(b'\n');
    expect("/tmp/sshl-sub-dual", &dual);
    run(
        &[
            b"sh",
            b"-c",
            b"v=$(sshl --probe tag 131072 257 '\n'); echo \"<$v>\"",
        ],
        0,
        b"<>\n",
    );
    run(
        &[
            b"sh",
            b"-c",
            b"v=$(sshl --probe emit 1048576 | sshl --probe copy); echo unreachable",
        ],
        2,
        b"",
    );
    run(
        &[b"sh", b"-c", b"v=$(printf '\\000'); echo unreachable"],
        2,
        b"",
    );
    run(&[b"sh", b"-c", b"echo $(echo |)"], 2, b"");
    debug_log("[SSHL-SUB] grammar isolation quoting status stdout stderr PASS");
    run(&[b"sh", b"-c", b"NAME=parent; export NAME; v=$(sshl --probe env NAME); echo \"$v\"; v=$(NAME=temp sshl --probe env NAME); echo \"$v:$NAME\"; v=$({ echo bad < /absent/file; } 2> /tmp/sshl-sub-diagnostic); echo \"$?:$v\""],
        0, b"<parent>\n<temp>:parent\n1:\n");
    if file("/tmp/sshl-sub-diagnostic").is_empty() {
        fail("substitution error descriptor routing");
    }
    debug_log("[SSHL-SUB] exact 8192 dual 16384 capture 131072 overflow early-close PASS");
    {
        let (reader, writer) = libc::pipe().unwrap();
        let mut fds = [libc::PollFd {
            fd: reader.0 as i32,
            events: 1,
            revents: 0,
        }];
        if libc::poll(&mut fds, 0) != Ok(0) {
            fail("native capture poll empty");
        }
        if libc::poll(&mut fds, 1).is_ok() || libc::poll(&mut [], 0).is_ok() {
            fail("native capture poll bounds");
        }
        fds[0].fd = writer.0 as i32;
        if libc::poll(&mut fds, 0).is_ok() {
            fail("native capture poll writer rejection");
        }
        fds[0].fd = -1;
        if libc::poll(&mut fds, 0).is_ok() {
            fail("native capture poll invalid descriptor");
        }
        fds[0].fd = reader.0 as i32;
        libc::close(writer).unwrap();
        if libc::poll(&mut fds, -1) != Ok(1) || fds[0].revents != 1 {
            fail("native capture poll EOF readiness");
        }
        libc::close(reader).unwrap();
    }
    {
        let mut context = native_script::context();
        let mut runtime = native_script::NativeRuntime::new();
        let source = "v=$(echo \"$(sshl --probe emit 17 | sshl --probe copy)\"); unset v; :";
        sunshell::script::execute(source, &mut context, &mut runtime).unwrap();
        let before = libc::alloc::heap_stats();
        let fd_before = libc::dup(libc::STDOUT).unwrap();
        libc::close(fd_before).unwrap();
        for _ in 0..100 {
            if !matches!(
                sunshell::script::execute(source, &mut context, &mut runtime),
                Ok(sunshell::script::Flow::Status(0))
            ) {
                fail("substitution repeat");
            }
        }
        for failure_source in [
            "v=$(sshl --probe emit 1048576 | sshl --probe copy)",
            "v=$(printf '\\000')",
            "v=$(sshl --probe emit 16385)",
        ] {
            if sunshell::script::execute(failure_source, &mut context, &mut runtime).is_ok() {
                fail("substitution resource rejection");
            }
            sunshell::script::execute(
                "v=$(echo recovered); unset v; :",
                &mut context,
                &mut runtime,
            )
            .unwrap();
        }
        sunshell::script::execute(source, &mut context, &mut runtime).unwrap();
        let fd_after = libc::dup(libc::STDOUT).unwrap();
        libc::close(fd_after).unwrap();
        let after = libc::alloc::heap_stats();
        if fd_before != fd_after
            || after.requested_user_bytes > before.requested_user_bytes + 1024
            || after.failed_allocation_count != before.failed_allocation_count
        {
            fail("substitution descriptor or heap growth");
        }
        debug_log("[SSHL-SUB] repeat=100 descriptors heap cancellation recovery PASS");
    }
    // Warm up and check the parent descriptor allocator and child reaping path.
    run(
        &[
            b"sh",
            b"-c",
            b"echo repeat | sshl --probe copy; (x=child); :",
        ],
        0,
        b"repeat\n",
    );
    let before = libc::alloc::heap_stats();
    let fd_before = libc::dup(libc::STDOUT).unwrap();
    libc::close(fd_before).unwrap();
    for _ in 0..25 {
        run(
            &[
                b"sh",
                b"-c",
                b"echo repeat | sshl --probe copy; (x=child); :",
            ],
            0,
            b"repeat\n",
        );
    }
    let fd_after = libc::dup(libc::STDOUT).unwrap();
    libc::close(fd_after).unwrap();
    let after = libc::alloc::heap_stats();
    if fd_before != fd_after
        || after.requested_user_bytes > before.requested_user_bytes + 1024
        || after.failed_allocation_count != before.failed_allocation_count
    {
        fail("script repeat resource growth");
    }
    // Exercise repeated scopes within one interpreter, including failing opens.
    let mut context = native_script::context();
    let mut runtime = native_script::NativeRuntime::new();
    let before = libc::alloc::heap_stats();
    let fd_before = libc::dup(libc::STDOUT).unwrap();
    libc::close(fd_before).unwrap();
    for _ in 0..100 {
        let result = sunshell::script::execute(
            "{ :; } > /tmp/sshl-scope; (x=child); :",
            &mut context,
            &mut runtime,
        );
        if !matches!(result, Ok(sunshell::script::Flow::Status(0))) {
            fail("script repeated scope");
        }
        if sunshell::script::execute(
            "{ :; } > /tmp/sshl-scope < /absent/file",
            &mut context,
            &mut runtime,
        )
        .is_ok()
        {
            fail("script failed redirection");
        }
    }
    let after = libc::alloc::heap_stats();
    let fd_after = libc::dup(libc::STDOUT).unwrap();
    libc::close(fd_after).unwrap();
    if fd_before != fd_after
        || after.requested_user_bytes > before.requested_user_bytes + 1024
        || after.failed_allocation_count != before.failed_allocation_count
    {
        fail("interpreter resource growth");
    }
    debug_log("[SSHL-SCRIPT] repeated processes descriptors scopes PASS");
    {
        let pty = PtyRunner::new();
        exercise("script-pty", &mut |line| pty.run(line));
        for (line, witness) in [
            ("echo \"$(echo PTY-substitution)\"", "PTY-substitution"),
            ("echo \"$(echo \"$(echo PTY-nested)\")\"", "PTY-nested"),
            (
                "echo \"$(sshl --probe emit 8192 | sshl --probe count)\"",
                "8192",
            ),
        ] {
            pty.send(&format!("{line}\n"));
            let output = pty.prompt();
            if !output
                .windows(witness.len())
                .any(|bytes| bytes == witness.as_bytes())
            {
                fail("PTY substitution witness");
            }
        }
        if pty.run("v=$(false)") != 1 {
            fail("PTY substitution assignment status");
        }
        if pty.run("v=$(sshl --probe emit 1048576)") != 2 {
            fail("PTY substitution overflow");
        }
        if pty.run("v=$(echo recovered)") != 0 {
            fail("PTY substitution recovery/input");
        }
        debug_log("[SSHL-SUB] PTY nested large status recovery input PASS");
        for line in ["NAME=Sunlight", "export NAME"] {
            if pty.run(line) != 0 {
                fail("PTY environment builtin");
            }
        }
        for (line, witness) in [
            ("sh -c 'echo \"$NAME\"'", "Sunlight"),
            ("NAME=temporary sh -c 'echo \"<$NAME>\"'", "<temporary>"),
            ("sshl --probe env NAME", "<Sunlight>"),
            (
                "NAME=pipe sshl --probe env NAME | sshl --probe copy",
                "<pipe>",
            ),
        ] {
            pty.send(&format!("{line}\n"));
            let output = pty.prompt();
            if !output
                .windows(witness.len())
                .any(|s| s == witness.as_bytes())
            {
                fail("PTY environment child output");
            }
        }
        if pty.run("unset NAME") != 0 {
            fail("PTY unset");
        }
        pty.send("sshl --probe env NAME\n");
        if !pty
            .prompt()
            .windows(b"<unset>".len())
            .any(|s| s == b"<unset>")
        {
            fail("PTY unset child output");
        }
        debug_log("[SSHL-ENV] PTY export temporary assignment unset PASS");
        let script = "sh -c 'if true; then echo PTY-$1-$2; fi; exit 7' named script-output witness";
        pty.send(&format!("{script}\n"));
        let output = pty.prompt();
        if !output.windows(b"PTY-script-output-witness".len()).any(|s| s == b"PTY-script-output-witness") { fail("PTY script output association"); }
        if pty.run(script) != 7 { fail("PTY sh -c status"); }
        pty.send("sshl --probe emit 65536\n");
        let output = pty.prompt();
        if output.iter().filter(|b| **b == b'x').count() != 65536 { fail("PTY streamed regression output"); }
        debug_log("[SSHL-SCRIPT] PTY script and live output PASS");
    }
    debug_log("[SSHL-SCRIPT] native gate PASS");
    libc::exit(0)
}

/// Exercise the kernel boundary directly, bypassing libc's validation so these
/// checks cannot pass solely because the wrapper rejected the input.
fn environment_transport_suite() {
    use sunlight_ipc::spawn_env::{SpawnEnvironment, ENV_EXTENSION, MAX_BYTES, MAX_ENTRIES};
    let path = b"/bin/sshl\0";
    let args = [
        b"sshl\0".as_ptr(),
        b"--probe\0".as_ptr(),
        b"exit\0".as_ptr(),
        b"0\0".as_ptr(),
        core::ptr::null(),
    ];
    let raw = |header: u64| unsafe {
        libc::sys::syscall6(
            libc::sys::SYS_SPAWN,
            path.as_ptr() as u64,
            header,
            u64::MAX,
            u64::MAX,
            u64::MAX,
            ENV_EXTENSION,
        )
    };
    let reject = |blob: &[u8]| {
        let header = SpawnEnvironment {
            argv: args.as_ptr() as u64,
            bytes: blob.as_ptr() as u64,
            len: blob.len() as u64,
        };
        if libc::sys::check(raw(&header as *const _ as u64)).is_ok() {
            fail("kernel accepted invalid environment");
        }
    };
    for blob in [
        b"A=x\0A=y\0".as_slice(),
        b"=x\0",
        b"1NAME=x\0",
        b"A=x",
        b"A=\xff\0",
        b"\0",
    ] {
        reject(blob);
    }
    reject(&alloc::vec![b'x'; MAX_BYTES + 1]);
    reject(format!("A={}\0", "x".repeat(4094)).as_bytes());
    let mut too_many = Vec::new();
    for i in 0..=MAX_ENTRIES {
        too_many.extend_from_slice(format!("V{i}=\0").as_bytes());
    }
    reject(&too_many);
    for address in [0, 0xffff_8000_0000_0000u64, 0x0000_8000_0000_0000u64] {
        if libc::sys::check(raw(address)).is_ok() {
            fail("kernel accepted invalid environment header address");
        }
    }
    let bad_pointer = SpawnEnvironment {
        argv: args.as_ptr() as u64,
        bytes: 0,
        len: 1,
    };
    if libc::sys::check(raw(&bad_pointer as *const _ as u64)).is_ok() {
        fail("kernel accepted invalid environment data address");
    }

    let run = |argv: &[&[u8]], blob: &[u8], expected: &[u8]| {
        let fd = libc::open_with_flags_mode(
            b"/tmp/sshl-env-probe",
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o644,
        )
        .unwrap();
        let pid = libc::spawn_with_stdio_env(b"/bin/sshl", argv, None, Some(fd), None, blob)
            .unwrap_or_else(|_| fail("environment spawn"));
        libc::close(fd).unwrap();
        if libc::waitpid(pid).unwrap() != 0 {
            fail("environment probe status");
        }
        expect("/tmp/sshl-env-probe", expected);
    };
    run(&[b"sshl", b"--probe", b"env", b"HOME"], b"", b"<unset>\n");
    run(
        &[b"sshl", b"-c", b"echo \"<$HOME>\"; echo \"<$PATH>\""],
        b"",
        b"<>\n<>\n",
    );
    run(
        &[b"sshl", b"--pipeline-builtin", b"env"],
        b"NAME=only\0",
        b"NAME=only\n",
    );
    let mut full = Vec::new();
    for i in 0..MAX_ENTRIES {
        full.extend_from_slice(format!("V{i}=value\0").as_bytes());
    }
    run(&[b"sshl", b"--probe", b"env", b"V127"], &full, b"<value>\n");
    run(
        &[b"sshl", b"-c", b"echo $V0:$V127"],
        &full,
        b"value:value\n",
    );
    let mut maximum = Vec::new();
    for name in ["A", "B", "C", "D"] {
        maximum.extend_from_slice(format!("{name}={}\0", "x".repeat(4093)).as_bytes());
    }
    if maximum.len() != MAX_BYTES {
        fail("maximum environment fixture");
    }
    run(
        &[b"sshl", b"--probe", b"env-length", b"D"],
        &maximum,
        b"4093\n",
    );
    let mut context = native_script::context();
    let mut runtime = native_script::NativeRuntime::new();
    context.env.assign("BIG", &"x".repeat(4096)).unwrap();
    context.env.export("BIG");
    if sunshell::script::execute("sshl --probe exit 0", &mut context, &mut runtime).is_ok() {
        fail("oversized shell environment launched child");
    }
    if !matches!(
        sunshell::script::execute("unset BIG; sshl --probe exit 0", &mut context, &mut runtime),
        Ok(sunshell::script::Flow::Status(0))
    ) {
        fail("shell unusable after environment limit failure");
    }
    let expected = format!(
        "{}:{}:{}\n",
        libc::getuid(),
        libc::getgid(),
        nameserver_lookup("sshl6").is_some()
    );
    run(
        &[b"sshl", b"--probe", b"identity"],
        b"UID=0\0GID=0\0CAPABILITIES=all\0ROLE=admin\0",
        expected.as_bytes(),
    );
    // Legacy three-argument callers ignore unrecognized extension registers.
    let pid = unsafe {
        libc::sys::syscall6(
            libc::sys::SYS_SPAWN,
            path.as_ptr() as u64,
            args.as_ptr() as u64,
            u64::MAX,
            999999,
            999999,
            0xfeed,
        )
    };
    let pid = libc::sys::check(pid).unwrap_or_else(|_| fail("legacy spawn ABI"));
    if libc::waitpid(pid).unwrap() != 0 {
        fail("legacy spawn wait");
    }
    // A normal legacy child still inherits the original process environment.
    let home = libc::env::getenv(b"HOME").unwrap_or("unset");
    let fd = libc::open_with_flags_mode(
        b"/tmp/sshl-env-probe",
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
        0o644,
    )
    .unwrap();
    let pid = libc::spawn_with_stdio(
        b"/bin/sshl",
        &[b"sshl", b"--probe", b"env", b"HOME"],
        None,
        Some(fd),
        None,
    )
    .unwrap();
    libc::close(fd).unwrap();
    if libc::waitpid(pid).unwrap() != 0 {
        fail("legacy inherited env wait");
    }
    expect("/tmp/sshl-env-probe", format!("<{home}>\n").as_bytes());
}
