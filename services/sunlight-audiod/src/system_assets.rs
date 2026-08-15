//! Authoritative semantic mapping for the built-in "Sunlight Default" theme.

use sunlight_audio::{parse_pcm_wav, SystemSound, WavError, WavPcm};

pub const THEME_NAME: &str = "Sunlight Default";

pub struct SystemSoundAsset {
    pub canonical_name: &'static str,
    pub wav: &'static [u8],
}

pub fn asset_for(sound: SystemSound) -> SystemSoundAsset {
    let (canonical_name, wav): (&str, &[u8]) = match sound {
        SystemSound::Notification => (
            "notification",
            include_bytes!("../../../assets/sounds/Sunlight Default/notification.wav"),
        ),
        SystemSound::Message => (
            "message",
            include_bytes!("../../../assets/sounds/Sunlight Default/message.wav"),
        ),
        SystemSound::Success => (
            "success",
            include_bytes!("../../../assets/sounds/Sunlight Default/success.wav"),
        ),
        SystemSound::Warning => (
            "warning",
            include_bytes!("../../../assets/sounds/Sunlight Default/warning.wav"),
        ),
        SystemSound::Error => (
            "error",
            include_bytes!("../../../assets/sounds/Sunlight Default/error.wav"),
        ),
        SystemSound::Question => (
            "question",
            include_bytes!("../../../assets/sounds/Sunlight Default/question.wav"),
        ),
        SystemSound::Critical => (
            "critical",
            include_bytes!("../../../assets/sounds/Sunlight Default/critical.wav"),
        ),
        SystemSound::DeviceConnected => (
            "device-connected",
            include_bytes!("../../../assets/sounds/Sunlight Default/device-connected.wav"),
        ),
        SystemSound::DeviceDisconnected => (
            "device-disconnected",
            include_bytes!("../../../assets/sounds/Sunlight Default/device-disconnected.wav"),
        ),
        SystemSound::VolumeChanged => (
            "volume-changed",
            include_bytes!("../../../assets/sounds/Sunlight Default/volume-changed.wav"),
        ),
    };
    SystemSoundAsset {
        canonical_name,
        wav,
    }
}

pub fn resolve(sound: SystemSound) -> Result<WavPcm<'static>, WavError> {
    parse_pcm_wav(asset_for(sound).wav)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use sunlight_audio::{SystemSound, NATIVE_RATE_HZ};

    #[test]
    fn every_semantic_id_maps_to_valid_unique_native_pcm() {
        let mut names = BTreeSet::new();
        assert_eq!(THEME_NAME, "Sunlight Default");
        for sound in SystemSound::ALL {
            let asset = asset_for(sound);
            assert!(names.insert(asset.canonical_name));
            let parsed = resolve(sound).unwrap();
            assert!(parsed.format.is_native());
            assert_eq!(parsed.format.sample_rate_hz, NATIVE_RATE_HZ);
            assert!(!parsed.pcm.is_empty());
        }
        assert_eq!(names.len(), SystemSound::ALL.len());
    }
}
