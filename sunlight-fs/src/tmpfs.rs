//! Bounded temporary storage. Payloads live in owned PMM pages; only bounded
//! paths, inode records and frame addresses use the fallible metadata allocator.
mod storage;
use crate::ramfs::{RamEntry, RAMFS_MAX_ENTRIES, RAMFS_MAX_HANDLES};
use crate::vfs::{
    make_local_handle, mode, next_handle_generation, split_local_handle, FileHandle, FileStat,
    FileSystem, FileType, VfsDirEntry,
};
use crate::{path, FsError};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;
pub use storage::{
    TmpfsInode, TmpfsLimits, TMPFS_DEFAULT_MAX_BYTES, TMPFS_DEFAULT_MAX_INODES, TMPFS_PATH_MAX,
};
#[cfg(test)]
use sunlight_mm::PAGE_SIZE;

fn copy_path(bytes: &[u8]) -> Result<Vec<u8>, FsError> {
    if bytes.len() > TMPFS_PATH_MAX {
        return Err(FsError::InvalidPath);
    }
    let mut path = Vec::new();
    path.try_reserve_exact(bytes.len())
        .map_err(|_| FsError::NoMemory)?;
    path.extend_from_slice(bytes);
    Ok(path)
}
/// A dynamic entry created at runtime (e.g., by mkdir or write).
struct DynamicEntry {
    path: Vec<u8>,
    data: TmpfsInode,
    uid: u32,
    gid: u32,
    mode: u16,
    is_dir: bool,
}

/// Lightweight directory index. Full local paths map to stable inode IDs.
/// Entries and path lengths are bounded by the mount quota and PATH_MAX.
#[derive(Default)]
pub struct TmpfsDir {
    entries: Vec<DynamicEntry>,
}
impl TmpfsDir {
    pub fn lookup(&self, path: &str) -> Option<u64> {
        self.entries
            .iter()
            .find(|e| e.path == path.as_bytes())
            .map(|e| e.data.id)
    }
}
pub struct Tmpfs {
    entries: &'static [RamEntry],
    handles: [Option<RamOpen>; RAMFS_MAX_HANDLES],
    generations: [u32; RAMFS_MAX_HANDLES],
    /// Mutable data copies for static entries. Indexed by entry index.
    buffers: Vec<Option<TmpfsInode>>,
    limits: Option<Arc<TmpfsLimits>>,
    next_id: u64,
    /// Dynamic entries created at runtime.
    dynamic: TmpfsDir,
}

#[derive(Clone, Copy)]
struct RamOpen {
    entry_idx: usize,
    generation: u32,
}

impl Tmpfs {
    /// Allocation-free mount of a trusted boot seed using the live defaults.
    /// For untrusted seeds or configurable quotas use `with_limits`.
    pub fn new(entries: &'static [RamEntry]) -> Self {
        Self {
            entries,
            handles: [None; RAMFS_MAX_HANDLES],
            generations: [0; RAMFS_MAX_HANDLES],
            buffers: Vec::new(),
            dynamic: TmpfsDir::default(),
            limits: None,
            next_id: entries.len() as u64 + 1,
        }
    }

    fn all_entry_count(&self) -> usize {
        self.entries.len() + self.dynamic.entries.len()
    }

    fn entry_idx(&self, path: &str) -> Result<usize, FsError> {
        validate_path(path)?;
        if path == "/" && !self.entries.iter().any(|e| e.path == "/") {
            return Ok(usize::MAX);
        }
        if let Some(idx) = self.entries.iter().position(|e| e.path == path) {
            if idx >= RAMFS_MAX_ENTRIES {
                return Err(FsError::NoSpace);
            }
            return Ok(idx);
        }
        if let Some(idx) = self
            .dynamic
            .entries
            .iter()
            .position(|e| core::str::from_utf8(&e.path).ok() == Some(path))
        {
            return Ok(self.entries.len() + idx);
        }
        Err(FsError::NotFound)
    }

    fn is_dir(&self, idx: usize) -> bool {
        if idx == usize::MAX {
            return true;
        }
        if idx < self.entries.len() {
            self.entries[idx].is_dir
        } else {
            self.dynamic.entries[idx - self.entries.len()].is_dir
        }
    }

