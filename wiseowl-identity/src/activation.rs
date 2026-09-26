//! Local installation and activation identifiers. These are not global
//! identity, device identifiers, or runtime process identifiers.

use crate::EntropyError;
use core::fmt;

macro_rules! fixed_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);

        impl $name {
            pub fn from_bytes(bytes: [u8; 16]) -> Option<Self> {
                (bytes != [0; 16]).then_some(Self(bytes))
            }

            pub const fn as_bytes(&self) -> &[u8; 16] { &self.0 }
            pub const fn encode(self) -> [u8; 16] { self.0 }
            pub fn fingerprint(self) -> [u8; 8] {
                crate::codec::short_hex(&self.0)
            }
            pub fn generate_with(mut fill: impl FnMut(&mut [u8]) -> Result<(), EntropyError>) -> Result<Self, EntropyError> {
                let mut bytes = [0; 16];
                fill(&mut bytes)?;
                Self::from_bytes(bytes).ok_or(EntropyError)
            }
            #[cfg(feature = "host")]
            pub fn generate() -> Result<Self, EntropyError> {
                Self::generate_with(|bytes| getrandom::getrandom(bytes).map_err(|_| EntropyError))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:02X}{:02X}{:02X}{:02X}…)", stringify!($name), self.0[0], self.0[1], self.0[2], self.0[3])
            }
        }
    };
}

fixed_id!(InstallationId);
fixed_id!(ActivationId);

/// Phase C local lifecycle states. Numeric values are part of LOCAL v1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ActivationState {
    Dormant = 1,
    Activating = 2,
    Active = 3,
    RecoveringLocal = 4,
    Suspended = 5,
}

impl ActivationState {
    pub fn decode(value: u8) -> Option<Self> {
        Some(match value { 1 => Self::Dormant, 2 => Self::Activating, 3 => Self::Active, 4 => Self::RecoveringLocal, 5 => Self::Suspended, _ => return None })
    }
}
