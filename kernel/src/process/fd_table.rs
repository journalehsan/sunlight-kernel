use core::num::NonZeroU32;

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
    pub const CONNECT: u64 = 1 << 10;  // Phase 5: network
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
        Self { bits: self.bits & other.bits }
    }

    pub fn union(self, other: Self) -> Self {
        Self { bits: self.bits | other.bits }
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
    /// Marks a handle backed by the kernel VFS (only meaningful when the
    /// pipe flag is clear; bits 0..30 carry the packed Vfs handle).
    const VFS_FLAG: u32 = 0x4000_0000;

    pub fn is_pipe(self) -> bool {
        (self.0 & Self::PIPE_FLAG) != 0
    }

    pub fn pipe_index(self) -> u32 {
        self.0 & 0x7FFF_FFFF
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
    pub flags: u32,  // O_RDONLY, O_WRONLY, O_RDWR, O_CLOEXEC
    /// Current read/write position (VFS-backed fds only).
    pub offset: usize,
}

/// File descriptor table for a process (max 256 open fds)
pub struct FdTable {
    entries: [Option<FileDescriptor>; 256],
    next_fd: usize,
}

impl FdTable {
    pub fn new() -> Self {
        let entries: [Option<FileDescriptor>; 256] = [None; 256];
        let mut table = Self {
            entries,
            next_fd: 3,  // Reserve 0=stdin, 1=stdout, 2=stderr
        };
        // Initialize standard streams
        table.entries[0] = Some(FileDescriptor {
            fd: 0,
            handle: FileHandle(0),
            rights: CapRights::new(CapRights::READ | CapRights::FSTAT),
            flags: 0,  // O_RDONLY
            offset: 0,
        });
        table.entries[1] = Some(FileDescriptor {
            fd: 1,
            handle: FileHandle(1),
            rights: CapRights::new(CapRights::WRITE | CapRights::FSTAT),
            flags: 1,  // O_WRONLY
            offset: 0,
        });
        table.entries[2] = Some(FileDescriptor {
            fd: 2,
            handle: FileHandle(2),
            rights: CapRights::new(CapRights::WRITE | CapRights::FSTAT),
            flags: 1,  // O_WRONLY
            offset: 0,
        });
        table
    }

    /// Open a new file descriptor
    pub fn open(&mut self, handle: FileHandle, rights: CapRights, flags: u32) -> Result<i32, FdError> {
        if self.next_fd >= 256 {
            return Err(FdError::NoSlots);
        }

        let fd = self.next_fd as i32;
        self.entries[self.next_fd] = Some(FileDescriptor {
            fd,
            handle,
            rights,
            flags,
            offset: 0,
        });
        self.next_fd += 1;

        Ok(fd)
    }

    /// Close a file descriptor
    pub fn close(&mut self, fd: i32) -> Result<(), FdError> {
        if fd < 0 || fd >= 256 {
            return Err(FdError::InvalidFd);
        }
        if self.entries[fd as usize].is_none() {
            return Err(FdError::InvalidFd);
        }
        self.entries[fd as usize] = None;
        Ok(())
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
    pub fn install_at(&mut self, fd: i32, handle: FileHandle, rights: CapRights, flags: u32) -> Result<(), FdError> {
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
        let orig = self.get(fd).ok_or(FdError::InvalidFd)?;
        let new_fd = self.next_fd as i32;

        if self.next_fd >= 256 {
            return Err(FdError::NoSlots);
        }

        self.entries[self.next_fd] = Some(FileDescriptor {
            fd: new_fd,
            handle: orig.handle,
            rights: orig.rights,
            flags: orig.flags,
            offset: orig.offset,
        });
        self.next_fd += 1;

        Ok(new_fd)
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
