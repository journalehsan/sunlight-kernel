//! Copied regular-file operation channel. Successful syscall copies establish
//! kernel origin; ordinary IPC messages and decoded frames never do. Every
//! frame is fixed-size and carries the full registration and caller identity so
//! the service can correlate it independently of the payload.
use super::{
    lifecycle::Caller,
    registration::{EndpointOwner, Registration},
    ProtocolError, VERSION,
};

pub const OPEN: u64 = 0;
pub const READ: u64 = 1;
pub const WRITE: u64 = 2;
pub const SEEK: u64 = 3;
pub const STAT: u64 = 4;
pub const TRUNCATE: u64 = 5;
pub const CLOSE: u64 = 6;
pub const DUP: u64 = 7;
/// Kernel-origin ownership transfer for a surviving non-CLOEXEC descriptor.
/// The request caller is the old owner; the payload names the new owner.
pub const TRANSFER: u64 = 8;
/// Bounded positional read on the retained description. `arg0` is the explicit
/// file offset and `arg1` the length; the shared cursor is never consulted or
/// advanced. Kernel origin is required exactly as for every other operation,
/// so this codec entry alone grants no routing authority.
pub const PREAD: u64 = 9;
pub const PREPARE_STORAGE: u64 = 12;
pub const ABORT_STORAGE: u64 = 13;
pub const PUBLISH_STORAGE: u64 = 14;
pub const INVENTORY_STORAGE: u64 = 15;
pub const UNMOUNT_STORAGE: u64 = 16;
pub const MOUNTINFO: u64 = 17;
pub const STATFS: u64 = 18;

pub fn mount_observation(op: u64) -> bool {
    matches!(op, MOUNTINFO | STATFS)
}

/// Kernel pull/reply operations on the copied retention syscall.
pub const NEXT: u64 = 10;
pub const REPLY: u64 = 11;
pub const EMPTY: u64 = 1;

/// Operations that name a service-owned description. `PREAD` sits outside the
/// historical contiguous range, so both decoders use this predicate instead of
/// the range alone.
fn is_file_op(op: u64) -> bool {
    (OPEN..=TRANSFER).contains(&op)
        || matches!(op, PREAD | PREPARE_STORAGE | ABORT_STORAGE | PUBLISH_STORAGE
            | INVENTORY_STORAGE | UNMOUNT_STORAGE) || mount_observation(op)
}

pub const HEADER_WORDS: usize = 22;
pub const HEADER_BYTES: usize = HEADER_WORDS * 8;
pub const IO_BYTES: usize = 4096;
pub const FRAME_BYTES: usize = HEADER_BYTES + IO_BYTES;

pub const ACCESS_READ: u64 = 1;
pub const ACCESS_WRITE: u64 = 2;

pub const SEEK_START: u64 = 0;
pub const SEEK_CURRENT: u64 = 1;
pub const SEEK_END: u64 = 2;

/// Native 24-byte fstat record. This is the same layout returned by the
/// existing native `fstat` syscall, so a copied reply needs no ABI change.
pub const STAT_BYTES: usize = 24;
pub const TRANSFER_BYTES: usize = 24;

/// Structural request view. Authentication remains a transport prerequisite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub registration: Registration,
    pub caller: Caller,
    pub request_id: u64,
    pub op: u64,
    pub token: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub length: usize,
    pub namespace_id: u64,
    pub namespace_generation: u64,
    pub receipt_id: u64,
    pub receipt_generation: u64,
    pub uid: u32,
    pub gid: u32,
    pub payload: &'a [u8],
}

