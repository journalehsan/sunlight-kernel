//! Per-process resource accounting (currently all-zero / unenumerable).
//!
//! TODO: kernel syscall — enumerate resources for a pid: the kernel does not
//! expose IPC queue ownership, shared-memory grants, tmpfs handles, or open
//! file descriptors per-process. `ProcessResources` is defined for future
//! use and so the cleanup-report shape is documented, but `enumerate()`
//! always returns all-zero counts today.

pub const MAX_QUEUES: usize = 16;
pub const MAX_SHM: usize = 16;
pub const MAX_TMP: usize = 16;
pub const MAX_FDS: usize = 32;

pub struct ProcessResources {
    pub pid: usize,
    pub name: heapless::String<32>,
    pub ipc_queues: [u64; MAX_QUEUES],
    pub n_queues: usize,
    pub shared_mem: [u64; MAX_SHM],
    pub n_shm: usize,
    pub tmpfs: [heapless::String<64>; MAX_TMP],
    pub n_tmp: usize,
    pub open_fds: [u32; MAX_FDS],
    pub n_fds: usize,
}

impl ProcessResources {
    /// TODO: kernel syscall — enumerate resources for pid `pid`. Returns an
    /// all-zero-count `ProcessResources` since no enumeration syscall
    /// exists yet.
    pub fn enumerate(pid: usize, name: &str) -> Self {
        let mut n = heapless::String::new();
        let _ = n.push_str(name);
        Self {
            pid,
            name: n,
            ipc_queues: [0; MAX_QUEUES],
            n_queues: 0,
            shared_mem: [0; MAX_SHM],
            n_shm: 0,
            tmpfs: [const { heapless::String::new() }; MAX_TMP],
            n_tmp: 0,
            open_fds: [0; MAX_FDS],
            n_fds: 0,
        }
    }
}
