use sunlight_block::{BlockDevice, BLOCK_SIZE};

/// FAT32 cluster-chain terminator range (>= 0x0FFFFFF8) and reserved entries.
const FAT_ENTRY_MASK: u32 = 0x0FFF_FFFF;
const FAT_EOC: u32 = 0x0FFF_FFF8;

const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_LFN: u8 = 0x0F;

/// Maximum formatted 8.3 name length: 8 + '.' + 3.
pub const MAX_NAME_83: usize = 12;

/// Minimal read-only FAT32 driver over any [`BlockDevice`].
///
/// No heap allocation; designed for no_std kernel and server use.
pub struct Fat32<D: BlockDevice> {
    dev: D,
    spc: u8,        // sectors per cluster
    fat_start: u32, // first FAT sector (LBA)
    fds: u32,       // first data sector (LBA)
    rc: u32,        // root cluster number
}

/// Result of a path lookup: enough to read the object later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FatStat {
    pub first_cluster: u32,
    pub size: u32,
    pub is_dir: bool,
}

/// Parsed FAT32 directory entry (subset of fields we need).
struct DirEntry {
    name: [u8; 11],
    cluster: u32,
    size: u32,
    attr: u8,
}

impl<D: BlockDevice> Fat32<D> {
    /// Parse the BPB from LBA 0 and validate FAT32 signature.
    /// Returns None if not FAT32 or read fails.
    pub fn mount(mut dev: D) -> Option<Self> {
        let mut s = [0u8; BLOCK_SIZE];
        dev.read_block(0, &mut s).ok()?;

        let bps = u16::from_le_bytes([s[11], s[12]]);
        let spc = s[13];
        let reserved = u16::from_le_bytes([s[14], s[15]]) as u32;
        let num_fats = s[16] as u32;
        let root_ent_cnt = u16::from_le_bytes([s[17], s[18]]);
        let fat16_size = u16::from_le_bytes([s[22], s[23]]);
        let fat32_size = u32::from_le_bytes([s[36], s[37], s[38], s[39]]);
        let root_cluster = u32::from_le_bytes([s[44], s[45], s[46], s[47]]);

        // FAT32 must have bytes_per_sector=512, zero fat16_size and root_ent_cnt
        if bps != 512 || root_ent_cnt != 0 || fat16_size != 0 || fat32_size == 0 {
            return None;
        }
        if spc == 0 || root_cluster < 2 {
            return None;
        }
        // Check file-system type string at offset 82
        if &s[82..90] != b"FAT32   " {
            return None;
        }

        let fds = reserved + num_fats * fat32_size;
        Some(Fat32 {
            dev,
            spc,
            fat_start: reserved,
            fds,
            rc: root_cluster,
        })
    }

    fn cluster_lba(&self, cluster: u32) -> u64 {
        (self.fds + (cluster - 2) * self.spc as u32) as u64
    }

    fn cluster_bytes(&self) -> usize {
        self.spc as usize * BLOCK_SIZE
    }

    /// Follow the FAT chain one step. Returns None at end-of-chain or error.
    fn next_cluster(&mut self, cluster: u32) -> Option<u32> {
        let byte = cluster as u64 * 4;
        let lba = self.fat_start as u64 + byte / BLOCK_SIZE as u64;
        let off = (byte % BLOCK_SIZE as u64) as usize;

        let mut sec = [0u8; BLOCK_SIZE];
        self.dev.read_block(lba, &mut sec).ok()?;

        let val = u32::from_le_bytes([sec[off], sec[off + 1], sec[off + 2], sec[off + 3]])
            & FAT_ENTRY_MASK;
        if val >= FAT_EOC || val < 2 {
            None
        } else {
            Some(val)
        }
    }

    /// Walk all live entries of the directory starting at `cluster`, following
    /// the FAT chain. `f` returns false to stop early. Returns None on I/O error.
    fn walk_dir(
        &mut self,
        cluster: u32,
        f: &mut dyn FnMut(&DirEntry) -> bool,
    ) -> Option<()> {
        let mut cluster = cluster;
        let mut sec = [0u8; BLOCK_SIZE];
        loop {
            let lba = self.cluster_lba(cluster);
            for s_off in 0..self.spc as u64 {
                self.dev.read_block(lba + s_off, &mut sec).ok()?;
                for i in 0..(BLOCK_SIZE / 32) {
                    let o = i * 32;
                    if sec[o] == 0x00 {
                        return Some(()); // end of directory
                    }
                    if sec[o] == 0xE5 {
                        continue; // deleted
                    }
                    let attr = sec[o + 11];
                    if attr == ATTR_LFN || attr & ATTR_VOLUME_ID != 0 {
                        continue; // LFN entry or volume label — skip
                    }
                    let mut name = [0u8; 11];
                    name.copy_from_slice(&sec[o..o + 11]);
                    let chi = u16::from_le_bytes([sec[o + 20], sec[o + 21]]) as u32;
                    let clo = u16::from_le_bytes([sec[o + 26], sec[o + 27]]) as u32;
                    let entry = DirEntry {
                        name,
                        cluster: (chi << 16) | clo,
                        size: u32::from_le_bytes([
                            sec[o + 28],
                            sec[o + 29],
                            sec[o + 30],
                            sec[o + 31],
                        ]),
                        attr,
                    };
                    if !f(&entry) {
                        return Some(());
                    }
                }
            }
            cluster = match self.next_cluster(cluster) {
                Some(c) => c,
                None => return Some(()),
            };
        }
    }

