#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

//! Bounded PTY broker.
//!
//! `master -> input -> slave` and `slave -> output -> master` are separate
//! fixed-size FIFOs. Endpoint authority is an opaque role token issued by this
//! service and bound to a kernel-authenticated caller PID; an ID alone never
//! authorizes an operation.

use sunlight_ipc::{
    debug_log, pty_caller_credentials, tty_attach_process, tty_publish_winsize, CapabilityToken,
    IpcMsg, PtyCallerCredentials, PtyMsg, TerminalWinsize,
};

#[cfg(not(test))]
use sunlight_ipc::{
    endpoint_create, entropy_u64, ipc_recv, ipc_reply_and_wait, nameserver_register,
    secure_entropy_ready,
};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Six graphical tabs, eight planned remote sessions, and two recovery slots.
pub const MAX_SESSIONS: usize = 16;
pub const BUFFER_CAP: usize = 8192;
/// Register IPC leaves one payload word after a generation-qualified identity
/// and request length, so each transport operation carries at most eight bytes.
const CHUNK_BYTES: usize = 8;
/// Startup fallback used only until a terminal frontend publishes its real
/// drawable grid. A frontend-supplied size is passed in `CREATE` and therefore
/// becomes authoritative before the shell is attached.
const DEFAULT_WINDOW_SIZE: TerminalWinsize = TerminalWinsize::new(100, 30, 0, 0);

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
        self.buf = [0; N];
        self.head = 0;
        self.len = 0;
    }

    fn push_slice(&mut self, bytes: &[u8]) -> usize {
        let accepted = bytes.len().min(N - self.len);
        for &byte in &bytes[..accepted] {
            let tail = (self.head + self.len) % N;
            self.buf[tail] = byte;
            self.len += 1;
        }
        accepted
    }

    fn pop_slice(&mut self, out: &mut [u8]) -> usize {
        let count = out.len().min(self.len);
        for slot in out.iter_mut().take(count) {
            *slot = self.buf[self.head];
            self.buf[self.head] = 0;
            self.head = (self.head + 1) % N;
            self.len -= 1;
        }
        count
    }

    const fn len(&self) -> usize {
        self.len
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PtyRole {
    Master,
    Slave,
    Control,
}

#[derive(Clone, Copy)]
struct PtyOwner {
    uid: u32,
    gid: u32,
    creator_pid: u64,
}

impl PtyOwner {
    const fn empty() -> Self {
        Self {
            uid: 0,
            gid: 0,
            creator_pid: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Authority {
    token: CapabilityToken,
    pid: u64,
}

impl Authority {
    const fn empty() -> Self {
        Self {
            token: CapabilityToken::INVALID,
            pid: 0,
        }
    }

    fn matches(self, token: CapabilityToken, pid: u64) -> bool {
        self.token != CapabilityToken::INVALID
            && self.token == token
            && (self.pid == 0 || self.pid == pid)
    }

    fn clear(&mut self) {
        self.token = CapabilityToken::INVALID;
        self.pid = 0;
    }
}

#[derive(Clone, Copy)]
struct PtySession {
    live: bool,
    id: u64,
    generation: u64,
    owner: PtyOwner,
    mode_flags: u64,
    size: TerminalWinsize,
    foreground_pid: Option<u64>,
    master: Authority,
    slave: Authority,
    control: Authority,
    master_open: bool,
    slave_open: bool,
    control_open: bool,
    closing: bool,
    output: ByteRing<BUFFER_CAP>,
    input: ByteRing<BUFFER_CAP>,
}

impl PtySession {
    const fn empty(id: u64) -> Self {
        Self {
            live: false,
            id,
            generation: 0,
            owner: PtyOwner::empty(),
            mode_flags: 0,
            size: DEFAULT_WINDOW_SIZE,
            foreground_pid: None,
            master: Authority::empty(),
            slave: Authority::empty(),
            control: Authority::empty(),
            master_open: false,
            slave_open: false,
            control_open: false,
            closing: false,
            output: ByteRing::new(),
            input: ByteRing::new(),
        }
    }

    fn destroy(&mut self) {
        self.live = false;
        self.owner = PtyOwner::empty();
        self.mode_flags = 0;
        self.size = DEFAULT_WINDOW_SIZE;
        self.foreground_pid = None;
        self.master.clear();
        self.slave.clear();
        self.control.clear();
        self.master_open = false;
        self.slave_open = false;
        self.control_open = false;
        self.closing = true;
        self.output.clear();
        self.input.clear();
    }

    const fn state_bits(&self) -> u64 {
        (self.master_open as u64 * PtyMsg::STATE_MASTER_OPEN)
            | (self.slave_open as u64 * PtyMsg::STATE_SLAVE_OPEN)
            | (self.control_open as u64 * PtyMsg::STATE_CONTROL_OPEN)
            | (self.closing as u64 * PtyMsg::STATE_CLOSING)
    }

    fn authority(&self, role: PtyRole) -> Authority {
        match role {
            PtyRole::Master => self.master,
            PtyRole::Slave => self.slave,
            PtyRole::Control => self.control,
        }
    }
}

struct PtyServer {
    sessions: [PtySession; MAX_SESSIONS],
    next_token: u64,
}

impl PtyServer {
    const fn new() -> Self {
        Self {
            sessions: [
                PtySession::empty(1),
                PtySession::empty(2),
                PtySession::empty(3),
                PtySession::empty(4),
                PtySession::empty(5),
                PtySession::empty(6),
                PtySession::empty(7),
                PtySession::empty(8),
                PtySession::empty(9),
                PtySession::empty(10),
                PtySession::empty(11),
                PtySession::empty(12),
                PtySession::empty(13),
                PtySession::empty(14),
                PtySession::empty(15),
                PtySession::empty(16),
            ],
            next_token: 1,
        }
    }

    fn mint_token(
        &mut self,
        id: u64,
        generation: u64,
        pid: u64,
        role: PtyRole,
    ) -> Result<CapabilityToken, u64> {
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let role_tag = match role {
            PtyRole::Master => 0x4d41_5354_4552u64,
            PtyRole::Slave => 0x534c_4156_4500u64,
            PtyRole::Control => 0x4354_524c_0000u64,
        };
        let entropy = {
            #[cfg(test)]
            {
                0x6a09_e667_f3bc_c909
            }
            #[cfg(not(test))]
            {
                if !secure_entropy_ready() {
                    return Err(PtyMsg::ERR_SERVICE_UNAVAILABLE);
                }
                entropy_u64()
            }
        };
        let mut token = entropy
            ^ self
                .next_token
                .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                .rotate_left((id as u32) & 31)
            ^ generation.rotate_left(17)
            ^ pid.rotate_left(29)
            ^ role_tag;
        if token == 0 {
            token = 1;
        }
        Ok(CapabilityToken(token))
    }

    fn locate(&self, id: u64, generation: u64) -> Result<usize, u64> {
        if id == 0 || id as usize > MAX_SESSIONS {
            return Err(PtyMsg::ERR_INVALID_PTY);
        }
        let session = &self.sessions[id as usize - 1];
        if !session.live || session.generation != generation {
            return Err(PtyMsg::ERR_STALE_HANDLE);
        }
        Ok(id as usize - 1)
    }

    fn check(
        &self,
        msg: &IpcMsg,
        caller: PtyCallerCredentials,
        role: PtyRole,
    ) -> Result<usize, u64> {
        let index = self.locate(msg.words[0], msg.words[1])?;
        let session = &self.sessions[index];
        if !session.authority(role).matches(msg.caps[0], caller.pid) {
            return Err(PtyMsg::ERR_PERMISSION_DENIED);
        }
        Ok(index)
    }

    fn create(
        &mut self,
        caller: PtyCallerCredentials,
        mode_flags: u64,
        size: TerminalWinsize,
    ) -> Result<(u64, u64, CapabilityToken, CapabilityToken), u64> {
        let index = self
            .sessions
            .iter()
            .position(|session| !session.live)
            .ok_or(PtyMsg::ERR_NO_SLOTS)?;
        let id = self.sessions[index].id;
        let generation = self.sessions[index].generation.wrapping_add(1).max(1);
        let master = self.mint_token(id, generation, caller.pid, PtyRole::Master)?;
        let control = self.mint_token(id, generation, caller.pid, PtyRole::Control)?;
        let session = &mut self.sessions[index];
        session.live = true;
        session.generation = generation;
        session.owner = PtyOwner {
            uid: caller.uid,
            gid: caller.gid,
            creator_pid: caller.pid,
        };
        session.mode_flags = mode_flags;
        session.size = size;
        session.foreground_pid = None;
        session.master = Authority {
            token: master,
            pid: caller.pid,
        };
        session.slave.clear();
        session.control = Authority {
            token: control,
            pid: caller.pid,
        };
        session.master_open = true;
        session.slave_open = true;
        session.control_open = true;
        session.closing = false;
        session.output.clear();
        session.input.clear();
        Ok((id, generation, master, control))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALLER: PtyCallerCredentials = PtyCallerCredentials {
        pid: 42,
        uid: 1000,
        gid: 1000,
    };

    #[test]
    fn ring_is_bounded_and_preserves_order() {
        let mut ring = ByteRing::<4>::new();
        assert_eq!(ring.push_slice(b"abcde"), 4);
        let mut first = [0u8; 2];
        assert_eq!(ring.pop_slice(&mut first), 2);
        assert_eq!(&first, b"ab");
        assert_eq!(ring.push_slice(b"ef"), 2);
        let mut rest = [0u8; 4];
        assert_eq!(ring.pop_slice(&mut rest), 4);
        assert_eq!(&rest, b"cdef");
    }

    #[test]
    fn geometry_rejects_invalid_values() {
        assert!(TerminalWinsize::from_wire(0).is_none());
        assert!(TerminalWinsize::from_wire((1u64 << 16) | 513).is_none());
        assert!(TerminalWinsize::from_wire(80 | (25 << 16)).is_some());
    }

    #[test]
    fn capacity_is_bounded() {
        let mut server = PtyServer::new();
        for _ in 0..MAX_SESSIONS {
            assert!(server
                .create(CALLER, PtyMsg::FLAG_CANONICAL, DEFAULT_WINDOW_SIZE)
                .is_ok());
        }
        assert_eq!(
            server.create(CALLER, 0, DEFAULT_WINDOW_SIZE),
            Err(PtyMsg::ERR_NO_SLOTS)
        );
    }

    #[test]
    fn reused_slot_receives_a_new_generation() {
        let mut server = PtyServer::new();
        let (id, first_generation, _, _) = server.create(CALLER, 0, DEFAULT_WINDOW_SIZE).unwrap();
        let index = server.locate(id, first_generation).unwrap();
        server.sessions[index].destroy();
        let (next_id, next_generation, _, _) =
            server.create(CALLER, 0, DEFAULT_WINDOW_SIZE).unwrap();
        assert_eq!(id, next_id);
        assert_ne!(first_generation, next_generation);
        assert_eq!(
            server.locate(id, first_generation),
            Err(PtyMsg::ERR_STALE_HANDLE)
        );
    }

    #[test]
    fn sessions_keep_independent_authoritative_sizes() {
        let mut server = PtyServer::new();
        let a = TerminalWinsize::new(120, 40, 960, 640);
        let b = TerminalWinsize::new(80, 25, 640, 400);
        let (a_id, a_generation, _, _) = server.create(CALLER, 0, a).unwrap();
        let (b_id, b_generation, _, _) = server.create(CALLER, 0, b).unwrap();
        let a_index = server.locate(a_id, a_generation).unwrap();
        let b_index = server.locate(b_id, b_generation).unwrap();
        assert_eq!(server.sessions[a_index].size, a);
        assert_eq!(server.sessions[b_index].size, b);

        let resized_a = TerminalWinsize::new(170, 50, 1360, 800);
        server.sessions[a_index].size = resized_a;
        assert_eq!(server.sessions[a_index].size, resized_a);
        assert_eq!(server.sessions[b_index].size, b);
    }

    #[test]
    fn endpoint_closure_retains_buffer_until_drain() {
        let mut output = ByteRing::<BUFFER_CAP>::new();
        assert_eq!(output.push_slice(b"ok"), 2);
        let reply = read_ring(1, 1, &mut output, false, 8);
        assert_eq!(reply.label, PtyMsg::REPLY);
        assert_eq!(reply.words[2], 2);
        assert_eq!(
            read_ring(1, 1, &mut output, false, 8).words[0],
            PtyMsg::ERR_PEER_CLOSED
        );
    }
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[PTY] pty_server started\n");
    let endpoint = endpoint_create();
    nameserver_register("pty", endpoint);
    debug_log("[PTY] registered\n");

    let mut server = PtyServer::new();
    let mut msg = ipc_recv(endpoint);
    loop {
        let reply = handle_message(&mut server, &msg);
        msg = ipc_reply_and_wait(endpoint, reply);
    }
}

fn handle_message(server: &mut PtyServer, msg: &IpcMsg) -> IpcMsg {
    let Some(caller) = pty_caller_credentials(msg.badge) else {
        return error(PtyMsg::ERR_PERMISSION_DENIED);
    };

    match msg.label {
        PtyMsg::CREATE => {
            let size = if msg.words[1] == 0 {
                DEFAULT_WINDOW_SIZE
            } else {
                match TerminalWinsize::from_wire(msg.words[1]) {
                    Some(size) => size,
                    None => return error(PtyMsg::ERR_INVALID_WINDOW_SIZE),
                }
            };
            match server.create(caller, msg.words[0], size) {
                Ok((id, generation, master, control)) => {
                    if !tty_publish_winsize(id, generation, Some(size)) {
                        if let Ok(index) = server.locate(id, generation) {
                            server.sessions[index].destroy();
                        }
                        return error(PtyMsg::ERR_INTERNAL);
                    }
                    debug_log("[PTY] create\n");
                    let mut reply = IpcMsg::with_label(PtyMsg::REPLY)
                        .word(0, id)
                        .word(1, generation);
                    reply.caps[0] = master;
                    reply.caps[1] = control;
                    reply.cap_count = 2;
                    reply
                }
                Err(code) => error(code),
            }
        }
        PtyMsg::ATTACH_SLAVE => attach_slave(server, msg, caller),
        PtyMsg::SET_MODE => set_mode(server, msg, caller),
        PtyMsg::SET_WINDOW_SIZE => set_window_size(server, msg, caller),
        PtyMsg::GET_WINDOW_SIZE => get_window_size(server, msg, caller),
        PtyMsg::SET_FOREGROUND_PROCESS => set_foreground_process(server, msg, caller),
        PtyMsg::GET_FOREGROUND_PROCESS => get_foreground_process(server, msg, caller),
        PtyMsg::WRITE_MASTER => write_master(server, msg, caller),
        PtyMsg::WRITE_SLAVE => write_slave(server, msg, caller),
        PtyMsg::READ_MASTER => read_master(server, msg, caller),
        PtyMsg::READ_SLAVE => read_slave(server, msg, caller),
        PtyMsg::CLOSE_MASTER => close_endpoint(server, msg, caller, PtyRole::Master),
        PtyMsg::CLOSE_SLAVE => close_endpoint(server, msg, caller, PtyRole::Slave),
        PtyMsg::CLOSE_SESSION => close_session(server, msg, caller),
        PtyMsg::GET_STATE => get_state(server, msg, caller),
        _ => error(PtyMsg::ERR_INVALID_PTY),
    }
}

fn attach_slave(server: &mut PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Control) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let (id, generation, owner_uid, owner_gid, control_open) = {
        let session = &server.sessions[index];
        (
            session.id,
            session.generation,
            session.owner.uid,
            session.owner.gid,
            session.control_open,
        )
    };
    if !control_open {
        return error(PtyMsg::ERR_SESSION_CLOSING);
    }
    let target_pid = if msg.words[2] == 0 {
        0
    } else {
        let Some(target) = pty_caller_credentials(msg.words[2]) else {
            return error(PtyMsg::ERR_INVALID_PTY);
        };
        if target.uid != owner_uid || target.gid != owner_gid {
            return error(PtyMsg::ERR_PERMISSION_DENIED);
        }
        target.pid
    };
    let token = match server.mint_token(id, generation, target_pid, PtyRole::Slave) {
        Ok(token) => token,
        Err(code) => return error(code),
    };
    let session = &mut server.sessions[index];
    session.slave = Authority {
        token,
        pid: target_pid,
    };
    session.slave_open = true;
    let mut reply = ok_identity(session.id, session.generation);
    reply.caps[0] = token;
    reply.cap_count = 1;
    reply
}

fn set_mode(server: &mut PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Control) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let session = &mut server.sessions[index];
    if !session.control_open {
        return error(PtyMsg::ERR_SESSION_CLOSING);
    }
    session.mode_flags = msg.words[2];
    ok_identity(session.id, session.generation)
}

fn set_window_size(server: &mut PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Control) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let Some(size) = TerminalWinsize::from_wire(msg.words[2]) else {
        return error(PtyMsg::ERR_INVALID_WINDOW_SIZE);
    };
    let session = &mut server.sessions[index];
    if !session.control_open {
        return error(PtyMsg::ERR_SESSION_CLOSING);
    }
    let changed = session.size != size;
    if changed {
        if !tty_publish_winsize(session.id, session.generation, Some(size)) {
            return error(PtyMsg::ERR_INTERNAL);
        }
        session.size = size;
        debug_log("[PTY] resize\n");
    }
    ok_identity(session.id, session.generation).word(2, changed as u64)
}

fn get_window_size(server: &PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server
        .check(msg, caller, PtyRole::Master)
        .or_else(|_| server.check(msg, caller, PtyRole::Slave))
        .or_else(|_| server.check(msg, caller, PtyRole::Control))
    {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let session = &server.sessions[index];
    ok_identity(session.id, session.generation).word(2, session.size.to_wire())
}

fn set_foreground_process(
    server: &mut PtyServer,
    msg: &IpcMsg,
    caller: PtyCallerCredentials,
) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Control) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let target_pid = msg.words[2];
    if target_pid == 0 {
        server.sessions[index].foreground_pid = None;
        return ok_identity(server.sessions[index].id, server.sessions[index].generation);
    }
    let Some(target) = pty_caller_credentials(target_pid) else {
        return error(PtyMsg::ERR_INVALID_PTY);
    };
    let session = &mut server.sessions[index];
    if target.uid != session.owner.uid || target.gid != session.owner.gid {
        return error(PtyMsg::ERR_PERMISSION_DENIED);
    }
    if !tty_attach_process(target.pid, session.id, session.generation) {
        return error(PtyMsg::ERR_INTERNAL);
    }
    session.foreground_pid = Some(target.pid);
    ok_identity(session.id, session.generation)
}

fn get_foreground_process(
    server: &PtyServer,
    msg: &IpcMsg,
    caller: PtyCallerCredentials,
) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Control) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let session = &server.sessions[index];
    ok_identity(session.id, session.generation).word(2, session.foreground_pid.unwrap_or(0))
}

