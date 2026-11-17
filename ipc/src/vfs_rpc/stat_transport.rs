//! Copied root-stat pilot. Successful service pulls establish kernel origin;
//! ordinary IPC, encoded contexts and caller-supplied IDs grant no authority.
use super::{
    codec::Request,
    context,
    response::{Metadata, Reply, ReplyContract, METADATA_BYTES},
    Completion, ProtocolError,
};

pub const ROOT_METADATA: u64 = 0;
pub const PATH_METADATA: u64 = 3;
/// Bound supported by mounted tmpfs traversal, without silent truncation.
pub const MAX_LOOKUP_PATH: usize = super::MAX_PATH_BYTES;

pub fn path_length(output_bytes: u64, path_bytes: u64, reserved: [u64; 2]) -> Option<usize> {
    (output_bytes == METADATA_BYTES as u64
        && path_bytes > 0
        && path_bytes <= MAX_LOOKUP_PATH as u64
        && reserved == [0; 2])
        .then_some(path_bytes as usize)
}
pub const NEXT: u64 = 1;
pub const COMPLETE: u64 = 2;
pub const INPUT_BYTES: usize =
    context::HEADER_BYTES + context::MAX_GROUPS * 4 + super::MAX_PATH_BYTES + 16;
pub const PACKET_BYTES: usize = 8 + INPUT_BYTES;
pub const REPLY_BYTES: usize = 32 + METADATA_BYTES;
pub const EMPTY: u64 = 1;
/// Outside the errno range. Only this result means a caller owns a parked call.
pub const PENDING: u64 = u64::MAX - 4096;
pub const NOT_READY: u64 = (-11i64) as u64;
pub const INVALID: u64 = (-22i64) as u64;
pub const IO_ERROR: u64 = (-5i64) as u64;
pub const TIMED_OUT: u64 = (-110i64) as u64;

pub fn copy_length(operation: u64, epoch: u64, length: u64, reserved: [u64; 3]) -> Option<usize> {
    let expected = match operation {
        ROOT_METADATA if epoch == 0 => METADATA_BYTES,
        NEXT if epoch != 0 => PACKET_BYTES,
        COMPLETE if epoch != 0 => REPLY_BYTES,
        _ => return None,
    };
    (length == expected as u64 && reserved == [0; 3]).then_some(expected)
}

pub fn input(packet: &[u8; PACKET_BYTES]) -> Result<&[u8], ProtocolError> {
    let length = usize::try_from(u64::from_le_bytes(packet[..8].try_into().unwrap()))
        .map_err(|_| ProtocolError::Bounds)?;
    if length == 0 || length > INPUT_BYTES || packet[8 + length..].iter().any(|b| *b != 0) {
        return Err(ProtocolError::Bounds);
    }
    Ok(&packet[8..8 + length])
}

pub fn encode_reply(
    id: u64,
    errno: u64,
    output: &[u8],
) -> Result<[u8; REPLY_BYTES], ProtocolError> {
    if id == 0 || output.len() > METADATA_BYTES {
        return Err(ProtocolError::Bounds);
    }
    ReplyContract::for_request(Request::Stat { path: "/" })?.validate(
        Completion {
            errno: u16::try_from(errno).map_err(|_| ProtocolError::Status)?,
            output_bytes: output.len() as u64,
        },
        output,
    )?;
    let mut bytes = [0; REPLY_BYTES];
    for (chunk, word) in
        bytes[..32]
            .chunks_exact_mut(8)
            .zip([super::VERSION, id, errno, output.len() as u64])
    {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes[32..32 + output.len()].copy_from_slice(output);
    Ok(bytes)
}

pub fn decode_reply(
    bytes: &[u8; REPLY_BYTES],
) -> Result<super::response::ReplyMessage<'_>, ProtocolError> {
    let mut words = [0; 4];
    for (word, chunk) in words.iter_mut().zip(bytes[..32].chunks_exact(8)) {
        *word = u64::from_le_bytes(chunk.try_into().unwrap());
    }
    let length = usize::try_from(words[3]).map_err(|_| ProtocolError::Bounds)?;
    if length > METADATA_BYTES || bytes[32 + length..].iter().any(|b| *b != 0) {
        return Err(ProtocolError::Bounds);
    }
    Ok(super::response::ReplyMessage {
        label: super::REPLY,
        words,
        word_count: 4,
        cap_count: 0,
        output: &bytes[32..32 + length],
    })
}

pub fn next(epoch: u64) -> Result<Option<[u8; PACKET_BYTES]>, ()> {
    let mut bytes = [0; PACKET_BYTES];
    let (result, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsStat,
            NEXT,
            bytes.as_mut_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
    };
    match result {
        0 => Ok(Some(bytes)),
        EMPTY => Ok(None),
        _ => Err(()),
    }
}

/// Dedicated native gate: consume work with an invalid output destination.
#[cfg(feature = "stat-failure-test")]
pub fn fault_next_copy(epoch: u64) -> u64 {
    unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsStat,
            NEXT,
            0,
            PACKET_BYTES as u64,
            epoch,
            0,
            0,
            0,
        )
        .0
    }
}