    /// Look up `name8` + `ext3` in the directory rooted at `cluster`.
    fn find_in_dir(
        &mut self,
        cluster: u32,
        name8: &[u8; 8],
        ext3: &[u8; 3],
    ) -> Option<DirEntry> {
        let mut found: Option<DirEntry> = None;
        self.walk_dir(cluster, &mut |entry| {
            if &entry.name[..8] == name8 && &entry.name[8..11] == ext3 {
                found = Some(DirEntry {
                    name: entry.name,
                    cluster: entry.cluster,
                    size: entry.size,
                    attr: entry.attr,
                });
                false
            } else {
                true
            }
        })?;
        found
    }

    /// Resolve a path (bytes, '/'-separated, any depth) to its entry.
    /// `b"/"` and `b""` resolve to the root directory.
    pub fn stat_path(&mut self, path: &[u8]) -> Option<FatStat> {
        let mut current = FatStat {
            first_cluster: self.rc,
            size: 0,
            is_dir: true,
        };
        for component in path.split(|&b| b == b'/') {
            if component.is_empty() {
                continue;
            }
            if !current.is_dir {
                return None; // path descends through a file
            }
            let (name, ext) = parse_83(component)?;
            let entry = self.find_in_dir(current.first_cluster, &name, &ext)?;
            current = FatStat {
                first_cluster: entry.cluster,
                size: entry.size,
                is_dir: entry.attr & ATTR_DIRECTORY != 0,
            };
        }
        Some(current)
    }

    /// Read file content starting at `offset` into `out`, following the FAT
    /// cluster chain. `first_cluster`/`file_size` come from [`Fat32::stat_path`].
    /// Returns bytes read (0 when `offset` is at or past EOF).
    pub fn read_at(
        &mut self,
        first_cluster: u32,
        file_size: u32,
        offset: usize,
        out: &mut [u8],
    ) -> Option<usize> {
        let total = (file_size as usize).saturating_sub(offset).min(out.len());
        if total == 0 {
            return Some(0);
        }
        if first_cluster < 2 {
            return None; // non-empty file must have a data cluster
        }

        let cb = self.cluster_bytes();
        let mut cluster = first_cluster;
        for _ in 0..offset / cb {
            cluster = self.next_cluster(cluster)?;
        }

        let mut pos = offset % cb; // byte position within the current cluster
        let mut written = 0usize;
        let mut sec = [0u8; BLOCK_SIZE];
        while written < total {
            let s_idx = (pos / BLOCK_SIZE) as u64;
            let s_off = pos % BLOCK_SIZE;
            self.dev.read_block(self.cluster_lba(cluster) + s_idx, &mut sec).ok()?;

            let chunk = (BLOCK_SIZE - s_off).min(total - written);
            out[written..written + chunk].copy_from_slice(&sec[s_off..s_off + chunk]);
            written += chunk;
            pos += chunk;

            if pos == cb {
                pos = 0;
                cluster = match self.next_cluster(cluster) {
                    Some(c) => c,
                    None => break,
                };
            }
        }
        Some(written)
    }

    /// Read a whole file at `path` into `out`. Returns bytes read.
    pub fn read_file(&mut self, path: &[u8], out: &mut [u8]) -> Option<usize> {
        let stat = self.stat_path(path)?;
        if stat.is_dir {
            return None;
        }
        self.read_at(stat.first_cluster, stat.size, 0, out)
    }

    /// List the directory at `path`. Calls `f(name, is_dir, size)` per entry
    /// with the formatted 8.3 name (e.g. b"HELLO.TXT"); `f` returns false to
    /// stop early. Returns None if `path` is not a directory or on I/O error.
    pub fn read_dir_raw(
        &mut self,
        path: &[u8],
        f: &mut dyn FnMut(&[u8], bool, u32) -> bool,
    ) -> Option<()> {
        let stat = self.stat_path(path)?;
        if !stat.is_dir {
            return None;
        }
        self.walk_dir(stat.first_cluster, &mut |entry| {
            let mut name = [0u8; MAX_NAME_83];
            let len = format_83(&entry.name, &mut name);
            f(&name[..len], entry.attr & ATTR_DIRECTORY != 0, entry.size)
        })
    }
}

/// Convert an ASCII filename component to FAT32 8.3 form (uppercase, space-padded).
fn parse_83(component: &[u8]) -> Option<([u8; 8], [u8; 3])> {
    let mut name = [b' '; 8];
    let mut ext = [b' '; 3];

    if let Some(dot) = component.iter().position(|&b| b == b'.') {
        let base = &component[..dot];
        let xtn = &component[dot + 1..];
        if base.len() > 8 || xtn.len() > 3 || base.is_empty() {
            return None;
        }
        for (i, &c) in base.iter().enumerate() {
            name[i] = c.to_ascii_uppercase();
        }
        for (i, &c) in xtn.iter().enumerate() {
            ext[i] = c.to_ascii_uppercase();
        }
    } else {
        if component.len() > 8 || component.is_empty() {
            return None;
        }
        for (i, &c) in component.iter().enumerate() {
            name[i] = c.to_ascii_uppercase();
        }
    }
    Some((name, ext))
}

/// Format an on-disk 11-byte name as "NAME.EXT". Returns the length used.
fn format_83(raw: &[u8; 11], out: &mut [u8; MAX_NAME_83]) -> usize {
    let base_len = raw[..8].iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
    let ext_len = raw[8..11].iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);

    out[..base_len].copy_from_slice(&raw[..base_len]);
    let mut len = base_len;
    if ext_len > 0 {
        out[len] = b'.';
        out[len + 1..len + 1 + ext_len].copy_from_slice(&raw[8..8 + ext_len]);
        len += 1 + ext_len;
    }
    len
}
