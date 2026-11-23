//! Bounded, kernel-only NETLINK_ROUTE discovery. One link dump captures the
//! snapshot used by the following address dump on the same open socket.
use crate::arch::x86_64::syscall::SyscallFrame;
use crate::memory::user::{copy_from_process_bytes, copy_to_process_bytes};
use crate::sched::{Scheduler, SCHEDULER};
use spin::Mutex;
use x86_64::VirtAddr;

use super::fd_table::{CapRights, FileHandle};

const GETLINK: u16 = 18;
const GETADDR: u16 = 22;
// Two aligned link messages (52 + 56 bytes) and the 20-byte terminator.
const MAX_DATAGRAM: usize = 128;
const IFF_UP: u32 = 0x1;
const IFF_BROADCAST: u32 = 0x2;
const IFF_LOOPBACK: u32 = 0x8;
const IFF_RUNNING: u32 = 0x40;
const IFF_MULTICAST: u32 = 0x1000;
const IFF_LOWER_UP: u32 = 0x10000;

#[derive(Clone, Copy)]
struct Address {
    ip: [u8; 4],
    prefix: u8,
}

#[derive(Clone, Copy)]
struct Publication {
    owner: usize,
    generation: u64,
    backend: u64,
    ethernet: Option<Address>,
    loopback: Option<Address>,
}

#[derive(Clone, Copy)]
struct Snapshot {
    config: Publication,
    mac: [u8; 6],
    carrier: bool,
}

static PUBLICATION: Mutex<Option<Publication>> = Mutex::new(None);

fn errno(n: u32) -> u64 {
    sunlight_compat_linux::abi::errno_result(n)
}

fn address(word: u64) -> Result<Option<Address>, ()> {
    if word == 0 {
        return Ok(None);
    }
    let ip = (word as u32).to_le_bytes();
    let prefix = word >> 32;
    if ip == [0; 4] || prefix > 32 {
        return Err(());
    }
    Ok(Some(Address {
        ip,
        prefix: prefix as u8,
    }))
}

fn backend_key(device: &sunlight_net::ActiveNetworkBackend) -> u64 {
    let mut bytes = [0; 8];
    bytes[..6].copy_from_slice(&device.mac());
    u64::from_le_bytes(bytes) | ((device.kind() as u64) << 48)
}

pub fn publish(frame: &SyscallFrame) -> u64 {
    let sched = SCHEDULER.lock();
    let process = sched.current_process();
    if !process.trusted_network_discovery || process.is_linux_compat() || process.is_exiting() {
        return errno(1);
    }
    if frame.rdi != 0 || frame.rsi != 0 {
        return errno(22);
    }
    let (Ok(ethernet), Ok(loopback)) = (address(frame.r9), address(frame.r10)) else {
        return errno(22);
    };
    if ethernet.is_some_and(|a| a.ip[0] == 127) || loopback.is_some_and(|a| a.ip[0] != 127) {
        return errno(22);
    }
    let device = crate::ACTIVE_NET_DEVICE.lock();
    if !device
        .as_ref()
        .is_some_and(|dev| backend_key(dev) == frame.r8)
    {
        return errno(19);
    }
    *PUBLICATION.lock() = Some(Publication {
        owner: process.pid,
        generation: process.address_space.identity().generation,
        backend: frame.r8,
        ethernet,
        loopback,
    });
    1
}

fn snapshot(sched: &Scheduler) -> Option<Snapshot> {
    let device = crate::ACTIVE_NET_DEVICE.lock();
    let dev = device.as_ref()?;
    let config = (*PUBLICATION.lock())?;
    if backend_key(dev) != config.backend
        || !sched.processes.iter().any(|p| {
            p.pid == config.owner
                && p.address_space.identity().generation == config.generation
                && p.trusted_network_discovery
                && !p.is_exiting()
        })
    {
        return None;
    }
    Some(Snapshot {
        config,
        mac: dev.mac(),
        carrier: dev.link_up(),
    })
}