    fn entry_mode(&self, idx: usize) -> u16 {
        if idx == usize::MAX {
            return mode::DIR_755;
        }
        if idx < self.entries.len() {
            self.entries[idx].mode
        } else {
            self.dynamic.entries[idx - self.entries.len()].mode
        }
    }

    fn entry_uid(&self, idx: usize) -> u32 {
        if idx == usize::MAX {
            return 0;
        }
        if idx < self.entries.len() {
            self.entries[idx].uid
        } else {
            self.dynamic.entries[idx - self.entries.len()].uid
        }
    }

    fn entry_gid(&self, idx: usize) -> u32 {
        if idx == usize::MAX {
            return 0;
        }
        if idx < self.entries.len() {
            self.entries[idx].gid
        } else {
            self.dynamic.entries[idx - self.entries.len()].gid
        }
    }

    fn entry_data_len(&self, idx: usize) -> usize {
        if idx == usize::MAX {
            return 0;
        }
        if idx < self.entries.len() {
            self.buffers
                .get(idx)
                .and_then(Option::as_ref)
                .map(|v| v.size())
                .unwrap_or(self.entries[idx].data.len())
        } else {
            self.dynamic.entries[idx - self.entries.len()].data.size()
        }
    }

    pub fn with_limits(entries: &'static [RamEntry], limits: TmpfsLimits) -> Result<Self, FsError> {
        if entries.len() > RAMFS_MAX_ENTRIES
            || (entries.len() + usize::from(!entries.iter().any(|e| e.path == "/")))
                > limits.max_inodes
        {
            return Err(FsError::NoSpace);
        }
        if limits.used_bytes() != 0 || limits.used_inodes() != 0 {
            return Err(FsError::InvalidPath);
        }
        let mut fs = Self::new(entries);
        limits.used_inodes.store(
            entries.len() + usize::from(!entries.iter().any(|e| e.path == "/")),
            Ordering::Release,
        );
        fs.limits = Some(Arc::try_new(limits).map_err(|_| FsError::NoMemory)?);
        Ok(fs)
    }

    fn quota(&mut self) -> Result<Arc<TmpfsLimits>, FsError> {
        if self.entries.len() > RAMFS_MAX_ENTRIES {
            return Err(FsError::NoSpace);
        }
        if self.limits.is_none() {
            let limits = TmpfsLimits::new(TMPFS_DEFAULT_MAX_BYTES, TMPFS_DEFAULT_MAX_INODES);
            limits.used_inodes.store(
                self.entries.len() + usize::from(!self.entries.iter().any(|e| e.path == "/")),
                Ordering::Release,
            );
            self.limits = Some(Arc::try_new(limits).map_err(|_| FsError::NoMemory)?);
        }
        self.limits.as_ref().cloned().ok_or(FsError::NoMemory)
    }

    pub fn usage(&self) -> (usize, usize) {
        self.limits
            .as_ref()
            .map(|l| (l.used_bytes(), l.used_inodes()))
            .unwrap_or((
                0,
                self.entries.len() + usize::from(!self.entries.iter().any(|e| e.path == "/")),
            ))
    }

    /// Return the stable inode number for a filesystem-local path.
    pub fn lookup(&self, path: &str) -> Result<u64, FsError> {
        let idx = self.entry_idx(path)?;
        if idx == usize::MAX {
            return Ok(0);
        }
        Ok(if idx < self.entries.len() {
            self.buffers
                .get(idx)
                .and_then(Option::as_ref)
                .map(|inode| inode.id)
                .unwrap_or(idx as u64 + 1)
        } else {
            self.dynamic.entries[idx - self.entries.len()].data.id
        })
    }

    fn add_entry(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
        is_dir: bool,
    ) -> Result<usize, FsError> {
        let path = copy_path(path.as_bytes())?;
        self.dynamic
            .entries
            .try_reserve_exact(1)
            .map_err(|_| FsError::NoMemory)?;
        let limits = self.quota()?;
        let id = self.next_id;
        let next = id.checked_add(1).ok_or(FsError::NoSpace)?;
        let data = TmpfsInode::new(id, mode, limits)?;
        self.dynamic.entries.push(DynamicEntry {
            path,
            data,
            uid,
            gid,
            mode,
            is_dir,
        });
        self.next_id = next;
        Ok(self.all_entry_count() - 1)
    }