impl<'a> Request<'a> {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], ProtocolError> {
        validate(self)?;
        let owner = self.registration.owner;
        let mut bytes = [0; FRAME_BYTES];
        let words = [
            VERSION,
            self.op,
            owner.endpoint,
            owner.pid,
            owner.incarnation,
            owner.address_space_generation,
            self.registration.epoch,
            self.caller.pid,
            self.caller.incarnation,
            self.caller.address_space_generation,
            self.request_id,
            self.token,
            self.arg0,
            self.arg1,
            self.length as u64,
            0,
            self.namespace_id,
            self.namespace_generation,
            self.receipt_id,
            self.receipt_generation,
            u64::from(self.uid),
            u64::from(self.gid),
        ];
        for (word, chunk) in words.iter().zip(bytes[..HEADER_BYTES].chunks_exact_mut(8)) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        bytes[HEADER_BYTES..HEADER_BYTES + self.length].copy_from_slice(self.payload);
        Ok(bytes)
    }

    pub fn decode(bytes: &'a [u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != FRAME_BYTES {
            return Err(ProtocolError::Encoding);
        }
        let mut w = [0; HEADER_WORDS];
        for (word, chunk) in w.iter_mut().zip(bytes[..HEADER_BYTES].chunks_exact(8)) {
            *word = u64::from_le_bytes(chunk.try_into().unwrap());
        }
        if w[0] != VERSION || !is_file_op(w[1]) {
            return Err(ProtocolError::Operation);
        }
        if w[3..11].contains(&0) || w[15] != 0 {
            return Err(ProtocolError::Encoding);
        }
        let length = usize::try_from(w[14]).map_err(|_| ProtocolError::Bounds)?;
        if length > IO_BYTES || bytes[HEADER_BYTES + length..].iter().any(|byte| *byte != 0) {
            return Err(ProtocolError::Bounds);
        }
        let request = Self {
            registration: Registration {
                owner: EndpointOwner {
                    endpoint: w[2],
                    pid: w[3],
                    incarnation: w[4],
                    address_space_generation: w[5],
                },
                epoch: w[6],
            },
            caller: Caller {
                pid: w[7],
                incarnation: w[8],
                address_space_generation: w[9],
            },
            request_id: w[10],
            op: w[1],
            token: w[11],
            arg0: w[12],
            arg1: w[13],
            length,
            namespace_id: w[16],
            namespace_generation: w[17],
            receipt_id: w[18],
            receipt_generation: w[19],
            uid: u32::try_from(w[20]).map_err(|_| ProtocolError::Encoding)?,
            gid: u32::try_from(w[21]).map_err(|_| ProtocolError::Encoding)?,
            payload: &bytes[HEADER_BYTES..HEADER_BYTES + length],
        };
        validate(&request)?;
        Ok(request)
    }
}

