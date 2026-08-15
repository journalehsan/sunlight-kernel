//! Typed client and host-testable policy helpers for audiod (`audio.v1`).

#![no_std]

#[cfg(test)]
extern crate std;

#[cfg(test)]
extern crate alloc;

use sunlight_audio::{
    parse_persisted, volume_icon, AudioDeviceState, AudioFormat, MasterVolume, OutputDeviceKind,
    PersistedAudio, VolumeIconKind, DEFAULT_VOLUME, MAX_PCM_BYTES,
};
use sunlight_ipc::{
    ipc_call_timeout, nameserver_lookup_timeout, unpack_audio_status, AudiodMsg, IpcMsg,
};

pub const LOOKUP_TIMEOUT_MS: u64 = 50;
pub const REQUEST_TIMEOUT_MS: u64 = 80;
pub const DEFAULT_TONE_HZ: u32 = 440;
pub const DEFAULT_TONE_MS: u32 = 1000;
/// Short tick used after a volume change so the new level is audible.
pub const VOLUME_PREVIEW_HZ: u32 = 880;
pub const VOLUME_PREVIEW_MS: u32 = 180;
pub const MAX_QUEUE_BYTES: usize = MAX_PCM_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioSnapshot {
    pub service_generation: u64,
    pub state: AudioDeviceState,
    pub volume: u8,
    pub muted: bool,
    pub last_nonzero: u8,
    pub kind: OutputDeviceKind,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub bits: u8,
    pub underruns: u32,
    pub frames_played: u32,
    pub vendor_id: u16,
    pub device_id: u16,
}

impl AudioSnapshot {
    pub fn available(self) -> bool {
        self.state.is_usable()
    }

