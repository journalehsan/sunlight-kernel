/// Minimal read-only FAT32 driver.
///
/// Generic over `R: FnMut(u64, &mut [u8; 512]) -> bool` for block reads.
/// No heap allocation; designed for no_std kernel use.
pub struct Fat32<R: FnMut(u64, &mut [u8; 512]) -> bool> {
    reader: R,
    spc: u8,   // sectors per cluster
    fds: u32,  // first data sector (LBA)
    rc: u32,   // root cluster number
}

/// Parsed FAT32 directory entry (subset of fields we need).
struct DirEntry {
    cluster: u32,
    size: u32,
    attr: u8,
}

impl<R: FnMut(u64, &mut [u8; 512]) -> bool> Fat32<R> {
    /// Parse the BPB from LBA 0 and validate FAT32 signature.
    /// Returns None if not FAT32 or read fails.
    pub fn mount(mut reader: R) -> Option<Self> {
        let mut s = [0u8; 512];
        if !reader(0, &mut s) {
            return None;
        }

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
        // Check file-system type string at offset 82
        if &s[82..90] != b"FAT32   " {
            return None;
        }

        let fds = reserved + num_fats * fat32_size;
        Some(Fat32 { reader, spc, fds, rc: root_cluster })
    }

    fn cluster_lba(&self, cluster: u32) -> u64 {
        (self.fds + (cluster - 2) * self.spc as u32) as u64
    }

    /// Look up `name8` + `ext3` in the directory rooted at `cluster`.
    fn find_in_dir(
        &mut self,
        cluster: u32,
        name8: &[u8; 8],
        ext3: &[u8; 3],
    ) -> Option<DirEntry> {
        let lba = self.cluster_lba(cluster);
        let mut sec = [0u8; 512];
        for s_off in 0..self.spc as u64 {
            if !(self.reader)(lba + s_off, &mut sec) {
                return None;
            }
            for i in 0..(512 / 32) {
                let o = i * 32;
                if sec[o] == 0x00 {
                    return None; // end of directory
                }
                if sec[o] == 0xE5 {
                    continue; // deleted
                }
                let attr = sec[o + 11];
                if attr == 0x0F {
                    continue; // LFN entry — skip
                }
                if &sec[o..o + 8] == name8 && &sec[o + 8..o + 11] == ext3 {
                    let chi = u16::from_le_bytes([sec[o + 20], sec[o + 21]]) as u32;
                    let clo = u16::from_le_bytes([sec[o + 26], sec[o + 27]]) as u32;
                    let cluster = (chi << 16) | clo;
                    let size = u32::from_le_bytes([sec[o+28], sec[o+29], sec[o+30], sec[o+31]]);
                    return Some(DirEntry { cluster, size, attr });
                }
            }
        }
        None
    }

    /// Read file data for `entry` into `out`. Returns bytes read.
    fn read_file_data(&mut self, entry: &DirEntry, out: &mut [u8]) -> Option<usize> {
        if entry.cluster < 2 {
            return None;
        }
        let limit = (entry.size as usize).min(out.len());
        let lba = self.cluster_lba(entry.cluster);
        let mut sec = [0u8; 512];
        let mut written = 0usize;
        for s_off in 0..self.spc as u64 {
            if written >= limit {
                break;
            }
            if !(self.reader)(lba + s_off, &mut sec) {
                return None;
            }
            let remaining = limit - written;
            let chunk = remaining.min(512);
            out[written..written + chunk].copy_from_slice(&sec[..chunk]);
            written += chunk;
        }
        Some(written)
    }

    /// Read a file at `path` (bytes, relative to FAT32 root) into `out`.
    ///
    /// Supports single-level paths like b"/HELLO.TXT"
    /// and two-level paths like b"/BOOT/PHASE35.TXT".
    /// Returns number of bytes written on success.
    pub fn read_file(&mut self, path: &[u8], out: &mut [u8]) -> Option<usize> {
        // Strip leading slash
        let path = if path.first() == Some(&b'/') { &path[1..] } else { path };

        if let Some(slash) = path.iter().position(|&b| b == b'/') {
            // Two-component: directory / file
            let dir_part = &path[..slash];
            let file_part = &path[slash + 1..];

            let (dn, de) = parse_83(dir_part)?;
            let dir_ent = self.find_in_dir(self.rc, &dn, &de)?;
            if dir_ent.attr & 0x10 == 0 {
                return None; // not a directory
            }

            let (fn_, fe) = parse_83(file_part)?;
            let file_ent = self.find_in_dir(dir_ent.cluster, &fn_, &fe)?;
            if file_ent.attr & 0x10 != 0 {
                return None; // is a directory
            }

            self.read_file_data(&file_ent, out)
        } else {
            // Single-component: file in root
            let (fn_, fe) = parse_83(path)?;
            let file_ent = self.find_in_dir(self.rc, &fn_, &fe)?;
            if file_ent.attr & 0x10 != 0 {
                return None; // is a directory
            }
            self.read_file_data(&file_ent, out)
        }
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