fn validate(request: &Request<'_>) -> Result<(), ProtocolError> {
    if request.request_id == 0 || request.length > IO_BYTES {
        return Err(ProtocolError::Bounds);
    }
    match request.op {
        UNMOUNT_STORAGE | INVENTORY_STORAGE => {
            if request.arg0 != 0 || request.arg1 != 0 || request.length != 0
                || request.namespace_id != 0 || request.namespace_generation != 0
                || request.receipt_id != 0 || request.receipt_generation != 0
                || (request.op == UNMOUNT_STORAGE
                    && request.token & crate::storage::SERVICE_MOUNT_BIT == 0)
            {
                return Err(ProtocolError::Encoding);
            }
        }
        ABORT_STORAGE | PUBLISH_STORAGE => {
            if request.token == 0 || request.arg0 != 0 || request.arg1 != 0
                || request.length != 0 || request.namespace_id != 0
                || request.namespace_generation != 0 || request.receipt_id != 0
                || request.receipt_generation != 0 || request.uid != 0 || request.gid != 0
            {
                return Err(ProtocolError::Encoding);
            }
        }
        PREPARE_STORAGE => {
            let path = core::str::from_utf8(request.payload).map_err(|_| ProtocolError::Path)?;
            if request.token == 0 || request.arg0 != 0 || request.arg1 != 0
                || path.len() != request.length || path.len() >= crate::storage::TARGET_MAX
                || !path.starts_with('/') || path.as_bytes().contains(&0)
                || request.namespace_id != 0 || request.namespace_generation != 0
                || request.receipt_id != 0 || request.receipt_generation != 0
                || request.uid != 0 || request.gid != 0
            {
                return Err(ProtocolError::Encoding);
            }
        }
        MOUNTINFO | STATFS => {
            let path = core::str::from_utf8(request.payload).map_err(|_| ProtocolError::Path)?;
            if request.token != 0 || request.arg0 != 0 || request.arg1 != 0
                || request.namespace_id == 0 || request.namespace_generation == 0
                || request.receipt_id == 0 || request.receipt_generation == 0
                || path.as_bytes().contains(&0)
                || (request.op == MOUNTINFO) != path.is_empty()
            {
                return Err(ProtocolError::Encoding);
            }
        }
        OPEN => {
            if request.token != 0
                || request.arg0 & !(ACCESS_READ | ACCESS_WRITE) != 0
                || request.arg0 == 0
                || request.arg1 != 0
                || request.namespace_id == 0
                || request.namespace_generation == 0
                || request.receipt_id == 0
                || request.receipt_generation == 0
            {
                return Err(ProtocolError::Encoding);
            }
            let path = core::str::from_utf8(request.payload).map_err(|_| ProtocolError::Path)?;
            if path.is_empty() || path.len() > super::MAX_PATH_BYTES || path.as_bytes().contains(&0)
            {
                return Err(ProtocolError::Path);
            }
        }
        READ => {
            if request.token == 0
                || request.arg0 != 0
                || request.arg1 == 0
                || request.arg1 > IO_BYTES as u64
                || request.length != 0
                || request.namespace_id != 0
                || request.namespace_generation != 0
                || request.receipt_id != 0
                || request.receipt_generation != 0
                || request.uid != 0
                || request.gid != 0
            {
                return Err(ProtocolError::Encoding);
            }
        }
        PREAD => {
            // `arg0` is an arbitrary explicit offset; the service, not the
            // codec, decides whether offset plus length is representable.
            if request.token == 0
                || request.arg1 == 0
                || request.arg1 > IO_BYTES as u64
                || request.length != 0
                || request.namespace_id != 0
                || request.namespace_generation != 0
                || request.receipt_id != 0
                || request.receipt_generation != 0
                || request.uid != 0
                || request.gid != 0
            {
                return Err(ProtocolError::Encoding);
            }
        }
        WRITE => {
            if request.token == 0
                || request.arg0 != 0
                || request.arg1 != 0
                || request.length == 0
                || request.namespace_id != 0
                || request.namespace_generation != 0
                || request.receipt_id != 0
                || request.receipt_generation != 0
                || request.uid != 0
                || request.gid != 0
            {
                return Err(ProtocolError::Encoding);
            }
        }
        SEEK => {
            if request.token == 0
                || request.arg0 > SEEK_END
                || request.length != 0
                || request.namespace_id != 0
                || request.namespace_generation != 0
                || request.receipt_id != 0
                || request.receipt_generation != 0
                || request.uid != 0
                || request.gid != 0
            {
                return Err(ProtocolError::Encoding);
            }
        }
        STAT | TRUNCATE | CLOSE | DUP => {
            if request.token == 0
                || request.arg0 != 0
                || request.arg1 != 0
                || request.length != 0
                || request.namespace_id != 0
                || request.namespace_generation != 0
                || request.receipt_id != 0
                || request.receipt_generation != 0
                || request.uid != 0
                || request.gid != 0
            {
                return Err(ProtocolError::Encoding);
            }
        }
        TRANSFER => {
            if request.token == 0
                || request.arg0 != 0
                || request.arg1 != 0
                || request.length != TRANSFER_BYTES
                || request.namespace_id != 0
                || request.namespace_generation != 0
                || request.receipt_id != 0
                || request.receipt_generation != 0
                || request.uid != 0
                || request.gid != 0
            {
                return Err(ProtocolError::Encoding);
            }
            decode_caller(request.payload)?;
        }
        _ => return Err(ProtocolError::Operation),
    }
    Ok(())
}

