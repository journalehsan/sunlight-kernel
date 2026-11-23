//! Helios-only virtual nodes. An open description owns a bounded sample;
//! path lookup and every use of per-process data authorize the actual caller.
use alloc::{format, string::String, vec::Vec};
use core::fmt::Write;
use core::sync::atomic::{AtomicUsize, Ordering};
use sunlight_fs::vfs::{mode, FileStat, FileType};

use crate::{process::Process, sched::Scheduler, timekeeping};

const MAX_OPEN: usize = 128;
const MAX_PIDS: usize = 128;
const MAX_TASK_SCAN: usize = 512;
const MAX_BYTES: usize = 8192;
const HZ: u64 = 100;
const _: () = assert!(HZ == timekeeping::TICK_HZ);
const NS_PER_CLOCK_TICK: u64 = 1_000_000_000 / HZ;
static OPEN: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Identity {
    pid: usize,
    incarnation: u64,
    vm: u64,
}

impl Identity {
    pub(crate) fn of(p: &Process) -> Self {
        Self {
            pid: p.pid,
            incarnation: p.foreground.incarnation,
            vm: p.address_space.identity().generation,
        }
    }
}

// Republish the existing resident page walk result, keyed by the full live
// identity. A sample cannot bind to a reused PID or to a replacement image.
static RSS: spin::Mutex<(u64, Vec<(Identity, usize)>)> = spin::Mutex::new((0, Vec::new()));

