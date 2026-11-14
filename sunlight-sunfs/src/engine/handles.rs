//! Opaque descriptor registration and shared descriptions, under one engine owner.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    Read,
    Write,
    ReadWrite,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handle {
    session: u64,
    token: u64,
}
pub(super) struct Description {
    pub node: NodeId,
    offset: u64,
    access: AccessMode,
    refs: usize,
}
#[derive(Debug, PartialEq, Eq)]
pub struct RuntimeAccounting {
    pub namespace_nodes: usize,
    pub orphan_nodes: usize,
    pub pinned_orphans: usize,
    pub reap_candidates: usize,
    pub descriptions: usize,
    pub handles: usize,
}
impl<D: BlockDevice> Filesystem<D> {
    pub(super) fn pins(&self, node: &NodeId) -> usize {
        self.descriptions
            .values()
            .filter(|d| d.node == *node)
            .count()
    }
    pub fn runtime_accounting(&self) -> RuntimeAccounting {
        let orphan_nodes = self
            .files
            .iter()
            .filter(|f| f.orphan_generation != 0)
            .count();
        let pinned_orphans = self
            .files
            .iter()
            .filter(|f| f.orphan_generation != 0 && self.pins(&f.id) != 0)
            .count();
        RuntimeAccounting {
            namespace_nodes: self.files.len() + 1 - orphan_nodes,
            orphan_nodes,
            pinned_orphans,
            reap_candidates: orphan_nodes - pinned_orphans,
            descriptions: self.descriptions.len(),
            handles: self.handles.len(),
        }
    }
    fn token(&mut self) -> Result<u64> {
        if self.handles.len() >= MAX_HANDLES {
            return Err(FsError::Limit);
        }
        let token = self.next_handle;
        self.next_handle = token.checked_add(1).ok_or(Error::Overflow)?;
        Ok(token)
    }
    /// Caller must already have VFS authority. Modes bound subsequent descriptor
    /// I/O, but node UUID/identity alone does not constitute user authorization.
    pub fn open(&mut self, id: &FileId, access: AccessMode) -> Result<Handle> {
        if access != AccessMode::Read {
            self.writable()?;
        }
        self.identity(id)?;
        if id.node == NodeId::ROOT {
            return Err(FsError::IsDirectory);
        }
        let i = self.index(id)?;
        if self.files[i].kind != 1 {
            return Err(FsError::IsDirectory);
        }
        if self.descriptions.len() >= MAX_DESCRIPTIONS {
            return Err(FsError::Limit);
        }
        let token = self.token()?;
        self.descriptions.insert(
            token,
            Description {
                node: id.node.clone(),
                offset: 0,
                access,
                refs: 1,
            },
        );
        self.handles.insert(token, token);
        Ok(Handle {
            session: self.session,
            token,
        })
    }
    fn description(&self, handle: &Handle) -> Result<u64> {
        if handle.session != self.session {
            return Err(FsError::Stale);
        }
        self.handles
            .get(&handle.token)
            .copied()
            .ok_or(FsError::Stale)
    }
    pub fn dup(&mut self, handle: &Handle) -> Result<Handle> {
        let id = self.description(handle)?;
        let refs = self
            .descriptions
            .get(&id)
            .ok_or(FsError::Stale)?
            .refs
            .checked_add(1)
            .ok_or(Error::Overflow)?;
        let token = self.token()?;
        self.handles.insert(token, id);
        self.descriptions.get_mut(&id).ok_or(FsError::Stale)?.refs = refs;
        Ok(Handle {
            session: self.session,
            token,
        })
    }
    pub fn close(&mut self, handle: &Handle) -> Result<()> {
        let id = self.description(handle)?;
        let d = self.descriptions.get_mut(&id).ok_or(FsError::Stale)?;
        let refs = d.refs.checked_sub(1).ok_or(FsError::Corrupt)?;
        self.handles.remove(&handle.token);
        if refs == 0 {
            self.descriptions.remove(&id);
        } else {
            d.refs = refs;
        }
        Ok(())
    }
    pub fn handle_identity(&self, handle: &Handle) -> Result<([u8; 16], NodeId)> {
        let d = self
            .descriptions
            .get(&self.description(handle)?)
            .ok_or(FsError::Stale)?;
        Ok((self.uuid, d.node.clone()))
    }
    pub fn position(&self, handle: &Handle) -> Result<u64> {
        Ok(self
            .descriptions
            .get(&self.description(handle)?)
            .ok_or(FsError::Stale)?
            .offset)
    }
    pub fn seek(&mut self, handle: &Handle, offset: u64) -> Result<()> {
        let id = self.description(handle)?;
        if offset > MAX_FILE_BYTES as u64 {
            return Err(FsError::Limit);
        }
        self.descriptions.get_mut(&id).ok_or(FsError::Stale)?.offset = offset;
        Ok(())
    }
    pub fn read_handle(&mut self, handle: &Handle, out: &mut [u8]) -> Result<usize> {
        let id = self.description(handle)?;
        let d = self.descriptions.get(&id).ok_or(FsError::Stale)?;
        if d.access == AccessMode::Write {
            return Err(FsError::Access);
        }
        let i = self
            .files
            .iter()
            .position(|f| f.id == d.node)
            .ok_or(FsError::Stale)?;
        let n = self.read_index(i, d.offset, out)?;
        self.descriptions.get_mut(&id).ok_or(FsError::Stale)?.offset += n as u64;
        Ok(n)
    }
    pub fn write_handle(&mut self, handle: &Handle, bytes: &[u8]) -> Result<usize> {
        let id = self.description(handle)?;
        let d = self.descriptions.get(&id).ok_or(FsError::Stale)?;
        if d.access == AccessMode::Read {
            return Err(FsError::Access);
        }
        let i = self
            .files
            .iter()
            .position(|f| f.id == d.node)
            .ok_or(FsError::Stale)?;
        let offset = d.offset;
        let n = self.write_index(i, offset, bytes)?;
        self.descriptions.get_mut(&id).ok_or(FsError::Stale)?.offset += n as u64;
        Ok(n)
    }
    pub fn fsync_handle(&mut self, handle: &Handle) -> Result<()> {
        self.description(handle)?;
        self.sync_all()
    }
}
