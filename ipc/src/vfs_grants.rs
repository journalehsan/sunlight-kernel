//! One-use VFS read/execute authorizations. These never authorize SHM mapping,
//! credential changes, or transfer to another process/image.
use crate::{CapabilityToken, IpcMsg, SunlightSyscall};

pub const DELEGATE: u64 = 3;
pub const BASE_CAPS: u64 = 4;
pub const REPLY_OK: u64 = 1;
pub const REPLY_ERR: u64 = 0xff;
pub const MINT_EXACT: u64 = 0;
pub const MINT_BASE: u64 = 1;
pub const CONSUME: u64 = 2;
pub const CANCEL: u64 = 3;

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssuanceError {
    Denied = 1,
    TokenHistoryFull = 2,
    Capacity = 3,
    Unavailable = 4,
}

impl IssuanceError {
    pub fn from_raw(raw: u64) -> Self {
        match raw {
            1 => Self::Denied,
            2 => Self::TokenHistoryFull,
            3 => Self::Capacity,
            _ => Self::Unavailable,
        }
    }
}

/// Reject aliases before applying the directory policy. This protocol accepts
/// canonical absolute ASCII paths only; it does not perform path resolution.
pub fn executable_path(path: &str) -> bool {
    path.len() < 64
        && (path.starts_with("/bin/") || path.starts_with("/usr/bin/"))
        && path[1..].split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.bytes().all(|b| b.is_ascii_graphic() && b != b'\\')
        })
}

/// UAC only: exchange the current IPC caller's delivered root password proof
/// for one exact-path grant or the two base directory grants. The kernel
/// resolves the caller from its reply target; no target PID is supplied here.
pub fn mint(proof: CapabilityToken, path: Option<&str>) -> Option<IpcMsg> {
    try_mint(proof, path).ok()
}

/// Capacity exhaustion is distinct from denial. A full boot history requires
/// lifecycle recovery, not a retry loop; existing grants remain consumable.
pub fn try_mint(proof: CapabilityToken, path: Option<&str>) -> Result<IpcMsg, IssuanceError> {
    let (operation, pointer, length) = match path {
        Some(path) if executable_path(path) => {
            (MINT_EXACT, path.as_ptr() as u64, path.len() as u64)
        }
        Some(_) => return Err(IssuanceError::Denied),
        None => (MINT_BASE, 0, 0),
    };
    let (result, reply) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::VfsGrant,
            operation,
            proof.0,
            pointer,
            length,
            0,
            0,
            0,
        )
    };
    if result == 0 {
        Ok(reply)
    } else {
        Err(IssuanceError::from_raw(result))
    }
}

/// VFS only: consume authority for its current kernel-authenticated caller.
/// A payload PID is never accepted as a recipient identity.
pub fn consume(token: CapabilityToken, path: &str) -> bool {
    if !executable_path(path) {
        return false;
    }
    let (result, _) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::VfsGrant,
            CONSUME,
            token.0,
            path.as_ptr() as u64,
            path.len() as u64,
            0,
            0,
            0,
        )
    };
    result == 0
}

/// Recipient-only, idempotent withdrawal. Cancelling an IPC call also retires
/// all undelivered grants from that call through normal broker settlement.
pub fn cancel(token: CapabilityToken) -> bool {
    let (result, _) =
        unsafe { crate::raw_syscall(SunlightSyscall::VfsGrant, CANCEL, token.0, 0, 0, 0, 0, 0) };
    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn executable_scope_rejects_aliases_and_sibling_prefixes() {
        for path in ["/bin/true", "/usr/bin/tool", "/bin/sub/tool"] {
            assert!(executable_path(path), "{path}");
        }
        for path in [
            "/bin",
            "/bin/",
            "/binary/tool",
            "/bin/../etc/shadow",
            "/bin/./x",
            "/bin//x",
            "bin/x",
            "/usr/binx/x",
            "/bin/a\0b",
            "/bin/a\\b",
            "/bin/é",
        ] {
            assert!(!executable_path(path), "{path}");
        }
    }
}