    fn prepare_buffers(&mut self) -> Result<(), FsError> {
        if self.buffers.len() < self.entries.len() {
            self.buffers
                .try_reserve_exact(self.entries.len() - self.buffers.len())
                .map_err(|_| FsError::NoMemory)?;
            self.buffers.resize_with(self.entries.len(), || None);
        }
        Ok(())
    }

    fn materialize(&mut self, idx: usize, capacity: usize, preserve: bool) -> Result<(), FsError> {
        if idx < self.entries.len() && self.buffers.get(idx).and_then(Option::as_ref).is_none() {
            self.prepare_buffers()?;
            let limits = self.quota()?;
            let original = if preserve {
                self.entries[idx].data
            } else {
                &[]
            };
            let mut inode = TmpfsInode {
                id: idx as u64 + 1,
                mode: self.entries[idx].mode,
                size: 0,
                pages: Vec::new(),
                limits: limits.clone(),
                charged_inode: false,
            };
            inode.reserve_pages(capacity.max(original.len()))?;
            inode.write_at(0, original, &limits)?;
            self.buffers[idx] = Some(inode);
        }
        Ok(())
    }

    fn inode_mut(&mut self, idx: usize) -> Result<&mut TmpfsInode, FsError> {
        if idx < self.entries.len() {
            self.buffers[idx].as_mut().ok_or(FsError::Io)
        } else {
            Ok(&mut self.dynamic.entries[idx - self.entries.len()].data)
        }
    }

    fn set_entry_mode(&mut self, idx: usize, mode: u16) {
        if idx < self.entries.len() {
            // Static entries are immutable for mode; ignore or we could add a buffer for metadata
        } else {
            let entry = &mut self.dynamic.entries[idx - self.entries.len()];
            entry.mode = (entry.mode & 0o170000) | (mode & 0o7777);
            entry.data.mode = entry.mode;
        }
    }

    fn set_entry_owner(&mut self, idx: usize, uid: u32, gid: u32) {
        if idx < self.entries.len() {
            // Static entries are immutable for owner
        } else {
            self.dynamic.entries[idx - self.entries.len()].uid = uid;
            self.dynamic.entries[idx - self.entries.len()].gid = gid;
        }
    }

    fn entry_path(&self, idx: usize) -> Option<&str> {
        if idx == usize::MAX {
            return Some("/");
        }
        if idx < self.entries.len() {
            Some(self.entries[idx].path)
        } else {
            core::str::from_utf8(&self.dynamic.entries[idx - self.entries.len()].path).ok()
        }
    }

    fn alloc_handle(&mut self, entry_idx: usize) -> Result<FileHandle, FsError> {
        for (idx, slot) in self.handles.iter_mut().enumerate() {
            if slot.is_none() {
                let generation = next_handle_generation(self.generations[idx]);
                self.generations[idx] = generation;
                *slot = Some(RamOpen {
                    entry_idx,
                    generation,
                });
                return Ok(make_local_handle(idx, generation));
            }
        }
        Err(FsError::TooManyOpenFiles)
    }

    fn handle_entry_idx(&self, handle: FileHandle) -> Result<usize, FsError> {
        let (idx, generation) = split_local_handle(handle)?;
        self.handles
            .get(idx)
            .and_then(|slot| slot.filter(|open| open.generation == generation))
            .map(|open| open.entry_idx)
            .ok_or(FsError::BadHandle)
    }

    fn parent_is_dir(&self, path: &str) -> Result<bool, FsError> {
        let parent = parent_path(path)?;
        if parent == "/" {
            return Ok(true);
        }
        let idx = self.entry_idx(parent)?;
        Ok(self.is_dir(idx))
    }
}

impl Drop for Tmpfs {
    fn drop(&mut self) {
        if let Some(limits) = &self.limits {
            let count =
                self.entries.len() + usize::from(!self.entries.iter().any(|e| e.path == "/"));
            for _ in 0..count {
                limits.free_inode();
            }
        }
    }
}

impl FileSystem for Tmpfs {
    fn open(&mut self, path: &str) -> Result<FileHandle, FsError> {
        let entry_idx = self.entry_idx(path)?;
        self.alloc_handle(entry_idx)
    }

