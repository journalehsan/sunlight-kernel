//! Validated Linux x86-64 open flags, independent of native VFS flags.

use crate::abi;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,
    pub directory: bool,
    pub truncate: bool,
    /// Linux-visible status kept on the shared open file description.
    pub status: u32,
    /// Descriptor-local close-on-exec state.
    pub cloexec: bool,
}

pub fn parse(raw: u64) -> Result<OpenFlags, ()> {
    if raw & !abi::OPEN_SUPPORTED_FLAGS != 0 {
        return Err(());
    }
    let access = raw & abi::O_ACCMODE;
    let read = access == abi::O_RDONLY || access == abi::O_RDWR;
    let write = access == abi::O_WRONLY || access == abi::O_RDWR;
    let create = raw & abi::O_CREAT != 0;
    let exclusive = raw & abi::O_EXCL != 0;
    let truncate = raw & abi::O_TRUNC != 0;
    if access == abi::O_ACCMODE || (exclusive && !create) || (truncate && !write) {
        return Err(());
    }
    Ok(OpenFlags {
        read,
        write,
        create,
        exclusive,
        directory: raw & abi::O_DIRECTORY != 0,
        truncate,
        // F_GETFL exposes access and status flags, including LARGEFILE and
        // the directory/path constraints. Creation-only flags (CREAT, EXCL,
        // TRUNC, NOCTTY) and descriptor-local CLOEXEC do not survive here.
        status: (raw
            & (abi::O_ACCMODE
                | abi::O_APPEND
                | abi::O_NONBLOCK
                | abi::O_LARGEFILE
                | abi::O_DIRECTORY
                | abi::O_NOFOLLOW)) as u32,
        cloexec: raw & abi::O_CLOEXEC != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_accepts_largefile_readonly() {
        let parsed = parse(abi::O_RDONLY | abi::O_LARGEFILE).unwrap();
        assert!(parsed.read);
        assert!(!parsed.write);
        assert_eq!(parsed.status, abi::O_LARGEFILE as u32);
    }

    #[test]
    fn open_largefile_preserves_cloexec_separately() {
        let parsed = parse(0x88000).unwrap();
        assert!(parsed.read);
        assert!(parsed.cloexec);
        assert_eq!(parsed.status, abi::O_LARGEFILE as u32);
        assert_eq!(parsed.status & abi::O_CLOEXEC as u32, 0);
    }

    #[test]
    fn open_creation_flags_do_not_leak_into_getfl() {
        let parsed = parse(
            abi::O_RDWR
                | abi::O_CREAT
                | abi::O_EXCL
                | abi::O_TRUNC
                | abi::O_NOCTTY
                | abi::O_LARGEFILE
                | abi::O_CLOEXEC,
        )
        .unwrap();
        assert!(parsed.create);
        assert!(parsed.exclusive);
        assert!(parsed.truncate);
        assert!(parsed.cloexec);
        assert_eq!(parsed.status, (abi::O_RDWR | abi::O_LARGEFILE) as u32);
    }

    #[test]
    fn open_directory_constraints_remain_visible_in_getfl() {
        let parsed = parse(
            abi::O_RDONLY | abi::O_DIRECTORY | abi::O_NOFOLLOW | abi::O_LARGEFILE,
        )
        .unwrap();
        assert!(parsed.directory);
        assert_eq!(
            parsed.status,
            (abi::O_DIRECTORY | abi::O_NOFOLLOW | abi::O_LARGEFILE) as u32
        );
    }

    #[test]
    fn open_rejects_unknown_and_unsupported_flags() {
        assert!(parse(abi::O_LARGEFILE | 0x40000000).is_err());
        assert!(parse(abi::O_DIRECT).is_err());
        assert!(parse(abi::O_LARGEFILE | abi::O_DIRECT).is_err());
        assert!(parse(abi::O_LARGEFILE | abi::O_DIRECTORY).is_ok());
    }
}