pub(crate) fn publish_rss(tick: u64, rows: impl Iterator<Item = (Identity, usize)>) {
    *RSS.lock() = (tick, rows.collect());
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Node {
    Root,
    Cpu,
    Memory,
    Uptime,
    Net,
    NetDev,
    EthernetRxBytes,
    EthernetTxBytes,
    Pid(Identity),
    Comm(Identity),
    Args(Identity),
    Status(Identity),
    Stat(Identity),
    WaitCheckpoint(Identity),
}

const ETHERNET_RX: &str = "/sys/class/net/eth0/statistics/rx_bytes";
const ETHERNET_TX: &str = "/sys/class/net/eth0/statistics/tx_bytes";

pub(crate) fn is_path(path: &str) -> bool {
    path == "/proc"
        || path.starts_with("/proc/")
        || matches!(path.trim_end_matches('/'), ETHERNET_RX | ETHERNET_TX)
}

fn ethernet_counters(wait: bool) -> Result<sunlight_net::NetDeviceCounters, u64> {
    let device = if wait {
        // NetInfo drops the device lock before copyout to respect the
        // scheduler -> device order used by the virtual file dispatcher.
        crate::ACTIVE_NET_DEVICE.lock()
    } else {
        crate::ACTIVE_NET_DEVICE.try_lock().ok_or(11u64)?
    };
    Ok(device.as_ref().ok_or(19u64)?.counters())
}

fn visible(s: &Scheduler, idx: usize) -> bool {
    let p = &s.processes[idx];
    let caller = s.current_process();
    !p.is_exiting()
        && p.is_linux_compat()
        && p.linux_tgid.unwrap_or(p.pid) == p.pid
        && caller.uid == p.uid
        && (caller.linux_tgid.unwrap_or(caller.pid) == p.pid
            || s.current_process_index()
                .is_some_and(|i| s.can_signal_foreground(i, idx)))
}

fn find(s: &Scheduler, pid: usize) -> Option<usize> {
    if s.processes.len() > MAX_TASK_SCAN {
        return None;
    }
    s.processes
        .iter()
        .position(|p| p.pid == pid)
        .filter(|&idx| visible(s, idx))
}

fn same_linux_group(s: &Scheduler, idx: usize) -> bool {
    let caller = s.current_process();
    let target = &s.processes[idx];
    caller.is_linux_compat() && target.is_linux_compat() && !target.is_exiting()
        && caller.linux_tgid.unwrap_or(caller.pid) == target.linux_tgid.unwrap_or(target.pid)
        && caller.address_space.identity() == target.address_space.identity()
}

impl Node {
    pub(crate) fn lookup(s: &Scheduler, path: &str) -> Result<Self, u64> {
        match path.trim_end_matches('/') {
            ETHERNET_RX => {
                ethernet_counters(true)?;
                return Ok(Self::EthernetRxBytes);
            }
            ETHERNET_TX => {
                ethernet_counters(true)?;
                return Ok(Self::EthernetTxBytes);
            }
            _ => {}
        }
        if option_env!("SUNLIGHT_BTOP06_WAIT_TRACE") == Some("1") {
            if let Some(tid) = path.strip_prefix("/proc/self/task/")
                .and_then(|path| path.strip_suffix("/syscall"))
                .and_then(|tid| tid.parse().ok())
            {
                let idx = s.process_index_by_pid(tid).ok_or(2u64)?;
                if !same_linux_group(s, idx) {
                    return Err(2);
                }
                return Ok(Self::WaitCheckpoint(Identity::of(&s.processes[idx])));
            }
        }
        let mut parts = path.split('/').filter(|p| !p.is_empty());
        if parts.next() != Some("proc") {
            return Err(2);
        }
        let Some(name) = parts.next() else {
            return Ok(Self::Root);
        };
        let child = parts.next();
        if parts.next().is_some() {
            return Err(2);
        }
        match (name, child) {
            ("net", None) => return Ok(Self::Net),
            ("net", Some("dev")) => return Ok(Self::NetDev),
            ("stat", None) => return Ok(Self::Cpu),
            ("meminfo", None) => return Ok(Self::Memory),
            ("uptime", None) => return Ok(Self::Uptime),
            ("stat" | "meminfo" | "uptime", Some(_)) => return Err(20),
            _ => {}
        }
        if s.processes.len() > MAX_TASK_SCAN {
            return Err(75);
        }
        let pid = if name == "self" {
            let p = s.current_process();
            p.linux_tgid.unwrap_or(p.pid)
        } else {
            if !name.bytes().all(|b| b.is_ascii_digit()) || name.starts_with('0') {
                return Err(2);
            }
            name.parse().map_err(|_| 2u64)?
        };
        let id = Identity::of(&s.processes[find(s, pid).ok_or(2u64)?]);
        match child {
            None => Ok(Self::Pid(id)),
            Some("comm") => Ok(Self::Comm(id)),
            Some("cmdline") => Ok(Self::Args(id)),
            Some("status") => Ok(Self::Status(id)),
            Some("stat") => Ok(Self::Stat(id)),
            _ => Err(2),
        }
    }

    fn identity(self) -> Option<Identity> {
        match self {
            Self::Pid(id) | Self::Comm(id) | Self::Args(id) | Self::Status(id) | Self::Stat(id)
            | Self::WaitCheckpoint(id) => {
                Some(id)
            }
            _ => None,
        }
    }

    pub(crate) fn live(self, s: &Scheduler) -> bool {
        if let Self::WaitCheckpoint(id) = self {
            return s.process_index_by_pid(id.pid).is_some_and(|idx| {
                same_linux_group(s, idx) && Identity::of(&s.processes[idx]) == id
            });
        }
        s.current_process().is_linux_compat()
            && self.identity().is_none_or(|id| {
                find(s, id.pid).is_some_and(|idx| Identity::of(&s.processes[idx]) == id)
            })
    }

    pub(crate) fn directory(self) -> bool {
        matches!(self, Self::Root | Self::Net | Self::Pid(_))
    }

    pub(crate) fn path(self) -> String {
        match self {
            Self::Root => String::from("/proc"),
            Self::Cpu => String::from("/proc/stat"),
            Self::Memory => String::from("/proc/meminfo"),
            Self::Uptime => String::from("/proc/uptime"),
            Self::Net => String::from("/proc/net"),
            Self::NetDev => String::from("/proc/net/dev"),
            Self::EthernetRxBytes => String::from(ETHERNET_RX),
            Self::EthernetTxBytes => String::from(ETHERNET_TX),
            Self::Pid(id) => format!("/proc/{}", id.pid),
            Self::Comm(id) => format!("/proc/{}/comm", id.pid),
            Self::Args(id) => format!("/proc/{}/cmdline", id.pid),
            Self::Status(id) => format!("/proc/{}/status", id.pid),
            Self::WaitCheckpoint(id) => format!("/proc/self/task/{}/syscall", id.pid),
            Self::Stat(id) => format!("/proc/{}/stat", id.pid),
        }
    }

    pub(crate) fn stat(self, s: &Scheduler) -> FileStat {
        let owner = self
            .identity()
            .and_then(|id| find(s, id.pid))
            .map(|idx| &s.processes[idx]);
        let directory = self.directory();
        FileStat {
            file_type: if directory {
                FileType::Directory
            } else {
                FileType::File
            },
            size: 0,
            uid: owner.map_or(0, |p| p.uid),
            gid: owner.map_or(0, |p| p.gid),
            mode: if directory {
                mode::S_IFDIR | 0o555
            } else {
                mode::S_IFREG | 0o444
            },
            nlinks: if directory { 2 } else { 1 },
        }
    }

    pub(crate) fn ino(self) -> u64 {
        let kind = match self {
            Self::Root => 1,
            Self::Cpu => 2,
            Self::Memory => 3,
            Self::Uptime => 4,
            Self::Pid(_) => 5,
            Self::Comm(_) => 6,
            Self::Args(_) => 7,
            Self::Status(_) => 8,
            Self::Stat(_) => 9,
            Self::Net => 11,
            Self::NetDev => 12,
            Self::EthernetRxBytes => 13,
            Self::EthernetTxBytes => 14,
            Self::WaitCheckpoint(_) => 15,
        };
        self.identity()
            .map_or(kind, |id| {
                id.incarnation.rotate_left(13) ^ id.vm.rotate_left(7) ^ kind
            })
            .max(1)
    }
}

pub(crate) struct OpenFile {
    pub(crate) node: Node,
    pub(crate) bytes: Vec<u8>,
    pids: Vec<Identity>,
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        OPEN.fetch_sub(1, Ordering::Relaxed);
    }
}

