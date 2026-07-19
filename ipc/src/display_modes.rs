//! Backend-neutral display mode capability and transaction types.

use crate::{DisplayMetrics, ScreenBackend};

pub const MAX_DISPLAY_MODES: usize = 16;
pub const DEFAULT_MODE_PREVIEW_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub bits_per_pixel: u32,
    pub pitch_bytes: u32,
    pub preferred: bool,
    pub current: bool,
}

impl DisplayMode {
    pub const fn from_metrics(metrics: DisplayMetrics) -> Self {
        Self {
            width: metrics.width_px,
            height: metrics.height_px,
            bits_per_pixel: 32,
            pitch_bytes: metrics.stride_bytes,
            preferred: false,
            current: true,
        }
    }

    pub const fn geometry_word(self) -> u64 {
        self.width as u64 | ((self.height as u64) << 32)
    }

    pub const fn format_word(self) -> u64 {
        self.pitch_bytes as u64 | ((self.bits_per_pixel as u64) << 32)
    }

    pub const fn flags_word(self) -> u64 {
        self.preferred as u64 | ((self.current as u64) << 1)
    }

    pub const fn from_words(geometry: u64, format: u64, flags: u64) -> Self {
        Self {
            width: geometry as u32,
            height: (geometry >> 32) as u32,
            pitch_bytes: format as u32,
            bits_per_pixel: (format >> 32) as u32,
            preferred: flags & 1 != 0,
            current: flags & 2 != 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DisplayModeManagement {
    ReadOnly = 0,
    Manual = 1,
    Automatic = 2,
}

impl DisplayModeManagement {
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Manual,
            2 => Self::Automatic,
            _ => Self::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DisplayModeReadOnlyReason {
    None = 0,
    AutomaticallyManaged = 1,
    FirmwareFramebuffer = 2,
    DriverUnavailable = 3,
    PreviewUnavailable = 4,
}

impl DisplayModeReadOnlyReason {
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::AutomaticallyManaged,
            2 => Self::FirmwareFramebuffer,
            3 => Self::DriverUnavailable,
            4 => Self::PreviewUnavailable,
            _ => Self::None,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::None => "",
            Self::AutomaticallyManaged => "Resolution follows the virtual display automatically.",
            Self::FirmwareFramebuffer => {
                "Runtime resolution changes are unavailable with the firmware framebuffer."
            }
            Self::DriverUnavailable => "The active display driver cannot change resolution.",
            Self::PreviewUnavailable => "Safe display mode preview is unavailable.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayModeCapabilities {
    pub backend: ScreenBackend,
    pub current_mode: DisplayMode,
    pub mode_count: u32,
    pub management: DisplayModeManagement,
    pub read_only_reason: DisplayModeReadOnlyReason,
}

impl DisplayModeCapabilities {
    pub const fn mode_change_supported(self) -> bool {
        matches!(self.management, DisplayModeManagement::Manual) && self.mode_count > 0
    }

    pub const fn automatically_managed(self) -> bool {
        matches!(self.management, DisplayModeManagement::Automatic)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayModeTransaction {
    pub token: u64,
    pub applied_mode: DisplayMode,
    pub deadline_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PixelFormat, ScreenBackend};

    #[test]
    fn display_mode_round_trips_wire_words() {
        let mode = DisplayMode {
            width: 1440,
            height: 900,
            bits_per_pixel: 32,
            pitch_bytes: 5760,
            preferred: true,
            current: false,
        };
        assert_eq!(
            DisplayMode::from_words(mode.geometry_word(), mode.format_word(), mode.flags_word()),
            mode
        );
    }

    #[test]
    fn metrics_convert_to_current_mode() {
        let metrics = DisplayMetrics::new(
            1024,
            768,
            4096,
            PixelFormat::Xrgb8888,
            ScreenBackend::VmwareSvga,
        );
        let mode = DisplayMode::from_metrics(metrics);
        assert!(mode.current);
        assert_eq!(mode.pitch_bytes, 4096);
        assert_eq!((mode.width, mode.height), (1024, 768));
    }
}