#[derive(Clone, Copy)]
struct Socket {
    refs: usize,
    snapshot: Option<Snapshot>,
    request: Option<(u16, u32)>,
}

const EMPTY: Socket = Socket {
    refs: 0,
    snapshot: None,
    request: None,
};
static SOCKETS: Mutex<[Socket; 64]> = Mutex::new([EMPTY; 64]);

pub fn retain(index: u32) {
    let mut sockets = SOCKETS.lock();
    let socket = &mut sockets[index as usize];
    assert_ne!(socket.refs, 0);
    socket.refs = socket
        .refs
        .checked_add(1)
        .expect("socket reference overflow");
}

pub fn release(index: u32) {
    let mut sockets = SOCKETS.lock();
    let socket = &mut sockets[index as usize];
    assert_ne!(socket.refs, 0);
    socket.refs -= 1;
    if socket.refs == 0 {
        *socket = EMPTY;
    }
}

pub fn socket(kind: u32) -> u64 {
    if kind & 0xf != 3 {
        return errno(94); // SOCK_RAW
    }
    if kind & !(0xf | 0x80000 | 0x800) != 0 {
        return errno(22);
    }
    let mut sched = SCHEDULER.lock();
    if !sched.current_process().is_linux_compat() {
        return errno(38);
    }
    let mut sockets = SOCKETS.lock();
    let Some(index) = sockets.iter().position(|entry| entry.refs == 0) else {
        return errno(105);
    };
    match sched.current_shared_process_mut().fd_table.open(
        FileHandle::route_netlink(index as u32),
        CapRights::new(CapRights::READ | CapRights::WRITE),
        kind & !0xf | 2,
    ) {
        Ok(fd) => {
            sockets[index].refs = 1;
            fd as u64
        }
        Err(_) => errno(24),
    }
}

fn hhdm() -> VirtAddr {
    VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset)
}

fn endpoint(sched: &Scheduler, fd: i32, rights: u64) -> Result<usize, u64> {
    if !sched.current_process().is_linux_compat() {
        return Err(errno(38));
    }
    let entry = sched
        .current_shared_process()
        .fd_table
        .get(fd)
        .ok_or(errno(9))?;
    if !entry.handle.is_route_netlink() {
        return Err(errno(88));
    }
    if !entry.rights.contains(CapRights::new(rights)) {
        return Err(errno(9));
    }
    Ok(entry.handle.route_netlink_index() as usize)
}

pub fn sendto(frame: &SyscallFrame) -> u64 {
    if frame.rdx != 20 || frame.r10 != 0 || frame.r8 != 0 || frame.r9 != 0 {
        return errno(22);
    }
    let sched = SCHEDULER.lock();
    let index = match endpoint(&sched, frame.rdi as i32, CapRights::WRITE) {
        Ok(index) => index,
        Err(error) => return error,
    };
    let mut bytes = [0; 20];
    if copy_from_process_bytes(sched.current_process(), hhdm(), frame.rsi, &mut bytes).is_err() {
        return errno(14);
    }
    let kind = u16::from_le_bytes([bytes[4], bytes[5]]);
    if !matches!(kind, GETLINK | GETADDR) {
        return errno(95);
    }
    if bytes[..4] != 20u32.to_le_bytes()
        || bytes[6..8] != 0x301u16.to_le_bytes()
        || bytes[12..] != [0; 8]
    {
        return errno(22);
    }
    let seq = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let config = if kind == GETLINK {
        let Some(config) = snapshot(&sched) else {
            return errno(100);
        }; // ENETDOWN
        Some(config)
    } else {
        None
    };
    let mut sockets = SOCKETS.lock();
    let socket = &mut sockets[index];
    if socket.request.is_some() {
        return errno(16);
    }
    if kind == GETLINK {
        socket.snapshot = config;
    } else if socket.snapshot.is_none() {
        return errno(22);
    }
    socket.request = Some((kind, seq));
    20
}