    pub fn device_name(self) -> &'static str {
        self.kind.as_str()
    }

    pub fn state_label(self) -> &'static str {
        self.state.as_str()
    }

    pub fn icon(self) -> VolumeIconKind {
        volume_icon(self.volume, self.muted, self.available())
    }

    pub fn format(self) -> Option<AudioFormat> {
        if self.sample_rate_hz == 0 {
            None
        } else {
            Some(AudioFormat {
                sample_rate_hz: self.sample_rate_hz,
                channels: self.channels,
                bits_per_sample: self.bits,
                signed: true,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioClientError {
    ServiceUnavailable,
    Timeout,
    Transport,
    Unavailable,
    BadRequest,
    InvalidFormat,
    Overflow,
    DeviceFailed,
}

pub struct AudioClient;

impl AudioClient {
    pub const fn new() -> Self {
        Self
    }

    pub fn snapshot(&self) -> Result<AudioSnapshot, AudioClientError> {
        let cap = nameserver_lookup_timeout("audiod", LOOKUP_TIMEOUT_MS)
            .ok_or(AudioClientError::ServiceUnavailable)?;
        let reply = ipc_call_timeout(
            cap,
            IpcMsg::with_label(AudiodMsg::GET_STATUS),
            REQUEST_TIMEOUT_MS,
        )
        .map_err(|err| match err {
            sunlight_ipc::IpcCallError::Timeout => AudioClientError::Timeout,
            _ => AudioClientError::Transport,
        })?;
        if reply.label == AudiodMsg::ERROR {
            return Err(decode_error(&reply));
        }
        if reply.label != AudiodMsg::REPLY {
            return Err(AudioClientError::Transport);
        }
        let status = unpack_audio_status(&reply).ok_or(AudioClientError::Transport)?;
        let state = AudioDeviceState::from_u64(status.state as u64);
        let device = ipc_call_timeout(
            cap,
            IpcMsg::with_label(AudiodMsg::GET_DEVICE),
            REQUEST_TIMEOUT_MS,
        )
        .ok();
        let (kind, vendor_id, device_id) = device
            .and_then(|msg| {
                if msg.label == AudiodMsg::REPLY {
                    Some((
                        OutputDeviceKind::from_u64(msg.words[0]),
                        msg.words[1] as u16,
                        (msg.words[1] >> 16) as u16,
                    ))
                } else {
                    None
                }
            })
            .unwrap_or((OutputDeviceKind::None, 0, 0));
        Ok(AudioSnapshot {
            service_generation: cap.0,
            state,
            volume: status.volume,
            muted: status.muted,
            last_nonzero: status.last_nonzero,
            kind,
            sample_rate_hz: status.sample_rate_hz,
            channels: status.channels,
            bits: status.bits,
            underruns: status.underruns,
            frames_played: status.frames_played,
            vendor_id,
            device_id,
        })
    }

    pub fn set_volume(&self, volume: u8) -> Result<AudioSnapshot, AudioClientError> {
        self.call(IpcMsg::with_label(AudiodMsg::SET_VOLUME).word(0, volume as u64))?;
        self.snapshot()
    }

    pub fn set_mute(&self, muted: bool) -> Result<AudioSnapshot, AudioClientError> {
        self.call(IpcMsg::with_label(AudiodMsg::SET_MUTE).word(0, muted as u64))?;
        self.snapshot()
    }

    pub fn play_tone(&self, freq_hz: u32, duration_ms: u32) -> Result<(), AudioClientError> {
        self.call(
            IpcMsg::with_label(AudiodMsg::PLAY_TONE)
                .word(0, freq_hz as u64)
                .word(1, duration_ms as u64),
        )?;
        Ok(())
    }

    /// One-second 440 Hz tone used by Control Panel "Test Sound" and `audioctl test`.
    pub fn play_test_sound(&self) -> Result<(), AudioClientError> {
        self.play_tone(DEFAULT_TONE_HZ, DEFAULT_TONE_MS)
    }

    /// Brief tick at the current master volume. Callers should skip this while
    /// a slider is mid-drag so IPC is not flooded.
    pub fn play_volume_preview(&self) -> Result<(), AudioClientError> {
        self.play_tone(VOLUME_PREVIEW_HZ, VOLUME_PREVIEW_MS)
    }

    fn call(&self, msg: IpcMsg) -> Result<IpcMsg, AudioClientError> {
        let cap = nameserver_lookup_timeout("audiod", LOOKUP_TIMEOUT_MS)
            .ok_or(AudioClientError::ServiceUnavailable)?;
        let reply = ipc_call_timeout(cap, msg, REQUEST_TIMEOUT_MS).map_err(|err| match err {
            sunlight_ipc::IpcCallError::Timeout => AudioClientError::Timeout,
            _ => AudioClientError::Transport,
        })?;
        if reply.label == AudiodMsg::ERROR {
            return Err(decode_error(&reply));
        }
        if reply.label != AudiodMsg::REPLY {
            return Err(AudioClientError::Transport);
        }
        Ok(reply)
    }
}

fn decode_error(reply: &IpcMsg) -> AudioClientError {
    match reply.words[0] {
        AudiodMsg::ERR_UNAVAILABLE => AudioClientError::Unavailable,
        AudiodMsg::ERR_BAD_REQUEST => AudioClientError::BadRequest,
        AudiodMsg::ERR_INVALID_FORMAT => AudioClientError::InvalidFormat,
        AudiodMsg::ERR_OVERFLOW => AudioClientError::Overflow,
        AudiodMsg::ERR_DEVICE_FAILED => AudioClientError::DeviceFailed,
        _ => AudioClientError::Transport,
    }
}

/// In-memory PCM queue used by audiod. Host-tested independently of hardware.
pub struct PcmQueue {
    buf: [u8; MAX_QUEUE_BYTES],
    head: usize,
    len: usize,
}

impl PcmQueue {
    pub const fn new() -> Self {
        Self {
            buf: [0; MAX_QUEUE_BYTES],
            head: 0,
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<(), AudioClientError> {
        if bytes.len() > MAX_QUEUE_BYTES || self.len + bytes.len() > MAX_QUEUE_BYTES {
            return Err(AudioClientError::Overflow);
        }
        for &b in bytes {
            let idx = (self.head + self.len) % MAX_QUEUE_BYTES;
            self.buf[idx] = b;
            self.len += 1;
        }
        Ok(())
    }

    pub fn pop_into(&mut self, dest: &mut [u8]) -> usize {
        let n = dest.len().min(self.len);
        for slot in dest.iter_mut().take(n) {
            *slot = self.buf[self.head];
            self.head = (self.head + 1) % MAX_QUEUE_BYTES;
            self.len -= 1;
        }
        n
    }
}

pub fn restore_volume(text: Option<&str>) -> MasterVolume {
    match text {
        Some(raw) => MasterVolume::from_persisted(parse_persisted(raw)),
        None => MasterVolume::from_persisted(PersistedAudio::safe_defaults()),
    }
}

pub fn map_page_state(snapshot: Result<AudioSnapshot, AudioClientError>) -> SoundPageView {
    match snapshot {
        Ok(snap) => SoundPageView {
            available: snap.available(),
            device_name: snap.device_name(),
            state_label: snap.state_label(),
            volume: snap.volume,
            muted: snap.muted,
            icon: snap.icon(),
            format: snap.format(),
            service_missing: false,
        },
        Err(AudioClientError::ServiceUnavailable)
        | Err(AudioClientError::Timeout)
        | Err(AudioClientError::Transport) => SoundPageView {
            available: false,
            device_name: "Audio service unavailable",
            state_label: "Unavailable",
            volume: DEFAULT_VOLUME,
            muted: false,
            icon: VolumeIconKind::Unavailable,
            format: None,
            service_missing: true,
        },
        Err(_) => SoundPageView {
            available: false,
            device_name: OutputDeviceKind::None.as_str(),
            state_label: AudioDeviceState::Unavailable.as_str(),
            volume: DEFAULT_VOLUME,
            muted: false,
            icon: VolumeIconKind::Unavailable,
            format: None,
            service_missing: false,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundPageView {
    pub available: bool,
    pub device_name: &'static str,
    pub state_label: &'static str,
    pub volume: u8,
    pub muted: bool,
    pub icon: VolumeIconKind,
    pub format: Option<AudioFormat>,
    pub service_missing: bool,
}

pub fn sound_settings_page_id() -> &'static [u8] {
    b"sound"
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_audio::MasterVolume;

    #[test]
    fn queue_bounds_and_disconnect_cleanup() {
        let mut q = PcmQueue::new();
        assert!(q.push(&[1, 2, 3, 4]).is_ok());
        assert_eq!(q.len(), 4);
        let too_big = alloc::vec![0u8; MAX_QUEUE_BYTES + 1];
        assert_eq!(q.push(&too_big), Err(AudioClientError::Overflow));
        let mut out = [0u8; 2];
        assert_eq!(q.pop_into(&mut out), 2);
        assert_eq!(&out, &[1, 2]);
        q.clear();
        assert!(q.is_empty());
    }

    #[test]
    fn persisted_volume_restoration() {
        let v = restore_volume(Some("[audio]\nmaster_volume = 22\nmuted = true\n"));
        assert_eq!(v.volume(), 22);
        assert!(v.muted());
        let v = restore_volume(Some("[audio]\nmaster_volume = 250\n"));
        assert_eq!(v.volume(), 100);
        let v = restore_volume(None);
        assert_eq!(v.volume(), DEFAULT_VOLUME);
        assert!(!v.muted());
    }

    #[test]
    fn page_mapping_unavailable_and_mute() {
        let mut live = AudioSnapshot {
            service_generation: 1,
            state: AudioDeviceState::Ready,
            volume: 70,
            muted: true,
            last_nonzero: 70,
            kind: OutputDeviceKind::QemuHdAudio,
            sample_rate_hz: 48_000,
            channels: 2,
            bits: 16,
            underruns: 0,
            frames_played: 0,
            vendor_id: 0x8086,
            device_id: 0x2668,
        };
        let view = map_page_state(Ok(live));
        assert_eq!(view.device_name, "QEMU HD Audio");
        assert_eq!(view.icon, VolumeIconKind::Off);
        live.muted = false;
        live.volume = 20;
        assert_eq!(map_page_state(Ok(live)).icon, VolumeIconKind::Low);
        let missing = map_page_state(Err(AudioClientError::ServiceUnavailable));
        assert!(missing.service_missing);
        assert_eq!(missing.icon, VolumeIconKind::Unavailable);
    }

    #[test]
    fn mute_transitions_keep_last_nonzero() {
        let mut v = MasterVolume::default_live();
        v.set_volume(61);
        v.toggle_mute();
        assert_eq!(v.last_nonzero(), 61);
        v.toggle_mute();
        assert_eq!(v.volume(), 61);
    }

    #[test]
    fn deep_link_id_is_sound() {
        assert_eq!(sound_settings_page_id(), b"sound");
    }

    #[test]
    fn preview_tone_is_shorter_than_test_tone() {
        assert!(VOLUME_PREVIEW_MS < DEFAULT_TONE_MS);
        assert_ne!(VOLUME_PREVIEW_HZ, DEFAULT_TONE_HZ);
    }
}
