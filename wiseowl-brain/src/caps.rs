#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BrainCapability {
    InvokeWiseOwlBrain = 0,
    InvokeGreetingProvider = 1,
    InspectOwnBrainContext = 2,
    InspectAnyBrainContext = 3,
    AdminBrain = 4,
}

impl BrainCapability {
    pub const fn bit(self) -> u64 {
        1u64 << (self as u8)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvokeWiseOwlBrain => "invoke-wiseowl-brain",
            Self::InvokeGreetingProvider => "invoke-greeting-provider",
            Self::InspectOwnBrainContext => "inspect-own-brain-context",
            Self::InspectAnyBrainContext => "inspect-any-brain-context",
            Self::AdminBrain => "admin-brain",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BrainCapabilitySet {
    mask: u64,
}

impl BrainCapabilitySet {
    pub const fn empty() -> Self {
        Self { mask: 0 }
    }

    pub const fn from_mask(mask: u64) -> Self {
        Self { mask }
    }

    pub const fn mask(self) -> u64 {
        self.mask
    }

    pub fn grant(mut self, cap: BrainCapability) -> Self {
        self.mask |= cap.bit();
        self
    }

    pub fn has(self, cap: BrainCapability) -> bool {
        if self.mask & BrainCapability::AdminBrain.bit() != 0 {
            return true;
        }
        self.mask & cap.bit() != 0
    }

    pub fn require(self, cap: BrainCapability) -> Result<(), crate::error::BrainError> {
        if self.has(cap) {
            Ok(())
        } else {
            Err(crate::error::BrainError::PermissionDenied(cap.as_str()))
        }
    }

    /// Default client: can invoke the brain and request greetings.
    pub fn default_client() -> Self {
        Self::empty()
            .grant(BrainCapability::InvokeWiseOwlBrain)
            .grant(BrainCapability::InvokeGreetingProvider)
            .grant(BrainCapability::InspectOwnBrainContext)
    }

    /// Diagnostic operator.
    pub fn diagnostic() -> Self {
        Self::default_client().grant(BrainCapability::InspectAnyBrainContext)
    }

    pub fn admin() -> Self {
        Self::from_mask(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_client_can_invoke() {
        let c = BrainCapabilitySet::default_client();
        assert!(c.require(BrainCapability::InvokeWiseOwlBrain).is_ok());
        assert!(c.require(BrainCapability::InvokeGreetingProvider).is_ok());
    }

    #[test]
    fn default_client_cannot_admin() {
        let c = BrainCapabilitySet::default_client();
        assert!(c.require(BrainCapability::AdminBrain).is_err());
        assert!(c.require(BrainCapability::InspectAnyBrainContext).is_err());
    }

    #[test]
    fn admin_has_all() {
        let a = BrainCapabilitySet::admin();
        assert!(a.has(BrainCapability::AdminBrain));
        assert!(a.has(BrainCapability::InvokeWiseOwlBrain));
    }

    #[test]
    fn diagnostic_can_inspect_any() {
        let d = BrainCapabilitySet::diagnostic();
        assert!(d.has(BrainCapability::InspectAnyBrainContext));
    }
}
