//! Copied service-only cwd/root transaction, correlated with an outstanding retain.
//! Successful kernel copies supply authority; ordinary IPC never calls this API.
use super::{
    retention::{transport::CWD_BYTES, wire::FRAME_BYTES, Scope},
    ProtocolError,
};
pub const CONTEXT: u64 = 6;
pub const FINISH: u64 = 7;
const HEADER_BYTES: usize = 72;

/// Authenticated operation kind supplied only by the copied kernel context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Kind {
    Directory = 0,
    Root = 1,
}
pub const CONTEXT_BYTES: usize = HEADER_BYTES + 4096;
pub const FINISH_BYTES: usize = FRAME_BYTES + 8 + CWD_BYTES;

pub struct Context<'a> {
    pub kind: Kind,
    pub old: Scope,
    pub uid: u32,
    pub gid: u32,
    pub path: &'a str,
}

pub fn encode_context(
    old: Scope,
    uid: u32,
    gid: u32,
    path: &str,
) -> Result<[u8; CONTEXT_BYTES], ProtocolError> {
    encode_transition(old, uid, gid, path, Kind::Directory)
}

pub fn encode_transition(
    old: Scope,
    uid: u32,
    gid: u32,
    path: &str,
    kind: Kind,
) -> Result<[u8; CONTEXT_BYTES], ProtocolError> {
    if path.is_empty() || path.len() > 4095 || path.as_bytes().contains(&0) {
        return Err(ProtocolError::Encoding);
    }
    let scope = [
        old.caller.pid,
        old.caller.incarnation,
        old.caller.address_space_generation,
        old.namespace_id,
        old.namespace_generation,
    ];
    if scope.contains(&0) {
        return Err(ProtocolError::Encoding);
    }
    let mut bytes = [0; CONTEXT_BYTES];
    bytes[..8].copy_from_slice(&super::VERSION.to_le_bytes());
    for (chunk, word) in bytes[8..48].chunks_exact_mut(8).zip(scope) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes[48..52].copy_from_slice(&uid.to_le_bytes());
    bytes[52..56].copy_from_slice(&gid.to_le_bytes());
    bytes[56..64].copy_from_slice(&(path.len() as u64).to_le_bytes());
    bytes[64..72].copy_from_slice(&(kind as u64).to_le_bytes());
    bytes[HEADER_BYTES..HEADER_BYTES + path.len()].copy_from_slice(path.as_bytes());
    Ok(bytes)
}
pub fn decode_context(bytes: &[u8; CONTEXT_BYTES]) -> Result<Context<'_>, ProtocolError> {
    let word = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    if word(0) != super::VERSION {
        return Err(ProtocolError::Version);
    }
    if (8..48).step_by(8).any(|offset| word(offset) == 0) {
        return Err(ProtocolError::Encoding);
    }
    let kind = match word(64) {
        0 => Kind::Directory,
        1 => Kind::Root,
        _ => return Err(ProtocolError::Operation),
    };
    let old = Scope {
        caller: super::lifecycle::Caller {
            pid: word(8),
            incarnation: word(16),
            address_space_generation: word(24),
        },
        namespace_id: word(32),
        namespace_generation: word(40),
    };
    let uid = u32::from_le_bytes(bytes[48..52].try_into().unwrap());
    let gid = u32::from_le_bytes(bytes[52..56].try_into().unwrap());
    let len = usize::try_from(word(56)).map_err(|_| ProtocolError::Bounds)?;
    if len == 0 || len > 4095 || bytes[HEADER_BYTES + len..].iter().any(|b| *b != 0) {
        return Err(ProtocolError::Bounds);
    }
    let path = core::str::from_utf8(&bytes[HEADER_BYTES..HEADER_BYTES + len])
        .map_err(|_| ProtocolError::Encoding)?;
    if path.as_bytes().contains(&0) {
        return Err(ProtocolError::Encoding);
    }
    Ok(Context {
        kind,
        old,
        uid,
        gid,
        path,
    })
}
pub fn context(epoch: u64, retain: &[u8; FRAME_BYTES]) -> Option<[u8; CONTEXT_BYTES]> {
    let mut bytes = [0; CONTEXT_BYTES];
    bytes[..FRAME_BYTES].copy_from_slice(retain);
    let status = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            CONTEXT,
            bytes.as_mut_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
        .0
    };
    (status == 0 && decode_context(&bytes).is_ok()).then_some(bytes)
}
pub fn finish(epoch: u64, reply: &[u8; FRAME_BYTES], errno: u64, cwd: &str) -> bool {
    let Ok(cwd) = super::retention::transport::encode_cwd(cwd) else {
        return false;
    };
    let mut bytes = [0; FINISH_BYTES];
    bytes[..FRAME_BYTES].copy_from_slice(reply);
    bytes[FRAME_BYTES..FRAME_BYTES + 8].copy_from_slice(&errno.to_le_bytes());
    bytes[FRAME_BYTES + 8..].copy_from_slice(&cwd);
    unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            FINISH,
            bytes.as_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
        .0 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copied_context_has_exact_scope_path_and_envelope() {
        let scope = Scope {
            caller: super::super::lifecycle::Caller {
                pid: 2,
                incarnation: 3,
                address_space_generation: 4,
            },
            namespace_id: 5,
            namespace_generation: 6,
        };
        for kind in [Kind::Directory, Kind::Root] {
            let encoded = encode_transition(scope, 10, 11, "/narrow", kind).unwrap();
            assert_eq!(decode_context(&encoded).unwrap().kind, kind);
        }
        let mut bad_kind = encode_context(scope, 0, 0, ".").unwrap();
        bad_kind[64..72].copy_from_slice(&2u64.to_le_bytes());
        assert!(decode_context(&bad_kind).is_err());
        for path in ["relative/..", "/", "../mounted"] {
            let encoded = encode_context(scope, 10, 11, path).unwrap();
            let context = decode_context(&encoded).unwrap();
            assert_eq!(context.old, scope);
            assert_eq!((context.uid, context.gid, context.path), (10, 11, path));
        }
        for path in ["", "a\0b"] {
            assert!(encode_context(scope, 0, 0, path).is_err());
        }
        let bytes = encode_context(scope, 0, 0, ".").unwrap();
        for offset in [0, 8, 16, 24, 32, 40, 56] {
            let mut bad = bytes;
            bad[offset..offset + 8].fill(0);
            assert!(decode_context(&bad).is_err());
        }
        let mut bad = bytes;
        bad[HEADER_BYTES + 2] = 1;
        assert!(decode_context(&bad).is_err());
        bad = bytes;
        bad[56..64].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode_context(&bad).is_err());
        use super::super::retention::transport::copy_length;
        for (op, length) in [(CONTEXT, CONTEXT_BYTES), (FINISH, FINISH_BYTES)] {
            assert_eq!(copy_length(op, 1, length as u64, [0; 3]), Some(length));
            assert_eq!(copy_length(op, 0, length as u64, [0; 3]), None);
            assert_eq!(copy_length(op, 1, length as u64 + 1, [0; 3]), None);
            assert_eq!(copy_length(op, 1, length as u64, [1, 0, 0]), None);
        }
    }
}
