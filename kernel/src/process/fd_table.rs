use alloc::vec::Vec;
use core::num::NonZeroU32;
use spin::Mutex;

#[cfg(test)]
extern crate alloc;

/// File descriptor rights (Capsicum-inspired)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapRights {
    bits: u64,
}

impl CapRights {
    pub const READ: u64 = 1 << 0;
    pub const WRITE: u64 = 1 << 1;
    pub const SEEK: u64 = 1 << 2;
    pub const FSTAT: u64 = 1 << 3;
    pub const FCHMOD: u64 = 1 << 4;
    pub const FCHOWN: u64 = 1 << 5;
    pub const FTRUNCATE: u64 = 1 << 6;
    pub const MMAP_R: u64 = 1 << 7;
    pub const MMAP_W: u64 = 1 << 8;
    pub const MMAP_X: u64 = 1 << 9;
    pub const CONNECT: u64 = 1 << 10; // Phase 5: network
    pub const BIND: u64 = 1 << 11;
    pub const ACCEPT: u64 = 1 << 12;

    pub const fn new(bits: u64) -> Self {
        Self { bits }
    }

    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    pub const fn all() -> Self {
        Self { bits: u64::MAX }
    }

    pub fn contains(self, other: Self) -> bool {
        (self.bits & other.bits) == other.bits
    }

    pub fn intersection(self, other: Self) -> Self {
        Self {
            bits: self.bits & other.bits,
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    pub fn bits(self) -> u64 {
        self.bits
    }
}

/// A file handle (opaque reference to open file)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileHandle(pub u32);

impl FileHandle {
    const PIPE_FLAG: u32 = 0x8000_0000;
    const PIPE_WRITE_FLAG: u32 = 0x4000_0000;
    const PIPE_INDEX_MASK: u32 = 0x3FFF_FFFF;
    /// Marks a handle backed by the kernel VFS (only meaningful when the
    /// pipe flag is clear; bits 0..30 carry the packed Vfs handle).
    const VFS_FLAG: u32 = 0x4000_0000;
    /// Marks a handle wired to a TTY tab's kernel stdin/stdout ring. Bits 0..8
    /// carry the tab index. Both flags keep bit31 (pipe) and bit30 (vfs) clear
    /// so `is_pipe`/`is_vfs` stay false for them (see process::tty_io).
    const TTY_STDIN_FLAG: u32 = 0x2000_0000;
    const TTY_STDOUT_FLAG: u32 = 0x1000_0000;
    /// Helios epoll instance handle. Bits 0..23 carry the epoll pool index.
    /// Chosen so it does not collide with pipe/vfs/tty tag bits.
    const EPOLL_FLAG: u32 = 0x0800_0000;
    const EPOLL_INDEX_MASK: u32 = 0x00FF_FFFF;
    const EVENTFD_FLAG: u32 = 0x0400_0000;
    const SOCKET_FLAG: u32 = 0x0200_0000;
    /// AF_UNIX endpoint created by `socket(2)` but not yet connected. Bits
    /// 0..23 carry the `process::unix_socket` pool index.
    const UNIX_FLAG: u32 = 0x0100_0000;
    const TTY_TAG_MASK: u32 = 0xF000_0000;
    const TTY_TAB_MASK: u32 = 0x0000_00FF;

    pub fn is_pipe(self) -> bool {
        (self.0 & Self::PIPE_FLAG) != 0
    }

    pub fn pipe_index(self) -> u32 {
        self.0 & Self::PIPE_INDEX_MASK
    }

    pub fn pipe_is_write(self) -> bool {
        (self.0 & Self::PIPE_WRITE_FLAG) != 0
    }

    pub fn vfs(packed: u32) -> Self {
        Self(Self::VFS_FLAG | (packed & 0x3FFF_FFFF))
    }

    pub fn is_vfs(self) -> bool {
        (self.0 & (Self::PIPE_FLAG | Self::VFS_FLAG)) == Self::VFS_FLAG
    }

    pub fn vfs_handle(self) -> u32 {
        self.0 & 0x3FFF_FFFF
    }

    /// fd0 handle wired to tab `tab`'s kernel stdin ring.
    pub fn tty_stdin(tab: u8) -> Self {
        Self(Self::TTY_STDIN_FLAG | (tab as u32 & Self::TTY_TAB_MASK))
    }

    /// fd1 handle wired to tab `tab`'s kernel stdout ring.
    pub fn tty_stdout(tab: u8) -> Self {
        Self(Self::TTY_STDOUT_FLAG | (tab as u32 & Self::TTY_TAB_MASK))
    }

    pub fn is_tty_stdin(self) -> bool {
        (self.0 & Self::TTY_TAG_MASK) == Self::TTY_STDIN_FLAG
    }

    pub fn is_tty_stdout(self) -> bool {
        (self.0 & Self::TTY_TAG_MASK) == Self::TTY_STDOUT_FLAG
    }

    /// Tab index for a TTY stdin/stdout handle.
    pub fn tty_tab(self) -> u8 {
        (self.0 & Self::TTY_TAB_MASK) as u8
    }

    pub fn epoll(pool_idx: u32) -> Self {
        Self(Self::EPOLL_FLAG | (pool_idx & Self::EPOLL_INDEX_MASK))
    }

    pub fn is_epoll(self) -> bool {
        (self.0 & Self::EPOLL_FLAG) != 0
            && !self.is_pipe()
            && !self.is_vfs()
            && !self.is_tty_stdin()
            && !self.is_tty_stdout()
    }

    pub fn epoll_index(self) -> u32 {
        self.0 & Self::EPOLL_INDEX_MASK
    }

    pub fn eventfd(pool_idx: u32) -> Self {
        Self(Self::EVENTFD_FLAG | (pool_idx & Self::EPOLL_INDEX_MASK))
    }

    pub fn is_eventfd(self) -> bool {
        self.0 & 0xFF00_0000 == Self::EVENTFD_FLAG
    }

    pub fn eventfd_index(self) -> u32 {
        self.0 & Self::EPOLL_INDEX_MASK
    }

    pub fn socket(index: u32, endpoint: u32) -> Self {
        Self(Self::SOCKET_FLAG | ((endpoint & 1) << 23) | (index & 0x007f_ffff))
    }

    pub fn is_socket(self) -> bool {
        self.0 & 0xff00_0000 == Self::SOCKET_FLAG
    }

    pub fn socket_index(self) -> u32 {
        self.0 & 0x007f_ffff
    }

    pub fn socket_endpoint(self) -> usize {
        ((self.0 >> 23) & 1) as usize
    }

    pub fn route_netlink(index: u32) -> Self {
        Self(0x0010_0000 | index)
    }

    pub fn is_route_netlink(self) -> bool {
        self.0 & 0xffff_0000 == 0x0010_0000
    }

    pub fn route_netlink_index(self) -> u32 {
        self.0 & 0xffff
    }

    pub fn inet_socket(index: u32) -> Self {
        Self(0x0080_0000 | index)
    }

    pub fn is_inet_socket(self) -> bool {
        self.0 & 0xff80_0000 == 0x0080_0000
    }

    pub fn inet_socket_index(self) -> u32 {
        self.0 & 0x007f_ffff
    }

    pub fn unix_socket(index: u32) -> Self {
        Self(Self::UNIX_FLAG | (index & 0x00FF_FFFF))
    }

    pub fn is_unix_socket(self) -> bool {
        self.0 & 0xFF00_0000 == Self::UNIX_FLAG
    }

    pub fn unix_socket_index(self) -> u32 {
        self.0 & 0x00FF_FFFF
    }

    pub(crate) const PROC: Self = Self(0x0020_0000);
    pub(crate) const SHARED_PAGE: Self = Self(0x0030_0000);
    pub(crate) const MEMFD: Self = Self(0x0030_0001);
    pub(crate) const NULL_DEVICE: Self = Self(0x0030_0002);

    pub(crate) fn is_proc(self) -> bool {
        self == Self::PROC
    }

    /// Descriptor backed by a service-owned description. The index names a
    /// kernel table row holding the opaque service token; the token alone never
    /// grants authority, and no backend handle number is exposed to userspace.
    const SERVICE_FLAG: u32 = 0x0040_0000;
    const SERVICE_INDEX_MASK: u32 = 0x003F_FFFF;

    pub fn service(index: u32) -> Self {
        Self(Self::SERVICE_FLAG | (index & Self::SERVICE_INDEX_MASK))
    }

    pub fn is_service(self) -> bool {
        (self.0 & 0xFFC0_0000) == Self::SERVICE_FLAG
    }

    pub fn service_index(self) -> u32 {
        self.0 & Self::SERVICE_INDEX_MASK
    }
}

fn retain_socket(handle: FileHandle) {
    if handle.is_socket() {
        super::pipe::socket_retain(handle.socket_index(), handle.socket_endpoint());
    }
}

fn retain_pipe(handle: FileHandle) {
    if handle.is_pipe() {
        super::pipe::pipe_retain_end(handle.pipe_index(), handle.pipe_is_write());
    }
}

fn retain_eventfd(handle: FileHandle) {
    if handle.is_eventfd() {
        super::eventfd::retain(handle.eventfd_index());
    }
}

fn release_eventfd(handle: FileHandle) {
    if handle.is_eventfd() {
        super::eventfd::release(handle.eventfd_index());
    }
}

fn retain_endpoint_socket(handle: FileHandle) {
    if handle.is_inet_socket() {
        super::inet_socket::retain(handle.inet_socket_index());
    }
    if handle.is_route_netlink() {
        super::route_netlink::retain(handle.route_netlink_index());
    }
    if handle.is_unix_socket() {
        super::unix_socket::retain(handle.unix_socket_index());
    }
}

fn release_endpoint_socket(handle: FileHandle) {
    if handle.is_inet_socket() {
        super::inet_socket::release(handle.inet_socket_index());
    }
    if handle.is_route_netlink() {
        super::route_netlink::release(handle.route_netlink_index());
    }
    if handle.is_unix_socket() {
        super::unix_socket::release(handle.unix_socket_index());
    }
}

/// Errors from file descriptor operations
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FdError {
    InvalidFd,
    AlreadyOpen,
    NoSlots,
    CapabilityDenied,
}

/// Open file descriptor with associated rights
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileDescriptor {
    pub fd: i32,
    pub handle: FileHandle,
    pub rights: CapRights,
    pub flags: u32, // O_RDONLY, O_WRONLY, O_RDWR, O_CLOEXEC
    /// Current read/write position (VFS-backed fds only).
    pub offset: usize,
    /// Index of the shared open file description. Duplication retains this
    /// index; close-on-exec stays in `flags` on the individual descriptor.
    description: u32,
    /// True on a consumed snapshot only when backend cleanup is required.
    pub release_backend: bool,
}

pub struct QueuedSharedPage(FileDescriptor);

impl Drop for QueuedSharedPage {
    fn drop(&mut self) {
        release_description(self.0.description);
    }
}

/// Result of a descriptor replacement. `service_close` is the exact service
/// token whose final kernel reference was displaced and must now be closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DupResult {
    pub fd: i32,
    pub displaced: Option<FileHandle>,
    pub service_close: Option<(u64, u64)>,
}

impl FileDescriptor {
    pub(crate) fn with_proc<R>(
        &self,
        f: impl FnOnce(&crate::linux_procfs::OpenFile) -> R,
    ) -> Option<R> {
        let pool = DESCRIPTIONS.lock();
        Some(f(pool[self.description as usize].as_ref()?.proc_file.as_ref()?))
    }