fn write_master(server: &mut PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Master) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let bytes = unpack_payload(msg);
    let session = &mut server.sessions[index];
    if !session.master_open {
        return error(PtyMsg::ERR_PEER_CLOSED);
    }
    if !session.slave_open {
        return error(PtyMsg::ERR_PEER_CLOSED);
    }
    let accepted = session.input.push_slice(bytes);
    if accepted == 0 && !bytes.is_empty() {
        return error(PtyMsg::ERR_WOULD_BLOCK);
    }
    ok_identity(session.id, session.generation).word(2, accepted as u64)
}

fn write_slave(server: &mut PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Slave) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let bytes = unpack_payload(msg);
    let session = &mut server.sessions[index];
    if !session.slave_open {
        return error(PtyMsg::ERR_PEER_CLOSED);
    }
    if !session.master_open {
        return error(PtyMsg::ERR_PEER_CLOSED);
    }
    let accepted = session.output.push_slice(bytes);
    if accepted == 0 && !bytes.is_empty() {
        return error(PtyMsg::ERR_WOULD_BLOCK);
    }
    ok_identity(session.id, session.generation).word(2, accepted as u64)
}

fn read_master(server: &mut PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Master) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let session = &mut server.sessions[index];
    if !session.master_open {
        return error(PtyMsg::ERR_PEER_CLOSED);
    }
    read_ring(
        session.id,
        session.generation,
        &mut session.output,
        session.slave_open,
        msg.words[2] as usize,
    )
}

