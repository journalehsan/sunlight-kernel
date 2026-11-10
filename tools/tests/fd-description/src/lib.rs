//! Host harness for shared open descriptions. Pipe/eventfd backends are inert;
//! their live behavior is covered by the native pipeline gate.
extern crate alloc;
mod pipe {
    pub fn socket_retain(_: u32, _: usize) {}
    pub fn pipe_retain_end(_: u32, _: bool) {}
    pub fn pipe_close_end(_: u32, _: bool) {}
    pub fn socket_close(_: u32, _: usize) {}
    pub fn socket_stream(_: u32, _: usize) -> Option<(u32, u32)> { None }
}
mod eventfd {
    pub fn retain(_: u32) {}
    pub fn release(_: u32) {}
}
#[path = "../../../../kernel/src/process/fd_table.rs"]
mod fd_table;

mod unix_socket {
    pub fn retain(_: u32) {}
    pub fn release(_: u32) {}
}
#[path = "../../../../kernel/src/process/thread_admission.rs"]
mod thread_admission;

#[cfg(test)]
mod open_unlink {
    use super::fd_table::{CapRights, FdTable, FileHandle};
    use sunlight_fs::{FileHandle as VfsHandle, FsError, RamEntry, Tmpfs, Vfs};

    #[test]
    fn vfs_open_unlink_seek_dup_and_exit_cleanup() {
        let baseline = sunlight_mm::testutil::live_frames();
        static SEED: &[RamEntry] = &[RamEntry::dir("/tmp", 0, 0, 0o41777)];
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_tmpfs("/", Tmpfs::new(SEED)).unwrap();
        let handle = vfs.create_file_exclusive("/tmp/contract", 1000, 1000, 0o600).unwrap();
        let mut parent = FdTable::new_boxed();
        let fd = parent.open(FileHandle::vfs(handle.0), CapRights::new(CapRights::READ | CapRights::WRITE | CapRights::FSTAT), 2).unwrap();
        let duplicate = parent.dup(fd).unwrap();
        let mut child = parent.clone_boxed();
        vfs.unlink("/tmp/contract").unwrap();
        assert_eq!(vfs.open("/tmp/contract"), Err(FsError::NotFound));
        // Compose the actual VFS and shared descriptor-offset operations used
        // by native read/write/lseek. This host test issues no guest syscall.
        let start = parent.get(fd).unwrap().current_offset();
        let n = vfs.write(handle, start, b"anonymous").unwrap();
        parent.get_mut(fd).unwrap().set_offset(start + n);
        assert_eq!(child.get(fd).unwrap().current_offset(), 9);
        parent.get_mut(duplicate).unwrap().set_offset(0);
        let start = child.get(fd).unwrap().current_offset();
        let mut out = [0; 9];
        assert_eq!(vfs.read(handle, start, &mut out), Ok(9));
        assert_eq!(&out, b"anonymous");
        assert_eq!(vfs.fstat_handle(handle).unwrap().nlinks, 0);
        assert_eq!(sunlight_mm::testutil::live_frames(), baseline + 1);
        assert!(!parent.take(fd).unwrap().release_backend);
        // The production process reaper consumes take_all_handles and closes
        // only final VFS references. Parent exit must not retire child refs.
        for h in parent.take_all_handles().into_iter().flatten().filter(|h| h.is_vfs()) {
            vfs.close(VfsHandle(h.vfs_handle())).unwrap();
        }
        assert_eq!(vfs.read(handle, 0, &mut out), Ok(9));
        for h in child.take_all_handles().into_iter().flatten().filter(|h| h.is_vfs()) {
            vfs.close(VfsHandle(h.vfs_handle())).unwrap();
        }
        assert_eq!(vfs.read(handle, 0, &mut out), Err(FsError::BadHandle));
        assert_eq!(sunlight_mm::testutil::live_frames(), baseline);
    }
}
