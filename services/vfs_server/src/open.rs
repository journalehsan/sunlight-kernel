//! Service-owned regular-file descriptions. This is a trusted lifecycle API,
//! not an ordinary IPC handle interface. Tokens alone never confer authority.
use alloc::sync::Arc;
use spin::Mutex;
use sunlight_fs::{BlockDevice, FileHandle, FileStat, FsError, NullDevice, Vfs};
use sunlight_ipc::vfs_rpc::lifecycle::{Caller, RequestIds};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    ReadWrite,
}
impl Access {
    pub(crate) fn read(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }
    pub(crate) fn write(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seek {
    Start(u64),
    Current(i64),
    End(i64),
}

/// Private backend ownership; only final description destruction closes it.
pub(crate) struct File<Device: BlockDevice> {
    filesystem: Arc<Mutex<Vfs<Device>>>,
    handle: FileHandle,
    directory: bool,
}
impl<Device: BlockDevice> File<Device> {
    pub(crate) fn new(filesystem: Arc<Mutex<Vfs<Device>>>, handle: FileHandle) -> Self {
        Self { filesystem, handle, directory: false }
    }
}
impl<Device: BlockDevice> File<Device> {
    pub(crate) fn directory(filesystem: Arc<Mutex<Vfs<Device>>>, handle: FileHandle) -> Self {
        Self { filesystem, handle, directory: true }
    }
}
impl<Device: BlockDevice> Drop for File<Device> {
    fn drop(&mut self) {
        self.filesystem
            .lock()
            .close(self.handle)
            .expect("private open handle remains owned");
    }
}
struct Description<Device: BlockDevice> {
    file: File<Device>,
    access: Access,
    offset: usize,
}
struct Entry<Device: BlockDevice> {
    owner: Caller,
    token: u64,
    description: Arc<Mutex<Description<Device>>>,
}

/// Each registration has a bounded descriptor inventory and a nonreusing token
/// allocator. Duplicates own separate rows and share one offset and backend pin.
pub(crate) struct Table<const N: usize, Device: BlockDevice = NullDevice> {
    entries: [Option<Entry<Device>>; N],
    ids: RequestIds,
}
impl<const N: usize, Device: BlockDevice> Table<N, Device> {
    pub(crate) const fn new() -> Self {
        Self {
            entries: [const { None }; N],
            ids: RequestIds::new(),
        }
    }
    #[cfg(feature = "vfs-d1b-test")]
    pub(crate) fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    pub(crate) fn capacity(&self) -> Result<(), FsError> {
        if self.entries.iter().any(Option::is_none) {
            Ok(())
        } else {
            Err(FsError::TooManyOpenFiles)
        }
    }
    pub(crate) fn insert(
        &mut self,
        owner: Caller,
        file: File<Device>,
        access: Access,
    ) -> Result<u64, FsError> {
        self.capacity()?;
        let description = Arc::try_new(Mutex::new(Description {
            file,
            access,
            offset: 0,
        }))
        .map_err(|_| FsError::NoMemory)?;
        self.publish(owner, description)
    }
    fn publish(
        &mut self,
        owner: Caller,
        description: Arc<Mutex<Description<Device>>>,
    ) -> Result<u64, FsError> {
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(FsError::TooManyOpenFiles)?;
        let token = self.ids.allocate().ok_or(FsError::TooManyOpenFiles)?;
        self.entries[slot] = Some(Entry {
            owner,
            token,
            description,
        });
        Ok(token)
    }
    fn index(&self, owner: Caller, token: u64) -> Result<usize, FsError> {
        self.entries
            .iter()
            .position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|e| e.owner == owner && e.token == token)
            })
            .ok_or(FsError::BadHandle)
    }
    fn description(&self, owner: Caller, token: u64) -> Result<&Mutex<Description<Device>>, FsError> {
        Ok(&self.entries[self.index(owner, token)?]
            .as_ref()
            .unwrap()
            .description)
    }
    pub(crate) fn directory_handle(&self, owner: Caller, token: u64) -> Result<FileHandle, FsError> {
        let description = self.description(owner, token)?.lock();
        if !description.file.directory {
            return Err(FsError::NotDir);
        }
        Ok(description.file.handle)
    }

    pub(crate) fn duplicate(&mut self, owner: Caller, token: u64) -> Result<u64, FsError> {
        let index = self.index(owner, token)?;
        let description = Arc::clone(&self.entries[index].as_ref().unwrap().description);
        self.publish(owner, description)
    }
    pub(crate) fn close(&mut self, owner: Caller, token: u64) -> Result<(), FsError> {
        let index = self.index(owner, token)?;
        self.entries[index] = None;
        Ok(())
    }
    /// Re-key one description to a replacement owner. The token and shared
    /// description (offset and backend pin) are preserved. The caller must have
    /// independently established that the new owner is the same process after
    /// exec or a thread in the same address space.
    pub(crate) fn transfer(
        &mut self,
        owner: Caller,
        token: u64,
        new_owner: Caller,
    ) -> Result<u64, FsError> {
        if owner == new_owner {
            return Err(FsError::InvalidPath);
        }
        let index = self.index(owner, token)?;
        self.entries[index].as_mut().unwrap().owner = new_owner;
        Ok(token)
    }
    pub(crate) fn retire_caller(&mut self, owner: Caller) {
        for entry in &mut self.entries {
            if entry.as_ref().is_some_and(|e| e.owner == owner) {
                *entry = None;
            }
        }
    }
    pub(crate) fn clear(&mut self) {
        for entry in &mut self.entries {
            *entry = None;
        }
    }
    pub(crate) fn read(
        &self,
        owner: Caller,
        token: u64,
        output: &mut [u8],
    ) -> Result<usize, FsError> {
        let mut d = self.description(owner, token)?.lock();
        if d.file.directory {
            return Err(FsError::IsDir);
        }
        if !d.access.read() {
            return Err(FsError::PermissionDenied);
        }
        d.offset
            .checked_add(output.len())
            .ok_or(FsError::InvalidPath)?;
        let count = d
            .file
            .filesystem
            .lock()
            .read(d.file.handle, d.offset, output)?;
        d.offset += count;
        Ok(count)
    }
    /// Bounded positional read on the retained description. The explicit
    /// offset selects the bytes and never observes or advances the shared
    /// cursor, so duplicates keep one cursor and independent offsets. Access
    /// rights, the open backend pin and its rename/unlink identity are the
    /// same ones ordinary `read` uses; an unrepresentable range fails before
    /// any backend call and a short or empty count is reported unchanged.
    pub(crate) fn read_at(
        &self,
        owner: Caller,
        token: u64,
        offset: u64,
        output: &mut [u8],
    ) -> Result<usize, FsError> {
        let d = self.description(owner, token)?.lock();
        if d.file.directory {
            return Err(FsError::IsDir);
        }
        if !d.access.read() {
            return Err(FsError::PermissionDenied);
        }
        let offset = usize::try_from(offset).map_err(|_| FsError::InvalidPath)?;
        offset
            .checked_add(output.len())
            .ok_or(FsError::InvalidPath)?;
        let count = d
            .file
            .filesystem
            .lock()
            .read(d.file.handle, offset, output)?;
        // The shared cursor is deliberately not updated: the caller asked for
        // an independent positional read on this description.
        Ok(count)
    }
    pub(crate) fn write(&self, owner: Caller, token: u64, input: &[u8]) -> Result<usize, FsError> {
        let mut d = self.description(owner, token)?.lock();
        if d.file.directory {
            return Err(FsError::IsDir);
        }
        if !d.access.write() {
            return Err(FsError::PermissionDenied);
        }
        d.offset
            .checked_add(input.len())
            .ok_or(FsError::InvalidPath)?;
        let count = d
            .file
            .filesystem
            .lock()
            .write(d.file.handle, d.offset, input)?;
        d.offset += count;
        Ok(count)
    }
    pub(crate) fn stat(&self, owner: Caller, token: u64) -> Result<FileStat, FsError> {
        let d = self.description(owner, token)?.lock();
        let result = d.file.filesystem.lock().fstat_handle(d.file.handle);
        result
    }
    pub(crate) fn truncate(&self, owner: Caller, token: u64) -> Result<(), FsError> {
        let d = self.description(owner, token)?.lock();
        if d.file.directory {
            return Err(FsError::IsDir);
        }
        if !d.access.write() {
            return Err(FsError::PermissionDenied);
        }
        let result = d.file.filesystem.lock().truncate(d.file.handle);
        result
    }
    pub(crate) fn seek(&self, owner: Caller, token: u64, seek: Seek) -> Result<usize, FsError> {
        let mut d = self.description(owner, token)?.lock();
        if d.file.directory {
            return Err(FsError::IsDir);
        }
        let position = match seek {
            Seek::Start(offset) => i128::from(offset),
            Seek::Current(delta) => d.offset as i128 + i128::from(delta),
            Seek::End(delta) => {
                d.file.filesystem.lock().fstat_handle(d.file.handle)?.size as i128
                    + i128::from(delta)
            }
        };
        let position = usize::try_from(position).map_err(|_| FsError::InvalidPath)?;
        d.offset = position;
        Ok(position)
    }
}
