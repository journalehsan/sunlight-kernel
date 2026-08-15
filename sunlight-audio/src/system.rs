//! Stable semantic system-sound vocabulary and policy primitives.

pub const SYSTEM_SOUND_PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_SYSTEM_SOUNDS_ENABLED: bool = true;
pub const DEFAULT_SYSTEM_SOUNDS_VOLUME: u8 = 60;
pub const SYSTEM_SOUND_COUNT: usize = 10;

/// Stable semantic IDs. Callers request intent; only audiod resolves assets.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemSound {
    Notification = 1,
    Message = 2,
    Success = 3,
    Warning = 4,
    Error = 5,
    Question = 6,
    Critical = 7,
    DeviceConnected = 8,
    DeviceDisconnected = 9,
    VolumeChanged = 10,
}

impl SystemSound {
    pub const ALL: [Self; SYSTEM_SOUND_COUNT] = [
        Self::Notification,
        Self::Message,
        Self::Success,
        Self::Warning,
        Self::Error,
        Self::Question,
        Self::Critical,
        Self::DeviceConnected,
        Self::DeviceDisconnected,
        Self::VolumeChanged,
    ];

    pub const fn from_wire(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::Notification),
            2 => Some(Self::Message),
            3 => Some(Self::Success),
            4 => Some(Self::Warning),
            5 => Some(Self::Error),
            6 => Some(Self::Question),
            7 => Some(Self::Critical),
            8 => Some(Self::DeviceConnected),
            9 => Some(Self::DeviceDisconnected),
            10 => Some(Self::VolumeChanged),
            _ => None,
        }
    }

    pub const fn index(self) -> usize {
        self as usize - 1
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Notification => "Notification",
            Self::Message => "Message",
            Self::Success => "Success",
            Self::Warning => "Warning",
            Self::Error => "Error",
            Self::Question => "Question",
            Self::Critical => "Critical",
            Self::DeviceConnected => "Device Connected",
            Self::DeviceDisconnected => "Device Disconnected",
            Self::VolumeChanged => "Volume Changed",
        }
    }

    /// Identical automatic requests inside this window are coalesced.
    pub const fn cooldown_ms(self) -> u64 {
        match self {
            Self::VolumeChanged => 120,
            Self::Message | Self::Success => 300,
            Self::Notification | Self::Question => 350,
            Self::Warning | Self::Error => 500,
            Self::DeviceConnected | Self::DeviceDisconnected => 600,
            Self::Critical => 750,
        }
    }

    pub const fn priority(self) -> u8 {
        match self {
            Self::Critical => 3,
            Self::Error | Self::Warning => 2,
            Self::Notification
            | Self::Message
            | Self::Success
            | Self::Question
            | Self::DeviceConnected
            | Self::DeviceDisconnected => 1,
            Self::VolumeChanged => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemSoundSettings {
    pub enabled: bool,
    pub volume: u8,
}

impl SystemSoundSettings {
    pub const fn safe_defaults() -> Self {
        Self {
            enabled: DEFAULT_SYSTEM_SOUNDS_ENABLED,
            volume: DEFAULT_SYSTEM_SOUNDS_VOLUME,
        }
    }

    pub const fn validated(enabled: bool, volume: u8) -> Self {
        Self {
            enabled,
            volume: if volume > 100 { 100 } else { volume },
        }
    }
}

/// Combine master and system-event gain without overflow.
pub const fn effective_system_gain(master_effective: u8, system_volume: u8) -> u8 {
    let master = if master_effective > 100 {
        100
    } else {
        master_effective
    } as u16;
    let system = if system_volume > 100 {
        100
    } else {
        system_volume
    } as u16;
    ((master * system + 50) / 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_values_are_stable_bounded_and_complete() {
        for (index, sound) in SystemSound::ALL.iter().copied().enumerate() {
            assert_eq!(SystemSound::from_wire(sound as u64), Some(sound));
            assert_eq!(sound.index(), index);
        }
        assert_eq!(SystemSound::from_wire(0), None);
        assert_eq!(SystemSound::from_wire(11), None);
        assert_eq!(SystemSound::from_wire(u64::MAX), None);
    }

    #[test]
    fn combined_gain_respects_both_controls() {
        assert_eq!(effective_system_gain(100, 100), 100);
        assert_eq!(effective_system_gain(50, 60), 30);
        assert_eq!(effective_system_gain(0, 100), 0);
        assert_eq!(effective_system_gain(100, 0), 0);
        assert_eq!(effective_system_gain(255, 255), 100);
    }
}
