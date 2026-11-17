//! Namespace dispatch context. Decoding establishes structure, never authority.
//! Only a kernel-origin transport may publish these bytes to the service.
use super::codec::{Reader, Request, Writer};
use super::lifecycle::{Caller, Service};
use super::response::ReplyContract;
use super::{ProtocolError, Transfer, MAX_TRANSFER, REQUEST, VERSION};

pub const MAX_GROUPS: usize = 32;
/// Fixed header, followed by group u32s and the existing request encoding.
pub const HEADER_BYTES: usize = 136;
/// v1 permits metadata lookup only. Zero grants never imply root authority.
pub const LOOKUP_METADATA: u64 = 1;

/// Object identities are scoped to namespace AND service registration epoch.
/// They are retained references, not paths, inode numbers, or descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Namespace {
    pub id: u64,
    pub generation: u64,
    pub root: u64,
    pub directory: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context<'a> {
    pub caller: Caller,
    pub service: Service,
    pub namespace: Namespace,
    pub uid: u32,
    pub gid: u32,
    /// Effective supplementary groups, snapshotted without truncation.
    pub groups: &'a [u32],
    /// Kernel-attenuated ceiling, independent of mode/ownership enforcement.
    pub grants: u64,
}

/// Input is immutable for the service operation. Output capacity describes the
/// separate result region; header bytes never overlap service-writable output.
#[derive(Debug)]
pub struct Dispatch<'a> {
    pub request_id: u64,
    pub context: Context<'a>,
    pub request: Request<'a>,
}

impl Dispatch<'_> {
    /// REQUEST attaches exactly two retained capabilities: immutable input,
    /// writable output. Transfer words bound only the output allocation. This
    /// check establishes no endpoint, mapping, or credential authority.
    pub fn transfer(
        &self,
        label: u64,
        words: [u64; 4],
        word_count: usize,
        cap_count: usize,
        output_buffer_bytes: u64,
    ) -> Result<Transfer, ProtocolError> {
        let contract = self.validate()?;
        if label != REQUEST || word_count != 4 || cap_count != 2 {
            return Err(ProtocolError::Encoding);
        }
        let transfer = Transfer::decode(words, output_buffer_bytes)?;
        if words[1] != self.request_id {
            return Err(ProtocolError::RequestId);
        }
        if words[3] < contract.output_capacity as u64 {
            return Err(ProtocolError::Bounds);
        }
        Ok(transfer)
    }

    fn validate(&self) -> Result<ReplyContract, ProtocolError> {
        let c = self.context;
        if self.request_id == 0 {
            return Err(ProtocolError::RequestId);
        }
        if [
            c.caller.pid,
            c.caller.incarnation,
            c.caller.address_space_generation,
            c.service.owner_pid,
            c.service.owner_incarnation,
            c.service.mount_epoch,
            c.namespace.id,
            c.namespace.generation,
            c.namespace.root,
            c.namespace.directory,
        ]
        .contains(&0)
        {
            return Err(ProtocolError::Encoding);
        }
        // Endpoint IDs are zero-based allocations. Zero is not an invalid
        // endpoint; the nameserver's zero selector is a different convention.
        if c.groups.len() > MAX_GROUPS {
            return Err(ProtocolError::Bounds);
        }
        if c.grants & !LOOKUP_METADATA != 0 {
            return Err(ProtocolError::Encoding);
        }
        if !matches!(self.request, Request::Stat { .. }) {
            return Err(ProtocolError::Operation);
        }
        let contract = ReplyContract::for_request(self.request)?;
        if self.encoded_len() as u64 > MAX_TRANSFER {
            return Err(ProtocolError::Bounds);
        }
        Ok(contract)
    }

    pub fn encoded_len(&self) -> usize {
        HEADER_BYTES + self.context.groups.len() * 4 + self.request.encoded_len()
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, ProtocolError> {
        let contract = self.validate()?;
        let c = self.context;
        let mut w = Writer { output, used: 0 };
        for value in [
            VERSION,
            self.request_id,
            c.service.endpoint,
            c.service.owner_pid,
            c.service.owner_incarnation,
            c.service.mount_epoch,
            c.caller.pid,
            c.caller.incarnation,
            c.caller.address_space_generation,
            c.namespace.id,
            c.namespace.generation,
            c.namespace.root,
            c.namespace.directory,
            c.grants,
        ] {
            w.u64(value)?;
        }
        w.u32(c.uid)?;
        w.u32(c.gid)?;
        w.u16(c.groups.len() as u16)?;
        w.u16(0)?; // reserved; unknown semantics fail closed
        w.u32(self.request.encoded_len() as u32)?;
        w.u32(contract.output_capacity as u32)?;
        w.u32(0)?;
        for group in c.groups {
            w.u32(*group)?;
        }
        let start = w.used;
        let count = self.request.encode(&mut w.output[start..])?;
        Ok(start + count)
    }
}