    pub fn current_offset(&self) -> usize {
        DESCRIPTIONS.lock()[self.description as usize]
            .as_ref()
            .expect("live description")
            .offset
    }
    pub fn set_offset(&mut self, offset: usize) {
        DESCRIPTIONS.lock()[self.description as usize]
            .as_mut()
            .expect("live description")
            .offset = offset;
        self.offset = offset;
    }
}

struct OpenDescription {
    offset: usize,
    status: u32,
    refs: usize,
    proc_file: Option<crate::linux_procfs::OpenFile>,
    memfd: Option<Memfd>,
}

static DESCRIPTIONS: Mutex<Vec<Option<OpenDescription>>> = Mutex::new(Vec::new());
static MEMFD_USAGE: Mutex<(usize, usize)> = Mutex::new((0, 0));
static MEMFD_RECLAIM: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

use crate::memory::pmm::PhysicalMemoryManager;
use core::sync::atomic::Ordering;
use x86_64::{structures::paging::{PhysFrame, Size4KiB}, VirtAddr};

struct Memfd {
    length: u64,
    frames: Vec<PhysFrame<Size4KiB>>,
}

impl FileDescriptor {
    pub(crate) fn memfd_view(&self) -> u32 {
        assert_eq!(self.handle, FileHandle::MEMFD);
        self.description
    }

