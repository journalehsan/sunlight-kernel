/// Virtual address in the vfs_server's address space where the kernel maps the
/// FAT32 share page. Must be in user-space range and not overlap code/stack.
pub const FAT_SHARE_VADDR: u64 = 0x0000_0000_2000_0000; // 512 MiB

pub const SHARE_MAGIC: u32 = 0xFA73_5AEF;
pub const MAX_SHARE_FILES: usize = 8;

/// Per-file entry within the share page.
/// `path` is the local path within the `/boot` mount (e.g. "/HELLO.TXT").
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FatShareFile {
    pub path: [u8; 48],
    pub path_len: u32,
    pub data: [u8; 128],
    pub data_len: u32,
}

// sizeof(FatShareFile) = 48 + 4 + 128 + 4 = 184 bytes

/// Single page (4096 bytes) shared between kernel and vfs_server.
/// Kernel writes file contents here; vfs_server mounts /boot from it.
#[repr(C)]
pub struct FatSharePage {
    pub magic: u32,
    pub count: u32,
    pub files: [FatShareFile; MAX_SHARE_FILES],
}

// sizeof(FatSharePage) = 4 + 4 + 8 * 184 = 1480 bytes < 4096

impl FatShareFile {
    pub const fn zeroed() -> Self {
        Self {
            path: [0; 48],
            path_len: 0,
            data: [0; 128],
            data_len: 0,
        }
    }

    pub fn path_bytes(&self) -> &[u8] {
        &self.path[..self.path_len as usize]
    }

    pub fn data_bytes(&self) -> &[u8] {
        &self.data[..self.data_len as usize]
    }
}

impl FatSharePage {
    pub const fn zeroed() -> Self {
        Self {
            magic: 0,
            count: 0,
            files: [FatShareFile::zeroed(); MAX_SHARE_FILES],
        }
    }
}
