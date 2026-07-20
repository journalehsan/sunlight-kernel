//! Bounded TCP socket table for `net_server`.

extern crate alloc;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};

const RX_BUF: usize = 8192;
const TX_BUF: usize = 4096;
const MAX_SOCKETS: usize = 128;
const MAX_BACKLOG: usize = 32;
const CONNECT_TIMEOUT_MS: u64 = 20_000;
const HALF_CLOSE_TIMEOUT_MS: u64 = 30_000;
const CLOSE_TIMEOUT_MS: u64 = 5_000;
const TERMINAL_REAP_DELAY_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpError {
    Timeout,
    WouldBlock,
    Refused,
    Reset,
    Closed,
    SocketError,
    NoSlots,
    InvalidSocket,
    AccessDenied,
    AddressInUse,
    InvalidState,
    NotConnected,
    BacklogFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketIdentity(pub u64);

impl SocketIdentity {
    pub const INVALID: Self = Self(0);

    pub const fn id(self) -> u32 {
        self.0 as u32
    }

    pub const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketReady(u32);

impl SocketReady {
    pub const ACCEPT: u32 = 1 << 0;
    pub const READ: u32 = 1 << 1;
    pub const WRITE: u32 = 1 << 2;
    pub const EOF: u32 = 1 << 3;
    pub const RESET: u32 = 1 << 4;
    pub const CLOSED: u32 = 1 << 5;
    pub const ERROR: u32 = 1 << 6;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    fn insert(&mut self, bits: u32) {
        self.0 |= bits;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalEndpoint {
    addr: [u8; 4],
    port: u16,
}

#[derive(Debug)]
enum SocketKind {
    Unbound,
    Listener {
        endpoint: LocalEndpoint,
        backlog: usize,
        pending: VecDeque<SocketIdentity>,
    },
    Connected,
}

struct TcpSlot {
    generation: u32,
    owner_pid: u64,
    handle: SocketHandle,
    kind: SocketKind,
    peer_closed: bool,
    reset: bool,
    local_closed: bool,
    last_tcp_state: tcp::State,
    deadline_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TcpDiagnostics {
    pub socket_alloc_total: u64,
    pub socket_release_total: u64,
    pub socket_live: u64,
    pub socket_peak_live: u64,
    pub listener_alloc_total: u64,
    pub listener_release_total: u64,
    pub listener_live: u64,
    pub stream_alloc_total: u64,
    pub stream_release_total: u64,
    pub stream_live: u64,
    pub rx_buffers_live: u64,
    pub tx_buffers_live: u64,
    pub rx_bytes_reserved: u64,
    pub tx_bytes_reserved: u64,
    pub allocation_failures_total: u64,
    pub allocation_rollbacks_total: u64,
    pub failed_connect_cleanup_total: u64,
    pub peer_reset_reaps_total: u64,
    pub half_close_reaps_total: u64,
    pub close_deadline_reaps_total: u64,
    pub owner_exit_reaps_total: u64,
}

pub struct TcpManager {
    slots: BTreeMap<u32, TcpSlot>,
    next_generation: u32,
    next_local_port: u16,
    diagnostics: TcpDiagnostics,
}

impl TcpManager {
    pub fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
            next_generation: 1,
            next_local_port: 49_152,
            diagnostics: TcpDiagnostics::default(),
        }
    }

    pub fn alloc_socket(
        &mut self,
        owner_pid: u64,
        sockets: &mut SocketSet<'static>,
    ) -> Result<SocketIdentity, TcpError> {
        if self.slots.len() >= MAX_SOCKETS {
            return Err(TcpError::NoSlots);
        }

        let socket_id = self.allocate_id()?;
        let generation = self.allocate_generation();
        let rx = match allocate_buffer(RX_BUF) {
            Ok(buffer) => buffer,
            Err(error) => {
                self.diagnostics.allocation_failures_total =
                    self.diagnostics.allocation_failures_total.saturating_add(1);
                return Err(error);
            }
        };
        let tx = match allocate_buffer(TX_BUF) {
            Ok(buffer) => buffer,
            Err(error) => {
                drop(rx);
                self.diagnostics.allocation_failures_total =
                    self.diagnostics.allocation_failures_total.saturating_add(1);
                self.diagnostics.allocation_rollbacks_total = self
                    .diagnostics
                    .allocation_rollbacks_total
                    .saturating_add(1);
                return Err(error);
            }
        };
        let handle = sockets.add(tcp::Socket::new(
            tcp::SocketBuffer::new(rx),
            tcp::SocketBuffer::new(tx),
        ));
        self.slots.insert(
            socket_id,
            TcpSlot {
                generation,
                owner_pid,
                handle,
                kind: SocketKind::Unbound,
                peer_closed: false,
                reset: false,
                local_closed: false,
                last_tcp_state: tcp::State::Closed,
                deadline_ms: None,
            },
        );
        self.diagnostics.socket_alloc_total = self.diagnostics.socket_alloc_total.saturating_add(1);
        self.diagnostics.socket_live = self.diagnostics.socket_live.saturating_add(1);
        self.diagnostics.socket_peak_live = self
            .diagnostics
            .socket_peak_live
            .max(self.diagnostics.socket_live);
        self.diagnostics.rx_buffers_live = self.diagnostics.rx_buffers_live.saturating_add(1);
        self.diagnostics.tx_buffers_live = self.diagnostics.tx_buffers_live.saturating_add(1);
        self.diagnostics.rx_bytes_reserved = self
            .diagnostics
            .rx_bytes_reserved
            .saturating_add(RX_BUF as u64);
        self.diagnostics.tx_bytes_reserved = self
            .diagnostics
            .tx_bytes_reserved
            .saturating_add(TX_BUF as u64);
        Ok(Self::identity(socket_id, generation))
    }

    pub fn bind(
        &mut self,
        owner_pid: u64,
        identity: SocketIdentity,
        addr: [u8; 4],
        port: u16,
        sockets: &SocketSet<'static>,
    ) -> Result<(), TcpError> {
        if port == 0 {
            return Err(TcpError::SocketError);
        }
        self.slot_for(identity, owner_pid, sockets)?;
        if self.slots.iter().any(|(&id, slot)| {
            id != identity.id()
                && matches!(
                    slot.kind,
                    SocketKind::Listener { endpoint, .. }
                        if endpoint.port == port && addresses_conflict(endpoint.addr, addr)
                )
        }) {
            return Err(TcpError::AddressInUse);
        }
        let slot = self.slot_mut(identity, owner_pid)?;
        if !matches!(slot.kind, SocketKind::Unbound)
            || sockets.get::<tcp::Socket>(slot.handle).is_open()
        {
            return Err(TcpError::InvalidState);
        }
        slot.kind = SocketKind::Listener {
            endpoint: LocalEndpoint { addr, port },
            backlog: 0,
            pending: VecDeque::new(),
        };
        self.diagnostics.listener_alloc_total =
            self.diagnostics.listener_alloc_total.saturating_add(1);
        self.diagnostics.listener_live = self.diagnostics.listener_live.saturating_add(1);
        Ok(())
    }

    pub fn listen(
        &mut self,
        owner_pid: u64,
        identity: SocketIdentity,
        backlog: usize,
        sockets: &mut SocketSet<'static>,
    ) -> Result<(), TcpError> {
        let slot = self.slot_mut(identity, owner_pid)?;
        let SocketKind::Listener {
            endpoint,
            backlog: existing_backlog,
            ..
        } = &mut slot.kind
        else {
            return Err(TcpError::InvalidState);
        };
        if backlog == 0 {
            return Err(TcpError::SocketError);
        }
        *existing_backlog = backlog.min(MAX_BACKLOG);
        sockets
            .get_mut::<tcp::Socket>(slot.handle)
            .listen(IpListenEndpoint {
                addr: ipv4_listen_addr(endpoint.addr),
                port: endpoint.port,
            })
            .map_err(|_| TcpError::SocketError)
    }

    pub fn connect<D: Device>(
        &mut self,
        owner_pid: u64,
        identity: SocketIdentity,
        remote_ip: [u8; 4],
        remote_port: u16,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        _device: &mut D,
    ) -> Result<(), TcpError> {
        let local_port = self.next_local_port;
        self.next_local_port = self.next_local_port.wrapping_add(1).max(49_152);
        let slot = self.slot_mut(identity, owner_pid)?;
        if !matches!(slot.kind, SocketKind::Unbound) {
            return Err(TcpError::InvalidState);
        }
        sockets
            .get_mut::<tcp::Socket>(slot.handle)
            .connect(
                iface.context(),
                IpEndpoint::new(
                    IpAddress::Ipv4(Ipv4Address::new(
                        remote_ip[0],
                        remote_ip[1],
                        remote_ip[2],
                        remote_ip[3],
                    )),
                    remote_port,
                ),
                local_port,
            )
            .map_err(|_| TcpError::SocketError)?;
        slot.kind = SocketKind::Connected;
        slot.deadline_ms =
            Some(sunlight_ipc::monotonic_millis().saturating_add(CONNECT_TIMEOUT_MS));
        self.diagnostics.stream_alloc_total = self.diagnostics.stream_alloc_total.saturating_add(1);
        self.diagnostics.stream_live = self.diagnostics.stream_live.saturating_add(1);
        Ok(())
    }

    pub fn progress<D: Device>(
        &mut self,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
        now_ms: u64,
    ) {
        iface.poll(Instant::from_millis(now_ms as i64), device, sockets);

        let listeners: Vec<SocketIdentity> = self
            .slots
            .iter()
            .filter_map(|(&id, slot)| {
                matches!(slot.kind, SocketKind::Listener { .. })
                    .then_some(Self::identity(id, slot.generation))
            })
            .collect();
        for listener in listeners {
            self.queue_completed_accept(listener, sockets);
        }

        let mut reap = Vec::new();
        for (&id, slot) in self.slots.iter_mut() {
            let socket = sockets.get::<tcp::Socket>(slot.handle);
            if matches!(slot.kind, SocketKind::Connected) {
                match socket.state() {
                    tcp::State::Established => {
                        if !slot.local_closed {
                            slot.deadline_ms = None;
                        }
                    }
                    tcp::State::CloseWait => {
                        if socket.can_recv() {
                            slot.peer_closed = false;
                        } else {
                            slot.peer_closed = true;
                            slot.deadline_ms = Some(now_ms.saturating_add(HALF_CLOSE_TIMEOUT_MS));
                        }
                    }
                    tcp::State::Closed | tcp::State::TimeWait => {
                        if !matches!(slot.last_tcp_state, tcp::State::CloseWait) {
                            slot.reset = true;
                        } else if socket.can_recv() {
                            slot.peer_closed = false;
                        } else {
                            slot.peer_closed = true;
                        }
                        slot.deadline_ms = Some(now_ms.saturating_add(TERMINAL_REAP_DELAY_MS));
                    }
                    _ => {}
                }
                slot.last_tcp_state = socket.state();
                if slot.deadline_ms.is_some_and(|deadline| now_ms >= deadline) {
                    reap.push((
                        Self::identity(id, slot.generation),
                        slot.reset,
                        slot.peer_closed,
                        slot.local_closed,
                    ));
                }
            }
        }
        for (identity, reset, half_closed, local_closed) in reap {
            if reset {
                self.diagnostics.peer_reset_reaps_total =
                    self.diagnostics.peer_reset_reaps_total.saturating_add(1);
            } else if half_closed {
                self.diagnostics.half_close_reaps_total =
                    self.diagnostics.half_close_reaps_total.saturating_add(1);
            } else if local_closed {
                self.diagnostics.close_deadline_reaps_total = self
                    .diagnostics
                    .close_deadline_reaps_total
                    .saturating_add(1);
            } else {
                self.diagnostics.failed_connect_cleanup_total = self
                    .diagnostics
                    .failed_connect_cleanup_total
                    .saturating_add(1);
            }
            let _ = self.close_internal(identity, None, sockets);
        }
    }

    pub fn accept(
        &mut self,
        owner_pid: u64,
        identity: SocketIdentity,
        sockets: &SocketSet<'static>,
    ) -> Result<SocketIdentity, TcpError> {
        let slot = self.slot_for(identity, owner_pid, sockets)?;
        let SocketKind::Listener { pending, .. } = &slot.kind else {
            return Err(TcpError::InvalidState);
        };
        pending.front().copied().ok_or(TcpError::WouldBlock)
    }

    pub fn take_accepted(
        &mut self,
        owner_pid: u64,
        identity: SocketIdentity,
    ) -> Result<SocketIdentity, TcpError> {
        let slot = self.slot_mut(identity, owner_pid)?;
        let SocketKind::Listener { pending, .. } = &mut slot.kind else {
            return Err(TcpError::InvalidState);
        };
        pending.pop_front().ok_or(TcpError::WouldBlock)
    }

    pub fn send(
        &mut self,
        owner_pid: u64,
        identity: SocketIdentity,
        data: &[u8],
        sockets: &mut SocketSet<'static>,
    ) -> Result<usize, TcpError> {
        let slot = self.slot_mut(identity, owner_pid)?;
        if slot.local_closed {
            return Err(TcpError::Closed);
        }
        if slot.reset {
            return Err(TcpError::Reset);
        }
        let socket = sockets.get_mut::<tcp::Socket>(slot.handle);
        if !socket.may_send() {
            return Err(if slot.peer_closed {
                TcpError::Closed
            } else {
                TcpError::NotConnected
            });
        }
        match socket.send_slice(data) {
            Ok(0) => Err(TcpError::WouldBlock),
            Ok(sent) => Ok(sent),
            Err(tcp::SendError::InvalidState) => Err(TcpError::NotConnected),
        }
    }

    pub fn recv(
        &mut self,
        owner_pid: u64,
        identity: SocketIdentity,
        max_len: usize,
        sockets: &mut SocketSet<'static>,
    ) -> Result<Vec<u8>, TcpError> {
        let take = max_len.max(1);
        let slot = self.slot_mut(identity, owner_pid)?;
        if slot.local_closed {
            return Err(TcpError::Closed);
        }
        if slot.reset {
            return Err(TcpError::Reset);
        }

        let socket = sockets.get_mut::<tcp::Socket>(slot.handle);
        if socket.can_recv() {
            let mut drained = Vec::new();
            drained
                .try_reserve_exact(take)
                .map_err(|_| TcpError::NoSlots)?;
            socket
                .recv(|buf| {
                    let count = buf.len().min(take);
                    drained.extend_from_slice(&buf[..count]);
                    (count, ())
                })
                .map_err(|_| TcpError::SocketError)?;
            return Ok(drained);
        }
        if !socket.may_recv() || slot.peer_closed {
            slot.peer_closed = true;
            return Ok(Vec::new());
        }
        Err(TcpError::WouldBlock)
    }

    pub fn ready(
        &self,
        owner_pid: u64,
        identity: SocketIdentity,
        sockets: &SocketSet<'static>,
    ) -> Result<SocketReady, TcpError> {
        let slot = self.slot_for(identity, owner_pid, sockets)?;
        let mut ready = SocketReady::empty();
        match &slot.kind {
            SocketKind::Listener { pending, .. } => {
                if !pending.is_empty() {
                    ready.insert(SocketReady::ACCEPT);
                }
            }
            SocketKind::Connected => {
                let socket = sockets.get::<tcp::Socket>(slot.handle);
                match socket.state() {
                    tcp::State::Established | tcp::State::CloseWait => {
                        if socket.can_recv() {
                            ready.insert(SocketReady::READ);
                        }
                        if socket.can_send() {
                            ready.insert(SocketReady::WRITE);
                        }
                        if slot.peer_closed
                            || (matches!(socket.state(), tcp::State::CloseWait)
                                && !socket.can_recv())
                        {
                            ready.insert(SocketReady::EOF);
                        }
                    }
                    tcp::State::Closed | tcp::State::TimeWait => {
                        ready.insert(SocketReady::CLOSED | SocketReady::EOF);
                    }
                    _ => {}
                }
                if slot.reset {
                    ready.insert(SocketReady::RESET);
                }
                if slot.local_closed {
                    ready.insert(SocketReady::CLOSED);
                }
            }
            SocketKind::Unbound => {}
        }
        Ok(ready)
    }

    pub fn close(
        &mut self,
        owner_pid: u64,
        identity: SocketIdentity,
        sockets: &mut SocketSet<'static>,
    ) -> Result<(), TcpError> {
        let slot = self.slot_mut(identity, owner_pid)?;
        if matches!(slot.kind, SocketKind::Connected) {
            if !slot.local_closed {
                sockets.get_mut::<tcp::Socket>(slot.handle).close();
                slot.local_closed = true;
                slot.deadline_ms =
                    Some(sunlight_ipc::monotonic_millis().saturating_add(CLOSE_TIMEOUT_MS));
            }
            return Ok(());
        }
        self.close_internal(identity, Some(owner_pid), sockets)
    }

    pub fn close_owner(&mut self, owner_pid: u64, sockets: &mut SocketSet<'static>) {
        let identities: Vec<SocketIdentity> = self
            .slots
            .iter()
            .filter_map(|(&id, slot)| {
                (slot.owner_pid == owner_pid).then_some(Self::identity(id, slot.generation))
            })
            .collect();
        for identity in identities {
            self.diagnostics.owner_exit_reaps_total =
                self.diagnostics.owner_exit_reaps_total.saturating_add(1);
            let _ = self.close_internal(identity, None, sockets);
        }
    }

    pub fn reap_dead_owners<F>(&mut self, sockets: &mut SocketSet<'static>, mut is_alive: F)
    where
        F: FnMut(u64) -> bool,
    {
        let owners: Vec<u64> = self
            .slots
            .values()
            .filter_map(|slot| (!is_alive(slot.owner_pid)).then_some(slot.owner_pid))
            .collect();
        for owner_pid in owners {
            self.close_owner(owner_pid, sockets);
        }
    }

    pub fn active_count(&self) -> usize {
        self.slots.len()
    }

    pub const fn diagnostics(&self) -> TcpDiagnostics {
        self.diagnostics
    }

    fn queue_completed_accept(
        &mut self,
        listener: SocketIdentity,
        sockets: &mut SocketSet<'static>,
    ) {
        let Ok(slot) = self.slot_for(listener, 0, sockets) else {
            return;
        };
        let (owner_pid, endpoint, backlog, pending_len, state) = match &slot.kind {
            SocketKind::Listener {
                endpoint,
                backlog,
                pending,
            } => (
                slot.owner_pid,
                *endpoint,
                *backlog,
                pending.len(),
                sockets.get::<tcp::Socket>(slot.handle).state(),
            ),
            _ => return,
        };
        if !matches!(state, tcp::State::SynReceived | tcp::State::Established) {
            return;
        }
        if pending_len >= backlog || self.slots.len() >= MAX_SOCKETS {
            if let Ok(slot) = self.slot_mut(listener, owner_pid) {
                sockets.get_mut::<tcp::Socket>(slot.handle).abort();
                let _ = sockets
                    .get_mut::<tcp::Socket>(slot.handle)
                    .listen(IpListenEndpoint {
                        addr: ipv4_listen_addr(endpoint.addr),
                        port: endpoint.port,
                    });
            }
            return;
        }

        let Ok(replacement) = self.alloc_socket(owner_pid, sockets) else {
            return;
        };
        let replacement_id = replacement.id();
        let Some(mut replacement_slot) = self.slots.remove(&replacement_id) else {
            return;
        };
        if sockets
            .get_mut::<tcp::Socket>(replacement_slot.handle)
            .listen(IpListenEndpoint {
                addr: ipv4_listen_addr(endpoint.addr),
                port: endpoint.port,
            })
            .is_err()
        {
            self.release_slot(replacement_slot, sockets);
            return;
        }
        replacement_slot.kind = SocketKind::Listener {
            endpoint,
            backlog,
            pending: VecDeque::new(),
        };

        let listener_id = listener.id();
        let Some(mut connected_slot) = self.slots.remove(&listener_id) else {
            self.release_slot(replacement_slot, sockets);
            return;
        };
        let listener_generation = connected_slot.generation;
        let mut pending = match &mut connected_slot.kind {
            SocketKind::Listener { pending, .. } => core::mem::take(pending),
            _ => VecDeque::new(),
        };
        let _replacement_pending = match replacement_slot.kind {
            SocketKind::Listener {
                ref mut pending, ..
            } => core::mem::take(pending),
            _ => VecDeque::new(),
        };
        connected_slot.kind = SocketKind::Connected;
        connected_slot.peer_closed = false;
        connected_slot.reset = false;
        connected_slot.local_closed = false;
        connected_slot.deadline_ms = None;
        connected_slot.last_tcp_state = sockets.get::<tcp::Socket>(connected_slot.handle).state();
        let client = Self::identity(replacement_id, replacement_slot.generation);
        pending.push_back(client);
        replacement_slot.generation = listener_generation;
        replacement_slot.kind = SocketKind::Listener {
            endpoint,
            backlog,
            pending,
        };
        connected_slot.generation = client.generation();
        replacement_slot.last_tcp_state =
            sockets.get::<tcp::Socket>(replacement_slot.handle).state();
        self.slots.insert(listener_id, replacement_slot);
        self.slots.insert(replacement_id, connected_slot);
        self.diagnostics.stream_alloc_total = self.diagnostics.stream_alloc_total.saturating_add(1);
        self.diagnostics.stream_live = self.diagnostics.stream_live.saturating_add(1);
    }

    fn close_internal(
        &mut self,
        identity: SocketIdentity,
        owner_pid: Option<u64>,
        sockets: &mut SocketSet<'static>,
    ) -> Result<(), TcpError> {
        let slot = self
            .slots
            .get(&identity.id())
            .ok_or(TcpError::InvalidSocket)?;
        if slot.generation != identity.generation() {
            return Err(TcpError::InvalidSocket);
        }
        if owner_pid.is_some_and(|owner| owner != slot.owner_pid) {
            return Err(TcpError::AccessDenied);
        }
        let slot = self
            .slots
            .remove(&identity.id())
            .ok_or(TcpError::InvalidSocket)?;
        self.release_slot(slot, sockets);
        Ok(())
    }

    fn release_slot(&mut self, mut slot: TcpSlot, sockets: &mut SocketSet<'static>) {
        if let SocketKind::Listener { pending, .. } = &mut slot.kind {
            while let Some(client) = pending.pop_front() {
                let _ = self.close_internal(client, None, sockets);
            }
        }
        sockets.get_mut::<tcp::Socket>(slot.handle).abort();
        let removed = sockets.remove(slot.handle);
        drop(removed);
        self.diagnostics.socket_release_total =
            self.diagnostics.socket_release_total.saturating_add(1);
        self.diagnostics.socket_live = self.diagnostics.socket_live.saturating_sub(1);
        self.diagnostics.rx_buffers_live = self.diagnostics.rx_buffers_live.saturating_sub(1);
        self.diagnostics.tx_buffers_live = self.diagnostics.tx_buffers_live.saturating_sub(1);
        self.diagnostics.rx_bytes_reserved = self
            .diagnostics
            .rx_bytes_reserved
            .saturating_sub(RX_BUF as u64);
        self.diagnostics.tx_bytes_reserved = self
            .diagnostics
            .tx_bytes_reserved
            .saturating_sub(TX_BUF as u64);
        match slot.kind {
            SocketKind::Listener { .. } => {
                self.diagnostics.listener_release_total =
                    self.diagnostics.listener_release_total.saturating_add(1);
                self.diagnostics.listener_live = self.diagnostics.listener_live.saturating_sub(1);
            }
            SocketKind::Connected => {
                self.diagnostics.stream_release_total =
                    self.diagnostics.stream_release_total.saturating_add(1);
                self.diagnostics.stream_live = self.diagnostics.stream_live.saturating_sub(1);
            }
            SocketKind::Unbound => {}
        }
    }

    fn slot_for(
        &self,
        identity: SocketIdentity,
        owner_pid: u64,
        _sockets: &SocketSet<'static>,
    ) -> Result<&TcpSlot, TcpError> {
        let slot = self
            .slots
            .get(&identity.id())
            .ok_or(TcpError::InvalidSocket)?;
        if slot.generation != identity.generation() {
            return Err(TcpError::InvalidSocket);
        }
        if owner_pid != 0 && slot.owner_pid != owner_pid {
            return Err(TcpError::AccessDenied);
        }
        Ok(slot)
    }

    fn slot_mut(
        &mut self,
        identity: SocketIdentity,
        owner_pid: u64,
    ) -> Result<&mut TcpSlot, TcpError> {
        let slot = self
            .slots
            .get_mut(&identity.id())
            .ok_or(TcpError::InvalidSocket)?;
        if slot.generation != identity.generation() {
            return Err(TcpError::InvalidSocket);
        }
        if owner_pid != 0 && slot.owner_pid != owner_pid {
            return Err(TcpError::AccessDenied);
        }
        Ok(slot)
    }

    fn allocate_id(&mut self) -> Result<u32, TcpError> {
        for id in 1..=MAX_SOCKETS as u32 {
            if !self.slots.contains_key(&id) {
                return Ok(id);
            }
        }
        Err(TcpError::NoSlots)
    }

    fn allocate_generation(&mut self) -> u32 {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        generation
    }

    const fn identity(id: u32, generation: u32) -> SocketIdentity {
        SocketIdentity(((generation as u64) << 32) | id as u64)
    }
}

fn allocate_buffer(capacity: usize) -> Result<Vec<u8>, TcpError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| TcpError::NoSlots)?;
    buffer.resize(capacity, 0);
    Ok(buffer)
}

fn addresses_conflict(left: [u8; 4], right: [u8; 4]) -> bool {
    left == [0, 0, 0, 0] || right == [0, 0, 0, 0] || left == right
}

fn ipv4_listen_addr(addr: [u8; 4]) -> Option<IpAddress> {
    (addr != [0, 0, 0, 0]).then_some(IpAddress::Ipv4(Ipv4Address::new(
        addr[0], addr[1], addr[2], addr[3],
    )))
}

#[cfg(test)]
mod tests {
    use super::{TcpError, TcpManager};
    use alloc::vec;
    use smoltcp::iface::{SocketSet, SocketStorage};

    fn socket_set() -> SocketSet<'static> {
        SocketSet::new(vec![
            SocketStorage::EMPTY,
            SocketStorage::EMPTY,
            SocketStorage::EMPTY,
            SocketStorage::EMPTY,
        ])
    }

    #[test]
    fn socket_handles_are_owner_scoped() {
        let mut sockets = socket_set();
        let mut manager = TcpManager::new();
        let socket = manager.alloc_socket(41, &mut sockets).unwrap();

        assert_eq!(
            manager.bind(42, socket, [0, 0, 0, 0], 8080, &sockets),
            Err(TcpError::AccessDenied)
        );
        assert!(manager
            .bind(41, socket, [0, 0, 0, 0], 8080, &sockets)
            .is_ok());
    }

    #[test]
    fn wildcard_and_specific_bindings_conflict() {
        let mut sockets = socket_set();
        let mut manager = TcpManager::new();
        let wildcard = manager.alloc_socket(7, &mut sockets).unwrap();
        let specific = manager.alloc_socket(7, &mut sockets).unwrap();

        manager
            .bind(7, wildcard, [0, 0, 0, 0], 2222, &sockets)
            .unwrap();
        assert_eq!(
            manager.bind(7, specific, [10, 0, 2, 15], 2222, &sockets),
            Err(TcpError::AddressInUse)
        );
    }

    #[test]
    fn closed_handle_cannot_be_reused() {
        let mut sockets = socket_set();
        let mut manager = TcpManager::new();
        let socket = manager.alloc_socket(11, &mut sockets).unwrap();

        manager.close(11, socket, &mut sockets).unwrap();
        assert_eq!(
            manager.bind(11, socket, [0, 0, 0, 0], 8080, &sockets),
            Err(TcpError::InvalidSocket)
        );
    }

    #[test]
    fn reused_slot_receives_a_new_generation_and_balances_counters() {
        let mut sockets = socket_set();
        let mut manager = TcpManager::new();
        let original = manager.alloc_socket(11, &mut sockets).unwrap();

        manager.close(11, original, &mut sockets).unwrap();
        let replacement = manager.alloc_socket(11, &mut sockets).unwrap();

        assert_eq!(replacement.id(), original.id());
        assert_ne!(replacement.generation(), original.generation());
        assert_eq!(
            manager.bind(11, original, [0, 0, 0, 0], 8080, &sockets),
            Err(TcpError::InvalidSocket)
        );
        manager.close(11, replacement, &mut sockets).unwrap();

        let diagnostics = manager.diagnostics();
        assert_eq!(diagnostics.socket_live, 0);
        assert_eq!(diagnostics.rx_buffers_live, 0);
        assert_eq!(diagnostics.tx_buffers_live, 0);
        assert_eq!(
            diagnostics.socket_alloc_total - diagnostics.socket_release_total,
            diagnostics.socket_live
        );
    }
}