    pub(crate) fn size_memfd(
        &self,
        length: u64,
        pmm: &mut PhysicalMemoryManager,
        hhdm: VirtAddr,
    ) -> Result<(), u32> {
        use sunlight_compat_linux::abi;
        if self.handle != FileHandle::MEMFD {
            return Err(abi::EOPNOTSUPP);
        }
        if !self.rights.contains(CapRights::new(CapRights::FTRUNCATE)) {
            return Err(abi::EACCES);
        }
        if length == 0 || length > i64::MAX as u64 {
            return Err(abi::EINVAL);
        }
        if length > 1024 * 1024 {
            return Err(abi::EFBIG);
        }
        let mut descriptions = DESCRIPTIONS.lock();
        let backing = descriptions[self.description as usize]
            .as_mut().unwrap().memfd.as_mut().unwrap();
        if backing.length == length {
            return Ok(());
        }
        if backing.length != 0 {
            return Err(abi::EPERM);
        }
        let pages = length.div_ceil(4096) as usize;
        let mut usage = MEMFD_USAGE.lock();
        if usage.1 + pages > 4096 {
            return Err(abi::ENOMEM);
        }
        backing.frames.try_reserve_exact(pages).map_err(|_| abi::ENOMEM)?;
        for _ in 0..pages {
            let Some(physical) = pmm.alloc_frame_class(
                crate::memory::accounting::PhysicalMemoryClass::SharedMemory,
            ) else {
                for frame in backing.frames.drain(..) {
                    pmm.free_frame(frame.start_address());
                }
                return Err(abi::ENOMEM);
            };
            assert!(crate::memory::security::sanitize_user_frame(physical, hhdm));
            backing.frames.push(PhysFrame::containing_address(physical));
        }
        backing.length = length;
        usage.1 += pages;
        #[cfg(all(debug_assertions, feature = "fd_shared_mmap_test"))]
        crate::serial_println!("[MEMFD] sized object={} pages={} live_pages={}", self.description, pages, usage.1);
        Ok(())
    }
}

pub(crate) fn with_memfd_frames<R>(
    id: u32,
    f: impl FnOnce(u64, &[PhysFrame<Size4KiB>]) -> R,
) -> R {
    let descriptions = DESCRIPTIONS.lock();
    let backing = descriptions[id as usize].as_ref().unwrap().memfd.as_ref().unwrap();
    f(backing.length, &backing.frames)
}

pub(crate) fn retain_memfd_view(id: u32) {
    retain_description(id);
}

pub(crate) fn release_memfd_view(id: u32) {
    release_description(id);
}

pub(crate) fn memfd_reclaim_pending() -> bool {
    MEMFD_RECLAIM.load(Ordering::Acquire)
}

pub(crate) fn drain_memfd_frames(pmm: &mut PhysicalMemoryManager) {
    if !memfd_reclaim_pending() {
        return;
    }
    let mut descriptions = DESCRIPTIONS.lock();
    MEMFD_RECLAIM.store(false, Ordering::Release);
    let mut usage = MEMFD_USAGE.lock();
    for (_id, slot) in descriptions.iter_mut().enumerate() {
        if !slot.as_ref().is_some_and(|row| row.refs == 0) {
            continue;
        }
        let backing = slot.take().unwrap().memfd.unwrap();
        let pages = backing.frames.len();
        for frame in backing.frames {
            pmm.free_frame(frame.start_address());
        }
        usage.0 -= 1;
        usage.1 -= pages;
        #[cfg(all(debug_assertions, feature = "fd_shared_mmap_test"))]
        crate::serial_println!("[MEMFD] freed object={} pages={} live_objects={} live_pages={}", _id, pages, usage.0, usage.1);
    }
}

fn new_description(flags: u32) -> u32 {
    try_new_description(flags).expect("open description allocation failed")
}

fn try_new_description(flags: u32) -> Result<u32, ()> {
    let mut pool = DESCRIPTIONS.lock();
    let item = OpenDescription {
        offset: 0,
        status: flags & !0x0008_0000,
        refs: 1,
        proc_file: None,
        memfd: None,
    };
    if let Some((idx, slot)) = pool.iter_mut().enumerate().find(|(_, s)| s.is_none()) {
        *slot = Some(item);
        return Ok(idx as u32);
    }
    let idx = u32::try_from(pool.len()).map_err(|_| ())?;
    pool.try_reserve(1).map_err(|_| ())?;
    pool.push(Some(item));
    Ok(idx)
}

fn retain_description(idx: u32) {
    DESCRIPTIONS.lock()[idx as usize]
        .as_mut()
        .expect("live description")
        .refs += 1;
}

fn release_description(idx: u32) -> bool {
    let mut pool = DESCRIPTIONS.lock();
    let item = pool[idx as usize].as_mut().expect("live description");
    item.refs -= 1;
    if item.refs == 0 {
        if item.memfd.is_some() {
            MEMFD_RECLAIM.store(true, Ordering::Release);
        } else {
            pool[idx as usize] = None;
        }
        true
    } else {
        false
    }
}

/// Kernel-side identity of one service-owned description. Duplicates and fork
/// inherit the same row and refcount; only the final kernel reference may send
/// a service close. The token never appears in the descriptor number itself.
struct ServiceDescription {
    token: u64,
    epoch: u64,
    refs: usize,
}

static SERVICE_DESCRIPTIONS: Mutex<Vec<Option<ServiceDescription>>> = Mutex::new(Vec::new());

fn new_service_description(token: u64, epoch: u64) -> Option<u32> {
    if token == 0 || epoch == 0 {
        return None;
    }
    let mut pool = SERVICE_DESCRIPTIONS.lock();
    let item = ServiceDescription {
        token,
        epoch,
        refs: 0,
    };
    if let Some((idx, slot)) = pool.iter_mut().enumerate().find(|(_, s)| s.is_none()) {
        *slot = Some(item);
        return Some(idx as u32);
    }
    pool.try_reserve(1).ok()?;
    let idx = pool.len();
    pool.push(Some(item));
    Some(idx as u32)
}

pub(crate) fn retain_service(handle: FileHandle) {
    if handle.is_service() {
        SERVICE_DESCRIPTIONS.lock()[handle.service_index() as usize]
            .as_mut()
            .expect("live service description")
            .refs += 1;
    }
}

/// Release one kernel reference. Returns true when it was the final reference
/// and the caller now owns a service close for the returned token.
pub(crate) fn release_service(handle: FileHandle) -> bool {
    if !handle.is_service() {
        return true;
    }
    let mut pool = SERVICE_DESCRIPTIONS.lock();
    let item = pool[handle.service_index() as usize]
        .as_mut()
        .expect("live service description");
    item.refs -= 1;
    if item.refs == 0 {
        pool[handle.service_index() as usize] = None;
        true
    } else {
        false
    }
}

/// Opaque identity used only to build a trusted close request. Reading it never
/// grants authority and it must never be copied to userspace.
pub(crate) fn service_description(index: u32) -> Option<(u64, u64)> {
    SERVICE_DESCRIPTIONS
        .lock()
        .get(index as usize)?
        .as_ref()
        .map(|item| (item.token, item.epoch))
}

/// File descriptor table for a process (max 256 open fds)
pub struct FdTable {
    entries: [Option<FileDescriptor>; 256],
}

impl FdTable {
    pub(crate) fn open_memfd(&mut self, flags: u32) -> Result<i32, u32> {
        use sunlight_compat_linux::abi;
        if MEMFD_USAGE.lock().0 >= 128 {
            return Err(abi::ENOMEM);
        }
        let rights = CapRights::new(
            CapRights::READ | CapRights::WRITE | CapRights::FTRUNCATE |
            CapRights::MMAP_R | CapRights::MMAP_W,
        );
        let fd = self.open(FileHandle::MEMFD, rights, flags).map_err(|_| abi::EMFILE)?;
        let description = self.get(fd).unwrap().description;
        DESCRIPTIONS.lock()[description as usize].as_mut().unwrap().memfd =
            Some(Memfd { length: 0, frames: Vec::new() });
        MEMFD_USAGE.lock().0 += 1;
        Ok(fd)
    }

    pub fn retain_shared_page(&self, fd: i32) -> Result<QueuedSharedPage, u32> {
        use sunlight_compat_linux::abi;
        let entry = *self.get(fd).ok_or(abi::EBADF)?;
        if !matches!(entry.handle, FileHandle::SHARED_PAGE | FileHandle::MEMFD) {
            return Err(abi::EOPNOTSUPP);
        }
        retain_description(entry.description);
        Ok(QueuedSharedPage(entry))
    }

