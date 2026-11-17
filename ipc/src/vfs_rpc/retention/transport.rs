//! Service-only copied control channel. Successful syscall copies establish
//! kernel origin; ordinary IPC messages and decoded frames never do.
use super::wire::FRAME_BYTES;
use crate::vfs_rpc::{
    registration::{EndpointOwner, Registration},
    ProtocolError,
};

pub const SNAPSHOT: u64 = 0;
pub const NEXT: u64 = 1;
pub const REPLY: u64 = 2;
/// Read-only lifecycle status for an independently authenticated retained epoch.
pub const STATUS: u64 = 3;
/// Validate an exact delivered retain against independently staged root policy.
/// Success authorizes only root="/", directory="/" for this pending scope.
/// It is not authority for arbitrary paths or a future request after retirement.
pub const AUTHORIZE_ROOT: u64 = 4;
/// Return the independently staged initial cwd for an exact delivered retain.
/// Root remains `/`; this is provisioning, never per-request path substitution.
/// The buffer starts with FRAME_BYTES of retain input and has CWD_BYTES output
/// capacity. Success replaces it with u64 length, UTF-8 bytes and zero padding.
pub const INITIAL_CWD: u64 = 5;
/// Exact retained source for a kernel-staged spawn/exec handoff. The source
/// lease stays owned until the child preparation settles, even after retirement.
pub const DERIVE_SOURCE: u64 = 8;
pub const CWD_BYTES: usize = 8 + 4096;

pub fn has_input(operation: u64) -> bool {
    matches!(
        operation,
        REPLY
            | DERIVE_SOURCE
            | AUTHORIZE_ROOT
            | INITIAL_CWD
            | crate::vfs_rpc::file_transport::REPLY
            | super::super::chdir_transport::CONTEXT
            | super::super::chdir_transport::FINISH
    )
}
pub const STATUS_BYTES: usize = REGISTRATION_BYTES + 8;
pub const REGISTRATION_BYTES: usize = 40;
pub const EMPTY: u64 = 1;
/// Payload-free scheduling hint, never registration or lifecycle authority.
/// Ordinary IPC senders receive a nonzero kernel-stamped PID badge.
pub const CONTROL_WAKEUP: u64 = 0x5646_5357;

pub fn is_control_wakeup(message: &crate::IpcMsg) -> bool {
    message.label == CONTROL_WAKEUP
        && message.badge == 0
        && message.word_count == 0
        && message.cap_count == 0
}

/// Exact syscall envelope. No attachments, variable-size records, or reserved
/// argument values are accepted. The epoch selects an independently owned row.
pub fn copy_length(operation: u64, epoch: u64, length: u64, reserved: [u64; 3]) -> Option<usize> {
    let expected = match operation {
        SNAPSHOT if epoch == 0 => REGISTRATION_BYTES,
        NEXT | REPLY | AUTHORIZE_ROOT | DERIVE_SOURCE if epoch != 0 => FRAME_BYTES,
        STATUS if epoch != 0 => STATUS_BYTES,
        INITIAL_CWD if epoch != 0 => CWD_BYTES,
        super::super::chdir_transport::CONTEXT if epoch != 0 => {
            super::super::chdir_transport::CONTEXT_BYTES
        }
        super::super::chdir_transport::FINISH if epoch != 0 => {
            super::super::chdir_transport::FINISH_BYTES
        }
        crate::vfs_rpc::file_transport::NEXT | crate::vfs_rpc::file_transport::REPLY
            if epoch != 0 =>
        {
            crate::vfs_rpc::file_transport::FRAME_BYTES
        }
        _ => return None,
    };
    (length == expected as u64 && reserved == [0; 3]).then_some(expected)
}

pub fn encode_cwd(path: &str) -> Result<[u8; CWD_BYTES], ProtocolError> {
    if !path.starts_with('/') || path.len() > 4095 || path.as_bytes().contains(&0) {
        return Err(ProtocolError::Encoding);
    }
    let mut bytes = [0; CWD_BYTES];
    bytes[..8].copy_from_slice(&(path.len() as u64).to_le_bytes());
    bytes[8..8 + path.len()].copy_from_slice(path.as_bytes());
    Ok(bytes)
}

pub fn decode_cwd(bytes: &[u8; CWD_BYTES]) -> Result<&str, ProtocolError> {
    let len = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        .map_err(|_| ProtocolError::Encoding)?;
    if len == 0 || len > 4095 || bytes[8 + len..].iter().any(|b| *b != 0) {
        return Err(ProtocolError::Encoding);
    }
    let path = core::str::from_utf8(&bytes[8..8 + len]).map_err(|_| ProtocolError::Encoding)?;
    encode_cwd(path)?;
    Ok(path)
}