impl OpenFile {
    pub(crate) fn new(s: &Scheduler, node: Node) -> Result<Self, u64> {
        OPEN.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            (n < MAX_OPEN).then_some(n + 1)
        })
        .map_err(|_| 23u64)?;
        let mut sample = Self {
            node,
            bytes: Vec::new(),
            pids: Vec::new(),
        };
        if node == Node::Root {
            if s.processes.len() > MAX_TASK_SCAN {
                return Err(75);
            }
            for (idx, p) in s.processes.iter().enumerate() {
                if visible(s, idx) {
                    if sample.pids.len() == MAX_PIDS {
                        return Err(75);
                    }
                    sample.pids.push(Identity::of(p));
                }
            }
            sample.pids.sort_unstable_by_key(|p| p.pid);
        } else if !node.directory() && !matches!(node, Node::WaitCheckpoint(_)) {
            sample.bytes = render(s, node)?;
            if sample.bytes.len() > MAX_BYTES {
                return Err(75);
            }
        }
        Ok(sample)
    }

    /// The test-only observer reads live wait state, unlike snapshot nodes.
    /// It exposes no arguments, pointers, or register contents.
    pub(crate) fn read_bytes<'a>(&'a self, s: &Scheduler) -> &'a [u8] {
        if let Node::WaitCheckpoint(id) = self.node {
            let waiting = s.process_index_by_pid(id.pid).is_some_and(|idx| {
                let target = &s.processes[idx];
                target.state == crate::process::ProcessState::BlockedOnTimer
                    && target.linux_signal.wait.is_some()
            });
            return if waiting { b"270\n" } else { b"running\n" };
        }
        &self.bytes
    }

    pub(crate) fn entry_count(&self) -> usize {
        match self.node {
            Node::Root => 7 + self.pids.len(),
            Node::Net => 3,
            Node::Pid(_) => 6,
            _ => 0,
        }
    }

    // Deleted/reused/inaccessible entries occupy a cursor slot but are skipped.
    pub(crate) fn entry(&self, s: &Scheduler, cursor: usize) -> Option<(String, u8, u64)> {
        use sunlight_compat_linux::abi::{DT_DIR, DT_REG};
        let directory = self.node;
        let (name, node) = match (directory, cursor) {
            (_, 0) => (String::from("."), directory),
            (Node::Root, 1) => return Some((String::from(".."), DT_DIR, 1)),
            (_, 1) => (String::from(".."), Node::Root),
            (Node::Root, 2) => (String::from("stat"), Node::Cpu),
            (Node::Root, 3) => (String::from("meminfo"), Node::Memory),
            (Node::Root, 4) => (String::from("uptime"), Node::Uptime),
            (Node::Root, 5) => return Some((String::from("self"), 10, 10)),
            (Node::Root, 6) => (String::from("net"), Node::Net),
            (Node::Net, 2) => (String::from("dev"), Node::NetDev),
            (Node::Root, n) => {
                let id = *self.pids.get(n - 7)?;
                (format!("{}", id.pid), Node::Pid(id))
            }
            (Node::Pid(id), 2) => (String::from("comm"), Node::Comm(id)),
            (Node::Pid(id), 3) => (String::from("cmdline"), Node::Args(id)),
            (Node::Pid(id), 4) => (String::from("status"), Node::Status(id)),
            (Node::Pid(id), 5) => (String::from("stat"), Node::Stat(id)),
            _ => return None,
        };
        node.live(s).then(|| {
            (
                name,
                if node.directory() { DT_DIR } else { DT_REG },
                node.ino(),
            )
        })
    }
}