fn read_slave(server: &mut PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Slave) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let session = &mut server.sessions[index];
    if !session.slave_open {
        return error(PtyMsg::ERR_PEER_CLOSED);
    }
    read_ring(
        session.id,
        session.generation,
        &mut session.input,
        session.master_open,
        msg.words[2] as usize,
    )
}

fn read_ring(
    id: u64,
    generation: u64,
    ring: &mut ByteRing<BUFFER_CAP>,
    peer_open: bool,
    requested: usize,
) -> IpcMsg {
    if ring.len() == 0 {
        return error(if peer_open {
            PtyMsg::ERR_WOULD_BLOCK
        } else {
            PtyMsg::ERR_PEER_CLOSED
        });
    }
    let mut bytes = [0u8; CHUNK_BYTES];
    let count = ring.pop_slice(&mut bytes[..requested.min(CHUNK_BYTES)]);
    pack_reply(id, generation, &bytes[..count])
}

fn close_endpoint(
    server: &mut PtyServer,
    msg: &IpcMsg,
    caller: PtyCallerCredentials,
    role: PtyRole,
) -> IpcMsg {
    let index = match server.check(msg, caller, role) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let session = &mut server.sessions[index];
    match role {
        PtyRole::Master => session.master_open = false,
        PtyRole::Slave => session.slave_open = false,
        PtyRole::Control => session.control_open = false,
    }
    if !session.master_open && !session.slave_open && !session.control_open {
        let id = session.id;
        let generation = session.generation;
        let _ = tty_publish_winsize(id, generation, None);
        session.destroy();
        debug_log("[PTY] destroy\n");
        return ok_identity(id, generation);
    }
    ok_identity(session.id, session.generation)
}