    fn create_file(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        validate_path(path)?;
        if self.entry_idx(path).is_ok() {
            return self.open(path);
        }
        if !self.parent_is_dir(path)? {
            return Err(FsError::NotDir);
        }
        if self.handles.iter().all(Option::is_some) {
            return Err(FsError::TooManyOpenFiles);
        }
        let idx = self.add_entry(path, uid, gid, mode::S_IFREG | (mode & 0o7777), false)?;
        self.alloc_handle(idx)
    }

    fn create_file_exclusive(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        validate_path(path)?;
        if self.entry_idx(path).is_ok() {
            return Err(FsError::AlreadyExists);
        }
        if !self.parent_is_dir(path)? {
            return Err(FsError::NotDir);
        }
        if self.handles.iter().all(Option::is_some) {
            return Err(FsError::TooManyOpenFiles);
        }
        let idx = self.add_entry(path, uid, gid, mode::S_IFREG | (mode & 0o7777), false)?;
        self.alloc_handle(idx)
    }

    fn read(
        &mut self,
        handle: FileHandle,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        let entry_idx = self.handle_entry_idx(handle)?;
        if self.is_dir(entry_idx) {
            return Err(FsError::IsDir);
        }
        if entry_idx < self.entries.len()
            && self
                .buffers
                .get(entry_idx)
                .and_then(Option::as_ref)
                .is_none()
        {
            let data = self.entries[entry_idx].data;
            let len = buf.len().min(data.len().saturating_sub(offset));
            if len != 0 {
                buf[..len].copy_from_slice(&data[offset..offset + len]);
            }
            return Ok(len);
        }
        self.inode_mut(entry_idx)?.read_at(offset, buf)
    }

    fn write(&mut self, handle: FileHandle, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        let idx = self.handle_entry_idx(handle)?;
        if self.is_dir(idx) {
            return Err(FsError::IsDir);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let end = offset.checked_add(buf.len()).ok_or(FsError::NoSpace)?;
        self.materialize(idx, end, true)?;
        let limits = self.quota()?;
        self.inode_mut(idx)?.write_at(offset, buf, &limits)
    }

    fn reserve(&mut self, handle: FileHandle, size: usize) -> Result<(), FsError> {
        let idx = self.handle_entry_idx(handle)?;
        if self.is_dir(idx) {
            return Err(FsError::IsDir);
        }
        self.materialize(idx, size, true)?;
        self.inode_mut(idx)?.reserve_pages(size)
    }

    fn truncate(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let idx = self.handle_entry_idx(handle)?;
        if self.is_dir(idx) {
            return Err(FsError::IsDir);
        }
        self.materialize(idx, 0, false)?;
        self.inode_mut(idx)?.truncate_to(0)
    }

    fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let (idx, generation) = split_local_handle(handle)?;
        let slot = self.handles.get_mut(idx).ok_or(FsError::BadHandle)?;
        if slot
            .as_ref()
            .map(|open| open.generation != generation)
            .unwrap_or(true)
        {
            return Err(FsError::BadHandle);
        }
        *slot = None;
        Ok(())
    }

    fn sync_file(&mut self, handle: FileHandle) -> Result<(), FsError> {
        self.handle_entry_idx(handle)?;
        Err(FsError::Unsupported)
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), FsError> {
        let entry_idx = self.entry_idx(path)?;
        if !self.is_dir(entry_idx) {
            return Err(FsError::NotDir);
        }
        Err(FsError::Unsupported)
    }

    fn fstat_handle(&mut self, handle: FileHandle) -> Result<FileStat, FsError> {
        let idx = self.handle_entry_idx(handle)?;
        let ft = if self.is_dir(idx) {
            FileType::Directory
        } else {
            FileType::File
        };
        Ok(FileStat {
            file_type: ft,
            size: if ft == FileType::Directory {
                0
            } else {
                self.entry_data_len(idx)
            },
            uid: self.entry_uid(idx),
            gid: self.entry_gid(idx),
            mode: self.entry_mode(idx),
            nlinks: if ft == FileType::Directory { 2 } else { 1 },
        })
    }

    fn stat(&mut self, path: &str) -> Result<FileStat, FsError> {
        let entry_idx = self.entry_idx(path)?;
        let ft = if self.is_dir(entry_idx) {
            FileType::Directory
        } else {
            FileType::File
        };
        let size = if ft == FileType::Directory {
            0
        } else {
            self.entry_data_len(entry_idx)
        };
        let nlinks = if ft == FileType::Directory { 2 } else { 1 };
        Ok(FileStat {
            file_type: ft,
            size,
            uid: self.entry_uid(entry_idx),
            gid: self.entry_gid(entry_idx),
            mode: self.entry_mode(entry_idx),
            nlinks,
        })
    }