    pub fn install_shared_page(&mut self, queued: &QueuedSharedPage) -> Result<i32, FdError> {
        let entry = &queued.0;
        self.open_from(0, entry.handle, entry.rights, entry.flags | 0x80000,
            entry.offset, Some(entry.description))
    }

    pub fn new() -> Self {
        let entries: [Option<FileDescriptor>; 256] = [None; 256];
        let mut table = Self { entries };
        // Initialize standard streams
        table.entries[0] = Some(FileDescriptor {
            fd: 0,
            handle: FileHandle(0),
            rights: CapRights::new(CapRights::READ | CapRights::FSTAT),
            flags: 0, // O_RDONLY
            offset: 0,
            description: new_description(0),
            release_backend: true,
        });
        table.entries[1] = Some(FileDescriptor {
            fd: 1,
            handle: FileHandle(1),
            rights: CapRights::new(CapRights::WRITE | CapRights::FSTAT),
            flags: 1, // O_WRONLY
            offset: 0,
            description: new_description(1),
            release_backend: true,
        });
        table.entries[2] = Some(FileDescriptor {
            fd: 2,
            handle: FileHandle(2),
            rights: CapRights::new(CapRights::WRITE | CapRights::FSTAT),
            flags: 1, // O_WRONLY
            offset: 0,
            description: new_description(1),
            release_backend: true,
        });
        table
    }

    pub fn new_boxed() -> alloc::boxed::Box<Self> {
        Self::try_init_boxed(alloc::boxed::Box::<Self>::new_uninit(), || Ok(()))
            .expect("initial descriptor allocation failed")
    }

    pub(crate) fn try_init_boxed(
        mut storage: alloc::boxed::Box<core::mem::MaybeUninit<Self>>,
        mut checkpoint: impl FnMut() -> Result<(), ()>,
    ) -> Result<alloc::boxed::Box<Self>, ()> {
        let ptr = storage.as_mut_ptr();
        let mut table = unsafe {
            let entries = core::ptr::addr_of_mut!((*ptr).entries) as *mut Option<FileDescriptor>;
            for idx in 0..256 {
                entries.add(idx).write(None);
            }
            storage.assume_init()
        };
        for fd in 0..3 {
            let flags = u32::from(fd != 0);
            let description = match checkpoint().and_then(|()| try_new_description(flags)) {
                Ok(description) => description,
                Err(()) => {
                    for initialized in 0..fd {
                        table.take(initialized).expect("initialized stdio");
                    }
                    return Err(());
                }
            };
            let access = if fd == 0 { CapRights::READ } else { CapRights::WRITE };
            table.entries[fd as usize] = Some(FileDescriptor {
                fd,
                handle: FileHandle(fd as u32),
                rights: CapRights::new(access | CapRights::FSTAT),
                flags,
                offset: 0,
                description,
                release_backend: true,
            });
        }
        Ok(table)
    }

    /// Inherit only initial stream descriptions with no backend lease.
    pub(crate) fn inherit_initial_stream(&mut self, fd: i32, mut source: FileDescriptor) {
        assert!((0..3).contains(&fd) && source.handle.0 == fd as u32);
        retain_description(source.description);
        let old = self.take(fd).expect("initial stream");
        assert_eq!(old.handle.0, fd as u32);
        source.fd = fd;
        source.flags &= !0x0008_0000;
        source.offset = source.current_offset();
        self.entries[fd as usize] = Some(source);
    }

    #[cfg(all(debug_assertions, any(feature = "vfs_l1p_test", feature = "vfs_d1b_test")))]
    pub(crate) fn description_counts() -> (usize, usize) {
        let pool = DESCRIPTIONS.lock();
        (pool.iter().flatten().count(), pool.iter().flatten().map(|row| row.refs).sum())
    }

    #[cfg(all(debug_assertions, any(feature = "vfs_d1b_test", feature = "vfs_c1e12_test")))]
    pub(crate) fn service_description_counts() -> (usize, usize) {
        let pool = SERVICE_DESCRIPTIONS.lock();
        (pool.iter().flatten().count(), pool.iter().flatten().map(|row| row.refs).sum())
    }

    /// Open a new file descriptor in the lowest unused slot, including 0–2.
    pub fn open(
        &mut self,
        handle: FileHandle,
        rights: CapRights,
        flags: u32,
    ) -> Result<i32, FdError> {
        self.open_from(0, handle, rights, flags, 0, None)
    }

    pub(crate) fn open_proc(
        &mut self,
        file: crate::linux_procfs::OpenFile,
        flags: u32,
    ) -> Result<i32, FdError> {
        let fd = self.open(FileHandle::PROC, CapRights::new(CapRights::READ | CapRights::FSTAT), flags)?;
        let idx = self.get(fd).expect("installed proc descriptor").description;
        DESCRIPTIONS.lock()[idx as usize].as_mut().expect("live description").proc_file = Some(file);
        Ok(fd)
    }

    fn open_from(
        &mut self,
        min_fd: i32,
        handle: FileHandle,
        rights: CapRights,
        flags: u32,
        offset: usize,
        shared_description: Option<u32>,
    ) -> Result<i32, FdError> {
        if min_fd < 0 {
            return Err(FdError::InvalidFd);
        }
        let start = min_fd as usize;
        let slot = self
            .entries
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(idx, entry)| entry.is_none().then_some(idx))
            .ok_or(FdError::NoSlots)?;
        let fd = slot as i32;
        let description = if let Some(idx) = shared_description {
            retain_description(idx);
            retain_socket(handle);
            retain_pipe(handle);
            retain_endpoint_socket(handle);
            retain_service(handle);
            idx
        } else {
            new_description(flags)
        };
        retain_eventfd(handle);
        if shared_description.is_none() {
            retain_service(handle);
        }
        self.entries[slot] = Some(FileDescriptor {
            fd,
            handle,
            rights,
            flags,
            offset,
            description,
            release_backend: true,
        });

        Ok(fd)
    }

    /// Install an fd backed by a service-owned description. The kernel row
    /// owns one reference; `dup`/fork share it and only the final close may
    /// release the service object.
    pub fn open_service(
        &mut self,
        token: u64,
        epoch: u64,
        rights: CapRights,
        flags: u32,
    ) -> Result<i32, FdError> {
        let index = new_service_description(token, epoch).ok_or(FdError::NoSlots)?;
        let handle = FileHandle::service(index);
        match self.open(handle, rights, flags) {
            Ok(fd) => Ok(fd),
            Err(error) => {
                // No reference was acquired before the failing admission step.
                SERVICE_DESCRIPTIONS.lock()[index as usize] = None;
                Err(error)
            }
        }
    }

    /// Close a file descriptor
    pub fn close(&mut self, fd: i32) -> Result<(), FdError> {
        self.take(fd).map(|_| ())
    }

