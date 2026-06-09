use crate::vfs::{mode, FileStat};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credential {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermCheck {
    Read,
    Write,
    Execute,
}

/// Check whether `cred` has `want` access to the file described by `stat`.
/// Root (uid == 0) always passes.
pub fn check_permission(stat: &FileStat, cred: &Credential, want: PermCheck) -> bool {
    if cred.uid == 0 {
        return true;
    }

    let (r, w, x) = if stat.uid == cred.uid {
        (
            stat.mode & mode::S_IRUSR != 0,
            stat.mode & mode::S_IWUSR != 0,
            stat.mode & mode::S_IXUSR != 0,
        )
    } else if stat.gid == cred.gid {
        (
            stat.mode & mode::S_IRGRP != 0,
            stat.mode & mode::S_IWGRP != 0,
            stat.mode & mode::S_IXGRP != 0,
        )
    } else {
        (
            stat.mode & mode::S_IROTH != 0,
            stat.mode & mode::S_IWOTH != 0,
            stat.mode & mode::S_IXOTH != 0,
        )
    };

    match want {
        PermCheck::Read => r,
        PermCheck::Write => w,
        PermCheck::Execute => x,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{FileType, mode};

    fn make_stat(uid: u32, gid: u32, mode: u16) -> FileStat {
        FileStat { file_type: FileType::File, size: 0, uid, gid, mode, nlinks: 1 }
    }

    #[test]
    fn root_bypasses_all_checks() {
        let stat = make_stat(0, 0, mode::FILE_600);
        let root = Credential { uid: 0, gid: 0 };
        assert!(check_permission(&stat, &root, PermCheck::Read));
        assert!(check_permission(&stat, &root, PermCheck::Write));
        assert!(check_permission(&stat, &root, PermCheck::Execute));
    }

    #[test]
    fn user_reads_world_readable_file() {
        let stat = make_stat(0, 0, mode::FILE_644);
        let user = Credential { uid: 1000, gid: 1000 };
        assert!(check_permission(&stat, &user, PermCheck::Read));
        assert!(!check_permission(&stat, &user, PermCheck::Write));
    }

    #[test]
    fn user_denied_root_only_file() {
        let stat = make_stat(0, 0, mode::FILE_600);
        let user = Credential { uid: 1000, gid: 1000 };
        assert!(!check_permission(&stat, &user, PermCheck::Read));
        assert!(!check_permission(&stat, &user, PermCheck::Write));
    }

    #[test]
    fn owner_has_owner_bits() {
        let stat = make_stat(1000, 1000, mode::FILE_600);
        let user = Credential { uid: 1000, gid: 1000 };
        assert!(check_permission(&stat, &user, PermCheck::Read));
        assert!(check_permission(&stat, &user, PermCheck::Write));
    }

    #[test]
    fn group_member_has_group_bits() {
        // File owned by uid=0, gid=10 with mode 0o640
        let stat = make_stat(0, 10, 0o100_640);
        let user = Credential { uid: 1000, gid: 10 };
        assert!(check_permission(&stat, &user, PermCheck::Read));
        assert!(!check_permission(&stat, &user, PermCheck::Write));
    }
}
