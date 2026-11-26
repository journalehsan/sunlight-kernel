use crate::{ramfs::RAMFS_MAX_ENTRIES, FsError, RamEntry};
use alloc::{boxed::Box, vec::Vec};

pub const MAGIC: &[u8; 8] = b"SUNRES01";
pub const BLOCK: usize = 4096;
pub const MAX_BYTES: usize = 256 * 1024 * 1024;
pub const USER_ADDRESS: u64 = 0x0000_0020_0000_0000;
pub const HEADER: usize = 24;
pub const ENTRY: usize = 24;
pub const DESCRIPTOR: usize = 12;

fn word(bytes: &[u8], offset: usize) -> Result<usize, FsError> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(FsError::Io)?
            .try_into()
            .unwrap(),
    ) as usize)
}

pub fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(2166136261u32, |hash, &byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16777619)
    })
}

pub fn archive_size(header: &[u8]) -> Result<usize, FsError> {
    let size = word(header, 8)?;
    if header.get(..8) != Some(MAGIC) || !(HEADER..=MAX_BYTES).contains(&size) {
        return Err(FsError::Io);
    }
    Ok(size)
}

pub(crate) struct Resource {
    archive: &'static [u8],
    descriptors: &'static [u8],
    pub(crate) len: usize,
}

impl Resource {
    fn block(&self, index: usize) -> Result<(&[u8], u32), FsError> {
        let at = index * DESCRIPTOR;
        let start = word(self.descriptors, at)?;
        let size = word(self.descriptors, at + 4)?;
        let expected = BLOCK.min(self.len - index * BLOCK);
        if start < word(self.archive, 16)? || !(1..=expected).contains(&size) {
            return Err(FsError::Io);
        }
        let bytes = self.archive.get(start..start + size).ok_or(FsError::Io)?;
        Ok((bytes, word(self.descriptors, at + 8)? as u32))
    }

    pub(crate) fn read(&self, offset: usize, output: &mut [u8]) -> Result<usize, FsError> {
        let count = output.len().min(self.len.saturating_sub(offset));
        let mut done = 0;
        let mut scratch = [0; BLOCK];
        while done < count {
            let position = offset + done;
            let index = position / BLOCK;
            let expected = BLOCK.min(self.len - index * BLOCK);
            let (input, sum) = self.block(index)?;
            let block = if input.len() == expected {
                input
            } else {
                let target = &mut scratch[..expected];
                if lz4_flex::block::decompress_into(input, target).map_err(|_| FsError::Io)?
                    != expected
                {
                    return Err(FsError::Io);
                }
                target
            };
            if checksum(block) != sum {
                return Err(FsError::Io);
            }
            let within = position % BLOCK;
            let take = (expected - within).min(count - done);
            output[done..done + take].copy_from_slice(&block[within..within + take]);
            done += take;
        }
        Ok(done)
    }
}

pub fn entries(archive: &'static [u8]) -> Result<&'static [RamEntry], FsError> {
    let count = word(archive, 12)?;
    let payload = word(archive, 16)?;
    let inventory = word(archive, 20)?;
    if archive_size(archive)? != archive.len()
        || count > RAMFS_MAX_ENTRIES
        || !(HEADER..=payload).contains(&inventory)
        || payload > archive.len()
    {
        return Err(FsError::Io);
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(count)
        .map_err(|_| FsError::NoMemory)?;
    let mut cursor = HEADER;
    for _ in 0..count {
        let record = archive.get(cursor..cursor + ENTRY).ok_or(FsError::Io)?;
        cursor += ENTRY;
        let end = cursor + word(record, 0)?;
        if end > inventory {
            return Err(FsError::Io);
        }
        let path = core::str::from_utf8(&archive[cursor..end]).map_err(|_| FsError::Io)?;
        if !path.starts_with('/') || path.contains('\0') {
            return Err(FsError::Io);
        }
        cursor = end;
        let uid = word(record, 4)? as u32;
        let gid = word(record, 8)? as u32;
        let mode = u16::from_le_bytes([record[12], record[13]]);
        let len = word(record, 16)?;
        let start = word(record, 20)?;
        let entry = match record[14..16] {
            [1, 0] if len == 0 && start == 0 => RamEntry::dir(path, uid, gid, mode),
            [0, 0] if len == 0 && start == 0 => RamEntry::file(path, uid, gid, mode, b""),
            [0, 0] if len <= MAX_BYTES && start >= inventory => {
                let end = start + len.div_ceil(BLOCK) * DESCRIPTOR;
                if end > payload {
                    return Err(FsError::Io);
                }
                let resource = Resource {
                    archive,
                    descriptors: &archive[start..end],
                    len,
                };
                for index in 0..len.div_ceil(BLOCK) {
                    resource.block(index)?;
                }
                let mut entry = RamEntry::file(path, uid, gid, mode, b"");
                entry.resource = Some(resource);
                entry
            }
            _ => return Err(FsError::Io),
        };
        entries.push(entry);
    }
    if cursor != inventory {
        return Err(FsError::Io);
    }
    Ok(Box::leak(entries.into_boxed_slice()))
}
