//! Synthetic FAT32 image builder for unit tests (test/`testutil` builds only).
//!
//! Produces a minimal valid image: 1 reserved sector (BPB), 1 FAT copy,
//! data area starting at cluster 2 (the root directory). Directories are
//! limited to one cluster, which is plenty for tests.

use alloc::vec;
use alloc::vec::Vec;

const EOC: u32 = 0x0FFF_FFFF;
const SECTOR: usize = 512;

pub struct FatImageBuilder {
    spc: u8,
    fat: Vec<u32>,
    clusters: Vec<Vec<u8>>, // clusters[i] is cluster i + 2
}

impl FatImageBuilder {
    pub fn new(spc: u8) -> Self {
        assert!(spc > 0);
        let mut builder = Self {
            spc,
            fat: vec![0x0FFF_FFF8, EOC], // entries 0 and 1 are reserved
            clusters: Vec::new(),
        };
        let root = builder.alloc_cluster();
        assert_eq!(root, 2);
        builder
    }

    pub fn root(&self) -> u32 {
        2
    }

    fn cluster_bytes(&self) -> usize {
        self.spc as usize * SECTOR
    }

    fn alloc_cluster(&mut self) -> u32 {
        let bytes = self.cluster_bytes();
        self.clusters.push(vec![0u8; bytes]);
        self.fat.push(EOC);
        (self.fat.len() - 1) as u32
    }

    fn alloc_chain(&mut self, data: &[u8]) -> u32 {
        let bytes = self.cluster_bytes();
        let first = self.alloc_cluster();
        let mut current = first;
        for (i, chunk) in data.chunks(bytes).enumerate() {
            if i > 0 {
                let next = self.alloc_cluster();
                self.fat[current as usize] = next;
                current = next;
            }
            self.clusters[(current - 2) as usize][..chunk.len()].copy_from_slice(chunk);
        }
        first
    }

    fn append_entry(&mut self, dir_cluster: u32, entry: [u8; 32]) {
        let dir = &mut self.clusters[(dir_cluster - 2) as usize];
        let slot = dir
            .chunks(32)
            .position(|e| e[0] == 0x00)
            .expect("test directory cluster full");
        dir[slot * 32..slot * 32 + 32].copy_from_slice(&entry);
    }

    pub fn add_file(&mut self, dir_cluster: u32, name: &str, data: &[u8]) {
        let cluster = if data.is_empty() { 0 } else { self.alloc_chain(data) };
        let entry = make_entry(name, 0x00, cluster, data.len() as u32);
        self.append_entry(dir_cluster, entry);
    }

    /// Add a subdirectory; returns its cluster for populating.
    pub fn add_dir(&mut self, parent_cluster: u32, name: &str) -> u32 {
        let cluster = self.alloc_cluster();
        let entry = make_entry(name, 0x10, cluster, 0);
        self.append_entry(parent_cluster, entry);
        cluster
    }

    pub fn build(&self) -> Vec<u8> {
        let fat_sectors = (self.fat.len() * 4).div_ceil(SECTOR).max(1);
        let reserved = 1usize;
        let total_sectors =
            reserved + fat_sectors + self.clusters.len() * self.spc as usize;
        let mut image = vec![0u8; total_sectors * SECTOR];

        // BPB
        image[11..13].copy_from_slice(&512u16.to_le_bytes());
        image[13] = self.spc;
        image[14..16].copy_from_slice(&(reserved as u16).to_le_bytes());
        image[16] = 1; // num FATs
        image[36..40].copy_from_slice(&(fat_sectors as u32).to_le_bytes());
        image[44..48].copy_from_slice(&2u32.to_le_bytes()); // root cluster
        image[82..90].copy_from_slice(b"FAT32   ");

        // FAT
        for (i, &entry) in self.fat.iter().enumerate() {
            let off = reserved * SECTOR + i * 4;
            image[off..off + 4].copy_from_slice(&entry.to_le_bytes());
        }

        // Data area
        let data_start = (reserved + fat_sectors) * SECTOR;
        for (i, cluster) in self.clusters.iter().enumerate() {
            let off = data_start + i * self.cluster_bytes();
            image[off..off + cluster.len()].copy_from_slice(cluster);
        }

        image
    }
}

fn make_entry(name: &str, attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut entry = [0u8; 32];
    entry[..11].copy_from_slice(&to_name11(name));
    entry[11] = attr;
    entry[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    entry[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    entry[28..32].copy_from_slice(&size.to_le_bytes());
    entry
}

fn to_name11(name: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    let bytes = name.as_bytes();
    let (base, ext) = match bytes.iter().position(|&b| b == b'.') {
        Some(dot) => (&bytes[..dot], &bytes[dot + 1..]),
        None => (bytes, &b""[..]),
    };
    assert!(base.len() <= 8 && ext.len() <= 3, "invalid 8.3 test name");
    for (i, &b) in base.iter().enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    for (i, &b) in ext.iter().enumerate() {
        out[8 + i] = b.to_ascii_uppercase();
    }
    out
}