pub fn recvfrom(frame: &SyscallFrame) -> u64 {
    // The observed musl call uses MSG_DONTWAIT, with no source address.
    if frame.r10 != 0x40 || frame.r8 != 0 || frame.r9 != 0 {
        return errno(95);
    }
    let sched = SCHEDULER.lock();
    let index = match endpoint(&sched, frame.rdi as i32, CapRights::READ) {
        Ok(index) => index,
        Err(error) => return error,
    };
    let mut sockets = SOCKETS.lock();
    let socket = &mut sockets[index];
    let (Some((kind, seq)), Some(snapshot)) = (socket.request, socket.snapshot) else {
        return errno(11);
    };
    let mut msg = Datagram {
        bytes: [0; MAX_DATAGRAM],
        len: 0,
        seq,
    };
    if kind == GETLINK {
        if snapshot.config.loopback.is_some() {
            msg.link(
                1,
                772,
                IFF_UP | IFF_LOOPBACK | IFF_RUNNING,
                b"lo\0",
                &[0; 6],
            );
        }
        let flags = IFF_UP
            | IFF_BROADCAST
            | IFF_MULTICAST
            | if snapshot.carrier {
                IFF_RUNNING | IFF_LOWER_UP
            } else {
                0
            };
        msg.link(2, 1, flags, b"eth0\0", &snapshot.mac);
    } else {
        if let Some(address) = snapshot.config.loopback {
            msg.address(1, 254, address);
        }
        if let Some(address) = snapshot.config.ethernet {
            msg.address(2, 0, address);
        }
    }
    msg.header(20, 3); // NLMSG_DONE, success status
    msg.put(&[0; 4]);
    if frame.rdx < msg.len as u64 {
        return errno(90); // keep the pending datagram on a short buffer
    }
    if copy_to_process_bytes(
        sched.current_process(),
        hhdm(),
        frame.rsi,
        &msg.bytes[..msg.len],
    )
    .is_err()
    {
        return errno(14);
    }
    socket.request = None;
    if kind == GETADDR {
        socket.snapshot = None;
    }
    msg.len as u64
}

struct Datagram {
    bytes: [u8; MAX_DATAGRAM],
    len: usize,
    seq: u32,
}

impl Datagram {
    fn put(&mut self, bytes: &[u8]) {
        self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    fn header(&mut self, length: u32, kind: u16) {
        self.put(&length.to_le_bytes());
        self.put(&kind.to_le_bytes());
        self.put(&2u16.to_le_bytes()); // NLM_F_MULTI
        self.put(&self.seq.to_le_bytes());
        self.put(&[0; 4]); // kernel sender
    }

    fn attr(&mut self, kind: u16, bytes: &[u8]) {
        self.put(&((bytes.len() + 4) as u16).to_le_bytes());
        self.put(&kind.to_le_bytes());
        self.put(bytes);
        self.len = (self.len + 3) & !3;
    }

    fn link(&mut self, index: u32, hardware: u16, flags: u32, name: &[u8], mac: &[u8; 6]) {
        self.header(32 + ((name.len() as u32 + 7) & !3) + 12, 16);
        self.put(&[0; 2]); // AF_UNSPEC, padding
        self.put(&hardware.to_le_bytes());
        self.put(&index.to_le_bytes());
        self.put(&flags.to_le_bytes());
        self.put(&[0; 4]); // change mask
        self.attr(3, name); // IFLA_IFNAME
        self.attr(1, mac); // IFLA_ADDRESS
    }

    fn address(&mut self, index: u32, scope: u8, address: Address) {
        self.header(32, 20); // RTM_NEWADDR
        self.put(&[2, address.prefix, 0, scope]);
        self.put(&index.to_le_bytes());
        self.attr(2, &address.ip); // IFA_LOCAL
    }
}