pub fn derive_source(epoch: u64, retain: &[u8; FRAME_BYTES]) -> Option<super::Receipt> {
    let mut bytes = *retain;
    let (status, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            DERIVE_SOURCE,
            bytes.as_mut_ptr() as u64,
            FRAME_BYTES as u64,
            epoch,
            0,
            0,
            0,
        )
    };
    match super::wire::Frame::decode(&bytes).ok()? {
        super::wire::Frame::Retained(receipt)
            if status == 0 && receipt.registration().epoch == epoch =>
        {
            Some(receipt)
        }
        _ => None,
    }
}

pub fn initial_cwd(epoch: u64, retain: &[u8; FRAME_BYTES]) -> Option<[u8; CWD_BYTES]> {
    let mut bytes = [0; CWD_BYTES];
    bytes[..FRAME_BYTES].copy_from_slice(retain);
    let (status, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            INITIAL_CWD,
            bytes.as_mut_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
    };
    (status == 0 && decode_cwd(&bytes).is_ok()).then_some(bytes)
}

pub fn encode_registration(registration: Registration) -> [u8; REGISTRATION_BYTES] {
    let owner = registration.owner;
    let mut bytes = [0; REGISTRATION_BYTES];
    for (chunk, word) in bytes.chunks_exact_mut(8).zip([
        owner.endpoint,
        owner.pid,
        owner.incarnation,
        owner.address_space_generation,
        registration.epoch,
    ]) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

pub fn decode_registration(bytes: &[u8]) -> Result<Registration, ProtocolError> {
    if bytes.len() != REGISTRATION_BYTES {
        return Err(ProtocolError::Encoding);
    }
    let mut words = [0; 5];
    for (word, chunk) in words.iter_mut().zip(bytes.chunks_exact(8)) {
        *word = u64::from_le_bytes(chunk.try_into().unwrap());
    }
    let registration = Registration {
        owner: EndpointOwner {
            endpoint: words[0],
            pid: words[1],
            incarnation: words[2],
            address_space_generation: words[3],
        },
        epoch: words[4],
    };
    super::Table::<(), 0>::new(registration).map_err(|_| ProtocolError::Encoding)?;
    Ok(registration)
}

/// Exact status envelope: registration followed by a canonical retirement bit.
/// A failed lookup/copy supplies no lifecycle evidence.
pub fn decode_status(bytes: &[u8]) -> Result<(Registration, bool), ProtocolError> {
    if bytes.len() != STATUS_BYTES {
        return Err(ProtocolError::Encoding);
    }
    let registration = decode_registration(&bytes[..REGISTRATION_BYTES])?;
    let retired = u64::from_le_bytes(bytes[REGISTRATION_BYTES..].try_into().unwrap());
    match retired {
        0 | 1 => Ok((registration, retired == 1)),
        _ => Err(ProtocolError::Encoding),
    }
}

pub fn status(epoch: u64) -> Result<(Registration, bool), ()> {
    let mut bytes = [0; STATUS_BYTES];
    let (result, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            STATUS,
            bytes.as_mut_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
    };
    if result != 0 {
        return Err(());
    }
    let status = decode_status(&bytes).map_err(|_| ())?;
    if status.0.epoch != epoch {
        return Err(());
    }
    Ok(status)
}

/// Snapshot is read-only and accepts no claimed owner or endpoint. A subsequent
/// call supplies only the epoch; the kernel resolves its owner independently.
pub fn snapshot() -> Option<Registration> {
    let mut bytes = [0; REGISTRATION_BYTES];
    let (status, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            SNAPSHOT,
            bytes.as_mut_ptr() as u64,
            bytes.len() as u64,
            0,
            0,
            0,
            0,
        )
    };
    (status == 0)
        .then(|| decode_registration(&bytes).ok())
        .flatten()
}

/// No retain retransmission: a failed user copy can leave an unknown remote
/// obligation. Releases remain retryable. Only successful copies are returned.
pub fn next(epoch: u64) -> Result<Option<[u8; FRAME_BYTES]>, ()> {
    let mut bytes = [0; FRAME_BYTES];
    let (status, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            NEXT,
            bytes.as_mut_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
    };
    match status {
        0 => Ok(Some(bytes)),
        EMPTY => Ok(None),
        _ => Err(()),
    }
}

