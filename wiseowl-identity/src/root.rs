use crate::codec::{read_u16, require_exact, sha256};
use crate::{GenesisEventKind, IdentityError, IdentityId, LineageEventId, IDENTITY_FORMAT_VERSION};

const MAGIC: &[u8; 8] = b"WOIDROOT";
const DOMAIN: &[u8] = b"wiseowl.identity.root.v1\0";
pub const IDENTITY_ROOT_LEN: usize = 116;
const CHECKSUM_OFFSET: usize = IDENTITY_ROOT_LEN - 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityRoot {
    identity_id: IdentityId,
    creation_event_id: LineageEventId,
    creation_event_kind: GenesisEventKind,
}

impl IdentityRoot {
    pub const fn new(
        identity_id: IdentityId,
        creation_event_id: LineageEventId,
        creation_event_kind: GenesisEventKind,
    ) -> Self {
        Self {
            identity_id,
            creation_event_id,
            creation_event_kind,
        }
    }

    pub const fn identity_id(self) -> IdentityId {
        self.identity_id
    }

    pub const fn creation_event_id(self) -> LineageEventId {
        self.creation_event_id
    }

    pub const fn creation_event_kind(self) -> GenesisEventKind {
        self.creation_event_kind
    }

    pub fn encode(self) -> [u8; IDENTITY_ROOT_LEN] {
        let mut out = [0u8; IDENTITY_ROOT_LEN];
        out[..8].copy_from_slice(MAGIC);
        out[8..10].copy_from_slice(&IDENTITY_FORMAT_VERSION.to_le_bytes());
        out[10..12].copy_from_slice(&(IDENTITY_ROOT_LEN as u16).to_le_bytes());
        out[12..44].copy_from_slice(self.identity_id.as_bytes());
        out[44..76].copy_from_slice(self.creation_event_id.as_bytes());
        out[76] = self.creation_event_kind as u8;
        // 77..84 is reserved version metadata and must remain zero.
        let checksum = sha256(DOMAIN, &out[..CHECKSUM_OFFSET]);
        out[CHECKSUM_OFFSET..].copy_from_slice(&checksum);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IdentityError> {
        require_exact(bytes, IDENTITY_ROOT_LEN)?;
        if &bytes[..8] != MAGIC {
            return Err(IdentityError::InvalidMagic);
        }
        if read_u16(bytes, 8) != IDENTITY_FORMAT_VERSION {
            return Err(IdentityError::UnsupportedVersion);
        }
        if read_u16(bytes, 10) as usize != IDENTITY_ROOT_LEN {
            return Err(IdentityError::InvalidLength);
        }
        if bytes[77..84] != [0; 7] {
            return Err(IdentityError::UnsupportedVersion);
        }
        if sha256(DOMAIN, &bytes[..CHECKSUM_OFFSET]) != bytes[CHECKSUM_OFFSET..] {
            return Err(IdentityError::ChecksumMismatch);
        }
        Ok(Self {
            identity_id: IdentityId::decode(&bytes[12..44])?,
            creation_event_id: LineageEventId::decode(&bytes[44..76])?,
            creation_event_kind: match bytes[76] {
                1 => GenesisEventKind::Created,
                2 => GenesisEventKind::ExistingStateAdopted,
                _ => return Err(IdentityError::InvalidEventKind),
            },
        })
    }
}