    fn mkdir(&mut self, path: &str, uid: u32, gid: u32, mode: u16) -> Result<(), FsError> {
        validate_path(path)?;
        if self.entry_idx(path).is_ok() {
            return Err(FsError::AlreadyExists);
        }
        if !self.parent_is_dir(path)? {
            return Err(FsError::NotDir);
        }
        self.add_entry(path, uid, gid, mode::S_IFDIR | (mode & 0o7777), true)?;
        Ok(())
    }

    fn chmod(&mut self, path: &str, mode: u16) -> Result<(), FsError> {
        let entry_idx = self.entry_idx(path)?;
        if entry_idx == usize::MAX || entry_idx < self.entries.len() {
            return Err(FsError::ReadOnlyFilesystem);
        }
        self.set_entry_mode(entry_idx, mode);
        Ok(())
    }

    fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), FsError> {
        let entry_idx = self.entry_idx(path)?;
        if entry_idx == usize::MAX {
            return Err(FsError::ReadOnlyFilesystem);
        }
        self.set_entry_owner(entry_idx, uid, gid);
        Ok(())
    }

    fn unlink(&mut self, path: &str) -> Result<(), FsError> {
        validate_path(path)?;
        let entry_idx = self.entry_idx(path)?;
        if self.is_dir(entry_idx) {
            return Err(FsError::IsDir);
        }
        if entry_idx == usize::MAX || entry_idx < self.entries.len() {
            return Err(FsError::ReadOnlyFilesystem);
        }
        let dyn_idx = entry_idx - self.entries.len();
        // Invalidate any open handles pointing at this entry or at entries
        // that shifted past it.
        for slot in self.handles.iter_mut() {
            if let Some(open) = slot.as_mut() {
                if open.entry_idx == entry_idx {
                    *slot = None;
                } else if open.entry_idx > entry_idx {
                    open.entry_idx -= 1;
                }
            }
        }
        self.dynamic.entries.remove(dyn_idx);
        Ok(())
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<(), FsError> {
        validate_path(old)?;
        validate_path(new)?;
        let entry_idx = self.entry_idx(old)?;
        if entry_idx == usize::MAX || entry_idx < self.entries.len() {
            return Err(FsError::ReadOnlyFilesystem);
        }
        let dyn_idx = entry_idx - self.entries.len();
        if old == new {
            return Ok(());
        }
        let parent = new
            .rsplit_once('/')
            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
            .ok_or(FsError::NotFound)?;
        if parent != "/" && !self.is_dir(self.entry_idx(parent)?) {
            return Err(FsError::NotDir);
        }
        if self.is_dir(entry_idx) {
            let mut prefix = copy_path(old.as_bytes())?;
            prefix.try_reserve_exact(1).map_err(|_| FsError::NoMemory)?;
            prefix.push(b'/');
            if self
                .entries
                .iter()
                .any(|entry| entry.path.as_bytes().starts_with(&prefix))
            {
                return Err(FsError::ReadOnlyFilesystem);
            }
            if new.as_bytes().starts_with(&prefix) {
                return Err(FsError::Unsupported);
            }
            if self.entry_idx(new).is_ok() {
                return Err(FsError::AlreadyExists);
            }
            // RAMFS paths are stored in full on every entry. Move the whole subtree.
            let mut updates = Vec::new();
            for (index, entry) in self.dynamic.entries.iter().enumerate() {
                if index == dyn_idx || entry.path.starts_with(&prefix) {
                    if new
                        .len()
                        .checked_add(entry.path.len() - old.len())
                        .filter(|n| *n <= TMPFS_PATH_MAX)
                        .is_none()
                    {
                        return Err(FsError::InvalidPath);
                    }
                    let mut path = copy_path(new.as_bytes())?;
                    path.try_reserve_exact(entry.path.len() - old.len())
                        .map_err(|_| FsError::NoMemory)?;
                    path.extend_from_slice(&entry.path[old.len()..]);
                    let text = core::str::from_utf8(&path).map_err(|_| FsError::Unsupported)?;
                    validate_path(text)?;
                    updates
                        .try_reserve_exact(1)
                        .map_err(|_| FsError::NoMemory)?;
                    updates.push((index, path));
                }
            }
            for (index, path) in updates {
                self.dynamic.entries[index].path = path;
            }
            return Ok(());
        }
        let new_path = copy_path(new.as_bytes())?;
        // If destination exists (and is a file), remove it first.
        if let Ok(dst_idx) = self.entry_idx(new) {
            if self.is_dir(dst_idx) {
                return Err(FsError::IsDir);
            }
            if dst_idx < self.entries.len() {
                self.prepare_buffers()?;
                let source = self.dynamic.entries.remove(dyn_idx);
                let mut data = source.data;
                data.limits.free_inode();
                data.charged_inode = false;
                self.buffers[dst_idx] = Some(data);
                for slot in self.handles.iter_mut() {
                    if let Some(open) = slot.as_mut() {
                        if open.entry_idx == entry_idx {
                            open.entry_idx = dst_idx;
                        } else if open.entry_idx > entry_idx {
                            open.entry_idx -= 1;
                        }
                    }
                }
                return Ok(());
            }
            if dst_idx >= self.entries.len() {
                let ddyn = dst_idx - self.entries.len();
                for slot in self.handles.iter_mut() {
                    if let Some(open) = slot.as_mut() {
                        if open.entry_idx == dst_idx {
                            *slot = None;
                        } else if open.entry_idx > dst_idx {
                            open.entry_idx -= 1;
                        }
                    }
                }
                self.dynamic.entries.remove(ddyn);
                // dyn_idx may have shifted if dst came before src in the vec
                let dyn_idx = self.entry_idx(old)? - self.entries.len();
                self.dynamic.entries[dyn_idx].path = new_path;
                return Ok(());
            }
            return Err(FsError::ReadOnlyFilesystem);
        }
        self.dynamic.entries[dyn_idx].path = new_path;
        Ok(())
    }

    fn read_dir(
        &mut self,
        path: &str,
        f: &mut dyn FnMut(&VfsDirEntry) -> bool,
    ) -> Result<(), FsError> {
        validate_path(path)?;
        if path != "/" {
            let idx = self.entry_idx(path)?;
            if !self.is_dir(idx) {
                return Err(FsError::NotDir);
            }
        }
        for idx in 0..self.all_entry_count() {
            let Some(entry_path) = self.entry_path(idx) else {
                continue;
            };
            let Some(name) = direct_child_name(entry_path, path) else {
                continue;
            };
            let entry = if self.is_dir(idx) {
                VfsDirEntry::from_bytes(name.as_bytes(), FileType::Directory, 0)
            } else {
                VfsDirEntry::from_bytes(name.as_bytes(), FileType::File, self.entry_data_len(idx))
            };
            if !f(&entry) {
                break;
            }
        }
        Ok(())
    }

    fn handle_path(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
        let idx = self.handle_entry_idx(handle)?;
        let path = self.entry_path(idx).ok_or(FsError::BadHandle)?;
        let bytes = path.as_bytes();
        if buf.len() < bytes.len() {
            return Err(FsError::InvalidPath);
        }
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}

fn parent_path(path: &str) -> Result<&str, FsError> {
    if path == "/" {
        return Err(FsError::InvalidPath);
    }
    match path.rfind('/') {
        Some(0) => Ok("/"),
        Some(idx) => Ok(&path[..idx]),
        None => Err(FsError::InvalidPath),
    }
}

/// If `entry_path` names a direct child of directory `dir`, return its name.
fn direct_child_name<'a>(entry_path: &'a str, dir: &str) -> Option<&'a str> {
    let rest = if dir == "/" {
        entry_path.strip_prefix('/')?
    } else {
        entry_path.strip_prefix(dir)?.strip_prefix('/')?
    };
    if rest.is_empty() || rest.contains('/') {
        None
    } else {
        Some(rest)
    }
}

fn validate_path(text: &str) -> Result<(), FsError> {
    path::validate_absolute(text)?;
    if text.len() > TMPFS_PATH_MAX
        || (text != "/"
            && text
                .split('/')
                .skip(1)
                .any(|s| s.is_empty() || s.len() > crate::VFS_NAME_MAX))
    {
        return Err(FsError::InvalidPath);
    }
    Ok(())
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod quota_tests;
