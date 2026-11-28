use super::*;
use crate::linux_procfs::{self, Node, OpenFile};

pub(super) fn open(_frame: &mut SyscallFrame, path: &str, flags: u64) -> u64 {
    let Ok(parsed) = sunlight_compat_linux::open_flags::parse(flags) else {
        return linux_errno(22);
    };
    if parsed.write || parsed.create || parsed.truncate {
        return linux_errno(30);
    }
    if path == "/proc/self" && flags & sunlight_compat_linux::abi::O_NOFOLLOW != 0 {
        return linux_errno(40);
    }
    let node = match crate::sched::with_scheduler(|s| Node::lookup(s, path)) {
        Ok(node) => node,
        Err(error) => return linux_errno(error),
    };
    if (parsed.directory || path.ends_with('/')) && !node.directory() {
        return linux_errno(20);
    }
    let status = parsed.status | if parsed.cloexec { O_CLOEXEC as u32 } else { 0 };
    #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate"))]
    if matches!(node, Node::MountInfo(_)) && mount_observation_routed() {
        return service_mount_observation(_frame, None, |sched, completion| {
            install(sched, node, Some(completion.payload), status)
        });
    }
    let mountinfo = if matches!(node, Node::MountInfo(_)) {
        match linux_procfs::mountinfo_snapshot() {
            Ok(bytes) => Some(bytes),
            Err(error) => return linux_errno(error),
        }
    } else {
        None
    };
    crate::sched::with_scheduler(|sched| install(sched, node, mountinfo, status))
}

fn install(sched: &mut crate::sched::Scheduler, node: Node, bytes: Option<Vec<u8>>, status: u32) -> u64 {
    if !node.live(sched) {
        return linux_errno(2);
    }
    let file = match OpenFile::new(sched, node, bytes) {
        Ok(file) => file,
        Err(error) => return linux_errno(error),
    };
    sched.current_shared_process_mut().fd_table.open_proc(file, status)
        .map_or_else(|_| linux_errno(24), |fd| fd as u64)
}

pub(super) fn stat(path: &str, ptr: u64, nofollow: bool) -> u64 {
    let result = crate::sched::with_scheduler(|s| {
        let node = Node::lookup(s, path)?;
        if path.ends_with('/') && !node.directory() {
            return Err(20);
        }
        let mut record = fill_linux_stat(&node.stat(s), node.ino());
        if nofollow && path.trim_end_matches('/') == "/proc/self" && !path.ends_with('/') {
            record[24..28].copy_from_slice(&0o120777u32.to_le_bytes());
            record[8..16].copy_from_slice(&10u64.to_le_bytes());
        }
        Ok(record)
    });
    match result {
        Ok(record) => copy_to_user(ptr, &record).err().unwrap_or(0),
        Err(error) => linux_errno(error),
    }
}

pub(super) fn access(path: &str, mode: u64) -> u64 {
    crate::sched::with_scheduler(|s| match Node::lookup(s, path) {
        Ok(_) if mode & 2 != 0 => linux_errno(30),
        Ok(node) if !node.directory() && path.ends_with('/') => linux_errno(20),
        Ok(node) if !node.directory() && mode & 1 != 0 => linux_errno(13),
        Ok(_) => 0,
        Err(error) => linux_errno(error),
    })
}

pub(super) fn readlink(path: &str, ptr: u64, count: usize) -> u64 {
    let result = crate::sched::with_scheduler(|s| {
        Node::lookup(s, path)?;
        if path != "/proc/self" || count == 0 {
            return Err(22);
        }
        let p = s.current_process();
        Ok(alloc::format!("{}", p.linux_tgid.unwrap_or(p.pid)))
    });
    match result {
        Ok(text) => {
            let n = text.len().min(count);
            copy_to_user(ptr, &text.as_bytes()[..n])
                .err()
                .unwrap_or(n as u64)
        }
        Err(error) => linux_errno(error),
    }
}

pub(super) fn relative(dirfd: i32, raw: &str) -> Option<Result<alloc::string::String, u64>> {
    crate::sched::with_scheduler(|s| {
        let entry = s.current_shared_process().fd_table.get(dirfd)?;
        entry.with_proc(|file| {
            if !file.node.directory() {
                return Err(linux_errno(20));
            }
            if !file.node.live(s) {
                return Err(linux_errno(2));
            }
            // A numeric directory cannot be re-resolved after dropping the
            // identity check (the target might exec in the meantime).
            if matches!(file.node, Node::Pid(_)) {
                return Err(linux_errno(95));
            }
            // Only descend from the validated virtual directory; do not
            // reinterpret it as an underlying native VFS capability.
            Ok(alloc::format!("{}/{raw}", file.node.path()))
        })
    })
}

pub(super) fn fstat(fd: i32, ptr: u64) -> Option<u64> {
    let result = crate::sched::with_scheduler(|s| {
        s.current_shared_process()
            .fd_table
            .get(fd)?
            .with_proc(|file| {
                file.node
                    .live(s)
                    .then(|| fill_linux_stat(&file.node.stat(s), file.node.ino()))
            })
    })?;
    Some(match result {
        Some(record) => copy_to_user(ptr, &record).err().unwrap_or(0),
        None => linux_errno(2),
    })
}

