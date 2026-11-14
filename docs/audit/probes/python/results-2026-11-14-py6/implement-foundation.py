from pathlib import Path
p=Path('sunlight-fs/src/vfs.rs');s=p.read_text();s=s.replace('mod storage_tests;','mod storage_tests;\n\nmod lookup;\npub use lookup::{BackendCapabilities, Metadata, MetadataSource, ResolvedPath, RESOLVE_PATH_MAX};')
needle='pub trait FileSystem {\n';s=s.replace(needle,needle+'''    /// Provenance of the native subset, independent of POSIX compatibility.
    fn capabilities(&self) -> BackendCapabilities;
    fn object_id(&self, _path: &str) -> Result<Option<u64>, FsError> { Ok(None) }
    fn handle_object_id(&self, _handle: FileHandle) -> Result<Option<u64>, FsError> { Ok(None) }
''')
s=s.replace('impl<D: BlockDevice> FileSystem for FatFs<D> {','''impl<D: BlockDevice> FileSystem for FatFs<D> {
    fn capabilities(&self) -> BackendCapabilities { BackendCapabilities::FAT }
''')
s=s.replace('impl<D: BlockDevice> FileSystem for FsNode<D> {','''impl<D: BlockDevice> FileSystem for FsNode<D> {
    fn capabilities(&self) -> BackendCapabilities {
        match self { Self::Ram(fs) => fs.capabilities(), Self::Fat(fs) => fs.capabilities() }
    }
    fn object_id(&self, path: &str) -> Result<Option<u64>, FsError> {
        match self { Self::Ram(fs) => fs.object_id(path), Self::Fat(fs) => fs.object_id(path) }
    }
    fn handle_object_id(&self, handle: FileHandle) -> Result<Option<u64>, FsError> {
        match self { Self::Ram(fs) => fs.handle_object_id(handle), Self::Fat(fs) => fs.handle_object_id(handle) }
    }
''');p.write_text(s)
p=Path('sunlight-fs/src/tmpfs.rs');s=p.read_text();s=s.replace('impl FileSystem for Tmpfs {','''impl FileSystem for Tmpfs {
    fn capabilities(&self) -> crate::vfs::BackendCapabilities {
        crate::vfs::BackendCapabilities::TMPFS
    }
    fn object_id(&self, path: &str) -> Result<Option<u64>, FsError> {
        self.lookup(path).map(Some)
    }
    fn handle_object_id(&self, handle: FileHandle) -> Result<Option<u64>, FsError> {
        let idx = self.handle_entry_idx(handle)?;
        Ok(Some(self.inode_id(idx)))
    }
''')
start=s.index('        if idx == usize::MAX {\n            return Ok(0);',s.index('pub fn lookup(&self, path: &str) -> Result<u64'))
end=s.index('\n    fn add_entry(',start)
s=s[:start]+'''        Ok(self.inode_id(idx))
    }

    fn inode_id(&self, idx: usize) -> u64 {
        if idx == usize::MAX { return 0; }
        if idx < self.entries.len() {
            self.buffers.get(idx).and_then(Option::as_ref)
                .map(|inode| inode.id).unwrap_or(idx as u64 + 1)
        } else {
            self.dynamic.entries[idx - self.entries.len()].data.id
        }
    }
''' +s[end:];p.write_text(s)