    /// Consume an fd table entry before releasing its backend object.
    ///
    /// `close(2)` must not leave a live entry around while its backend close is
    /// in progress: a second close could otherwise release the same object or
    /// a later descriptor incarnation.  The caller owns the returned snapshot
    /// and must attempt backend cleanup exactly once; it must never put the
    /// descriptor back merely to retry a close.
    pub fn take(&mut self, fd: i32) -> Result<FileDescriptor, FdError> {
        if fd < 0 || fd >= 256 {
            return Err(FdError::InvalidFd);
        }
        let mut entry = self.entries[fd as usize].take().ok_or(FdError::InvalidFd)?;
        let last = release_description(entry.description);
        entry.release_backend = if entry.handle.is_service() {
            release_service(entry.handle)
        } else {
            !entry.handle.is_vfs() || last
        };
        release_eventfd(entry.handle);
        Ok(entry)
    }

    /// Remove descriptors marked close-on-exec and return their backing
    /// handles so the caller can release VFS resources. Standard streams are
    /// included: Linux CLOEXEC applies to fds 0–2 as well.
    pub fn take_cloexec_handles(&mut self) -> [Option<FileHandle>; 256] {
        const O_CLOEXEC: u32 = 0x0008_0000;
        let mut handles = [None; 256];
        for (idx, entry) in self.entries.iter_mut().enumerate() {
            if entry
                .as_ref()
                .map(|descriptor| descriptor.flags & O_CLOEXEC != 0)
                .unwrap_or(false)
            {
                handles[idx] = entry.take().and_then(|descriptor| {
                    let last = release_description(descriptor.description);
                    release_eventfd(descriptor.handle);
                    if descriptor.handle.is_service() {
                        // The exit/exec file-owner barrier owns the service
                        // close; the kernel only drops its reference here.
                        let _ = release_service(descriptor.handle);
                        None
                    } else {
                        (!descriptor.handle.is_vfs() || last).then_some(descriptor.handle)
                    }
                });
            }
        }
        handles
    }

    /// Consume every live descriptor. Used on process exit so backend objects
    /// (VFS handles, pipes, epoll instances) are released exactly once.
    pub fn take_all_handles(&mut self) -> [Option<FileHandle>; 256] {
        let mut handles = [None; 256];
        for (idx, entry) in self.entries.iter_mut().enumerate() {
            handles[idx] = entry.take().and_then(|descriptor| {
                // Exit cleanup can observe a descriptor alias after its open
                // description was retired by an earlier entry in this same
                // table.  The alias no longer owns either the description or
                // its backend, so consuming it is the complete cleanup action.
                if !DESCRIPTIONS
                    .lock()
                    .get(descriptor.description as usize)
                    .is_some_and(Option::is_some)
                {
                    #[cfg(feature = "vfs_c1e2_test")]
                    crate::serial_println!(
                        "[VFS-C1E6B] stale descriptor alias consumed fd={}",
                        idx
                    );
                    return None;
                }
                let last = release_description(descriptor.description);
                release_eventfd(descriptor.handle);
                if descriptor.handle.is_service() {
                    let _ = release_service(descriptor.handle);
                    None
                } else {
                    (!descriptor.handle.is_vfs() || last).then_some(descriptor.handle)
                }
            });
        }
        handles
    }

    #[cfg(feature = "vfs_c1e2_test")]
    pub(crate) fn qualify_retirement() {
        let baseline = {
            let pool = DESCRIPTIONS.lock();
            (
                pool.iter().flatten().count(),
                pool.iter().flatten().map(|row| row.refs).sum::<usize>(),
            )
        };
        let mut table = Self::new_boxed();
        let ordinary = table.open(FileHandle::vfs(101), CapRights::all(), 0).unwrap();
        let ordinary_description = table.get(ordinary).unwrap().description;
        assert!(table.take(ordinary).unwrap().release_backend);
        assert!(DESCRIPTIONS.lock()[ordinary_description as usize].is_none());

        let handle = FileHandle::vfs(102);
        let fd = table.open(handle, CapRights::all(), 0).unwrap();
        let description = table.get(fd).unwrap().description;
        let duplicate = table.dup(fd).unwrap();
        table.get_mut(fd).unwrap().set_offset(42);
        assert_eq!(table.get(duplicate).unwrap().current_offset(), 42);
        assert!(!table.take(fd).unwrap().release_backend);
        assert_eq!(table.get(duplicate).unwrap().current_offset(), 42);
        table.dup2(duplicate, 24).unwrap();
        let handles = table.take_all_handles();
        assert_eq!(handles[duplicate as usize], None);
        assert_eq!(handles[24], Some(handle));
        assert_eq!(handles.iter().flatten().filter(|&&h| h == handle).count(), 1);
        assert!(table.entries.iter().all(Option::is_none));
        assert!(DESCRIPTIONS.lock()[description as usize].is_none());
        assert!(table.take_all_handles().iter().all(Option::is_none));
        let final_counts = {
            let pool = DESCRIPTIONS.lock();
            (
                pool.iter().flatten().count(),
                pool.iter().flatten().map(|row| row.refs).sum::<usize>(),
            )
        };
        assert_eq!(final_counts, baseline);
        crate::serial_println!("[VFS-C1E6B] descriptor retirement verified");
    }

    /// Get a file descriptor (for inspection)
    pub fn get(&self, fd: i32) -> Option<&FileDescriptor> {
        if fd < 0 || fd >= 256 {
            return None;
        }
        self.entries[fd as usize].as_ref()
    }

    /// Collect each distinct service row once. Duplicates and threads share a
    /// row by index, so transfer must be published per row, not per descriptor.
    pub fn service_tokens(&self, out: &mut [(u64, u64); 256]) -> usize {
        let mut count = 0;
        for entry in self.entries.iter().flatten() {
            if !entry.handle.is_service() {
                continue;
            }
            let Some(description) = service_description(entry.handle.service_index()) else {
                continue;
            };
            if out[..count].contains(&description) {
                continue;
            }
            out[count] = description;
            count += 1;
        }
        count
    }