/// Check before acquiring pins and again before retaining them. No claimed path,
/// caller, credentials, or object IDs are accepted as policy by this syscall.
pub fn authorize_root(epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool {
    let (status, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            AUTHORIZE_ROOT,
            bytes.as_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
    };
    status == 0
}

pub fn reply(epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool {
    let (status, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsRetention,
            REPLY,
            bytes.as_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
    };
    status == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_cwd_has_exact_bounded_encoding_and_envelope() {
        for path in ["/", "/etc", "/a/b"] {
            assert_eq!(decode_cwd(&encode_cwd(path).unwrap()), Ok(path));
        }
        for path in ["", "relative", "/a\0b"] {
            assert!(encode_cwd(path).is_err());
        }
        let longest = [b'/'; 4096];
        assert!(encode_cwd(core::str::from_utf8(&longest[..4095]).unwrap()).is_ok());
        assert!(encode_cwd(core::str::from_utf8(&longest).unwrap()).is_err());
        let mut bytes = encode_cwd("/etc").unwrap();
        bytes[CWD_BYTES - 1] = 1;
        assert!(decode_cwd(&bytes).is_err());
        bytes[..8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode_cwd(&bytes).is_err());
        assert_eq!(
            copy_length(INITIAL_CWD, 1, CWD_BYTES as u64, [0; 3]),
            Some(CWD_BYTES)
        );
        assert_eq!(copy_length(INITIAL_CWD, 0, CWD_BYTES as u64, [0; 3]), None);
        assert_eq!(
            copy_length(INITIAL_CWD, 1, FRAME_BYTES as u64, [0; 3]),
            None
        );
        assert_eq!(
            copy_length(INITIAL_CWD, 1, CWD_BYTES as u64, [0, 1, 0]),
            None
        );
    }

    #[test]
    fn control_wakeup_is_a_payload_free_kernel_hint() {
        let hint = crate::IpcMsg::with_label(CONTROL_WAKEUP);
        assert!(is_control_wakeup(&hint));
        for field in 0..4 {
            let mut ordinary = hint;
            match field {
                0 => ordinary.label += 1,
                1 => ordinary.badge = 10,
                2 => ordinary.word_count = 1,
                _ => ordinary.cap_count = 1,
            }
            assert!(!is_control_wakeup(&ordinary));
        }
    }

    #[test]
    fn exact_copy_envelopes_reject_aliases_and_reserved_arguments() {
        assert_eq!(copy_length(SNAPSHOT, 0, 40, [0; 3]), Some(40));
        for operation in [NEXT, REPLY, AUTHORIZE_ROOT, DERIVE_SOURCE] {
            assert_eq!(copy_length(operation, 1, 112, [0; 3]), Some(112));
            assert_eq!(copy_length(operation, 0, 112, [0; 3]), None);
            for length in [0, 40, 111, 113, u64::MAX] {
                assert_eq!(copy_length(operation, 1, length, [0; 3]), None);
            }
            for index in 0..3 {
                let mut reserved = [0; 3];
                reserved[index] = 1;
                assert_eq!(copy_length(operation, 1, 112, reserved), None);
            }
        }
        assert_eq!(copy_length(SNAPSHOT, 1, 40, [0; 3]), None);
        assert_eq!(copy_length(6, 1, 112, [0; 3]), None);
        assert_eq!(copy_length(STATUS, 1, 48, [0; 3]), Some(48));
        assert_eq!(copy_length(STATUS, 0, 48, [0; 3]), None);
        assert_eq!(copy_length(STATUS, 1, 112, [0; 3]), None);
        assert_eq!(copy_length(STATUS, 1, 48, [0, 1, 0]), None);
    }

    #[test]
    fn status_requires_exact_identity_and_canonical_retirement_bit() {
        let registration = Registration {
            owner: EndpointOwner {
                endpoint: 0,
                pid: 1,
                incarnation: 2,
                address_space_generation: 3,
            },
            epoch: 4,
        };
        let mut bytes = [0; STATUS_BYTES];
        bytes[..REGISTRATION_BYTES].copy_from_slice(&encode_registration(registration));
        assert_eq!(decode_status(&bytes), Ok((registration, false)));
        bytes[REGISTRATION_BYTES] = 1;
        assert_eq!(decode_status(&bytes), Ok((registration, true)));
        for value in [2, 255] {
            bytes[REGISTRATION_BYTES] = value;
            assert!(decode_status(&bytes).is_err());
        }
        assert!(decode_status(&bytes[..STATUS_BYTES - 1]).is_err());
        assert!(decode_status(&[0; STATUS_BYTES + 1]).is_err());
        assert!(decode_status(&[0; STATUS_BYTES]).is_err());
    }

    #[test]
    fn registration_snapshot_is_exact_and_keeps_endpoint_zero() {
        let r = Registration {
            owner: EndpointOwner {
                endpoint: 0,
                pid: 1,
                incarnation: 2,
                address_space_generation: 3,
            },
            epoch: 4,
        };
        let bytes = encode_registration(r);
        assert_eq!(decode_registration(&bytes), Ok(r));
        for length in 0..REGISTRATION_BYTES {
            assert!(decode_registration(&bytes[..length]).is_err());
        }
        assert!(decode_registration(&[0; REGISTRATION_BYTES + 1]).is_err());
        for word in 1..5 {
            let mut invalid = bytes;
            invalid[word * 8..word * 8 + 8].fill(0);
            assert!(decode_registration(&invalid).is_err());
        }
    }
}
