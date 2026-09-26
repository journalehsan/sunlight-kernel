use super::{
    format_83, parse_83, short_name_checksum, DirEntry, Fat32, FatStat, SlotLocation,
    ATTR_DIRECTORY, FAT_ENTRY_MASK, FAT_EOC, MAX_LFN, MAX_LFN_SLOTS,
};
use sunlight_block::{BlockDevice, BLOCK_SIZE};

const MAX_ENTRY_SLOTS: usize = MAX_LFN_SLOTS + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FatError {
    NotFound,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    InvalidName,
    NoSpace,
    NotEmpty,
    Io,
}

impl<D: BlockDevice> Fat32<D> {
    fn read_fat_entry(&mut self, cluster: u32) -> Result<u32, FatError> {
        if cluster >= self.cluster_count.saturating_add(2) {
            return Err(FatError::Io);
        }
        let byte = cluster as u64 * 4;
        let lba = self.fat_start as u64 + byte / BLOCK_SIZE as u64;
        let offset = (byte % BLOCK_SIZE as u64) as usize;
        let mut sector = [0u8; BLOCK_SIZE];
        self.dev
            .read_block(lba, &mut sector)
            .map_err(|_| FatError::Io)?;
        Ok(u32::from_le_bytes([
            sector[offset],
            sector[offset + 1],
            sector[offset + 2],
            sector[offset + 3],
        ]) & FAT_ENTRY_MASK)
    }

    fn write_fat_entry(&mut self, cluster: u32, value: u32) -> Result<(), FatError> {
        if cluster >= self.cluster_count.saturating_add(2) {
            return Err(FatError::Io);
        }
        let byte = cluster as u64 * 4;
        let sector_in_fat = byte / BLOCK_SIZE as u64;
        let offset = (byte % BLOCK_SIZE as u64) as usize;
        for copy in 0..self.num_fats as u64 {
            let lba = self.fat_start as u64 + copy * self.fat_size as u64 + sector_in_fat;
            let mut sector = [0u8; BLOCK_SIZE];
            self.dev
                .read_block(lba, &mut sector)
                .map_err(|_| FatError::Io)?;
            let old = u32::from_le_bytes([
                sector[offset],
                sector[offset + 1],
                sector[offset + 2],
                sector[offset + 3],
            ]);
            let encoded = (old & !FAT_ENTRY_MASK) | (value & FAT_ENTRY_MASK);
            sector[offset..offset + 4].copy_from_slice(&encoded.to_le_bytes());
            self.dev
                .write_block(lba, &sector)
                .map_err(|_| FatError::Io)?;
        }
        Ok(())
    }

    fn clear_cluster(&mut self, cluster: u32) -> Result<(), FatError> {
        let zero = [0u8; BLOCK_SIZE];
        let first = self.cluster_lba(cluster);
        for sector in 0..self.spc as u64 {
            self.dev
                .write_block(first + sector, &zero)
                .map_err(|_| FatError::Io)?;
        }
        Ok(())
    }

    fn allocate_cluster(&mut self) -> Result<u32, FatError> {
        for cluster in 2..self.cluster_count.saturating_add(2) {
            if self.read_fat_entry(cluster)? == 0 {
                self.write_fat_entry(cluster, FAT_EOC)?;
                self.clear_cluster(cluster)?;
                return Ok(cluster);
            }
        }
        Err(FatError::NoSpace)
    }

    fn free_chain(&mut self, first: u32) -> Result<(), FatError> {
        if first < 2 {
            return Ok(());
        }
        let mut cluster = first;
        for _ in 0..self.cluster_count {
            let next = self.read_fat_entry(cluster)?;
            self.write_fat_entry(cluster, 0)?;
            if next >= FAT_EOC || next < 2 {
                return Ok(());
            }
            cluster = next;
        }
        Err(FatError::Io)
    }

