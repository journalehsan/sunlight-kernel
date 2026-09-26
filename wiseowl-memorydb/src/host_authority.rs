//! Host-only process-owned writer lease. Native SunlightOS uses an owned
//! nameserver endpoint instead of depending on host flock semantics.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::os::fd::AsRawFd;

pub struct HostWriterAuthority {
    _file: File,
}

impl HostWriterAuthority {
    pub fn acquire(data_dir: &Path) -> std::io::Result<Self> {
        let parent = data_dir.parent().unwrap_or_else(|| Path::new("."));
        let leaf = data_dir.file_name().unwrap_or_default().to_string_lossy();
        let lock_path = parent.join(format!("{leaf}.authority.lock"));
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(lock_path)?;
        // SAFETY: flock acts on the live descriptor, with ownership released
        // automatically by the host kernel when this process exits.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_memorydb_authority_for_same_store_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let first = HostWriterAuthority::acquire(dir.path()).unwrap();
        assert!(HostWriterAuthority::acquire(dir.path()).is_err());
        drop(first);
        assert!(HostWriterAuthority::acquire(dir.path()).is_ok());
    }
}