/// Groups are decoded into caller-owned storage; this avoids native alignment,
/// native endianness, allocation, and borrowed mutable wire credentials.
pub fn decode<'a>(
    bytes: &'a [u8],
    groups: &'a mut [u32; MAX_GROUPS],
) -> Result<Dispatch<'a>, ProtocolError> {
    if bytes.len() as u64 > MAX_TRANSFER {
        return Err(ProtocolError::Bounds);
    }
    let mut r = Reader(bytes);
    if r.u64()? != VERSION {
        return Err(ProtocolError::Version);
    }
    let request_id = r.u64()?;
    let service = Service {
        endpoint: r.u64()?,
        owner_pid: r.u64()?,
        owner_incarnation: r.u64()?,
        mount_epoch: r.u64()?,
    };
    let caller = Caller {
        pid: r.u64()?,
        incarnation: r.u64()?,
        address_space_generation: r.u64()?,
    };
    let namespace = Namespace {
        id: r.u64()?,
        generation: r.u64()?,
        root: r.u64()?,
        directory: r.u64()?,
    };
    let grants = r.u64()?;
    let uid = r.u32()?;
    let gid = r.u32()?;
    let count = r.u16()? as usize;
    if count > MAX_GROUPS {
        return Err(ProtocolError::Bounds);
    }
    if r.u16()? != 0 {
        return Err(ProtocolError::Encoding);
    }
    let input_bytes = r.u32()? as usize;
    let output_bytes = r.u32()? as usize;
    if r.u32()? != 0 {
        return Err(ProtocolError::Encoding);
    }
    for group in &mut groups[..count] {
        *group = r.u32()?;
    }
    let request = Request::decode(r.take(input_bytes)?)?;
    if !r.0.is_empty() {
        return Err(ProtocolError::Encoding);
    }
    let dispatch = Dispatch {
        request_id,
        context: Context {
            caller,
            service,
            namespace,
            uid,
            gid,
            groups: &groups[..count],
            grants,
        },
        request,
    };
    let contract = dispatch.validate()?;
    if input_bytes != contract.input_bytes || output_bytes != contract.output_capacity {
        return Err(ProtocolError::Bounds);
    }
    Ok(dispatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dispatch() -> Dispatch<'static> {
        Dispatch {
            request_id: 7,
            context: Context {
                caller: Caller {
                    pid: 4,
                    incarnation: 5,
                    address_space_generation: 6,
                },
                service: Service {
                    endpoint: 8,
                    owner_pid: 9,
                    owner_incarnation: 10,
                    mount_epoch: 11,
                },
                namespace: Namespace {
                    id: 12,
                    generation: 13,
                    root: 14,
                    directory: 15,
                },
                uid: 1000,
                gid: 100,
                groups: &[20, 30],
                grants: LOOKUP_METADATA,
            },
            request: Request::Stat {
                path: "../mount/file/",
            },
        }
    }
    #[test]
    fn large_immutable_input_keeps_a_metadata_sized_output_region() {
        let path_bytes = [b'/'; super::super::MAX_PATH_BYTES];
        let path = core::str::from_utf8(&path_bytes).unwrap();
        let mut d = dispatch();
        d.request = Request::Stat { path: &path };
        let words = [
            VERSION,
            d.request_id,
            0,
            super::super::response::METADATA_BYTES as u64,
        ];
        assert!(d.transfer(REQUEST, words, 4, 2, words[3]).is_ok());
        let mut bytes = [0; HEADER_BYTES + MAX_GROUPS * 4 + super::super::MAX_PATH_BYTES + 16];
        let length = d.encode(&mut bytes).unwrap();
        let mut groups = [0; MAX_GROUPS];
        let decoded = decode(&bytes[..length], &mut groups).unwrap();
        assert_eq!(decoded.request, Request::Stat { path: &path });
    }

    #[test]
    fn exact_frame_round_trip_and_every_truncation() {
        let d = dispatch();
        let mut bytes = [0u8; 512];
        let len = d.encode(&mut bytes).unwrap();
        assert_eq!(len, d.encoded_len());
        assert_eq!(len, HEADER_BYTES + 8 + d.request.encoded_len());
        let mut groups = [0; MAX_GROUPS];
        let decoded = decode(&bytes[..len], &mut groups).unwrap();
        assert_eq!(decoded.context, d.context);
        assert_eq!(decoded.request, d.request);
        assert_eq!(decoded.request_id, d.request_id);
        for n in 0..len {
            assert!(decode(&bytes[..n], &mut groups).is_err(), "{n}");
        }
        assert!(decode(&bytes[..len + 1], &mut groups).is_err());
    }
    #[test]
    fn rejects_unknown_flags_lengths_versions_and_zero_identities() {
        let mut bytes = [0u8; 512];
        let len = dispatch().encode(&mut bytes).unwrap();
        let mut groups = [0; MAX_GROUPS];
        for (offset, replacement) in [
            (0, 2),
            (104, 2),
            (120, 33),
            (122, 1),
            (124, 1),
            (128, 55),
            (132, 1),
        ] {
            let mut bad = bytes;
            bad[offset] = replacement;
            assert!(decode(&bad[..len], &mut groups).is_err(), "offset={offset}");
        }
        for offset in (8..104).step_by(8) {
            if offset == 16 {
                continue; // endpoint zero is valid
            }
            let mut bad = bytes;
            bad[offset..offset + 8].fill(0);
            assert!(decode(&bad[..len], &mut groups).is_err(), "offset={offset}");
        }
    }
    #[test]
    fn unsupported_operations_and_excess_groups_fail_before_encoding() {
        let mut d = dispatch();
        let mut bytes = [0; 512];
        for request in [
            Request::Open {
                path: "/",
                flags: 0,
                mode: 0,
            },
            Request::Fstat { handle: 1 },
            Request::Unlink { path: "/" },
        ] {
            d.request = request;
            assert_eq!(d.encode(&mut bytes), Err(ProtocolError::Operation));
        }
        d.request = Request::Stat { path: "/" };
        d.context.groups = &[1; MAX_GROUPS + 1];
        assert_eq!(d.encode(&mut bytes), Err(ProtocolError::Bounds));
    }
    #[test]
    fn root_uid_and_empty_groups_do_not_invent_grants() {
        let mut d = dispatch();
        d.context.uid = 0;
        d.context.gid = 0;
        d.context.groups = &[];
        d.context.grants = 0;
        d.context.service.endpoint = 0;
        let mut bytes = [0; 512];
        let len = d.encode(&mut bytes).unwrap();
        let mut groups = [0; MAX_GROUPS];
        let decoded = decode(&bytes[..len], &mut groups).unwrap();
        assert_eq!(decoded.context.grants, 0);
        assert_eq!(decoded.context.service.endpoint, 0);
    }

    #[test]
    fn dispatch_envelope_binds_id_capabilities_and_output_range() {
        let d = dispatch();
        let words = [VERSION, d.request_id, 8, 56];
        assert!(d.transfer(REQUEST, words, 4, 2, 64).is_ok());
        for (label, words, count, caps, capacity) in [
            (super::super::REPLY, words, 4, 2, 64),
            (REQUEST, words, 3, 2, 64),
            (REQUEST, words, 4, 1, 64),
            (REQUEST, words, 4, 3, 64),
            (REQUEST, [VERSION, 8, 8, 56], 4, 2, 64),
            (REQUEST, [VERSION, 7, 8, 55], 4, 2, 64),
            (REQUEST, words, 4, 2, 63),
        ] {
            assert!(d.transfer(label, words, count, caps, capacity).is_err());
        }
    }
}
