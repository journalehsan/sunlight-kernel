//! Transport contract for the userspace VFS migration (not yet dispatched).
//! Four register words identify one bounded shared-memory payload. No raw user
//! address or caller-supplied credential is authority. The kernel must bind the
//! buffer capability and authenticated caller to the pending operation.

pub const VERSION: u64 = 1;
pub const MAX_TRANSFER: u64 = 64 * 1024;
pub const MAX_PATH_BYTES: usize = 4095;
pub const NEGOTIATE: u64 = 0x5646_5300;
pub const NEGOTIATE_REPLY: u64 = 0x5646_5301;
pub const REQUEST: u64 = 0x5646_5302;
pub const REPLY: u64 = 0x5646_5303;

#[path = "vfs_rpc/codec.rs"]
pub mod codec;
#[path = "vfs_rpc/context.rs"]
pub mod context;
#[path = "vfs_rpc/lifecycle.rs"]
pub mod lifecycle;
#[path = "vfs_rpc/pending_table.rs"]
pub mod pending_table;
#[path = "vfs_rpc/registration.rs"]
pub mod registration;
#[path = "vfs_rpc/response.rs"]
pub mod response;
#[path = "vfs_rpc/retention.rs"]
pub mod retention;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    Version,
    RequestId,
    Bounds,
    Status,
    ShortReply,
    Encoding,
    Operation,
    Path,
}

/// Negotiation advertises only implemented transport features. A zero feature
/// mask explicitly means that file requests must not be dispatched yet.
/// Request words: minimum version, maximum version, zero, zero; no capabilities.
/// Reply words: supported version, positive errno (zero = success), maximum
/// shared-buffer transfer bytes, feature mask. Limits/features are usable only
/// after a successful response from the intended service.
pub fn negotiate(words: [u64; 4], word_count: usize, cap_count: usize) -> [u64; 4] {
    let errno = if word_count != 4 || cap_count != 0 || words[2] != 0 || words[3] != 0 {
        22
    } else if words[0] != VERSION || words[1] < VERSION {
        93
    } else {
        0
    };
    [VERSION, errno, MAX_TRANSFER, 0]
}

/// Wire words: version, nonzero request ID, buffer offset, buffer length.
/// REQUEST labels carry operation payloads defined in `codec`; executable file
/// dispatch remains disabled until the authenticated kernel boundary exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transfer {
    request_id: u64,
    offset: u64,
    length: u64,
}

impl Transfer {
    pub fn decode(words: [u64; 4], buffer_bytes: u64) -> Result<Self, ProtocolError> {
        if words[0] != VERSION {
            return Err(ProtocolError::Version);
        }
        if words[1] == 0 {
            return Err(ProtocolError::RequestId);
        }
        let end = words[2]
            .checked_add(words[3])
            .ok_or(ProtocolError::Bounds)?;
        if words[3] > MAX_TRANSFER || end > buffer_bytes {
            return Err(ProtocolError::Bounds);
        }
        Ok(Self {
            request_id: words[1],
            offset: words[2],
            length: words[3],
        })
    }

    pub const fn words(self) -> [u64; 4] {
        [VERSION, self.request_id, self.offset, self.length]
    }

    /// Reply words: version, request ID, positive errno (zero = success),
    /// initialized output bytes. Successful short transfers are permitted.
    /// Endpoint/mount incarnation authentication is additionally required by
    /// the kernel; request IDs alone cannot authenticate a reply.
    pub fn validate_reply(self, words: [u64; 4]) -> Result<Completion, ProtocolError> {
        if words[0] != VERSION {
            return Err(ProtocolError::Version);
        }
        if words[1] != self.request_id {
            return Err(ProtocolError::RequestId);
        }
        if words[2] > 4095 || (words[2] != 0 && words[3] != 0) {
            return Err(ProtocolError::Status);
        }
        if words[3] > self.length {
            return Err(ProtocolError::Bounds);
        }
        Ok(Completion {
            errno: words[2] as u16,
            output_bytes: words[3],
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub errno: u16,
    pub output_bytes: u64,
}

/// Validate the actual register count before accepting a zero-filled array.
pub fn decode_reply(
    transfer: Transfer,
    words: [u64; 4],
    word_count: usize,
) -> Result<Completion, ProtocolError> {
    if word_count != 4 {
        return Err(ProtocolError::ShortReply);
    }
    transfer.validate_reply(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_never_advertises_unimplemented_file_dispatch() {
        assert_eq!(negotiate([1, 1, 0, 0], 4, 0), [1, 0, MAX_TRANSFER, 0]);
        assert_eq!(negotiate([1, 2, 0, 0], 4, 0), [1, 0, MAX_TRANSFER, 0]);
        for (words, count, caps, errno) in [
            ([2, 2, 0, 0], 4, 0, 93),
            ([1, 0, 0, 0], 4, 0, 93),
            ([1, 1, 1, 0], 4, 0, 22),
            ([1, 1, 0, 1], 4, 0, 22),
            ([1, 1, 0, 0], 3, 0, 22),
            ([1, 1, 0, 0], 4, 1, 22),
        ] {
            assert_eq!(negotiate(words, count, caps), [1, errno, MAX_TRANSFER, 0]);
        }
        assert_ne!(NEGOTIATE, NEGOTIATE_REPLY);
        assert_ne!(REQUEST, REPLY);
    }

    #[test]
    fn rejects_overflow_and_out_of_buffer_ranges() {
        for (offset, length, capacity) in [
            (u64::MAX, 1, u64::MAX),
            (8, 9, 16),
            (0, MAX_TRANSFER + 1, MAX_TRANSFER + 1),
        ] {
            assert_eq!(
                Transfer::decode([VERSION, 1, offset, length], capacity),
                Err(ProtocolError::Bounds)
            );
        }
        assert!(Transfer::decode([VERSION, 1, 16, 0], 16).is_ok());
    }

    #[test]
    fn rejects_wrong_version_and_zero_identity() {
        assert_eq!(
            Transfer::decode([2, 1, 0, 0], 0),
            Err(ProtocolError::Version)
        );
        assert_eq!(
            Transfer::decode([VERSION, 0, 0, 0], 0),
            Err(ProtocolError::RequestId)
        );
    }

    #[test]
    fn validates_reply_identity_count_status_and_initialized_length() {
        let transfer = Transfer::decode([VERSION, 42, 0, 16], 16).unwrap();
        assert_eq!(transfer.words(), [VERSION, 42, 0, 16]);
        assert_eq!(
            decode_reply(transfer, [VERSION, 42, 0, 8], 4),
            Ok(Completion {
                errno: 0,
                output_bytes: 8
            })
        );
        assert_eq!(
            decode_reply(transfer, [VERSION, 42, 5, 0], 4),
            Ok(Completion {
                errno: 5,
                output_bytes: 0
            })
        );
        for (words, count, error) in [
            ([VERSION, 41, 0, 8], 4, ProtocolError::RequestId),
            ([VERSION, 42, 0, 17], 4, ProtocolError::Bounds),
            ([VERSION, 42, 5, 1], 4, ProtocolError::Status),
            ([VERSION, 42, u64::MAX, 0], 4, ProtocolError::Status),
            ([VERSION, 42, 0, 0], 3, ProtocolError::ShortReply),
            ([2, 42, 0, 0], 4, ProtocolError::Version),
        ] {
            assert_eq!(decode_reply(transfer, words, count), Err(error));
        }
    }
}