/// Encode the exact new owner of a transferred description. The transfer is
/// only meaningful for the same live process (exec) or the same address space
/// (thread/fork); the service rechecks that relation before rekeying.
pub fn encode_caller(caller: Caller) -> [u8; TRANSFER_BYTES] {
    let mut bytes = [0; TRANSFER_BYTES];
    bytes[..8].copy_from_slice(&caller.pid.to_le_bytes());
    bytes[8..16].copy_from_slice(&caller.incarnation.to_le_bytes());
    bytes[16..].copy_from_slice(&caller.address_space_generation.to_le_bytes());
    bytes
}

pub fn decode_caller(bytes: &[u8]) -> Result<Caller, ProtocolError> {
    if bytes.len() != TRANSFER_BYTES {
        return Err(ProtocolError::Encoding);
    }
    let caller = Caller {
        pid: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
        incarnation: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        address_space_generation: u64::from_le_bytes(bytes[16..].try_into().unwrap()),
    };
    if caller.pid == 0 || caller.incarnation == 0 || caller.address_space_generation == 0 {
        return Err(ProtocolError::Encoding);
    }
    Ok(caller)
}

/// Structural reply view. The caller must compare it against the exact
/// authenticated request before accepting any value or payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply<'a> {
    pub registration: Registration,
    pub caller: Caller,
    pub request_id: u64,
    pub op: u64,
    pub errno: u64,
    pub result0: u64,
    pub result1: u64,
    pub length: usize,
    pub payload: &'a [u8],
}

impl<'a> Reply<'a> {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], ProtocolError> {
        validate_reply(self)?;
        let owner = self.registration.owner;
        let mut bytes = [0; FRAME_BYTES];
        let words = [
            VERSION,
            self.op,
            owner.endpoint,
            owner.pid,
            owner.incarnation,
            owner.address_space_generation,
            self.registration.epoch,
            self.caller.pid,
            self.caller.incarnation,
            self.caller.address_space_generation,
            self.request_id,
            self.errno,
            self.result0,
            self.result1,
            self.length as u64,
            0,
            0,
            0,
            0,
            0,
        ];
        for (word, chunk) in words.iter().zip(bytes[..HEADER_BYTES].chunks_exact_mut(8)) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        bytes[HEADER_BYTES..HEADER_BYTES + self.length].copy_from_slice(self.payload);
        Ok(bytes)
    }

    pub fn decode(bytes: &'a [u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != FRAME_BYTES {
            return Err(ProtocolError::Encoding);
        }
        let mut w = [0; HEADER_WORDS];
        for (word, chunk) in w.iter_mut().zip(bytes[..HEADER_BYTES].chunks_exact(8)) {
            *word = u64::from_le_bytes(chunk.try_into().unwrap());
        }
        if w[0] != VERSION || !is_file_op(w[1]) {
            return Err(ProtocolError::Operation);
        }
        if w[3..11].contains(&0) || w[15..22].iter().any(|word| *word != 0) {
            return Err(ProtocolError::Encoding);
        }
        let length = usize::try_from(w[14]).map_err(|_| ProtocolError::Bounds)?;
        if length > IO_BYTES || bytes[HEADER_BYTES + length..].iter().any(|byte| *byte != 0) {
            return Err(ProtocolError::Bounds);
        }
        let reply = Self {
            registration: Registration {
                owner: EndpointOwner {
                    endpoint: w[2],
                    pid: w[3],
                    incarnation: w[4],
                    address_space_generation: w[5],
                },
                epoch: w[6],
            },
            caller: Caller {
                pid: w[7],
                incarnation: w[8],
                address_space_generation: w[9],
            },
            request_id: w[10],
            op: w[1],
            errno: w[11],
            result0: w[12],
            result1: w[13],
            length,
            payload: &bytes[HEADER_BYTES..HEADER_BYTES + length],
        };
        validate_reply(&reply)?;
        Ok(reply)
    }
}