    pub fn contains_eventfd(&self, idx: u32) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.handle.is_eventfd() && entry.handle.eventfd_index() == idx)
    }

    pub fn contains_tty_stdin_tab(&self, tab: usize) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.handle.is_tty_stdin() && entry.handle.tty_tab() as usize == tab)
    }

    pub fn contains_pipe_ring(&self, ring: u32) -> bool {
        self.entries.iter().flatten().any(|entry| {
            let handle = entry.handle;
            handle.is_pipe() && handle.pipe_index() == ring
                || handle.is_socket()
                    && super::pipe::socket_stream(handle.socket_index(), handle.socket_endpoint())
                        .is_some_and(|(read, write)| read == ring || write == ring)
        })
    }

    pub fn contains_unix_socket(&self, idx: u32) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.handle.is_unix_socket() && entry.handle.unix_socket_index() == idx)
    }

    /// Get a mutable file descriptor
    pub fn get_mut(&mut self, fd: i32) -> Option<&mut FileDescriptor> {
        if fd < 0 || fd >= 256 {
            return None;
        }
        self.entries[fd as usize].as_mut()
    }

    pub fn status_flags(&self, fd: i32) -> Option<u32> {
        let idx = self.get(fd)?.description;
        DESCRIPTIONS
            .lock()
            .get(idx as usize)?
            .as_ref()
            .map(|item| item.status)
    }

    pub fn set_status_flags(&self, fd: i32, flags: u32) -> Result<(), FdError> {
        let idx = self.get(fd).ok_or(FdError::InvalidFd)?.description;
        DESCRIPTIONS.lock()[idx as usize]
            .as_mut()
            .expect("live description")
            .status = flags & !0x0008_0000;
        Ok(())
    }

    /// Install a descriptor at a fixed fd number, replacing any existing
    /// entry (used by Spawn to hand a parent's pipe end to the child's
    /// stdout). Unlike `dup2` this does not require the source fd to live
    /// in this table.
    pub fn install_at(
        &mut self,
        fd: i32,
        handle: FileHandle,
        rights: CapRights,
        flags: u32,
    ) -> Result<(), FdError> {
        if fd < 0 || fd >= 256 {
            return Err(FdError::InvalidFd);
        }
        retain_eventfd(handle);
        retain_socket(handle);
        retain_pipe(handle);
        retain_endpoint_socket(handle);
        retain_service(handle);
        if let Some(displaced) = self.entries[fd as usize] {
            release_eventfd(displaced.handle);
            release_endpoint_socket(displaced.handle);
            release_description(displaced.description);
            let _ = release_service(displaced.handle);
            if displaced.handle.is_pipe() {
                super::pipe::pipe_close_end(
                    displaced.handle.pipe_index(),
                    displaced.handle.pipe_is_write(),
                );
            } else if displaced.handle.is_socket() {
                super::pipe::socket_close(
                    displaced.handle.socket_index(),
                    displaced.handle.socket_endpoint(),
                );
            }
        }
        self.entries[fd as usize] = Some(FileDescriptor {
            fd,
            handle,
            rights,
            flags,
            offset: 0,
            description: new_description(flags),
            release_backend: true,
        });
        Ok(())
    }

    /// Swap the backing handle of an open descriptor in place, preserving the
    /// descriptor's flags and shared open-file description.
    ///
    /// The new handle must already own exactly one reference (as allocated by
    /// `pipe::alloc_socketpair`); this does not retain it. The displaced
    /// backing object is released exactly once.
    pub fn replace_handle(
        &mut self,
        fd: i32,
        handle: FileHandle,
        rights: CapRights,
    ) -> Result<(), FdError> {
        let entry = self.get_mut(fd).ok_or(FdError::InvalidFd)?;
        let displaced = entry.handle;
        release_eventfd(displaced);
        release_endpoint_socket(displaced);
        let _ = release_service(displaced);
        if displaced.is_pipe() {
            super::pipe::pipe_close_end(displaced.pipe_index(), displaced.pipe_is_write());
        } else if displaced.is_socket() {
            super::pipe::socket_close(displaced.socket_index(), displaced.socket_endpoint());
        }
        entry.handle = handle;
        entry.rights = rights;
        Ok(())
    }

    /// Spawn inherits an existing open description, including VFS lifetime and
    /// position. `install_at` is for installing a fresh backing object.
    pub fn install_from(&mut self, fd: i32, source: FileDescriptor) -> Result<(), FdError> {
        self.install_at(
            fd,
            source.handle,
            source.rights,
            source.flags & !0x0008_0000,
        )?;
        let entry = self.entries[fd as usize]
            .as_mut()
            .expect("installed descriptor");
        retain_description(source.description);
        release_description(entry.description);
        entry.description = source.description;
        entry.offset = source.current_offset();
        Ok(())
    }

    /// Check if fd has required rights
    pub fn check_rights(&self, fd: i32, required: CapRights) -> Result<(), FdError> {
        let fd_entry = self.get(fd).ok_or(FdError::InvalidFd)?;
        if !fd_entry.rights.contains(required) {
            return Err(FdError::CapabilityDenied);
        }
        Ok(())
    }

    /// Duplicate a file descriptor into the lowest unused slot.
    /// The new descriptor does not inherit close-on-exec (Linux/POSIX dup).
    pub fn dup(&mut self, fd: i32) -> Result<i32, FdError> {
        self.dup_from(fd, 0, false)
    }

    /// Duplicate `fd` into the lowest slot `>= min_fd`.
    /// `cloexec` sets the new descriptor's close-on-exec bit (F_DUPFD_CLOEXEC).
    pub fn dup_from(&mut self, fd: i32, min_fd: i32, cloexec: bool) -> Result<i32, FdError> {
        const O_CLOEXEC: u32 = 0x0008_0000;
        let orig = *self.get(fd).ok_or(FdError::InvalidFd)?;
        // open_from retains the shared eventfd open object.
        let mut flags = orig.flags & !O_CLOEXEC;
        if cloexec {
            flags |= O_CLOEXEC;
        }
        self.open_from(
            min_fd,
            orig.handle,
            orig.rights,
            flags,
            orig.offset,
            Some(orig.description),
        )
    }

    /// Duplicate `old_fd` onto `new_fd`.
    ///
    /// Returns the displaced backing handle when `new_fd` already referred to
    /// a different object. The caller must close that handle exactly once.
    /// `old_fd == new_fd` is a no-op that returns `(new_fd, None)` if valid.
    /// The new descriptor does not inherit close-on-exec.
    pub fn dup2(&mut self, old_fd: i32, new_fd: i32) -> Result<DupResult, FdError> {
        self.dup2_with_cloexec(old_fd, new_fd, false, true)
    }

    /// Linux `dup3`: like `dup2`, but `old_fd == new_fd` is an error and the
    /// new descriptor's close-on-exec bit is set from `cloexec`.
    pub fn dup3(&mut self, old_fd: i32, new_fd: i32, cloexec: bool) -> Result<DupResult, FdError> {
        self.dup2_with_cloexec(old_fd, new_fd, cloexec, false)
    }

    fn dup2_with_cloexec(
        &mut self,
        old_fd: i32,
        new_fd: i32,
        cloexec: bool,
        allow_same: bool,
    ) -> Result<DupResult, FdError> {
        if new_fd < 0 || new_fd >= 256 {
            return Err(FdError::InvalidFd);
        }
        if old_fd == new_fd {
            if allow_same {
                let _ = self.get(old_fd).ok_or(FdError::InvalidFd)?;
                return Ok(DupResult {
                    fd: new_fd,
                    displaced: None,
                    service_close: None,
                });
            }
            return Err(FdError::InvalidFd);
        }
        let orig = *self.get(old_fd).ok_or(FdError::InvalidFd)?;

        const O_CLOEXEC: u32 = 0x0008_0000;
        let mut service_close = None;
        let displaced = self.entries[new_fd as usize].take().and_then(|descriptor| {
            release_eventfd(descriptor.handle);
            let last = release_description(descriptor.description);
            if descriptor.handle.is_service() {
                // Only the final kernel reference owns the service close. The
                // caller sends it after the replacement descriptor is installed.
                let description = service_description(descriptor.handle.service_index());
                if release_service(descriptor.handle) {
                    service_close = description;
                }
                None
            } else {
                (!descriptor.handle.is_vfs() || last).then_some(descriptor.handle)
            }
        });
        retain_eventfd(orig.handle);
        retain_socket(orig.handle);
        retain_pipe(orig.handle);
        retain_endpoint_socket(orig.handle);
        retain_service(orig.handle);
        retain_description(orig.description);
        let mut flags = orig.flags & !O_CLOEXEC;
        if cloexec {
            flags |= O_CLOEXEC;
        }
        self.entries[new_fd as usize] = Some(FileDescriptor {
            fd: new_fd,
            handle: orig.handle,
            rights: orig.rights,
            flags,
            offset: orig.offset,
            description: orig.description,
            release_backend: true,
        });
        Ok(DupResult {
            fd: new_fd,
            displaced,
            service_close,
        })
    }

    /// Deep-copy this table into a new heap allocation.
    /// Used by `Process::new_thread` so a thread inherits the parent's open
    /// file descriptors without sharing the same Box (Phase 1; Arc sharing is
    /// deferred to Phase 2).
    pub fn clone_boxed(&self) -> alloc::boxed::Box<Self> {
        // Use new_uninit to avoid placing a 256-slot array on the kernel stack.
        let mut table = alloc::boxed::Box::<Self>::new_uninit();
        let ptr = table.as_mut_ptr();
        unsafe {
            let entries = core::ptr::addr_of_mut!((*ptr).entries) as *mut Option<FileDescriptor>;
            for idx in 0..256 {
                entries.add(idx).write(self.entries[idx]);
                if let Some(entry) = self.entries[idx] {
                    retain_eventfd(entry.handle);
                    retain_socket(entry.handle);
                    retain_pipe(entry.handle);
                    retain_endpoint_socket(entry.handle);
                    retain_service(entry.handle);
                    retain_description(entry.description);
                }
            }
            table.assume_init()
        }
    }

    /// An unpublished snapshot owns storage only. It acquires descriptor/backend
    /// references at the non-failing commit point, while SCHEDULER still protects
    /// the source. Dropping a failed snapshot must not release those references.
    pub(crate) fn try_thread_snapshot(
        source: Option<&Self>,
    ) -> Result<alloc::boxed::Box<Self>, super::thread_admission::Error> {
        let mut table = super::thread_admission::try_box_uninit::<Self>()?;
        unsafe {
            let entries = core::ptr::addr_of_mut!((*table.as_mut_ptr()).entries)
                as *mut Option<FileDescriptor>;
            for idx in 0..256 {
                entries
                    .add(idx)
                    .write(source.and_then(|source| source.entries[idx]));
            }
            Ok(table.assume_init())
        }
    }

    /// Must be called exactly once for a snapshot, immediately before insertion.
    pub(crate) fn retain_thread_snapshot(&self) {
        for entry in self.entries.iter().flatten() {
            retain_eventfd(entry.handle);
            retain_socket(entry.handle);
            retain_pipe(entry.handle);
            retain_endpoint_socket(entry.handle);
            retain_service(entry.handle);
            retain_description(entry.description);
        }
    }

    /// Reduce rights on a file descriptor (can never increase)
    pub fn reduce_rights(&mut self, fd: i32, new_rights: CapRights) -> Result<(), FdError> {
        let fd_entry = self.get_mut(fd).ok_or(FdError::InvalidFd)?;

        // Check that new_rights is a subset of current rights
        if !fd_entry.rights.contains(new_rights) {
            return Err(FdError::CapabilityDenied);
        }

        fd_entry.rights = new_rights;
        Ok(())
    }
}

