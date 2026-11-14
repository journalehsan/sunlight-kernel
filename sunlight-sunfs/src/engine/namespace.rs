//! Single-owner namespace operations. No path strings serve as retained pins.
use super::*;
impl<D: BlockDevice> Filesystem<D> {
    fn selector(&self, node: NodeId) -> FileId {
        FileId {
            volume: self.uuid,
            node,
            session: self.session,
        }
    }
    pub(super) fn identity(&self, id: &FileId) -> Result<()> {
        if id.volume != self.uuid || id.session != self.session {
            Err(FsError::Stale)
        } else {
            Ok(())
        }
    }
    pub fn root(&self) -> FileId {
        self.selector(NodeId::ROOT)
    }
    fn directory(&self, id: &FileId) -> Result<()> {
        self.identity(id)?;
        if id.node == NodeId::ROOT {
            return Ok(());
        }
        if self.files[self.index(id)?].kind != 2 {
            return Err(FsError::NotDirectory);
        }
        Ok(())
    }
    fn binding(&self, parent: &NodeId, name: &[u8]) -> Option<usize> {
        self.files
            .iter()
            .position(|f| f.orphan_generation == 0 && f.parent == *parent && f.name == name)
    }
    fn children(&self, parent: &NodeId) -> usize {
        self.files
            .iter()
            .filter(|f| f.orphan_generation == 0 && f.parent == *parent)
            .count()
    }
    fn component(name: &[u8]) -> Result<()> {
        if name.len() > MAX_NAME {
            return Err(FsError::Limit);
        }
        DirEntry {
            name: name.to_vec(),
            target: NodeId::ROOT,
            kind: 1,
        }
        .validate()?;
        Ok(())
    }
    /// Snapshot enumeration, no retained cookie or directory pin.
    pub fn entries_at(&self, parent: &FileId) -> Result<Vec<Entry>> {
        self.directory(parent)?;
        let mut entries: Vec<_> = self
            .files
            .iter()
            .filter(|f| f.orphan_generation == 0 && f.parent == parent.node)
            .map(|f| Entry {
                name: f.name.clone(),
                id: self.selector(f.id.clone()),
                size: f.data.len() as u64,
                kind: f.kind,
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }
    pub fn entries(&self) -> Vec<Entry> {
        self.entries_at(&self.root()).unwrap_or_default()
    }
    pub fn lookup(&self, name: &[u8]) -> Result<FileId> {
        self.lookup_at(&self.root(), name)
    }
    pub fn lookup_at(&self, parent: &FileId, name: &[u8]) -> Result<FileId> {
        self.directory(parent)?;
        if name == b"." {
            return Ok(parent.clone());
        }
        if name == b".." {
            return if parent.node == NodeId::ROOT {
                Ok(self.root())
            } else {
                Ok(self.selector(self.files[self.index(parent)?].parent.clone()))
            };
        }
        Self::component(name)?;
        self.binding(&parent.node, name)
            .map(|i| self.selector(self.files[i].id.clone()))
            .ok_or(FsError::NotFound)
    }
    /// Walk actual components, including dot operations only after type checking.
    /// No symlinks, mount crossing, credential checks or retained relative start.
    pub fn lookup_components(&self, components: &[&[u8]]) -> Result<FileId> {
        if components.len() > MAX_COMPONENTS {
            return Err(FsError::Limit);
        }
        let mut current = self.root();
        for name in components {
            current = self.lookup_at(&current, name)?;
        }
        Ok(current)
    }
    fn depth(&self, id: &NodeId) -> Result<usize> {
        let mut node = id.clone();
        for depth in 0..=MAX_DEPTH {
            if node == NodeId::ROOT {
                return Ok(depth);
            }
            node = self
                .files
                .iter()
                .find(|f| f.id == node && f.orphan_generation == 0)
                .ok_or(FsError::Stale)?
                .parent
                .clone();
        }
        Err(FsError::Limit)
    }
    pub fn create_at(&mut self, parent: &FileId, name: &[u8]) -> Result<FileId> {
        self.create_node(parent, name, 1)
    }
    pub fn mkdir(&mut self, parent: &FileId, name: &[u8]) -> Result<FileId> {
        self.create_node(parent, name, 2)
    }
    fn create_node(&mut self, parent: &FileId, name: &[u8], kind: u8) -> Result<FileId> {
        self.writable()?;
        self.directory(parent)?;
        Self::component(name)?;
        let cap = if kind == 1 {
            MAX_FILES
        } else {
            MAX_DIRECTORIES
        };
        if self.files.iter().filter(|f| f.kind == kind).count() >= cap
            || self.files.len() as u64 + 1 >= self.sb.maximum_nodes
            || self.children(&parent.node) >= MAX_ENTRIES
            || self.depth(&parent.node)? >= MAX_DEPTH
        {
            return Err(FsError::Limit);
        }
        if self.binding(&parent.node, name).is_some() {
            return Err(FsError::Exists);
        }
        let next = self.next_nid.checked_add(1).ok_or(Error::Overflow)?;
        let id = NodeId {
            nid: self.next_nid,
            incarnation: 1,
        };
        self.files.push(File {
            name: name.to_vec(),
            id: id.clone(),
            data: Vec::new(),
            kind,
            parent: parent.node.clone(),
            orphan_generation: 0,
        });
        self.next_nid = next;
        self.dirty = true;
        Ok(self.selector(id))
    }
    fn orphan_admission(&self) -> Result<u64> {
        let count = self
            .files
            .iter()
            .filter(|f| f.orphan_generation != 0)
            .count();
        if count >= MAX_ORPHANS || count as u64 >= self.sb.maximum_orphans {
            return Err(FsError::Limit);
        }
        self.generation()
            .checked_add(1)
            .ok_or(Error::Overflow.into())
    }
    pub fn unlink(&mut self, parent: &FileId, name: &[u8]) -> Result<()> {
        self.remove_binding(parent, name, 1)
    }
    pub fn rmdir(&mut self, parent: &FileId, name: &[u8]) -> Result<()> {
        self.remove_binding(parent, name, 2)
    }
    fn remove_binding(&mut self, parent: &FileId, name: &[u8], kind: u8) -> Result<()> {
        self.writable()?;
        self.directory(parent)?;
        Self::component(name)?;
        let i = self.binding(&parent.node, name).ok_or(FsError::NotFound)?;
        if self.files[i].kind != kind {
            return Err(if kind == 1 {
                FsError::IsDirectory
            } else {
                FsError::NotDirectory
            });
        }
        if kind == 2 && self.children(&self.files[i].id) != 0 {
            return Err(FsError::NotEmpty);
        }
        let generation = self.orphan_admission()?;
        self.files[i].orphan_generation = generation;
        self.files[i].name.clear();
        self.dirty = true;
        Ok(())
    }
    /// Change both bindings and any directory parent in one working epoch.
    /// One sync_all/fsync publishes the complete change through the sole CP.
    pub fn rename(&mut self, from: &FileId, old: &[u8], to: &FileId, new: &[u8]) -> Result<()> {
        self.writable()?;
        self.directory(from)?;
        self.directory(to)?;
        Self::component(old)?;
        Self::component(new)?;
        let src = self.binding(&from.node, old).ok_or(FsError::NotFound)?;
        let dst = self.binding(&to.node, new);
        if dst == Some(src) {
            return Ok(());
        }
        if let Some(d) = dst {
            if self.files[src].kind != self.files[d].kind {
                return Err(if self.files[src].kind == 1 {
                    FsError::IsDirectory
                } else {
                    FsError::NotDirectory
                });
            }
            if self.files[d].kind == 2 && self.children(&self.files[d].id) != 0 {
                return Err(FsError::NotEmpty);
            }
        } else if from.node != to.node && self.children(&to.node) >= MAX_ENTRIES {
            return Err(FsError::Limit);
        }
        // Iteratively validate every moved descendant's resulting depth. Also
        // detect the destination parent below the source before any mutation.
        let source_id = &self.files[src].id;
        let parent_depth = self.depth(&to.node)?;
        let mut ancestor = to.node.clone();
        for _ in 0..=MAX_DEPTH {
            if ancestor == *source_id {
                return Err(FsError::Cycle);
            }
            if ancestor == NodeId::ROOT {
                break;
            }
            ancestor = self
                .files
                .iter()
                .find(|f| f.id == ancestor && f.orphan_generation == 0)
                .ok_or(FsError::Stale)?
                .parent
                .clone();
        }
        for f in self.files.iter().filter(|f| f.orphan_generation == 0) {
            let mut ancestor = f.id.clone();
            for relative in 0..=MAX_DEPTH {
                if ancestor == *source_id {
                    if parent_depth + 1 + relative > MAX_DEPTH {
                        return Err(FsError::Limit);
                    }
                    break;
                }
                if ancestor == NodeId::ROOT {
                    break;
                }
                ancestor = self
                    .files
                    .iter()
                    .find(|n| n.id == ancestor && n.orphan_generation == 0)
                    .ok_or(FsError::Stale)?
                    .parent
                    .clone();
            }
        }
        let generation = if dst.is_some() {
            self.orphan_admission()?
        } else {
            0
        };
        if let Some(d) = dst {
            self.files[d].name.clear();
            self.files[d].orphan_generation = generation;
        }
        self.files[src].name = new.to_vec();
        self.files[src].parent = to.node.clone();
        self.dirty = true;
        Ok(())
    }
    /// Remove all unpinned orphans using the ordinary admitted commit protocol.
    /// Admission failure restores the working state; I/O failure freezes it and
    /// retains the entire plan. Mount itself never writes or assumes handles live.
    pub fn reap_orphans(&mut self) -> Result<usize> {
        self.writable()?;
        let count = self
            .files
            .iter()
            .filter(|f| f.orphan_generation != 0 && self.pins(&f.id) == 0)
            .count();
        if count == 0 {
            return Ok(0);
        }
        let previous = self.files.clone();
        let dirty = self.dirty;
        let pinned: BTreeSet<_> = self.descriptions.values().map(|d| d.node.nid).collect();
        self.files
            .retain(|f| f.orphan_generation == 0 || pinned.contains(&f.id.nid));
        self.dirty = true;
        if let Err(e) = self.sync_all() {
            if !self.frozen {
                self.files = previous;
                self.dirty = dirty;
            }
            return Err(e);
        }
        Ok(count)
    }
}