fn validate_reply(reply: &Reply<'_>) -> Result<(), ProtocolError> {
    if reply.request_id == 0 || reply.length > IO_BYTES || reply.errno > 4095 {
        return Err(ProtocolError::Status);
    }
    if reply.errno != 0 {
        if reply.length != 0 || reply.result0 != 0 || reply.result1 != 0 {
            return Err(ProtocolError::Status);
        }
        return Ok(());
    }
    match reply.op {
        MOUNTINFO if reply.result0 != 0 && reply.result1 == 0
            && reply.length != 0 && core::str::from_utf8(reply.payload).is_ok() => {}
        STATFS if reply.result0 != 0 && reply.length == 12
            && crate::storage::local_mount_id(reply.registration.epoch, reply.result1).is_some() => {}
        MOUNTINFO | STATFS => return Err(ProtocolError::Encoding),
        PUBLISH_STORAGE => return Err(ProtocolError::Status),
        INVENTORY_STORAGE => {
            match (reply.result0, reply.length) {
                (0, 0) if reply.result1 <= 8 => {}
                (1, crate::storage::MOUNT_RECORD_SIZE) if reply.result1 == 0 => {}
                _ => return Err(ProtocolError::Encoding),
            }
        }
        ABORT_STORAGE | UNMOUNT_STORAGE => {
            if reply.result0 != 0 || reply.result1 != 0 || reply.length != 0 {
                return Err(ProtocolError::Encoding);
            }
        }
        PREPARE_STORAGE => {
            if reply.result0 == 0 || reply.result1 != reply.request_id || reply.length != 0 {
                return Err(ProtocolError::Encoding);
            }
        }
        OPEN | DUP | TRANSFER => {
            if reply.result0 == 0 || reply.result1 != 0 || reply.length != 0 {
                return Err(ProtocolError::Encoding);
            }
        }
        READ | PREAD => {
            if reply.result0 as usize != reply.length
                || reply.result1 != 0
                || reply.length > IO_BYTES
            {
                return Err(ProtocolError::Encoding);
            }
        }
        WRITE => {
            if reply.result0 > IO_BYTES as u64 || reply.result1 != 0 || reply.length != 0 {
                return Err(ProtocolError::Encoding);
            }
        }
        SEEK => {
            if reply.result0 > i64::MAX as u64 || reply.result1 != 0 || reply.length != 0 {
                return Err(ProtocolError::Encoding);
            }
        }
        STAT => {
            if reply.result0 != 0 || reply.result1 != 0 || reply.length != STAT_BYTES {
                return Err(ProtocolError::Encoding);
            }
        }
        TRUNCATE | CLOSE => {
            if reply.result0 != 0 || reply.result1 != 0 || reply.length != 0 {
                return Err(ProtocolError::Encoding);
            }
        }
        _ => return Err(ProtocolError::Operation),
    }
    Ok(())
}

/// Encode the existing native fstat record from primitive fields. Keeping this
/// in the codec avoids a filesystem dependency in the transport crate.
pub fn encode_native_stat(
    size: u64,
    uid: u32,
    gid: u32,
    mode: u16,
    kind: u8,
    links: u32,
) -> [u8; STAT_BYTES] {
    let mut record = [0; STAT_BYTES];
    record[..8].copy_from_slice(&size.to_le_bytes());
    record[8..12].copy_from_slice(&uid.to_le_bytes());
    record[12..16].copy_from_slice(&gid.to_le_bytes());
    let type_bits: u16 = match kind {
        1 => 0o100000,
        _ => 0o040000,
    };
    record[16..18].copy_from_slice(&(mode | type_bits).to_le_bytes());
    record[18] = kind;
    record[20..24].copy_from_slice(&links.to_le_bytes());
    record
}

