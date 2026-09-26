use core::fmt;

use crate::codec::short_hex;
use crate::validation::IdentityError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntropyError;

/// Immutable, hardware-independent 256-bit Wise Owl identity identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdentityId([u8; 32]);

impl IdentityId {
    pub const BYTE_LEN: usize = 32;

    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, IdentityError> {
        if bytes == [0; 32] {
            return Err(IdentityError::InvalidIdentityId);
        }
        Ok(Self(bytes))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IdentityError> {
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| IdentityError::InvalidLength)?;
        Self::from_bytes(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub const fn encode(self) -> [u8; 32] {
        self.0
    }

    pub fn generate_with(
        mut fill: impl FnMut(&mut [u8]) -> Result<(), EntropyError>,
    ) -> Result<Self, EntropyError> {
        let mut bytes = [0u8; 32];
        fill(&mut bytes)?;
        Self::from_bytes(bytes).map_err(|_| EntropyError)
    }

    #[cfg(feature = "host")]
    pub fn generate() -> Result<Self, EntropyError> {
        Self::generate_with(|bytes| getrandom::getrandom(bytes).map_err(|_| EntropyError))
    }

    pub fn diagnostic_fingerprint(self) -> [u8; 8] {
        short_hex(&self.0)
    }
}

#[cfg(feature = "host")]
pub fn fill_host_entropy(bytes: &mut [u8]) -> Result<(), EntropyError> {
    getrandom::getrandom(bytes).map_err(|_| EntropyError)
}

impl fmt::Debug for IdentityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fingerprint = self.diagnostic_fingerprint();
        let text = core::str::from_utf8(&fingerprint).map_err(|_| fmt::Error)?;
        write!(formatter, "IdentityId({text}…)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_generation_is_nonzero_and_roundtrips() {
        let id = IdentityId::generate_with(|bytes| {
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = index as u8 + 1;
            }
            Ok(())
        })
        .unwrap();
        assert_ne!(id.as_bytes(), &[0; 32]);
        assert_eq!(IdentityId::decode(&id.encode()).unwrap(), id);
    }

    #[test]
    fn rejects_invalid_length_and_zero() {
        assert_eq!(IdentityId::decode(&[1; 31]), Err(IdentityError::InvalidLength));
        assert_eq!(IdentityId::decode(&[0; 32]), Err(IdentityError::InvalidIdentityId));
    }

    #[test]
    fn debug_is_bounded_and_does_not_expose_full_id() {
        let id = IdentityId::from_bytes([0xAB; 32]).unwrap();
        let debug = format!("{id:?}");
        assert_eq!(debug, "IdentityId(ABABABAB…)");
        assert!(!debug.contains(&"AB".repeat(32)));
    }
}