/// One publication/collection attempt for the native VM mutation gate.
///
/// # Safety
/// A successful collection writes METADATA_BYTES to `output`. The caller must
/// own any writable mapping at that address and avoid concurrent Rust access.
#[cfg(feature = "stat-failure-test")]
pub unsafe fn probe_stat_attempt(path: &str, output: u64) -> u64 {
    unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsStat,
            PATH_METADATA,
            output,
            METADATA_BYTES as u64,
            path.as_ptr() as u64,
            path.len() as u64,
            0,
            0,
        )
        .0
    }
}

pub fn complete(epoch: u64, bytes: &[u8; REPLY_BYTES]) -> bool {
    let (result, _) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::VfsStat,
            COMPLETE,
            bytes.as_ptr() as u64,
            bytes.len() as u64,
            epoch,
            0,
            0,
            0,
        )
    };
    result == 0
}

/// One logical operation across scheduler wakeups. NOT_READY owns no operation
/// and lets boot continue servicing nameserver traffic before retrying later.
pub fn root_metadata() -> Result<Metadata, u64> {
    metadata_call(None)
}

/// Metadata relative to the explicitly provisioned service root/cwd profile.
/// This does not select the process's still-kernel-owned namespace or cwd.
pub fn stat(path: &str) -> Result<Metadata, u64> {
    if path_length(METADATA_BYTES as u64, path.len() as u64, [0; 2]).is_none()
        || path.as_bytes().contains(&0)
    {
        return Err(INVALID);
    }
    metadata_call(Some(path))
}

fn metadata_call(path: Option<&str>) -> Result<Metadata, u64> {
    let mut bytes = [0; METADATA_BYTES];
    loop {
        let (result, _) = unsafe {
            crate::raw_syscall(
                crate::SunlightSyscall::VfsStat,
                if path.is_some() {
                    PATH_METADATA
                } else {
                    ROOT_METADATA
                },
                bytes.as_mut_ptr() as u64,
                bytes.len() as u64,
                path.map_or(0, |p| p.as_ptr() as u64),
                path.map_or(0, |p| p.len() as u64),
                0,
                0,
            )
        };
        if result == PENDING {
            crate::process_yield();
            continue;
        }
        if result != 0 {
            return Err(result);
        }
        return match ReplyContract::for_request(Request::Stat {
            path: path.unwrap_or("/"),
        })
        .unwrap()
        .validate(
            Completion {
                errno: 0,
                output_bytes: METADATA_BYTES as u64,
            },
            &bytes,
        ) {
            Ok(Reply::Metadata(metadata)) => Ok(metadata),
            _ => Err(IO_ERROR),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn envelopes_and_packets_are_exact() {
        assert_eq!(copy_length(ROOT_METADATA, 0, 56, [0; 3]), Some(56));
        for (op, epoch, len) in [
            (ROOT_METADATA, 0, 56),
            (NEXT, 1, PACKET_BYTES as u64),
            (COMPLETE, 1, REPLY_BYTES as u64),
        ] {
            assert!(copy_length(op, epoch, len, [0; 3]).is_some());
            assert!(copy_length(op, epoch, len + 1, [0; 3]).is_none());
            assert!(copy_length(op, epoch, len, [0, 1, 0]).is_none());
            assert!(copy_length(op, if epoch == 0 { 1 } else { 0 }, len, [0; 3]).is_none());
        }
        let mut packet = [0; PACKET_BYTES];
        assert!(input(&packet).is_err());
        packet[..8].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(input(&packet).unwrap(), &[0]);
        packet[9] = 1;
        assert!(input(&packet).is_err());
    }
    #[test]
    fn path_envelopes_reject_overflow_truncation_and_reserved_fields() {
        assert_eq!(path_length(56, 1, [0; 2]), Some(1));
        assert_eq!(
            path_length(56, MAX_LOOKUP_PATH as u64, [0; 2]),
            Some(MAX_LOOKUP_PATH)
        );
        for length in [0, MAX_LOOKUP_PATH as u64 + 1, u64::MAX] {
            assert_eq!(path_length(56, length, [0; 2]), None);
        }
        assert_eq!(path_length(55, 1, [0; 2]), None);
        assert_eq!(path_length(57, 1, [0; 2]), None);
        assert_eq!(path_length(56, 1, [1, 0]), None);
        assert_eq!(path_length(56, 1, [0, 1]), None);
        assert!(copy_length(PATH_METADATA, 1, 56, [0; 3]).is_none());
    }

    #[test]
    fn replies_preserve_id_and_bound_initialized_output() {
        let mut bytes = encode_reply(7, 2, &[]).unwrap();
        let decoded = decode_reply(&bytes).unwrap();
        assert_eq!(decoded.words, [1, 7, 2, 0]);
        bytes[32] = 1;
        assert!(decode_reply(&bytes).is_err());
        bytes[24..32].copy_from_slice(&57u64.to_le_bytes());
        assert!(decode_reply(&bytes).is_err());
        assert!(encode_reply(0, 2, &[]).is_err());
        assert!(encode_reply(7, 0, &[]).is_err());
    }
}