fn close_session(server: &mut PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server.check(msg, caller, PtyRole::Control) {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let session = &mut server.sessions[index];
    let id = session.id;
    let generation = session.generation;
    let _ = tty_publish_winsize(id, generation, None);
    session.destroy();
    debug_log("[PTY] destroy\n");
    ok_identity(id, generation)
}

fn get_state(server: &PtyServer, msg: &IpcMsg, caller: PtyCallerCredentials) -> IpcMsg {
    let index = match server
        .check(msg, caller, PtyRole::Master)
        .or_else(|_| server.check(msg, caller, PtyRole::Slave))
        .or_else(|_| server.check(msg, caller, PtyRole::Control))
    {
        Ok(index) => index,
        Err(code) => return error(code),
    };
    let session = &server.sessions[index];
    ok_identity(session.id, session.generation)
        .word(2, session.state_bits())
        .word(3, session.owner.creator_pid)
}

fn error(code: u64) -> IpcMsg {
    IpcMsg::with_label(PtyMsg::ERROR).word(0, code)
}

fn ok_identity(id: u64, generation: u64) -> IpcMsg {
    IpcMsg::with_label(PtyMsg::REPLY)
        .word(0, id)
        .word(1, generation)
}

fn pack_reply(id: u64, generation: u64, bytes: &[u8]) -> IpcMsg {
    ok_identity(id, generation)
        .word(2, bytes.len() as u64)
        .word(3, pack_u64(bytes))
}

fn unpack_payload(msg: &IpcMsg) -> &[u8] {
    let length = (msg.words[2] as usize).min(CHUNK_BYTES);
    // words 0..2 identify the session and encode the length; word 3 carries
    // the bytes. The authority remains in caps[0].
    unsafe { core::slice::from_raw_parts(&msg.words[3] as *const u64 as *const u8, length) }
}

fn pack_u64(bytes: &[u8]) -> u64 {
    let mut word = 0u64;
    for (index, byte) in bytes.iter().take(8).enumerate() {
        word |= (*byte as u64) << (index * 8);
    }
    word
}
