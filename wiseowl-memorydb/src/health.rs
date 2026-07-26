//! Service health and degraded reasons.

use alloc::string::String;
use alloc::vec::Vec;

/// Health state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum HealthState {
    Starting = 1,
    Ready = 2,
    Degraded = 3,
    Failed = 4,
}

/// Health report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct DbHealth {
    pub state: HealthState,
    pub reasons: Vec<String>,
    pub ready: bool,
}

impl DbHealth {
    pub fn starting() -> Self {
        Self {
            state: HealthState::Starting,
            reasons: Vec::new(),
            ready: false,
        }
    }

    pub fn ready() -> Self {
        Self {
            state: HealthState::Ready,
            reasons: Vec::new(),
            ready: true,
        }
    }

    pub fn degraded(reasons: Vec<String>) -> Self {
        Self {
            state: HealthState::Degraded,
            reasons,
            ready: true, // degraded still serves safe operations
        }
    }
}
