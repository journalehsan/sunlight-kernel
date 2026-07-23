use core::num::NonZeroU32;

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
#[derive(Clone, Copy)]
pub struct FileDescriptor {
    pub fd: i32,
    pub handle: FileHandle,
    pub rights: CapRights,
    pub flags: u32, // O_RDONLY, O_WRONLY, O_RDWR, O_CLOEXEC
    /// Current read/write position (VFS-backed fds only).
    pub offset: usize,
}

/// File descriptor table for a process (max 256 open fds)
pub struct FdTable {
    entries: [Option<FileDescriptor>; 256],
}

impl FdTable {
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
        });
        table.entries[1] = Some(FileDescriptor {
            fd: 1,
            handle: FileHandle(1),
            rights: CapRights::new(CapRights::WRITE | CapRights::FSTAT),
            flags: 1, // O_WRONLY
            offset: 0,
        });
        table.entries[2] = Some(FileDescriptor {
            fd: 2,
            handle: FileHandle(2),
            rights: CapRights::new(CapRights::WRITE | CapRights::FSTAT),
            flags: 1, // O_WRONLY
            offset: 0,
        });
        table
    }

    pub fn new_boxed() -> alloc::boxed::Box<Self> {
        let mut table = alloc::boxed::Box::<Self>::new_uninit();
        let ptr = table.as_mut_ptr();
        unsafe {
            let entries = core::ptr::addr_of_mut!((*ptr).entries) as *mut Option<FileDescriptor>;
            for idx in 0..256 {
                entries.add(idx).write(None);
            }
            let mut table = table.assume_init();
            table.entries[0] = Some(FileDescriptor {
                fd: 0,
                handle: FileHandle(0),
                rights: CapRights::new(CapRights::READ | CapRights::FSTAT),
                flags: 0,
                offset: 0,
            });
            table.entries[1] = Some(FileDescriptor {
                fd: 1,
                handle: FileHandle(1),
                rights: CapRights::new(CapRights::WRITE | CapRights::FSTAT),
                flags: 1,
                offset: 0,
            });
            table.entries[2] = Some(FileDescriptor {
                fd: 2,
                handle: FileHandle(2),
                rights: CapRights::new(CapRights::WRITE | CapRights::FSTAT),
                flags: 1,
                offset: 0,
            });
            table
        }
    }

    /// Open a new file descriptor
    pub fn open(
        &mut self,
        handle: FileHandle,
        rights: CapRights,
        flags: u32,
    ) -> Result<i32, FdError> {
        let slot = self
            .entries
            .iter()
            .enumerate()
            .skip(3)
            .find_map(|(idx, entry)| entry.is_none().then_some(idx))
            .ok_or(FdError::NoSlots)?;
        let fd = slot as i32;
        self.entries[slot] = Some(FileDescriptor {
            fd,
            handle,
            rights,
            flags,
            offset: 0,
        });

        Ok(fd)
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
        self.entries[fd as usize].take().ok_or(FdError::InvalidFd)
    }

    /// Remove descriptors marked close-on-exec and return their backing
    /// handles so the caller can release VFS resources.
    pub fn take_cloexec_handles(&mut self) -> [Option<FileHandle>; 256] {
        const O_CLOEXEC: u32 = 0x0008_0000;
        let mut handles = [None; 256];
        for (idx, entry) in self.entries.iter_mut().enumerate().skip(3) {
            if entry
                .as_ref()
                .map(|descriptor| descriptor.flags & O_CLOEXEC != 0)
                .unwrap_or(false)
            {
                handles[idx] = entry.take().map(|descriptor| descriptor.handle);
            }
        }
        handles
    }

    /// Get a file descriptor (for inspection)
    pub fn get(&self, fd: i32) -> Option<&FileDescriptor> {
        if fd < 0 || fd >= 256 {
            return None;
        }
        self.entries[fd as usize].as_ref()
    }

    /// Get a mutable file descriptor
    pub fn get_mut(&mut self, fd: i32) -> Option<&mut FileDescriptor> {
        if fd < 0 || fd >= 256 {
            return None;
        }
        self.entries[fd as usize].as_mut()
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
        self.entries[fd as usize] = Some(FileDescriptor {
            fd,
            handle,
            rights,
            flags,
            offset: 0,
        });
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

    /// Duplicate a file descriptor
    pub fn dup(&mut self, fd: i32) -> Result<i32, FdError> {
        let orig = *self.get(fd).ok_or(FdError::InvalidFd)?;
        self.open(orig.handle, orig.rights, orig.flags)
    }

    /// Duplicate fd to specific fd number (dup2)
    pub fn dup2(&mut self, old_fd: i32, new_fd: i32) -> Result<i32, FdError> {
        if new_fd < 0 || new_fd >= 256 {
            return Err(FdError::InvalidFd);
        }

        let orig = self.get(old_fd).ok_or(FdError::InvalidFd)?;
        let desc = FileDescriptor {
            fd: new_fd,
            handle: orig.handle,
            rights: orig.rights,
            flags: orig.flags,
            offset: orig.offset,
        };

        self.entries[new_fd as usize] = Some(desc);
        Ok(new_fd)
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
            }
            table.assume_init()
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
}