    fn split_parent<'a>(path: &'a [u8]) -> Result<(&'a [u8], &'a [u8]), FatError> {
        if path.first() != Some(&b'/') || path == b"/" || path.ends_with(b"/") {
            return Err(FatError::InvalidName);
        }
        let split = path
            .iter()
            .rposition(|byte| *byte == b'/')
            .ok_or(FatError::InvalidName)?;
        let parent = if split == 0 { b"/".as_slice() } else { &path[..split] };
        let leaf = &path[split + 1..];
        if leaf.is_empty() || leaf.len() > MAX_LFN || !leaf.is_ascii() {
            return Err(FatError::InvalidName);
        }
        Ok((parent, leaf))
    }

    fn resolve_entry(&mut self, path: &[u8]) -> Result<DirEntry, FatError> {
        let (parent_path, leaf) = Self::split_parent(path)?;
        let parent = self.stat_path(parent_path).ok_or(FatError::NotFound)?;
        if !parent.is_dir {
            return Err(FatError::NotDirectory);
        }
        self.find_in_dir(parent.first_cluster, leaf)
            .ok_or(FatError::NotFound)
    }

    fn short_exists(&mut self, directory: u32, name: &[u8; 11]) -> Result<bool, FatError> {
        let mut cluster = directory;
        let mut sector = [0u8; BLOCK_SIZE];
        for _ in 0..self.cluster_count {
            let first = self.cluster_lba(cluster);
            for sector_index in 0..self.spc as u64 {
                self.dev
                    .read_block(first + sector_index, &mut sector)
                    .map_err(|_| FatError::Io)?;
                for offset in (0..BLOCK_SIZE).step_by(32) {
                    if sector[offset] == 0 {
                        return Ok(false);
                    }
                    if sector[offset] != 0xE5
                        && sector[offset + 11] != 0x0F
                        && &sector[offset..offset + 11] == name
                    {
                        return Ok(true);
                    }
                }
            }
            let next = self.read_fat_entry(cluster)?;
            if next >= FAT_EOC || next < 2 {
                return Ok(false);
            }
            cluster = next;
        }
        Err(FatError::Io)
    }

    fn make_short_name(&mut self, directory: u32, leaf: &[u8]) -> Result<[u8; 11], FatError> {
        if let Some((base, extension)) = parse_83(leaf) {
            let mut candidate = [b' '; 11];
            candidate[..8].copy_from_slice(&base);
            candidate[8..].copy_from_slice(&extension);
            if !self.short_exists(directory, &candidate)? {
                return Ok(candidate);
            }
        }

        for nonce in 0u32..256 {
            let mut hash = 0x811C_9DC5u32 ^ nonce;
            for byte in leaf {
                hash ^= *byte as u32;
                hash = hash.wrapping_mul(0x0100_0193);
            }
            let mut candidate = [b' '; 11];
            for (index, byte) in leaf
                .iter()
                .copied()
                .filter(|byte| byte.is_ascii_alphanumeric())
                .take(2)
                .enumerate()
            {
                candidate[index] = byte.to_ascii_uppercase();
            }
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            for index in 0..6 {
                candidate[2 + index] = HEX[((hash >> ((5 - index) * 4)) & 0xF) as usize];
            }
            if !self.short_exists(directory, &candidate)? {
                return Ok(candidate);
            }
        }
        Err(FatError::NoSpace)
    }

    fn needs_lfn(leaf: &[u8], short: &[u8; 11]) -> bool {
        let mut formatted = [0u8; super::MAX_NAME_83];
        let len = format_83(short, &mut formatted);
        &formatted[..len] != leaf
    }

    fn free_slots(
        &mut self,
        directory: u32,
        needed: usize,
    ) -> Result<[SlotLocation; MAX_ENTRY_SLOTS], FatError> {
        let mut result = [SlotLocation { lba: 0, offset: 0 }; MAX_ENTRY_SLOTS];
        let mut count = 0usize;
        let mut cluster = directory;
        let mut sector = [0u8; BLOCK_SIZE];
        for _ in 0..self.cluster_count {
            let first = self.cluster_lba(cluster);
            for sector_index in 0..self.spc as u64 {
                let lba = first + sector_index;
                self.dev
                    .read_block(lba, &mut sector)
                    .map_err(|_| FatError::Io)?;
                for offset in (0..BLOCK_SIZE).step_by(32) {
                    if sector[offset] == 0 || sector[offset] == 0xE5 {
                        result[count] = SlotLocation {
                            lba,
                            offset: offset as u16,
                        };
                        count += 1;
                        if count == needed {
                            return Ok(result);
                        }
                    } else {
                        count = 0;
                    }
                }
            }
            let next = self.read_fat_entry(cluster)?;
            if next >= FAT_EOC || next < 2 {
                let new_cluster = self.allocate_cluster()?;
                self.write_fat_entry(cluster, new_cluster)?;
                cluster = new_cluster;
                count = 0;
            } else {
                cluster = next;
            }
        }
        Err(FatError::NoSpace)
    }

    fn write_slot(&mut self, location: SlotLocation, entry: &[u8; 32]) -> Result<(), FatError> {
        let mut sector = [0u8; BLOCK_SIZE];
        self.dev
            .read_block(location.lba, &mut sector)
            .map_err(|_| FatError::Io)?;
        let offset = location.offset as usize;
        sector[offset..offset + 32].copy_from_slice(entry);
        self.dev
            .write_block(location.lba, &sector)
            .map_err(|_| FatError::Io)
    }

    fn lfn_entry(leaf: &[u8], ordinal: usize, count: usize, checksum: u8) -> [u8; 32] {
        const OFFSETS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        let mut entry = [0xFFu8; 32];
        entry[0] = ordinal as u8 | if ordinal == count { 0x40 } else { 0 };
        entry[11] = 0x0F;
        entry[12] = 0;
        entry[13] = checksum;
        entry[26] = 0;
        entry[27] = 0;
        let base = (ordinal - 1) * 13;
        for (index, offset) in OFFSETS.iter().copied().enumerate() {
            let position = base + index;
            let unit = if position < leaf.len() {
                leaf[position] as u16
            } else if position == leaf.len() {
                0
            } else {
                0xFFFF
            };
            entry[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
        entry
    }

    fn create_link(
        &mut self,
        path: &[u8],
        attr: u8,
        first_cluster: u32,
        size: u32,
    ) -> Result<DirEntry, FatError> {
        let (parent_path, leaf) = Self::split_parent(path)?;
        let parent = self.stat_path(parent_path).ok_or(FatError::NotFound)?;
        if !parent.is_dir {
            return Err(FatError::NotDirectory);
        }
        if self.find_in_dir(parent.first_cluster, leaf).is_some() {
            return Err(FatError::AlreadyExists);
        }
        let short = self.make_short_name(parent.first_cluster, leaf)?;
        let lfn_count = if Self::needs_lfn(leaf, &short) {
            leaf.len().div_ceil(13)
        } else {
            0
        };
        if lfn_count > MAX_LFN_SLOTS {
            return Err(FatError::InvalidName);
        }
        let slots = self.free_slots(parent.first_cluster, lfn_count + 1)?;
        let checksum = short_name_checksum(&short);
        for disk_index in 0..lfn_count {
            let ordinal = lfn_count - disk_index;
            self.write_slot(slots[disk_index], &Self::lfn_entry(leaf, ordinal, lfn_count, checksum))?;
        }
        let mut short_entry = [0u8; 32];
        short_entry[..11].copy_from_slice(&short);
        short_entry[11] = attr;
        short_entry[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
        short_entry[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
        short_entry[28..32].copy_from_slice(&size.to_le_bytes());
        self.write_slot(slots[lfn_count], &short_entry)?;
        self.resolve_entry(path)
    }

    pub fn create_file_path(&mut self, path: &[u8]) -> Result<FatStat, FatError> {
        let entry = self.create_link(path, 0, 0, 0)?;
        Ok(FatStat {
            first_cluster: entry.cluster,
            size: entry.size,
            is_dir: false,
        })
    }

    pub fn mkdir_path(&mut self, path: &[u8]) -> Result<(), FatError> {
        let cluster = self.allocate_cluster()?;
        match self.create_link(path, ATTR_DIRECTORY, cluster, 0) {
            Ok(_) => Ok(()),
            Err(error) => {
                let _ = self.free_chain(cluster);
                Err(error)
            }
        }
    }

    fn chain_cluster(
        &mut self,
        first: &mut u32,
        index: usize,
    ) -> Result<u32, FatError> {
        if *first < 2 {
            *first = self.allocate_cluster()?;
        }
        let mut cluster = *first;
        for _ in 0..index {
            let next = self.read_fat_entry(cluster)?;
            if next >= FAT_EOC || next < 2 {
                let allocated = self.allocate_cluster()?;
                self.write_fat_entry(cluster, allocated)?;
                cluster = allocated;
            } else {
                cluster = next;
            }
        }
        Ok(cluster)
    }

    fn update_entry_data(
        &mut self,
        entry: &DirEntry,
        cluster: u32,
        size: u32,
    ) -> Result<(), FatError> {
        let mut sector = [0u8; BLOCK_SIZE];
        self.dev
            .read_block(entry.short_location.lba, &mut sector)
            .map_err(|_| FatError::Io)?;
        let offset = entry.short_location.offset as usize;
        sector[offset + 20..offset + 22]
            .copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        sector[offset + 26..offset + 28].copy_from_slice(&(cluster as u16).to_le_bytes());
        sector[offset + 28..offset + 32].copy_from_slice(&size.to_le_bytes());
        self.dev
            .write_block(entry.short_location.lba, &sector)
            .map_err(|_| FatError::Io)
    }

    pub fn write_path(
        &mut self,
        path: &[u8],
        offset: usize,
        data: &[u8],
    ) -> Result<usize, FatError> {
        let entry = self.resolve_entry(path)?;
        if entry.attr & ATTR_DIRECTORY != 0 {
            return Err(FatError::IsDirectory);
        }
        if data.is_empty() {
            return Ok(0);
        }
        let end = offset.checked_add(data.len()).ok_or(FatError::NoSpace)?;
        let new_size = (entry.size as usize).max(end);
        let new_size_u32 = u32::try_from(new_size).map_err(|_| FatError::NoSpace)?;
        let cluster_bytes = self.cluster_bytes();
        let mut first = entry.cluster;
        let mut written = 0usize;
        let mut position = offset;
        let mut sector = [0u8; BLOCK_SIZE];
        while written < data.len() {
            let cluster = self.chain_cluster(&mut first, position / cluster_bytes)?;
            let within_cluster = position % cluster_bytes;
            let sector_index = within_cluster / BLOCK_SIZE;
            let sector_offset = within_cluster % BLOCK_SIZE;
            let lba = self.cluster_lba(cluster) + sector_index as u64;
            self.dev
                .read_block(lba, &mut sector)
                .map_err(|_| FatError::Io)?;
            let count = (BLOCK_SIZE - sector_offset).min(data.len() - written);
            sector[sector_offset..sector_offset + count]
                .copy_from_slice(&data[written..written + count]);
            self.dev
                .write_block(lba, &sector)
                .map_err(|_| FatError::Io)?;
            written += count;
            position += count;
        }
        self.update_entry_data(&entry, first, new_size_u32)?;
        Ok(written)
    }

    pub fn truncate_path(&mut self, path: &[u8]) -> Result<(), FatError> {
        let entry = self.resolve_entry(path)?;
        if entry.attr & ATTR_DIRECTORY != 0 {
            return Err(FatError::IsDirectory);
        }
        self.update_entry_data(&entry, 0, 0)?;
        self.dev.flush().map_err(|_| FatError::Io)?;
        self.free_chain(entry.cluster)
    }

    fn mark_deleted(&mut self, entry: &DirEntry) -> Result<(), FatError> {
        let mut sector = [0u8; BLOCK_SIZE];
        for location in entry.lfn_locations[..entry.lfn_count as usize]
            .iter()
            .copied()
            .chain(core::iter::once(entry.short_location))
        {
            self.dev
                .read_block(location.lba, &mut sector)
                .map_err(|_| FatError::Io)?;
            sector[location.offset as usize] = 0xE5;
            self.dev
                .write_block(location.lba, &sector)
                .map_err(|_| FatError::Io)?;
        }
        Ok(())
    }

    pub fn unlink_path(&mut self, path: &[u8]) -> Result<(), FatError> {
        let entry = self.resolve_entry(path)?;
        if entry.attr & ATTR_DIRECTORY != 0 {
            return Err(FatError::IsDirectory);
        }
        self.mark_deleted(&entry)?;
        self.dev.flush().map_err(|_| FatError::Io)?;
        self.free_chain(entry.cluster)
    }

    fn replace_entry_data(&mut self, destination: &DirEntry, source: &DirEntry) -> Result<(), FatError> {
        let mut sector = [0u8; BLOCK_SIZE];
        self.dev
            .read_block(destination.short_location.lba, &mut sector)
            .map_err(|_| FatError::Io)?;
        let offset = destination.short_location.offset as usize;
        sector[offset + 11] = source.attr;
        sector[offset + 20..offset + 22]
            .copy_from_slice(&((source.cluster >> 16) as u16).to_le_bytes());
        sector[offset + 26..offset + 28]
            .copy_from_slice(&(source.cluster as u16).to_le_bytes());
        sector[offset + 28..offset + 32].copy_from_slice(&source.size.to_le_bytes());
        self.dev
            .write_block(destination.short_location.lba, &sector)
            .map_err(|_| FatError::Io)
    }

    pub fn rename_path(&mut self, old: &[u8], new: &[u8]) -> Result<(), FatError> {
        let source = self.resolve_entry(old)?;
        let destination = self.resolve_entry(new).ok();
        let replaced_chain = destination.map(|entry| entry.cluster).unwrap_or(0);
        if let Some(destination) = destination {
            if (destination.attr & ATTR_DIRECTORY != 0) != (source.attr & ATTR_DIRECTORY != 0) {
                return Err(FatError::InvalidName);
            }
            self.replace_entry_data(&destination, &source)?;
        } else {
            self.create_link(new, source.attr, source.cluster, source.size)?;
        }

        // The destination is made durable before the source name is removed.
        // A crash can therefore leave both names, but never neither; callers
        // can deterministically finish publication on restart.
        self.dev.flush().map_err(|_| FatError::Io)?;
        self.mark_deleted(&source)?;
        self.dev.flush().map_err(|_| FatError::Io)?;
        if replaced_chain >= 2 && replaced_chain != source.cluster {
            self.free_chain(replaced_chain)?;
        }
        Ok(())
    }
}
