#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, CapabilityToken,
    IpcMsg, PtyMsg,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

const MAX_SESSIONS: usize = 8;
const BUFFER_CAP: usize = 8192;
const LINE_MAX: usize = 256;
const HISTORY_MAX: usize = 32;
const CHUNK_BYTES: usize = 16;

#[derive(Clone, Copy)]
struct ByteRing<const N: usize> {
    buf: [u8; N],
    head: usize,
    len: usize,
}

impl<const N: usize> ByteRing<N> {
    const fn new() -> Self {
        Self {
            buf: [0; N],
            head: 0,
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    fn push_slice(&mut self, bytes: &[u8]) -> usize {
        let mut pushed = 0usize;
        for &b in bytes {
            if self.len == N {
                break;
            }
            let tail = (self.head + self.len) % N;
            self.buf[tail] = b;
            self.len += 1;
            pushed += 1;
        }
        pushed
    }

    fn pop_slice(&mut self, out: &mut [u8]) -> usize {
        let mut n = 0usize;
        while n < out.len() && self.len > 0 {
            out[n] = self.buf[self.head];
            self.head = (self.head + 1) % N;
            self.len -= 1;
            n += 1;
        }
        n
    }

    fn peek_len(&self) -> usize {
        self.len
    }
}

#[derive(Clone, Copy)]
struct LineEditor {
    cooked: bool,
    echo: bool,
    line: [u8; LINE_MAX],
    line_len: usize,
    cursor: usize,
    hist_pos: usize,
    hist_stash: [u8; LINE_MAX],
    hist_stash_len: usize,
    history: [[u8; LINE_MAX]; HISTORY_MAX],
    history_lens: [usize; HISTORY_MAX],
    history_head: usize,
    history_count: usize,
    escape_state: u8,
}

impl LineEditor {
    const fn new() -> Self {
        Self {
            cooked: true,
            echo: true,
            line: [0; LINE_MAX],
            line_len: 0,
            cursor: 0,
            hist_pos: 0,
            hist_stash: [0; LINE_MAX],
            hist_stash_len: 0,
            history: [[0; LINE_MAX]; HISTORY_MAX],
            history_lens: [0; HISTORY_MAX],
            history_head: 0,
            history_count: 0,
            escape_state: 0,
        }
    }

    fn reset(&mut self) {
        self.line_len = 0;
        self.cursor = 0;
        self.hist_pos = 0;
        self.hist_stash_len = 0;
        self.escape_state = 0;
    }

    fn set_mode(&mut self, flags: u64) {
        self.cooked = (flags & PtyMsg::FLAG_CANONICAL) != 0;
        self.echo = (flags & PtyMsg::FLAG_ECHO) != 0;
    }

    fn push_history(&mut self, line: &[u8]) {
        if line.is_empty() {
            return;
        }
        if self.history_count > 0 {
            let last = (self.history_head + self.history_count - 1) % HISTORY_MAX;
            if self.history_lens[last] == line.len() && &self.history[last][..line.len()] == line {
                return;
            }
        }
        let slot = if self.history_count == HISTORY_MAX {
            let oldest = self.history_head;
            self.history_head = (self.history_head + 1) % HISTORY_MAX;
            oldest
        } else {
            let next = (self.history_head + self.history_count) % HISTORY_MAX;
            self.history_count += 1;
            next
        };
        let n = line.len().min(LINE_MAX);
        self.history[slot][..n].copy_from_slice(&line[..n]);
        self.history_lens[slot] = n;
    }

    fn history_recent(&self, n: usize, out: &mut [u8]) -> Option<usize> {
        if n == 0 || n > self.history_count {
            return None;
        }
        let slot = (self.history_head + self.history_count - n) % HISTORY_MAX;
        let len = self.history_lens[slot].min(out.len());
        out[..len].copy_from_slice(&self.history[slot][..len]);
        Some(len)
    }

    fn apply_up(&mut self) -> Option<(usize, usize)> {
        if self.history_count == 0 || self.hist_pos >= self.history_count {
            return None;
        }
        if self.hist_pos == 0 {
            self.hist_stash = self.line;
            self.hist_stash_len = self.line_len;
        }
        self.hist_pos += 1;
        let mut buf = [0u8; LINE_MAX];
        let len = self.history_recent(self.hist_pos, &mut buf)?;
        let prev_len = self.line_len;
        self.line = buf;
        self.line_len = len;
        self.cursor = len;
        Some((prev_len, len))
    }

    fn apply_down(&mut self) -> Option<(usize, usize)> {
        if self.hist_pos == 0 {
            return None;
        }
        let prev_len = self.line_len;
        self.hist_pos -= 1;
        if self.hist_pos == 0 {
            self.line = self.hist_stash;
            self.line_len = self.hist_stash_len;
            self.cursor = self.hist_stash_len;
            return Some((prev_len, self.line_len));
        }
        let mut buf = [0u8; LINE_MAX];
        let len = self.history_recent(self.hist_pos, &mut buf)?;
        self.line = buf;
        self.line_len = len;
        self.cursor = len;
        Some((prev_len, len))
    }

    fn insert_byte(&mut self, byte: u8) {
        if self.line_len == LINE_MAX {
            return;
        }
        let mut i = self.line_len;
        while i > self.cursor {
            self.line[i] = self.line[i - 1];
            i -= 1;
        }
        self.line[self.cursor] = byte;
        self.line_len += 1;
        self.cursor += 1;
        self.hist_pos = 0;
    }

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let mut i = self.cursor - 1;
        while i + 1 < self.line_len {
            self.line[i] = self.line[i + 1];
            i += 1;
        }
        self.line_len -= 1;
        self.cursor -= 1;
        self.hist_pos = 0;
        true
    }

    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            false
        } else {
            self.cursor -= 1;
            true
        }
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.line_len {
            false
        } else {
            self.cursor += 1;
            true
        }
    }

    fn home(&mut self) -> bool {
        if self.cursor == 0 {
            false
        } else {
            self.cursor = 0;
            true
        }
    }

    fn end(&mut self) -> bool {
        if self.cursor == self.line_len {
            false
        } else {
            self.cursor = self.line_len;
            true
        }
    }

    fn feed(&mut self, byte: u8, slave_in: &mut ByteRing<BUFFER_CAP>, master_out: &mut ByteRing<BUFFER_CAP>) {
        if !self.cooked {
            if self.echo {
                let _ = master_out.push_slice(&[byte]);
            }
            let _ = slave_in.push_slice(&[byte]);
            return;
        }

        match self.escape_state {
            0 => match byte {
                0x1b => self.escape_state = 1,
                b'\r' | b'\n' => {
                    if self.echo {
                        let _ = master_out.push_slice(b"\r\n");
                    }
                    let line_len = self.line_len;
                    let mut completed = [0u8; LINE_MAX];
                    completed[..line_len].copy_from_slice(&self.line[..line_len]);
                    if line_len > 0 {
                        let _ = slave_in.push_slice(&completed[..line_len]);
                    }
                    let _ = slave_in.push_slice(b"\n");
                    self.push_history(&completed[..line_len]);
                    self.reset();
                }
                0x08 | 0x7f => {
                    if self.backspace() && self.echo {
                        let _ = master_out.push_slice(b"\x08 \x08");
                    }
                }
                b if (0x20..=0x7e).contains(&b) => {
                    self.insert_byte(b);
                    if self.echo {
                        let _ = master_out.push_slice(&[b]);
                    }
                }
                _ => {}
            },
            1 => {
                if byte == b'[' {
                    self.escape_state = 2;
                } else {
                    self.escape_state = 0;
                }
            }
            2 => {
                let redraw = match byte {
                    b'A' => self.apply_up(),
                    b'B' => self.apply_down(),
                    b'C' => {
                        if self.move_right() {
                            Some((0, 0))
                        } else {
                            None
                        }
                    }
                    b'D' => {
                        if self.move_left() {
                            Some((0, 0))
                        } else {
                            None
                        }
                    }
                    b'H' => {
                        if self.home() {
                            Some((0, 0))
                        } else {
                            None
                        }
                    }
                    b'F' => {
                        if self.end() {
                            Some((0, 0))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some((prev_len, new_len)) = redraw {
                    if self.echo {
                        if prev_len > 0 {
                            for _ in 0..prev_len {
                                let _ = master_out.push_slice(b"\x08 \x08");
                            }
                        }
                        if new_len > 0 {
                            let _ = master_out.push_slice(&self.line[..new_len]);
                        }
                    }
                }
                self.escape_state = 0;
            }
            _ => self.escape_state = 0,
        }
    }
}

#[derive(Clone, Copy)]
struct PtySession {
    active: bool,
    id: u64,
    mode_flags: u64,
    control_cap: CapabilityToken,
    master_cap: CapabilityToken,
    slave_cap: CapabilityToken,
    master_out: ByteRing<BUFFER_CAP>,
    slave_in: ByteRing<BUFFER_CAP>,
    editor: LineEditor,
}

impl PtySession {
    const fn empty() -> Self {
        Self {
            active: false,
            id: 0,
            mode_flags: 0,
            control_cap: CapabilityToken::INVALID,
            master_cap: CapabilityToken::INVALID,
            slave_cap: CapabilityToken::INVALID,
            master_out: ByteRing::new(),
            slave_in: ByteRing::new(),
            editor: LineEditor::new(),
        }
    }

    fn reset(&mut self, id: u64, cap: CapabilityToken, mode_flags: u64) {
        self.active = true;
        self.id = id;
        self.mode_flags = mode_flags;
        self.control_cap = cap;
        self.master_cap = cap;
        self.slave_cap = cap;
        self.master_out.clear();
        self.slave_in.clear();
        self.editor = LineEditor::new();
        self.editor.set_mode(self.mode_flags);
    }
}

struct PtyServer {
    sessions: [PtySession; MAX_SESSIONS],
    next_id: u64,
}

impl PtyServer {
    const fn new() -> Self {
        Self {
            sessions: [PtySession::empty(); MAX_SESSIONS],
            next_id: 1,
        }
    }

    fn create_session(&mut self, cap: CapabilityToken, mode_flags: u64) -> Option<&mut PtySession> {
        let idx = self.sessions.iter().position(|s| !s.active)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let session = &mut self.sessions[idx];
        session.reset(id, cap, mode_flags);
        Some(session)
    }

    fn session_mut(&mut self, id: u64) -> Option<&mut PtySession> {
        self.sessions.iter_mut().find(|s| s.active && s.id == id)
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[PTY] pty_server started");
    let ep = endpoint_create();
    nameserver_register("pty", ep);
    debug_log("[PTY] registered as 'pty'");

    let mut server = PtyServer::new();
    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle_message(&mut server, ep, &msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle_message(server: &mut PtyServer, ep: sunlight_ipc::EndpointId, msg: &IpcMsg) -> IpcMsg {
    match msg.label {
        PtyMsg::CREATE => {
            let cap = CapabilityToken(ep.0);
            let session = match server.create_session(cap, msg.words[0]) {
                Some(s) => s,
                None => return IpcMsg::with_label(PtyMsg::ERROR).word(0, 1),
            };
            let mut reply = IpcMsg::with_label(PtyMsg::REPLY)
                .word(0, session.id)
                .word(1, session.mode_flags);
            // Current transport: one mailbox, two logical ends. The returned
            // caps are aliases to the same endpoint token, and the client
            // chooses the side by the request label it uses later.
            reply.caps[0] = session.master_cap;
            reply.caps[1] = session.slave_cap;
            reply.cap_count = 2;
            reply
        }
        PtyMsg::SET_MODE => {
            let Some(session) = server.session_mut(msg.words[0]) else {
                return IpcMsg::with_label(PtyMsg::ERROR).word(0, 2);
            };
            session.mode_flags = msg.words[1];
            session.editor.set_mode(session.mode_flags);
            IpcMsg::with_label(PtyMsg::REPLY).word(0, session.id)
        }
        PtyMsg::WRITE_MASTER => write_master(server, msg),
        PtyMsg::WRITE_SLAVE => write_slave(server, msg),
        PtyMsg::READ_MASTER => read_master(server, msg),
        PtyMsg::READ_SLAVE => read_slave(server, msg),
        PtyMsg::CLOSE => {
            if let Some(session) = server.session_mut(msg.words[0]) {
                session.active = false;
            }
            IpcMsg::with_label(PtyMsg::REPLY)
        }
        _ => IpcMsg::with_label(PtyMsg::ERROR).word(0, 3),
    }
}

fn write_master(server: &mut PtyServer, msg: &IpcMsg) -> IpcMsg {
    let Some(session) = server.session_mut(msg.words[0]) else {
        return IpcMsg::with_label(PtyMsg::ERROR).word(0, 10);
    };
    let bytes = unpack_payload(msg);
    if !bytes.is_empty() {
        debug_log("[PTY] write_master\n");
    }
    for &b in bytes.iter() {
        session.editor.feed(b, &mut session.slave_in, &mut session.master_out);
    }
    IpcMsg::with_label(PtyMsg::REPLY)
        .word(0, session.id)
        .word(1, bytes.len() as u64)
}

fn write_slave(server: &mut PtyServer, msg: &IpcMsg) -> IpcMsg {
    let Some(session) = server.session_mut(msg.words[0]) else {
        return IpcMsg::with_label(PtyMsg::ERROR).word(0, 11);
    };
    let bytes = unpack_payload(msg);
    let _ = session.master_out.push_slice(bytes);
    IpcMsg::with_label(PtyMsg::REPLY)
        .word(0, session.id)
        .word(1, bytes.len() as u64)
}

fn read_master(server: &mut PtyServer, msg: &IpcMsg) -> IpcMsg {
    let Some(session) = server.session_mut(msg.words[0]) else {
        return IpcMsg::with_label(PtyMsg::ERROR).word(0, 12);
    };
    read_from_ring(session.id, &mut session.master_out, msg.words[1] as usize)
}

fn read_slave(server: &mut PtyServer, msg: &IpcMsg) -> IpcMsg {
    let Some(session) = server.session_mut(msg.words[0]) else {
        return IpcMsg::with_label(PtyMsg::ERROR).word(0, 13);
    };
    if session.slave_in.peek_len() > 0 {
        debug_log("[PTY] read_slave\n");
    }
    read_from_ring(session.id, &mut session.slave_in, msg.words[1] as usize)
}

fn read_from_ring(id: u64, ring: &mut ByteRing<BUFFER_CAP>, max_len: usize) -> IpcMsg {
    if ring.peek_len() == 0 {
        IpcMsg::with_label(PtyMsg::REPLY).word(0, id).word(1, 0)
    } else {
        let len = max_len.min(CHUNK_BYTES).min(ring.peek_len());
        let mut bytes = [0u8; CHUNK_BYTES];
        let n = ring.pop_slice(&mut bytes[..len]);
        pack_reply(id, &bytes[..n])
    }
}

fn pack_reply(id: u64, bytes: &[u8]) -> IpcMsg {
    IpcMsg::with_label(PtyMsg::REPLY)
        .word(0, id)
        .word(1, bytes.len() as u64)
        .word(2, pack_u64(bytes))
        .word(3, pack_u64(bytes.get(8..).unwrap_or(&[])))
}

fn unpack_payload(msg: &IpcMsg) -> &[u8] {
    let len = (msg.words[1] as usize).min(CHUNK_BYTES);
    // The current register IPC carries up to 16 payload bytes in words[2..4].
    // For longer writes, callers should loop and send another chunk.
    unsafe { core::slice::from_raw_parts(msg.words.as_ptr().add(2) as *const u8, len.min(16)) }
}

fn pack_u64(bytes: &[u8]) -> u64 {
    let mut out = 0u64;
    for (i, b) in bytes.iter().take(8).enumerate() {
        out |= (*b as u64) << (i * 8);
    }
    out
}
