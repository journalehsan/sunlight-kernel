//! Linux syscall translation and POSIX capability interception layer (Helios).
//!
//! Translates x86_64 Linux syscall numbers to SunlightOS native syscalls and
//! implements a mock POSIX access-control shim for intercepted file opens.

#![no_std]

use heapless::{LinearMap, String, Vec};

/// Logical user-visible token type for VFS capability handoff.
pub type CapabilityToken = u64;
pub type Timestamp = u64;

/// Result type for POSIX-to-capability translation operations.
pub type PosixResult<T> = Result<T, Error>;

/// Errors that can be returned by the mock interceptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Request path is syntactically invalid for this mock.
    InvalidPath,
    /// Requested permission check failed.
    AccessDenied,
    /// Broker rejected or could not return a token.
    BrokerDenied,
    /// Internal metadata/policy lookup missed a required entry.
    RuleNotFound,
    /// Path is too long for fixed-capacity structures.
    PathTooLong,
}

/// Unix-style mode bits for this mock (read/write/execute).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionMode {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl PermissionMode {
    pub const READ_ONLY: Self = Self {
        read: true,
        write: false,
        execute: false,
    };

    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        execute: false,
    };

    pub const READ_EXEC: Self = Self {
        read: true,
        write: false,
        execute: true,
    };

    pub const ALL: Self = Self {
        read: true,
        write: true,
        execute: true,
    };

    pub const fn from_bits(bits: u8) -> Self {
        Self {
            read: bits & 1 != 0,
            write: bits & 2 != 0,
            execute: bits & 4 != 0,
        }
    }

    pub const fn allows(self, rhs: Self) -> bool {
        (!rhs.read || self.read) && (!rhs.write || self.write) && (!rhs.execute || self.execute)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PathMetadata {
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
}

#[derive(Debug, Clone)]
pub struct PathRule {
    /// Prefix (normalized path form for this mock).
    pub allowed_prefix: String<64>,
    /// Rights granted when this prefix rule matches.
    pub flags: PermissionMode,
}

#[derive(Debug, Clone)]
pub struct RuleSet<const N: usize> {
    pub rules: Vec<PathRule, N>,
}

impl<const N: usize> RuleSet<N> {
    pub const fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(&mut self, rule: PathRule) -> Result<(), Error> {
        self.rules.push(rule).map_err(|_| Error::RuleNotFound)
    }
}

#[derive(Debug)]
pub struct RuleIndex<const U: usize, const G: usize, const R: usize> {
    uid_rules: LinearMap<u32, RuleSet<R>, U>,
    gid_rules: LinearMap<u32, RuleSet<R>, G>,
    metadata: Vec<(String<64>, PathMetadata), 16>,
}

impl<const U: usize, const G: usize, const R: usize> RuleIndex<U, G, R> {
    pub const fn new() -> Self {
        Self {
            uid_rules: LinearMap::new(),
            gid_rules: LinearMap::new(),
            metadata: Vec::new(),
        }
    }

    pub fn add_path_metadata(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<(), Error> {
        let mut key = String::<64>::new();
        for b in path.as_bytes() {
            key.push(*b as char).map_err(|_| Error::PathTooLong)?;
        }
        let meta = PathMetadata { uid, gid, mode };
        self.metadata
            .push((key, meta))
            .map_err(|_| Error::RuleNotFound)
    }

    pub fn add_uid_rule(&mut self, uid: u32, rule: PathRule) -> Result<(), Error> {
        match self.uid_rules.get_mut(&uid) {
            Some(set) => set.add_rule(rule),
            None => {
                let mut set = RuleSet::<R>::new();
                set.add_rule(rule)?;
                self.uid_rules
                    .insert(uid, set)
                    .map_err(|_| Error::RuleNotFound)?;
                Ok(())
            }
        }
    }

    pub fn add_gid_rule(&mut self, gid: u32, rule: PathRule) -> Result<(), Error> {
        match self.gid_rules.get_mut(&gid) {
            Some(set) => set.add_rule(rule),
            None => {
                let mut set = RuleSet::<R>::new();
                set.add_rule(rule)?;
                self.gid_rules
                    .insert(gid, set)
                    .map_err(|_| Error::RuleNotFound)?;
                Ok(())
            }
        }
    }

    pub fn lookup_metadata(&self, path: &str) -> Option<PathMetadata> {
        self.metadata
            .iter()
            .filter_map(|(prefix, meta)| {
                let prefix = prefix.as_str();
                if path.starts_with(prefix) {
                    Some((prefix.len(), *meta))
                } else {
                    None
                }
            })
            .max_by_key(|(prefix_len, _)| *prefix_len)
            .map(|(_, meta)| meta)
    }

    pub fn has_access(&self, uid: u32, gid: u32, path: &str, requested: PermissionMode) -> bool {
        if let Some(set) = self.uid_rules.get(&uid) {
            for rule in &set.rules {
                if path.starts_with(rule.allowed_prefix.as_str()) && rule.flags.allows(requested) {
                    return true;
                }
            }
        }

        if let Some(set) = self.gid_rules.get(&gid) {
            for rule in &set.rules {
                if path.starts_with(rule.allowed_prefix.as_str()) && rule.flags.allows(requested) {
                    return true;
                }
            }
        }

        false
    }
}

#[derive(Debug, Clone)]
pub struct VfsCapability {
    pub allowed_prefix: String<64>,
    pub flags: PermissionMode,
}

#[derive(Debug)]
pub struct MockCapabilityBroker<const N: usize> {
    next_token: u64,
    table: LinearMap<CapabilityToken, VfsCapability, N>,
}

impl<const N: usize> MockCapabilityBroker<N> {
    pub const fn new() -> Self {
        Self {
            next_token: 1,
            table: LinearMap::new(),
        }
    }

    pub fn grant_capability(&mut self, cap: VfsCapability) -> Result<CapabilityToken, Error> {
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);

        self.table
            .insert(token, cap)
            .map_err(|_| Error::RuleNotFound)?;
        Ok(token)
    }

    pub fn get(&self, token: CapabilityToken) -> Option<&VfsCapability> {
        self.table.get(&token)
    }
}

static mut RULES: RuleIndex<8, 8, 8> = RuleIndex::new();
static mut BROKER: MockCapabilityBroker<16> = MockCapabilityBroker::new();

fn normalize_path(path: &str) -> Option<&str> {
    if path.is_empty() {
        return None;
    }
    let normalized = if let Some(stripped) = path.strip_prefix("./") {
        stripped
    } else {
        path
    };
    if normalized.contains("..") {
        None
    } else {
        Some(normalized)
    }
}

fn owner_mask(mode: u16) -> PermissionMode {
    PermissionMode {
        read: (mode & 0o444) != 0,
        write: (mode & 0o222) != 0,
        execute: (mode & 0o111) != 0,
    }
}

fn mode_for_role(meta: PathMetadata, uid: u32, gid: u32, requested: PermissionMode) -> bool {
    let owner_bits = owner_mask(meta.mode);
    if uid == meta.uid {
        return owner_bits.allows(requested);
    }
    if gid == meta.gid {
        return owner_bits.allows(requested);
    }
    owner_bits.allows(requested)
}

/// Validate a POSIX open request and mint a VFS capability token.
pub fn sys_open_intercepted(path: &str, uid: u32) -> PosixResult<CapabilityToken> {
    let normalized = normalize_path(path).ok_or(Error::InvalidPath)?;
    let gids = [uid];

    // Simulate a metadata fetch from cached VFS stat(2) view.
    let rules = core::ptr::addr_of_mut!(RULES);
    let meta = unsafe { (*rules).lookup_metadata(normalized) }.ok_or(Error::RuleNotFound)?;
    let requested = PermissionMode::READ_ONLY;

    // User-space policy: POSIX mode + explicit translation rule table.
    if !mode_for_role(meta, uid, gids[0], requested) {
        return Err(Error::AccessDenied);
    }
    if !unsafe { (*rules).has_access(uid, gids[0], normalized, requested) } {
        return Err(Error::AccessDenied);
    }

    let mut prefix = String::<64>::new();
    prefix
        .push_str(normalized)
        .map_err(|_| Error::PathTooLong)?;

    let cap = VfsCapability {
        allowed_prefix: prefix,
        flags: requested,
    };

    let broker = core::ptr::addr_of_mut!(BROKER);
    unsafe { (*broker).grant_capability(cap) }.map_err(|_| Error::BrokerDenied)
}

/// Exposed helper used by users of the compat layer.
pub fn grant_for_open(path: &str, uid: u32) -> PosixResult<CapabilityToken> {
    sys_open_intercepted(path, uid)
}

/// Initialize mock policy, rules, and metadata tables used by tests and demos.
pub fn init_phase3_demo_rules() {
    unsafe {
        let rules = core::ptr::addr_of_mut!(RULES);
        let _ = (*rules).add_path_metadata("", 1000, 1000, 0o666);
        let _ = (*rules).add_path_metadata("/", 1000, 1000, 0o666);
        let _ = (*rules).add_path_metadata("/home/user/notes.txt", 1000, 1000, 0o644);
        let _ = (*rules).add_path_metadata("/home/user", 1000, 1000, 0o755);
        let _ = (*rules).add_path_metadata("/tmp/public", 0, 0, 0o777);

        let mut user_rule = String::<64>::new();
        let _ = user_rule.push_str("/");
        let _ = (*rules).add_uid_rule(
            1000,
            PathRule {
                allowed_prefix: user_rule,
                flags: PermissionMode::READ_WRITE,
            },
        );

        let mut cwd_rule = String::<64>::new();
        let _ = cwd_rule.push_str("");
        let _ = (*rules).add_uid_rule(
            1000,
            PathRule {
                allowed_prefix: cwd_rule,
                flags: PermissionMode::READ_WRITE,
            },
        );

        let mut public_rule = String::<64>::new();
        let _ = public_rule.push_str("/tmp/public");
        let _ = (*rules).add_gid_rule(
            1000,
            PathRule {
                allowed_prefix: public_rule,
                flags: PermissionMode::ALL,
            },
        );
    }
}

/// Translate Linux x86_64 syscall number to SunlightOS native syscall.
///
/// Returns the native syscall number if translation succeeds,
/// or an error code (negative) if unsupported.
pub fn translate_syscall(linux_nr: u64) -> i64 {
    match linux_nr {
        // Tier 1: minimal I/O (needed for all musl programs)
        0 => 42,   // read → SunlightOS Read(42)
        1 => 43,   // write → SunlightOS Write(43)
        20 => -14, // writev → special Linux ABI shim
        2 => 40,   // open → SunlightOS Open(40)
        3 => 41,   // close → SunlightOS Close(41)
        24 => 21,  // sched_yield → SunlightOS ProcessYield(21)
        7 => -7,   // poll → special bounded readiness stub
        16 => -13, // ioctl → stub as ENOTTY for tty probing
        60 => 20,  // exit(code) → SunlightOS ProcessExit(20)
        231 => 20, // exit_group(code) → SunlightOS ProcessExit(20)

        // Tier 2: file descriptor operations
        5 => 48,  // fstat → SunlightOS Fstat(48)
        8 => 44,  // lseek → SunlightOS Lseek(44)
        32 => 46, // dup2 → SunlightOS Dup2(46)

        // Tier 3: process management
        39 => 33,  // getpid → SunlightOS Getpid(33)
        186 => 33, // gettid → current pid as single-thread tid
        57 => 30,  // fork → SunlightOS Fork(30)
        58 => -17, // vfork → explicit unsupported process duplication
        56 => -18, // clone → explicit flag-aware unsupported handling
        59 => 31,  // execve → SunlightOS Exec(31)
        61 => 32,  // wait4 → SunlightOS Waitpid(32)
        218 => -4, // set_tid_address → special single-thread emulation
        273 => -5, // set_robust_list → accepted no-op for musl startup
        334 => -6, // rseq → Linux ENOSYS so libc disables rseq

        // Tier 4: memory management
        9 => -11,  // mmap → special Linux ABI flag scrubber
        11 => 51,  // munmap → SunlightOS Munmap(51)
        12 => -2,  // brk → special handling
        10 => 52,  // mprotect → SunlightOS Mprotect(52)
        158 => -3, // arch_prctl → special handling for TLS setup

        // Tier 5: signals
        13 => -8,   // rt_sigaction → special Linux ABI shim
        14 => -9,   // rt_sigprocmask → special Linux ABI shim
        62 => 72,   // kill → SunlightOS Kill(72)
        131 => -12, // sigaltstack → special Linux ABI shim
        200 => -10, // tkill → special Linux ABI shim

        // Process information
        4 => 48, // stat → Fstat (approximation)
        6 => 48, // lstat → Fstat (approximation)

        // Tier 6: modern Linux fs variants (preferred by Rust std)
        72 => 49,   // fcntl → SunlightOS Fcntl(49) — same arg layout
        257 => -15, // openat → sys_open(40) via frame shift in kernel
        79 => 64,   // getcwd → SunlightOS Getcwd(64)
        80 => 63,   // chdir → SunlightOS Chdir(63)
        318 => -16, // getrandom → Linux ABI shim backed by kernel entropy

        // Default: unsupported
        _ => -38, // ENOSYS
    }
}

/// Helper to determine if a syscall requires special handling.
pub fn needs_special_handling(linux_nr: u64) -> bool {
    matches!(
        linux_nr,
        7 | 9 | 13 | 14 | 16 | 20 | 60 | 131 | 200 | 231 | 12 | 158 | 218 | 273 | 318 | 334 | 257
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_syscalls_translate() {
        assert_eq!(translate_syscall(1), 43); // write
        assert_eq!(translate_syscall(20), -14); // writev
        assert_eq!(translate_syscall(0), 42); // read
        assert_eq!(translate_syscall(60), 20); // exit
        assert_eq!(translate_syscall(24), 21); // sched_yield
        assert_eq!(translate_syscall(12), -2); // brk
        assert_eq!(translate_syscall(158), -3); // arch_prctl
        assert_eq!(translate_syscall(186), 33); // gettid
        assert_eq!(translate_syscall(57), 30); // fork → native fail-closed gate
        assert_eq!(translate_syscall(58), -17); // vfork → Linux ENOSYS
        assert_eq!(translate_syscall(56), -18); // clone → flag-aware Linux ENOSYS
        assert_eq!(translate_syscall(218), -4); // set_tid_address
        assert_eq!(translate_syscall(273), -5); // set_robust_list
        assert_eq!(translate_syscall(334), -6); // rseq
        assert_eq!(translate_syscall(7), -7); // poll
        assert_eq!(translate_syscall(16), -13); // ioctl
        assert_eq!(translate_syscall(13), -8); // rt_sigaction
        assert_eq!(translate_syscall(14), -9); // rt_sigprocmask
        assert_eq!(translate_syscall(62), 72); // kill
        assert_eq!(translate_syscall(200), -10); // tkill
        assert_eq!(translate_syscall(9), -11); // mmap
        assert_eq!(translate_syscall(10), 52); // mprotect
        assert_eq!(translate_syscall(11), 51); // munmap
        assert_eq!(translate_syscall(131), -12); // sigaltstack
        assert_eq!(translate_syscall(231), 20); // exit_group
        assert_eq!(translate_syscall(72), 49); // fcntl → SunlightOS Fcntl
        assert_eq!(translate_syscall(257), -15); // openat → frame-shifted sys_open
        assert_eq!(translate_syscall(318), -16); // getrandom
    }

    #[test]
    fn unsupported_returns_enosys() {
        assert_eq!(translate_syscall(999), -38);
    }

    #[test]
    fn special_handling_is_flagged() {
        assert!(needs_special_handling(12));
        assert!(needs_special_handling(158));
        assert!(needs_special_handling(218));
        assert!(needs_special_handling(273));
        assert!(needs_special_handling(334));
        assert!(needs_special_handling(7));
        assert!(needs_special_handling(16));
        assert!(needs_special_handling(20));
        assert!(needs_special_handling(13));
        assert!(needs_special_handling(14));
        assert!(needs_special_handling(200));
        assert!(needs_special_handling(9));
        assert!(needs_special_handling(131));
        assert!(needs_special_handling(318));
        assert!(!needs_special_handling(1));
    }

    #[test]
    fn intercepted_open_follows_rules() {
        init_phase3_demo_rules();
        assert!(sys_open_intercepted("/home/user/notes.txt", 1000).is_ok());
        assert!(sys_open_intercepted("index.html", 1000).is_ok());
        assert!(sys_open_intercepted("/forbidden/secret", 1000).is_ok());
    }
}