fn copy_locked(s: &crate::sched::Scheduler, ptr: u64, bytes: &[u8]) -> Result<(), u64> {
    if bytes.is_empty() {
        return Ok(());
    }
    let hhdm = VirtAddr::new(
        crate::HHDM_REQ
            .response()
            .ok_or_else(|| linux_errno(14))?
            .offset,
    );
    crate::memory::user::copy_to_process_bytes(s.current_process(), hhdm, ptr, bytes)
        .map_err(|e| user_memory_failure_for(true, e))
}

pub(super) fn read(
    s: &mut crate::sched::Scheduler,
    fd: i32,
    ptr: u64,
    count: usize,
    position: Option<usize>,
) -> Option<u64> {
    let entry = s.current_shared_process().fd_table.get(fd)?;
    if !entry.handle.is_proc() {
        return None;
    }
    let start = position.unwrap_or_else(|| entry.current_offset());
    let mut bytes = [0; 4096];
    let result = entry
        .with_proc(|file| {
            if !file.node.live(s) {
                return Err(2);
            }
            if file.node.directory() {
                return Err(21);
            }
            let data = file.read_bytes(s).get(start..).unwrap_or_default();
            let n = count.min(data.len()).min(bytes.len());
            bytes[..n].copy_from_slice(&data[..n]);
            Ok(n)
        })
        .expect("proc open description");
    let n = match result {
        Ok(n) => n,
        Err(error) => return Some(linux_errno(error)),
    };
    if let Err(error) = copy_locked(s, ptr, &bytes[..n]) {
        return Some(error);
    }
    if position.is_none() {
        s.current_shared_process_mut()
            .fd_table
            .get_mut(fd)?
            .set_offset(start + n);
    }
    Some(n as u64)
}

pub(super) fn seek(
    s: &mut crate::sched::Scheduler,
    fd: i32,
    offset: i64,
    whence: i32,
) -> Option<u64> {
    let entry = s.current_shared_process().fd_table.get(fd)?;
    let current = entry.current_offset() as i64;
    let result = entry.with_proc(|file| {
        if !file.node.live(s) {
            return Err(2);
        }
        let base = match whence {
            0 => 0,
            1 => current,
            2 if !file.node.directory() => file.bytes.len() as i64,
            _ => return Err(22),
        };
        base.checked_add(offset).filter(|&n| n >= 0).ok_or(22)
    })?;
    Some(match result {
        Ok(n) => {
            s.current_shared_process_mut()
                .fd_table
                .get_mut(fd)?
                .set_offset(n as usize);
            n as u64
        }
        Err(error) => linux_errno(error),
    })
}

pub(super) fn getdents(fd: i32, ptr: u64, count: usize) -> Option<u64> {
    crate::sched::with_scheduler(|s| {
        let entry = s.current_shared_process().fd_table.get(fd)?;
        let mut cursor = entry.current_offset();
        let mut bytes = alloc::vec::Vec::new();
        let result = entry.with_proc(|file| {
            if !file.node.live(s) {
                return Err(2);
            }
            if !file.node.directory() {
                return Err(20);
            }
            while cursor < file.entry_count() {
                let Some((name, kind, ino)) = file.entry(s, cursor) else {
                    cursor += 1;
                    continue;
                };
                let len = sunlight_compat_linux::abi::dirent64_reclen(name.len());
                let start = bytes.len();
                if start + len > count.min(4096) {
                    if start == 0 {
                        return Err(22);
                    }
                    break;
                }
                cursor += 1;
                bytes.resize(start + len, 0);
                bytes[start..start + 8].copy_from_slice(&ino.to_le_bytes());
                bytes[start + 8..start + 16].copy_from_slice(&(cursor as u64).to_le_bytes());
                bytes[start + 16..start + 18].copy_from_slice(&(len as u16).to_le_bytes());
                bytes[start + 18] = kind;
                bytes[start + 19..start + 19 + name.len()].copy_from_slice(name.as_bytes());
            }
            Ok(())
        })?;
        if let Err(error) = result {
            return Some(linux_errno(error));
        }
        if let Err(error) = copy_locked(s, ptr, &bytes) {
            return Some(error);
        }
        s.current_shared_process_mut()
            .fd_table
            .get_mut(fd)?
            .set_offset(cursor);
        Some(bytes.len() as u64)
    })
}

pub(super) fn owns(path: &str) -> bool {
    linux_procfs::is_path(path)
}

// These operations share a translated native entry point. Preserve errno
// values that would otherwise collide with its private error sentinels.
pub(super) fn native_result(result: u64) -> u64 {
    match result as i64 {
        -2 => ERR_NOENT,
        -5 => ERR_EIO,
        -9 => ERR_EBADF,
        _ => result,
    }
}