fn render(s: &Scheduler, node: Node) -> Result<Vec<u8>, u64> {
    let mut out = String::new();
    match node {
        Node::Cpu | Node::Uptime => {
            let cores = s.online_cores.min(crate::sched::MAX_CORES);
            let mut total_busy = 0u64;
            let mut total_idle = 0u64;
            let mut rows = String::new();
            for cpu in 0..cores {
                let c = crate::sched::accounting::snapshot(cpu).ok_or(11u64)?;
                if c.elapsed_ns == 0 {
                    return Err(11);
                }
                let busy = c.busy_ns / NS_PER_CLOCK_TICK;
                let idle = c.idle_ns / NS_PER_CLOCK_TICK;
                total_busy = total_busy.saturating_add(busy);
                total_idle = total_idle.saturating_add(idle);
                if node == Node::Cpu {
                    let _ = writeln!(rows, "cpu{cpu} 0 0 {busy} {idle} 0 0 0 0 0 0");
                }
            }
            if node == Node::Cpu {
                let _ = writeln!(out, "cpu  0 0 {total_busy} {total_idle} 0 0 0 0 0 0");
                out.push_str(&rows);
                let wall = crate::arch::x86_64::rtc::unix_time();
                if wall != u64::MAX {
                    let _ = writeln!(
                        out,
                        "btime {}",
                        wall.saturating_sub(timekeeping::uptime_secs())
                    );
                }
            } else {
                let up = timekeeping::global_ticks();
                let _ = writeln!(
                    out,
                    "{}.{:02} {}.{:02}",
                    up / HZ,
                    up % HZ,
                    total_idle / HZ,
                    total_idle % HZ
                );
            }
        }
        Node::NetDev => {
            let c = ethernet_counters(false)?;
            out.push_str("Inter-|   Receive                                                |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n");
            let _ = writeln!(
                out,
                "  eth0: {} {} {} {} 0 0 0 0 {} {} {} 0 0 0 0 0",
                c.rx_bytes,
                c.rx_delivered,
                c.rx_errors.saturating_add(c.rx_bad_completion),
                c.rx_dropped,
                c.tx_bytes,
                c.tx_submitted,
                c.tx_errors
            );
        }
        Node::EthernetRxBytes | Node::EthernetTxBytes => {
            let counters = ethernet_counters(true)?;
            let value = if node == Node::EthernetRxBytes {
                counters.rx_bytes
            } else {
                counters.tx_bytes
            };
            let _ = writeln!(out, "{value}");
        }
        Node::Memory => {
            let (total, free) = crate::PMM.lock().stats();
            if total == 0 {
                return Err(11);
            }
            let (swap, stored, _) = crate::memory::zram::stats();
            let _ = write!(out,
                "MemTotal: {} kB\nMemFree: {} kB\nMemAvailable: {} kB\nSwapTotal: {} kB\nSwapFree: {} kB\n",
                total as u64 * 4, free as u64 * 4, free as u64 * 4,
                swap as u64 * 4, swap.saturating_sub(stored) as u64 * 4);
        }
        _ => {
            let id = node.identity().ok_or(21u64)?;
            let idx = find(s, id.pid).ok_or(2u64)?;
            let p = &s.processes[idx];
            let name = p.name_str().replace(['\n', '\r'], "?");
            let ppid = if find(s, p.ppid).is_some() { p.ppid } else { 0 };
            match node {
                Node::Args(_) => return Ok(p.proc_args.clone()),
                Node::Comm(_) => {
                    let _ = writeln!(out, "{name}");
                }
                Node::Status(_) => {
                    let _ = writeln!(out, "Name:\t{name}\nTgid:\t{2}\nPid:\t{3}\nPPid:\t{ppid}\nUid:\t{0}\t{0}\t{0}\t{0}\nGid:\t{1}\t{1}\t{1}\t{1}",
                        p.uid, p.gid, p.pid, p.pid);
                }
                Node::Stat(_) => {
                    let rss = RSS.lock();
                    if rss.0 == 0 || timekeeping::global_ticks().saturating_sub(rss.0) > 3 * HZ {
                        return Err(11);
                    }
                    let pages = rss.1.iter().find(|(key, _)| *key == id).ok_or(11u64)?.1;
                    drop(rss);
                    let now = s.now_ns();
                    let mut runtime = p.proc_reaped_runtime_ns;
                    let mut threads = 0u32;
                    for (i, task) in s.processes.iter().enumerate() {
                        if task.linux_tgid.unwrap_or(task.pid) == p.pid
                            && task.address_space.identity() == p.address_space.identity()
                            && task.state != crate::process::ProcessState::Reaped
                        {
                            runtime = runtime.saturating_add(s.effective_runtime_at(i, now));
                            if !task.is_exiting() {
                                threads += 1;
                            }
                        }
                    }
                    let state = match p.state {
                        crate::process::ProcessState::Ready
                        | crate::process::ProcessState::Running => 'R',
                        crate::process::ProcessState::Suspended => 'T',
                        _ => 'S',
                    };
                    // Fields 14+15 use the same CLOCK_TICK unit as /proc/stat.
                    // Only aggregate task runtime is accounted, in the stime bucket.
                    let _ = writeln!(out, "{} ({name}) {state} {ppid} 0 0 0 0 0 0 0 0 0 0 {} 0 0 {} {} {threads} 0 {} 0 {pages} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
                        p.pid, runtime / NS_PER_CLOCK_TICK, 20i16 + p.nice as i16, p.nice, p.proc_start_ticks);
                }
                _ => return Err(21),
            }
        }
    }
    Ok(out.into_bytes())
}
