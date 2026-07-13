use alloc::vec::Vec;

/// Conventional guest memory available to DOS process allocations.  The range
/// is `0x10000..0x9ffff`: low memory remains reserved for the IVT/BDA and
/// `0xa0000..0xfffff` remains reserved for video and BIOS-style regions.
pub const DOS_ARENA_START_SEGMENT: u16 = 0x1000;
pub const DOS_ARENA_END_SEGMENT: u16 = 0xa000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockState {
    Free,
    Allocated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBlock {
    pub segment: u16,
    pub paragraphs: u16,
    pub owner_psp: Option<u16>,
    pub state: BlockState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaError {
    InsufficientMemory { largest: u16 },
    InvalidBlock,
    InvalidOwner,
    DoubleFree,
    InvalidSize,
}

/// A bounded paragraph allocator that models the DOS process arena without
/// exposing raw MCBs to a guest yet.
#[derive(Clone, Debug)]
pub struct DosMemoryArena {
    blocks: Vec<MemoryBlock>,
}

impl Default for DosMemoryArena {
    fn default() -> Self {
        Self::new()
    }
}

impl DosMemoryArena {
    pub fn new() -> Self {
        Self {
            blocks: alloc::vec![MemoryBlock {
                segment: DOS_ARENA_START_SEGMENT,
                paragraphs: DOS_ARENA_END_SEGMENT - DOS_ARENA_START_SEGMENT,
                owner_psp: None,
                state: BlockState::Free,
            }],
        }
    }

    pub fn blocks(&self) -> &[MemoryBlock] {
        &self.blocks
    }

    pub fn largest_available(&self) -> u16 {
        self.blocks
            .iter()
            .filter(|block| block.state == BlockState::Free)
            .map(|block| block.paragraphs)
            .max()
            .unwrap_or(0)
    }

    pub fn allocate(&mut self, paragraphs: u16, owner_psp: u16) -> Result<u16, ArenaError> {
        self.allocate_inner(paragraphs, Some(owner_psp))
    }

    /// Allocates a process block whose PSP is placed at the start of the
    /// allocation, making the block's owner known without a provisional owner.
    pub fn allocate_process(&mut self, paragraphs: u16) -> Result<u16, ArenaError> {
        let segment = self.allocate_inner(paragraphs, None)?;
        let block = self
            .blocks
            .iter_mut()
            .find(|block| block.segment == segment)
            .expect("allocated process block exists");
        block.owner_psp = Some(segment);
        Ok(segment)
    }

    fn allocate_inner(
        &mut self,
        paragraphs: u16,
        owner_psp: Option<u16>,
    ) -> Result<u16, ArenaError> {
        if paragraphs == 0 {
            return Err(ArenaError::InvalidSize);
        }
        let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.state == BlockState::Free && block.paragraphs >= paragraphs)
        else {
            return Err(ArenaError::InsufficientMemory {
                largest: self.largest_available(),
            });
        };
        let block = self.blocks[index];
        let segment = block.segment;
        if block.paragraphs == paragraphs {
            self.blocks[index] = MemoryBlock {
                segment,
                paragraphs,
                owner_psp,
                state: BlockState::Allocated,
            };
        } else {
            self.blocks[index] = MemoryBlock {
                segment,
                paragraphs,
                owner_psp,
                state: BlockState::Allocated,
            };
            self.blocks.insert(
                index + 1,
                MemoryBlock {
                    segment: segment + paragraphs,
                    paragraphs: block.paragraphs - paragraphs,
                    owner_psp: None,
                    state: BlockState::Free,
                },
            );
        }
        Ok(segment)
    }

    pub fn free(&mut self, segment: u16, owner_psp: u16) -> Result<(), ArenaError> {
        let index = self
            .blocks
            .iter()
            .position(|block| block.segment == segment)
            .ok_or(ArenaError::InvalidBlock)?;
        let block = self.blocks[index];
        if block.state == BlockState::Free {
            return Err(ArenaError::DoubleFree);
        }
        if block.owner_psp != Some(owner_psp) {
            return Err(ArenaError::InvalidOwner);
        }
        self.blocks[index].state = BlockState::Free;
        self.blocks[index].owner_psp = None;
        self.coalesce();
        Ok(())
    }

    pub fn reassign_owner(&mut self, segment: u16, owner_psp: u16) -> Result<(), ArenaError> {
        let block = self
            .blocks
            .iter_mut()
            .find(|block| block.segment == segment)
            .ok_or(ArenaError::InvalidBlock)?;
        if block.state != BlockState::Allocated {
            return Err(ArenaError::InvalidBlock);
        }
        block.owner_psp = Some(owner_psp);
        Ok(())
    }

    /// Resizes in place only. A zero paragraph request is rejected so callers
    /// cannot silently turn a valid block into an invalid DOS allocation.
    pub fn resize(
        &mut self,
        segment: u16,
        paragraphs: u16,
        owner_psp: u16,
    ) -> Result<(), ArenaError> {
        if paragraphs == 0 {
            return Err(ArenaError::InvalidSize);
        }
        let index = self
            .blocks
            .iter()
            .position(|block| block.segment == segment)
            .ok_or(ArenaError::InvalidBlock)?;
        let block = self.blocks[index];
        if block.state != BlockState::Allocated {
            return Err(ArenaError::InvalidBlock);
        }
        if block.owner_psp != Some(owner_psp) {
            return Err(ArenaError::InvalidOwner);
        }
        if paragraphs == block.paragraphs {
            return Ok(());
        }
        if paragraphs < block.paragraphs {
            let released = block.paragraphs - paragraphs;
            self.blocks[index].paragraphs = paragraphs;
            self.blocks.insert(
                index + 1,
                MemoryBlock {
                    segment: segment + paragraphs,
                    paragraphs: released,
                    owner_psp: None,
                    state: BlockState::Free,
                },
            );
            self.coalesce();
            return Ok(());
        }
        let additional = paragraphs - block.paragraphs;
        let Some(next) = self.blocks.get(index + 1).copied() else {
            return Err(ArenaError::InsufficientMemory {
                largest: self.largest_available(),
            });
        };
        if next.state != BlockState::Free || next.paragraphs < additional {
            return Err(ArenaError::InsufficientMemory {
                largest: self.largest_available(),
            });
        }
        self.blocks[index].paragraphs = paragraphs;
        if next.paragraphs == additional {
            self.blocks.remove(index + 1);
        } else {
            self.blocks[index + 1].segment += additional;
            self.blocks[index + 1].paragraphs -= additional;
        }
        Ok(())
    }

    pub fn free_owner(&mut self, owner_psp: u16) {
        for block in &mut self.blocks {
            if block.state == BlockState::Allocated && block.owner_psp == Some(owner_psp) {
                block.state = BlockState::Free;
                block.owner_psp = None;
            }
        }
        self.coalesce();
    }

    pub fn contains_allocated(&self, segment: u16, owner_psp: u16) -> bool {
        self.blocks.iter().any(|block| {
            block.segment == segment
                && block.state == BlockState::Allocated
                && block.owner_psp == Some(owner_psp)
        })
    }

    pub fn owns_range(&self, owner_psp: u16, segment: u16, offset: u16, length: usize) -> bool {
        let start = (segment as usize) * 16 + offset as usize;
        let Some(end) = start.checked_add(length) else {
            return false;
        };
        self.blocks.iter().any(|block| {
            block.state == BlockState::Allocated
                && block.owner_psp == Some(owner_psp)
                && start >= block.segment as usize * 16
                && end <= (block.segment as usize + block.paragraphs as usize) * 16
        })
    }

    fn coalesce(&mut self) {
        let mut index = 0;
        while index + 1 < self.blocks.len() {
            let left = self.blocks[index];
            let right = self.blocks[index + 1];
            if left.state == BlockState::Free
                && right.state == BlockState::Free
                && left.segment + left.paragraphs == right.segment
            {
                self.blocks[index].paragraphs += right.paragraphs;
                self.blocks.remove(index + 1);
            } else {
                index += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ArenaError, DosMemoryArena};

    #[test]
    fn allocation_free_resize_and_coalescing_are_bounded() {
        let mut arena = DosMemoryArena::new();
        let first = arena.allocate(0x20, 0x1000).unwrap();
        let second = arena.allocate(0x20, 0x1000).unwrap();
        assert_eq!(second, first + 0x20);
        arena.resize(first, 0x10, 0x1000).unwrap();
        arena.resize(first, 0x20, 0x1000).unwrap();
        arena.free(first, 0x1000).unwrap();
        arena.free(second, 0x1000).unwrap();
        assert_eq!(arena.blocks().len(), 1);
        assert_eq!(arena.largest_available(), 0x9000);
    }

    #[test]
    fn ownership_and_double_free_are_rejected() {
        let mut arena = DosMemoryArena::new();
        let block = arena.allocate(1, 0x1111).unwrap();
        assert_eq!(arena.free(block, 0x2222), Err(ArenaError::InvalidOwner));
        arena.free(block, 0x1111).unwrap();
        assert_eq!(arena.free(block, 0x1111), Err(ArenaError::DoubleFree));
    }
}
