//! Generic audio-output types for SunlightOS.
//!
//! This crate is hardware-agnostic. Playback policy (volume/mute) and PCM
//! validation live here so `audiod`, Control Panel, Vortex, and host tests
//! share one model. The Intel HDA backend is an implementation detail of
//! `audiod` and is not part of the userspace protocol.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod hda;
pub mod pcm;
pub mod system;
pub mod volume;
pub mod wav;

pub use pcm::{
    apply_gain, generate_sine_s16le_stereo, validate_pcm, AudioBuffer, AudioError, AudioFormat,
    PcmValidation, MAX_PCM_BYTES, NATIVE_CHANNELS, NATIVE_RATE_HZ, NATIVE_SAMPLE_BITS,
};
pub use system::{
    effective_system_gain, SystemSound, SystemSoundSettings, DEFAULT_SYSTEM_SOUNDS_ENABLED,
    DEFAULT_SYSTEM_SOUNDS_VOLUME, SYSTEM_SOUND_COUNT, SYSTEM_SOUND_PROTOCOL_VERSION,
};
pub use volume::{
    parse_persisted, render_persisted, render_persisted_buf, volume_icon, MasterVolume,
    PersistedAudio, VolumeIconKind, DEFAULT_VOLUME, MAX_VOLUME,
};
pub use wav::{parse_pcm_wav, WavError, WavPcm};

/// Playback device readiness as published by audiod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AudioDeviceState {
    Unavailable = 0,
    Initializing = 1,
    Ready = 2,
    Playing = 3,
    Underrun = 4,
    Failed = 5,
}

impl AudioDeviceState {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Initializing,
            2 => Self::Ready,
            3 => Self::Playing,
            4 => Self::Underrun,
            5 => Self::Failed,
            _ => Self::Unavailable,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "Unavailable",
            Self::Initializing => "Initializing",
            Self::Ready => "Ready",
            Self::Playing => "Playing",
            Self::Underrun => "Underrun",
            Self::Failed => "Failed",
        }
    }

    pub const fn is_usable(self) -> bool {
        matches!(self, Self::Ready | Self::Playing | Self::Underrun)
    }
}

/// Well-known output device labels. Packed as a small tag so the IPC
/// protocol never ships raw PCI IDs as the primary name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OutputDeviceKind {
    None = 0,
    QemuHdAudio = 1,
    HdAudio = 2,
    GenericOutput = 3,
}

impl OutputDeviceKind {
    pub const fn from_pci(vendor_id: u16, device_id: u16) -> Self {
        if vendor_id == 0x8086 && matches!(device_id, 0x2668 | 0x293e) {
            Self::QemuHdAudio
        } else {
            Self::HdAudio
        }
    }

    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::QemuHdAudio,
            2 => Self::HdAudio,
            3 => Self::GenericOutput,
            _ => Self::None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "No audio output device available",
            Self::QemuHdAudio => "QEMU HD Audio",
            Self::HdAudio => "HD Audio",
            Self::GenericOutput => "Audio Output",
        }
    }
}

/// Negotiated stream configuration. v1 is fixed: 48 kHz, S16LE, stereo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioStreamConfig {
    pub format: AudioFormat,
}

impl AudioStreamConfig {
    pub const NATIVE: Self = Self {
        format: AudioFormat::NATIVE,
    };
}

/// Static device capability report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioCapabilities {
    pub kind: OutputDeviceKind,
    pub vendor_id: u16,
    pub device_id: u16,
    pub max_channels: u8,
    pub sample_bits: u8,
    pub native_rate_hz: u32,
    pub playback: bool,
}

impl AudioCapabilities {
    pub const NONE: Self = Self {
        kind: OutputDeviceKind::None,
        vendor_id: 0,
        device_id: 0,
        max_channels: 0,
        sample_bits: 0,
        native_rate_hz: 0,
        playback: false,
    };

    pub const fn qemu_hda(vendor_id: u16, device_id: u16) -> Self {
        Self {
            kind: OutputDeviceKind::QemuHdAudio,
            vendor_id,
            device_id,
            max_channels: NATIVE_CHANNELS,
            sample_bits: NATIVE_SAMPLE_BITS,
            native_rate_hz: NATIVE_RATE_HZ,
            playback: true,
        }
    }
}