impl Default for FdTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: CapRights = CapRights::new(CapRights::READ);

    #[test]
    fn spawn_and_dup_share_vfs_position_and_final_close_ownership() {
        let mut parent = FdTable::new();
        let fd = parent.open(FileHandle::vfs(101), READ, 0).unwrap();
        parent.get_mut(fd).unwrap().set_offset(12);
        let source = *parent.get(fd).unwrap();
        let mut child = FdTable::new();
        child.install_from(1, source).unwrap();
        assert_eq!(child.get(1).unwrap().current_offset(), 12);
        assert!(!parent.take(fd).unwrap().release_backend);
        child.get_mut(1).unwrap().set_offset(42);
        let dup = child.dup(1).unwrap();
        assert_eq!(child.get(dup).unwrap().current_offset(), 42);
        assert!(!child.take(1).unwrap().release_backend);
        assert!(child.take(dup).unwrap().release_backend);
        parent.take_all_handles();
        child.take_all_handles();
    }

    #[test]
    fn cloexec_and_reaping_release_vfs_only_once() {
        let mut table = FdTable::new();
        let fd = table.open(FileHandle::vfs(102), READ, 0x0008_0000).unwrap();
        let duplicate = table.dup(fd).unwrap();
        assert_eq!(table.take_cloexec_handles()[fd as usize], None);
        let handles = table.take_all_handles();
        assert_eq!(handles[duplicate as usize], Some(FileHandle::vfs(102)));
        assert_eq!(
            handles.into_iter().flatten().filter(|h| h.is_vfs()).count(),
            1
        );
    }

    #[test]
    fn status_is_shared_by_dup_but_not_independent_open() {
        let mut table = FdTable::new();
        let original = table.open(FileHandle(99), READ, 0).unwrap();
        let duplicate = table.dup(original).unwrap();
        let separate = table.open(FileHandle(99), READ, 0).unwrap();
        table.set_status_flags(original, 0x800).unwrap();
        assert_eq!(table.status_flags(duplicate).unwrap() & 0x800, 0x800);
        assert_eq!(table.status_flags(separate).unwrap() & 0x800, 0);
        table.close(original).unwrap();
        table.set_status_flags(duplicate, 0).unwrap();
        assert_eq!(table.status_flags(duplicate).unwrap() & 0x800, 0);
    }

    #[test]
    fn closed_descriptor_is_reused_indefinitely() {
        let mut table = FdTable::new();

        for handle in 1..1024 {
            let fd = table.open(FileHandle(handle), READ, 0).unwrap();
            assert_eq!(fd, 3);
            table.close(fd).unwrap();
        }
    }

    #[test]
    fn take_makes_close_consuming_and_prevents_a_second_release() {
        let mut table = FdTable::new();
        let fd = table.open(FileHandle(99), READ, 0).unwrap();

        let taken = table.take(fd).unwrap();
        assert_eq!(taken.handle, FileHandle(99));
        assert_eq!(table.take(fd), Err(FdError::InvalidFd));
        assert_eq!(table.open(FileHandle(100), READ, 0).unwrap(), fd);
    }

    #[test]
    fn cloexec_handles_are_removed_without_touching_standard_streams() {
        let mut table = FdTable::new();
        let cloexec = table.open(FileHandle::vfs(7), READ, 0x0008_0000).unwrap();
        let normal = table.open(FileHandle::vfs(8), READ, 0).unwrap();
        let taken = table.take_cloexec_handles();
        assert_eq!(taken[cloexec as usize], Some(FileHandle::vfs(7)));
        assert!(table.get(cloexec).is_none());
        assert!(table.get(normal).is_some());
        assert!(table.get(0).is_some());
    }

    #[test]
    fn cloexec_on_standard_streams_is_honoured() {
        let mut table = FdTable::new();
        table.get_mut(1).unwrap().flags |= 0x0008_0000;
        let taken = table.take_cloexec_handles();
        assert_eq!(taken[1], Some(FileHandle(1)));
        assert!(table.get(1).is_none());
        assert!(table.get(0).is_some());
        assert!(table.get(2).is_some());
    }

    #[test]
    fn closed_standard_stream_is_reused() {
        let mut table = FdTable::new();
        table.close(0).unwrap();
        assert_eq!(table.open(FileHandle(99), READ, 0).unwrap(), 0);
    }

    #[test]
    fn dup_clears_cloexec_on_the_new_descriptor() {
        let mut table = FdTable::new();
        let src = table.open(FileHandle::vfs(7), READ, 0x0008_0000).unwrap();
        let dst = table.dup(src).unwrap();
        assert_eq!(table.get(src).unwrap().flags & 0x0008_0000, 0x0008_0000);
        assert_eq!(table.get(dst).unwrap().flags & 0x0008_0000, 0);
        assert_eq!(table.get(dst).unwrap().handle, FileHandle::vfs(7));
    }

    #[test]
    fn dup2_returns_displaced_handle_and_clears_cloexec() {
        let mut table = FdTable::new();
        let old = table.open(FileHandle::vfs(7), READ, 0x0008_0000).unwrap();
        let occupied = table.open(FileHandle::vfs(8), READ, 0).unwrap();
        let result = table.dup2(old, occupied).unwrap();
        assert_eq!(result.fd, occupied);
        assert_eq!(result.displaced, Some(FileHandle::vfs(8)));
        assert_eq!(result.service_close, None);
        assert_eq!(table.get(result.fd).unwrap().handle, FileHandle::vfs(7));
        assert_eq!(table.get(result.fd).unwrap().flags & 0x0008_0000, 0);
        let same = table.dup2(old, old).unwrap();
        assert_eq!(same.fd, old);
        assert_eq!(same.displaced, None);
        assert_eq!(same.service_close, None);
    }

    #[test]
    fn take_all_handles_empties_the_table() {
        let mut table = FdTable::new();
        let _ = table.open(FileHandle::vfs(7), READ, 0).unwrap();
        let taken = table.take_all_handles();
        assert_eq!(taken[0], Some(FileHandle(0)));
        assert_eq!(taken[3], Some(FileHandle::vfs(7)));
        assert!(table.get(0).is_none());
        assert!(table.get(3).is_none());
    }

    #[test]
    fn open_and_dup_fill_the_lowest_hole() {
        let mut table = FdTable::new();
        let fd3 = table.open(FileHandle(10), READ, 0).unwrap();
        let fd4 = table.open(FileHandle(11), READ, 0).unwrap();
        assert_eq!((fd3, fd4), (3, 4));

        table.close(fd3).unwrap();
        assert_eq!(table.dup(fd4).unwrap(), 3);
        table.close(fd4).unwrap();
        assert_eq!(table.open(FileHandle(12), READ, 0).unwrap(), 4);
    }

    #[test]
    fn dup3_rejects_same_fd_and_honours_cloexec() {
        let mut table = FdTable::new();
        let old = table.open(FileHandle::vfs(7), READ, 0).unwrap();
        assert_eq!(table.dup3(old, old, true), Err(FdError::InvalidFd));

        let occupied = table.open(FileHandle::vfs(8), READ, 0).unwrap();
        let result = table.dup3(old, occupied, true).unwrap();
        assert_eq!(result.fd, occupied);
        assert_eq!(result.displaced, Some(FileHandle::vfs(8)));
        assert_eq!(result.service_close, None);
        assert_eq!(table.get(result.fd).unwrap().handle, FileHandle::vfs(7));
        assert_eq!(
            table.get(result.fd).unwrap().flags & 0x0008_0000,
            0x0008_0000
        );
        assert_eq!(table.get(old).unwrap().flags & 0x0008_0000, 0);
    }

    #[test]
    fn closing_each_service_duplicate_releases_its_reference() {
        let mut table = FdTable::new();
        let fd = table.open_service(71, 81, CapRights::new(CapRights::READ), 0).unwrap();
        let duplicate = table.dup(fd).unwrap();
        let index = table.get(fd).unwrap().handle.service_index();
        assert!(!table.take(fd).unwrap().release_backend);
        assert_eq!(service_description(index), Some((71, 81)));
        assert!(table.take(duplicate).unwrap().release_backend);
        assert_ne!(service_description(index), Some((71, 81)));
    }

    #[test]
    fn dup2_displacing_the_final_service_reference_returns_its_close_token() {
        let mut table = FdTable::new();
        let old = table.open(FileHandle::vfs(7), READ, 0).unwrap();
        let service = table
            .open_service(77, 88, READ, 0)
            .expect("service descriptor");
        let result = table.dup2(old, service).unwrap();
        assert_eq!(result.fd, service);
        assert_eq!(result.displaced, None);
        assert_eq!(result.service_close, Some((77, 88)));
        assert_eq!(table.get(service).unwrap().handle, FileHandle::vfs(7));

        // A second service descriptor sharing the row keeps the close deferred.
        let first = table.open_service(91, 92, READ, 0).unwrap();
        let second = table.dup(first).unwrap();
        let result = table.dup2(old, second).unwrap();
        assert_eq!(result.service_close, None);
        assert_eq!(table.get(first).unwrap().handle.is_service(), true);
    }

    #[test]
    fn exit_cleanup_consumes_stale_descriptor_alias_without_double_release() {
        let mut table = FdTable::new();
        let alias = table.entries[0].expect("stdin descriptor");
        table.entries[24] = Some(FileDescriptor { fd: 24, ..alias });

        let handles = table.take_all_handles();

        assert_eq!(handles[0], Some(FileHandle(0)));
        assert_eq!(handles[24], None);
        assert!((0..256).all(|fd| table.get(fd).is_none()));
    }
}

