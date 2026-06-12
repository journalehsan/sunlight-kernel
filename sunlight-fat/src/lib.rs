#![no_std]

#[cfg(any(test, feature = "testutil"))]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod share;
mod fat;
#[cfg(any(test, feature = "testutil"))]
pub mod testimg;

pub use fat::{Fat32, FatStat, MAX_NAME_83};
pub use share::{FatSharePage, FatShareFile, FAT_SHARE_VADDR, SHARE_MAGIC};

#[cfg(test)]
mod tests {
    use crate::testimg::FatImageBuilder;
    use crate::Fat32;
    use alloc::vec;
    use alloc::vec::Vec;
    use sunlight_block::MemDisk;

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn mounts_and_reads_root_file() {
        let mut builder = FatImageBuilder::new(1);
        builder.add_file(builder.root(), "HELLO.TXT", b"boot volume\n");
        let mut image = builder.build();

        let mut fat = Fat32::mount(MemDisk::new(&mut image)).expect("mount");
        let mut out = [0u8; 32];
        let n = fat.read_file(b"/HELLO.TXT", &mut out).expect("read");
        assert_eq!(&out[..n], b"boot volume\n");
    }

    #[test]
    fn rejects_non_fat32_image() {
        let mut image = vec![0u8; 4096];
        assert!(Fat32::mount(MemDisk::new(&mut image)).is_none());
    }

    #[test]
    fn reads_multi_cluster_file_through_fat_chain() {
        // spc=1 → 512-byte clusters; 3000 bytes spans 6 chained clusters.
        let data = pattern(3000);
        let mut builder = FatImageBuilder::new(1);
        builder.add_file(builder.root(), "BIG.BIN", &data);
        let mut image = builder.build();

        let mut fat = Fat32::mount(MemDisk::new(&mut image)).expect("mount");
        let mut out = vec![0u8; 4096];
        let n = fat.read_file(b"/BIG.BIN", &mut out).expect("read");
        assert_eq!(n, 3000);
        assert_eq!(&out[..n], &data[..]);
    }

    #[test]
    fn read_at_honors_offset_across_cluster_boundary() {
        let data = pattern(2000);
        let mut builder = FatImageBuilder::new(1);
        builder.add_file(builder.root(), "BIG.BIN", &data);
        let mut image = builder.build();

        let mut fat = Fat32::mount(MemDisk::new(&mut image)).expect("mount");
        let stat = fat.stat_path(b"/BIG.BIN").expect("stat");
        assert!(!stat.is_dir);
        assert_eq!(stat.size, 2000);

        // Offset 700 crosses the first cluster boundary (512).
        let mut out = [0u8; 1000];
        let n = fat.read_at(stat.first_cluster, stat.size, 700, &mut out).expect("read_at");
        assert_eq!(n, 1000);
        assert_eq!(&out[..], &data[700..1700]);

        // Reading past EOF returns 0 bytes.
        assert_eq!(fat.read_at(stat.first_cluster, stat.size, 2000, &mut out), Some(0));
    }

    #[test]
    fn resolves_nested_paths() {
        let mut builder = FatImageBuilder::new(1);
        let boot = builder.add_dir(builder.root(), "BOOT");
        let cfg = builder.add_dir(boot, "CFG");
        builder.add_file(cfg, "PHASE.TXT", b"nested!");
        let mut image = builder.build();

        let mut fat = Fat32::mount(MemDisk::new(&mut image)).expect("mount");
        let mut out = [0u8; 16];
        let n = fat.read_file(b"/BOOT/CFG/PHASE.TXT", &mut out).expect("read");
        assert_eq!(&out[..n], b"nested!");

        assert!(fat.stat_path(b"/BOOT/CFG").expect("stat dir").is_dir);
        assert!(fat.stat_path(b"/BOOT/MISSING.TXT").is_none());
        // Descending through a file is rejected.
        assert!(fat.stat_path(b"/BOOT/CFG/PHASE.TXT/X").is_none());
    }

    #[test]
    fn read_dir_lists_formatted_names() {
        let mut builder = FatImageBuilder::new(1);
        builder.add_file(builder.root(), "HELLO.TXT", b"hi");
        builder.add_dir(builder.root(), "BOOT");
        builder.add_file(builder.root(), "NOEXT", b"x");
        let mut image = builder.build();

        let mut fat = Fat32::mount(MemDisk::new(&mut image)).expect("mount");
        let mut listed: Vec<(Vec<u8>, bool, u32)> = Vec::new();
        fat.read_dir_raw(b"/", &mut |name, is_dir, size| {
            listed.push((name.to_vec(), is_dir, size));
            true
        })
        .expect("read_dir");

        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0], (b"HELLO.TXT".to_vec(), false, 2));
        assert_eq!(listed[1], (b"BOOT".to_vec(), true, 0));
        assert_eq!(listed[2], (b"NOEXT".to_vec(), false, 1));

        // Listing a file is an error.
        assert!(fat.read_dir_raw(b"/HELLO.TXT", &mut |_, _, _| true).is_none());
    }
}