/// Exact copied-channel envelope. The epoch selects one independently owned
/// registration; no user-selected length or reserved argument is accepted.
pub fn copy_length(operation: u64, epoch: u64, length: u64, reserved: [u64; 3]) -> Option<usize> {
    let expected = match operation {
        NEXT | REPLY if epoch != 0 => FRAME_BYTES,
        _ => return None,
    };
    (length == expected as u64 && reserved == [0; 3]).then_some(expected)
}

/// Pull the oldest undelivered request for one registration. Repeating the
/// call returns the same frame until its exact reply is accepted, allowing the
/// service to revalidate before publication.
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

    fn registration() -> Registration {
        Registration {
            owner: EndpointOwner {
                endpoint: 0,
                pid: 2,
                incarnation: 3,
                address_space_generation: 4,
            },
            epoch: 5,
        }
    }

    fn caller() -> Caller {
        Caller {
            pid: 6,
            incarnation: 7,
            address_space_generation: 8,
        }
    }

    fn open_request() -> Request<'static> {
        Request {
            registration: registration(),
            caller: caller(),
            request_id: 9,
            op: OPEN,
            token: 0,
            arg0: ACCESS_READ,
            arg1: 0,
            length: 4,
            namespace_id: 10,
            namespace_generation: 11,
            receipt_id: 12,
            receipt_generation: 13,
            uid: 0,
            gid: 0,
            payload: b"/etc",
        }
    }

    #[test]
    fn storage_prepare_requires_an_exact_reservation_and_lease_receipt() {
        let request = Request {
            op: PREPARE_STORAGE,
            token: 17,
            arg0: 0,
            namespace_id: 0,
            namespace_generation: 0,
            receipt_id: 0,
            receipt_generation: 0,
            payload: b"/mnt",
            ..open_request()
        };
        assert_eq!(Request::decode(&request.encode().unwrap()), Ok(request));
        assert!(Request { token: 0, ..request }.encode().is_err());
        assert!(Request { payload: b"mnt/", ..request }.encode().is_err());
        let reply = Reply {
            registration: request.registration,
            caller: request.caller,
            request_id: request.request_id,
            op: request.op,
            errno: 0,
            result0: 21,
            result1: request.request_id,
            length: 0,
            payload: &[],
        };
        assert_eq!(Reply::decode(&reply.encode().unwrap()), Ok(reply));
        assert!(Reply { result1: request.request_id + 1, ..reply }.encode().is_err());
        assert!(Reply { result0: 0, ..reply }.encode().is_err());
        assert!(Reply { errno: 5, ..reply }.encode().is_err());
    }

    #[test]
    fn storage_lifecycle_requests_carry_only_the_preparation_cookie() {
        let request = Request {
            op: ABORT_STORAGE,
            token: 8,
            arg0: 0,
            length: 0,
            namespace_id: 0,
            namespace_generation: 0,
            receipt_id: 0,
            receipt_generation: 0,
            payload: &[],
            ..open_request()
        };
        assert_eq!(Request::decode(&request.encode().unwrap()), Ok(request));
        assert!(Request { token: 0, ..request }.encode().is_err());
        assert!(Request { namespace_id: 1, ..request }.encode().is_err());
        let publish = Request { op: PUBLISH_STORAGE, ..request };
        assert_eq!(Request::decode(&publish.encode().unwrap()), Ok(publish));
        assert!(Request { token: 0, ..publish }.encode().is_err());
        assert!(Request { arg1: 1, ..publish }.encode().is_err());
        let refusal = Reply {
            registration: publish.registration,
            request_id: publish.request_id,
            caller: publish.caller,
            op: publish.op,
            errno: 95,
            result0: 0,
            result1: 0,
            length: 0,
            payload: &[],
        };
        assert_eq!(Reply::decode(&refusal.encode().unwrap()), Ok(refusal));
        assert!(Reply { errno: 0, ..refusal }.encode().is_err());
    }

    #[test]
    fn open_round_trip_and_exact_identity() {
        let request = open_request();
        let bytes = request.encode().unwrap();
        assert_eq!(Request::decode(&bytes), Ok(request));
        for word in [3, 4, 5, 6, 7, 8, 9, 10, 16, 17, 18, 19] {
            let mut altered = bytes;
            altered[word * 8..word * 8 + 8].fill(0);
            assert!(Request::decode(&altered).is_err());
        }
        let mut altered = bytes;
        altered[15 * 8] = 1;
        assert!(Request::decode(&altered).is_err());
        assert!(Request::decode(&bytes[..FRAME_BYTES - 1]).is_err());
    }

    #[test]
    fn io_requests_validate_payload_and_reserved_words() {
        let read = Request {
            request_id: 1,
            op: READ,
            token: 2,
            arg0: 0,
            arg1: 16,
            length: 0,
            namespace_id: 0,
            namespace_generation: 0,
            receipt_id: 0,
            receipt_generation: 0,
            payload: &[],
            ..open_request()
        };
        let bytes = read.encode().unwrap();
        assert_eq!(Request::decode(&bytes), Ok(read));
        let mut write = read;
        write.op = WRITE;
        write.arg1 = 0;
        write.length = 3;
        write.payload = b"abc";
        let bytes = write.encode().unwrap();
        assert_eq!(Request::decode(&bytes).unwrap().payload, b"abc");
        let mut invalid = write;
        invalid.namespace_id = 1;
        assert!(invalid.encode().is_err());
        let mut invalid = write;
        invalid.length = IO_BYTES + 1;
        assert!(invalid.encode().is_err());
        let mut invalid = write;
        invalid.token = 0;
        assert!(invalid.encode().is_err());
    }

    #[test]
    fn replies_correlate_identity_and_result_shape() {
        let request = open_request();
        let reply = Reply {
            registration: request.registration,
            caller: request.caller,
            request_id: request.request_id,
            op: OPEN,
            errno: 0,
            result0: 21,
            result1: 0,
            length: 0,
            payload: &[],
        };
        let bytes = reply.encode().unwrap();
        assert_eq!(Reply::decode(&bytes), Ok(reply));
        let mut read = reply;
        read.op = READ;
        read.result0 = 2;
        read.length = 2;
        read.payload = b"ab";
        let bytes = read.encode().unwrap();
        assert_eq!(Reply::decode(&bytes).unwrap().payload, b"ab");
        let mut invalid = read;
        invalid.result0 = 3;
        assert!(invalid.encode().is_err());
        let mut stat = reply;
        stat.op = STAT;
        stat.result0 = 0;
        stat.length = STAT_BYTES;
        stat.payload = &[0; STAT_BYTES];
        assert!(stat.encode().is_ok());
        let mut failed = reply;
        failed.errno = 2;
        failed.result0 = 0;
        assert!(failed.encode().is_ok());
        failed.length = 1;
        failed.payload = b"x";
        assert!(failed.encode().is_err());
    }

    #[test]
    fn native_stat_record_matches_existing_layout() {
        let record = encode_native_stat(6, 71, 72, 0o600, 1, 1);
        assert_eq!(&record[..8], &6u64.to_le_bytes());
        assert_eq!(&record[8..12], &71u32.to_le_bytes());
        assert_eq!(&record[12..16], &72u32.to_le_bytes());
        assert_eq!(&record[16..18], &0o100600u16.to_le_bytes());
        assert_eq!(record[18], 1);
        assert_eq!(&record[20..24], &1u32.to_le_bytes());
    }

    #[test]
    fn transfer_round_trip_requires_exact_owner_payload() {
        let new_owner = Caller {
            pid: 6,
            incarnation: 7,
            address_space_generation: 18,
        };
        let payload = encode_caller(new_owner);
        let request = Request {
            request_id: 1,
            op: TRANSFER,
            token: 2,
            arg0: 0,
            arg1: 0,
            length: TRANSFER_BYTES,
            namespace_id: 0,
            namespace_generation: 0,
            receipt_id: 0,
            receipt_generation: 0,
            uid: 0,
            gid: 0,
            payload: &payload,
            ..open_request()
        };
        let bytes = request.encode().unwrap();
        let decoded = Request::decode(&bytes).unwrap();
        assert_eq!(decoded.op, TRANSFER);
        assert_eq!(decode_caller(decoded.payload), Ok(new_owner));

        let mut invalid = request;
        invalid.length = 0;
        assert!(invalid.encode().is_err());
        let mut invalid = request;
        invalid.arg0 = 1;
        assert!(invalid.encode().is_err());
        let mut invalid = request;
        invalid.uid = 1;
        assert!(invalid.encode().is_err());
        let mut zero_owner = payload;
        zero_owner[..8].fill(0);
        let mut invalid = request;
        invalid.payload = &zero_owner;
        assert!(invalid.encode().is_err());

        let reply = Reply {
            registration: request.registration,
            caller: request.caller,
            request_id: request.request_id,
            op: TRANSFER,
            errno: 0,
            result0: 2,
            result1: 0,
            length: 0,
            payload: &[],
        };
        assert!(reply.encode().is_ok());
        let mut invalid = reply;
        invalid.result0 = 0;
        assert!(invalid.encode().is_err());
    }

    #[test]
    fn positional_read_carries_an_explicit_offset_and_no_payload() {
        let read = Request {
            request_id: 1,
            op: PREAD,
            token: 2,
            arg0: 4096,
            arg1: 16,
            length: 0,
            namespace_id: 0,
            namespace_generation: 0,
            receipt_id: 0,
            receipt_generation: 0,
            uid: 0,
            gid: 0,
            payload: &[],
            ..open_request()
        };
        let bytes = read.encode().unwrap();
        assert_eq!(Request::decode(&bytes), Ok(read));
        assert_eq!(Request::decode(&bytes).unwrap().arg0, 4096);

        // Offset zero is an ordinary offset, and the codec does not bound the
        // upper end: only the service can decide if offset plus length fits.
        let mut at_start = read;
        at_start.arg0 = 0;
        assert!(at_start.encode().is_ok());
        let mut top = read;
        top.arg0 = u64::MAX;
        assert_eq!(
            Request::decode(&top.encode().unwrap()).unwrap().arg0,
            u64::MAX
        );

        for invalid in [
            Request { arg1: 0, ..read },
            Request {
                arg1: IO_BYTES as u64 + 1,
                ..read
            },
            Request {
                token: 0,
                ..read
            },
            Request {
                namespace_id: 1,
                ..read
            },
            Request {
                receipt_id: 1,
                ..read
            },
            Request { uid: 1, ..read },
            Request { gid: 1, ..read },
        ] {
            assert!(invalid.encode().is_err());
        }
        let mut with_payload = read;
        with_payload.length = 1;
        with_payload.payload = b"x";
        assert!(with_payload.encode().is_err());

        let reply = Reply {
            registration: read.registration,
            caller: read.caller,
            request_id: read.request_id,
            op: PREAD,
            errno: 0,
            result0: 2,
            result1: 0,
            length: 2,
            payload: b"ef",
        };
        let bytes = reply.encode().unwrap();
        assert_eq!(Reply::decode(&bytes).unwrap().payload, b"ef");
        let mut short = reply;
        short.result0 = 1;
        assert!(short.encode().is_err());
        let mut eof = reply;
        eof.result0 = 0;
        eof.length = 0;
        eof.payload = &[];
        assert!(eof.encode().is_ok());
        let mut failed = eof;
        failed.errno = 22;
        assert!(failed.encode().is_ok());
    }
}