#[cfg(all(
    debug_assertions,
    any(
        feature = "thread_admission_test",
        feature = "thread_lifecycle_race_test",
        feature = "thread_migration_test",
        feature = "atomic_futex_test"
    )
))]
pub(crate) fn thread_description_refs() -> usize {
    DESCRIPTIONS.lock().iter().flatten().map(|d| d.refs).sum()
}

#[cfg(test)]
mod thread_snapshot_tests {
    use super::*;
    #[test]
    fn failed_and_uncommitted_snapshots_never_adopt_references() {
        let mut parent = FdTable::new();
        let description = parent.get(0).unwrap().description as usize;
        let refs = || DESCRIPTIONS.lock()[description].as_ref().unwrap().refs;
        let before = refs();
        super::super::thread_admission::host_fault::arm(1);
        assert!(FdTable::try_thread_snapshot(Some(&parent)).is_err());
        super::super::thread_admission::host_fault::arm(0);
        assert_eq!(refs(), before);
        drop(FdTable::try_thread_snapshot(Some(&parent)).unwrap());
        assert_eq!(refs(), before);
        let mut child = FdTable::try_thread_snapshot(Some(&parent)).unwrap();
        child.retain_thread_snapshot();
        assert_eq!(refs(), before + 1);
        child.take_all_handles();
        assert_eq!(refs(), before);
        parent.take_all_handles();
    }
    #[test]
    fn shared_descriptor_placeholder_allocates_no_descriptions() {
        let mut child = FdTable::try_thread_snapshot(None).unwrap();
        assert!((0..256).all(|fd| child.get(fd).is_none()));
        child.retain_thread_snapshot();
        assert!(child.take_all_handles().iter().all(Option::is_none));
    }
}
