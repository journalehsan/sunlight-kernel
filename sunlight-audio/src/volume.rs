//! Master volume and mute policy.
//!
//! `audiod` is authoritative. GUIs send intent; they do not keep a second
//! policy copy beyond the last snapshot they received.

/// Inclusive userspace range.
pub const MAX_VOLUME: u8 = 100;
/// Default used when preferences are missing or invalid (60–70 band).
pub const DEFAULT_VOLUME: u8 = 65;

/// Icon thresholds requested by the panel contract.
pub const VOLUME_HIGH_MIN: u8 = 67;
pub const VOLUME_MEDIUM_MIN: u8 = 34;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeIconKind {
    High,
    Medium,
    Low,
    Off,
    Unavailable,
}

/// Select the panel/control-panel speaker glyph.
pub fn volume_icon(volume: u8, muted: bool, available: bool) -> VolumeIconKind {
    if !available {
        return VolumeIconKind::Unavailable;
    }
    if muted || volume == 0 {
        return VolumeIconKind::Off;
    }
    if volume >= VOLUME_HIGH_MIN {
        VolumeIconKind::High
    } else if volume >= VOLUME_MEDIUM_MIN {
        VolumeIconKind::Medium
    } else {
        VolumeIconKind::Low
    }
}

/// Master output policy. Mute is independent of the remembered level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MasterVolume {
    volume: u8,
    muted: bool,
    last_nonzero: u8,
}

impl MasterVolume {
    pub const fn default_live() -> Self {
        Self {
            volume: DEFAULT_VOLUME,
            muted: false,
            last_nonzero: DEFAULT_VOLUME,
        }
    }

    pub fn from_persisted(persisted: PersistedAudio) -> Self {
        let volume = clamp_volume(persisted.volume);
        let last_nonzero = if persisted.last_nonzero == 0 {
            if volume == 0 {
                DEFAULT_VOLUME
            } else {
                volume
            }
        } else {
            clamp_volume(persisted.last_nonzero)
        };
        let last_nonzero = if last_nonzero == 0 {
            DEFAULT_VOLUME
        } else {
            last_nonzero
        };
        Self {
            volume,
            muted: persisted.muted,
            last_nonzero: if volume == 0 { last_nonzero } else { volume },
        }
    }

    pub const fn volume(self) -> u8 {
        self.volume
    }

    pub const fn muted(self) -> bool {
        self.muted
    }

    pub const fn last_nonzero(self) -> u8 {
        self.last_nonzero
    }

    /// Effective amplitude 0..100 after mute.
    pub const fn effective(self) -> u8 {
        if self.muted {
            0
        } else {
            self.volume
        }
    }

    pub fn set_volume(&mut self, value: u8) {
        self.volume = clamp_volume(value);
        if self.volume > 0 {
            self.last_nonzero = self.volume;
        }
    }

    pub fn set_muted(&mut self, muted: bool) {
        if muted && !self.muted && self.volume > 0 {
            self.last_nonzero = self.volume;
        }
        self.muted = muted;
    }

    pub fn toggle_mute(&mut self) {
        if self.muted {
            self.muted = false;
            if self.volume == 0 {
                self.volume = self.last_nonzero;
            }
        } else {
            if self.volume > 0 {
                self.last_nonzero = self.volume;
            }
            self.muted = true;
        }
    }

    pub fn snapshot(self) -> PersistedAudio {
        PersistedAudio {
            volume: self.volume,
            muted: self.muted,
            last_nonzero: self.last_nonzero,
        }
    }
}

const fn clamp_volume(value: u8) -> u8 {
    if value > MAX_VOLUME {
        MAX_VOLUME
    } else {
        value
    }
}

/// On-disk record. Invalid fields are repaired by [`MasterVolume::from_persisted`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistedAudio {
    pub volume: u8,
    pub muted: bool,
    pub last_nonzero: u8,
}

impl PersistedAudio {
    pub const fn safe_defaults() -> Self {
        Self {
            volume: DEFAULT_VOLUME,
            muted: false,
            last_nonzero: DEFAULT_VOLUME,
        }
    }
}

/// Parse the existing sunlight TOML-ish settings style:
///
/// ```text
/// [audio]
/// master_volume = 65
/// muted = false
/// last_nonzero = 65
/// ```
pub fn parse_persisted(text: &str) -> PersistedAudio {
    let mut out = PersistedAudio::safe_defaults();
    let mut in_audio = false;
    let mut saw_volume = false;
    let mut saw_muted = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[audio]" {
            in_audio = true;
            continue;
        }
        if line.starts_with('[') {
            in_audio = false;
            continue;
        }
        if !in_audio {
            continue;
        }
        if let Some(value) = line.strip_prefix("master_volume") {
            if let Some(n) = parse_u8_assignment(value) {
                out.volume = n;
                saw_volume = true;
            }
        } else if let Some(value) = line.strip_prefix("muted") {
            if let Some(flag) = parse_bool_assignment(value) {
                out.muted = flag;
                saw_muted = true;
            }
        } else if let Some(value) = line.strip_prefix("last_nonzero") {
            if let Some(n) = parse_u8_assignment(value) {
                out.last_nonzero = n;
            }
        }
    }
    if !saw_volume {
        out.volume = DEFAULT_VOLUME;
    }
    if !saw_muted {
        out.muted = false;
    }
    if out.volume > MAX_VOLUME {
        out.volume = MAX_VOLUME;
    }
    if out.last_nonzero == 0 || out.last_nonzero > MAX_VOLUME {
        out.last_nonzero = if out.volume == 0 {
            DEFAULT_VOLUME
        } else {
            out.volume
        };
    }
    out
}

pub fn render_persisted(cfg: PersistedAudio) -> heapless_audio_toml {
    // Implemented below as a small stack buffer helper so no_std services
    // do not need alloc just to persist two integers.
    render_persisted_buf(cfg)
}

/// Tiny TOML renderer that avoids pulling `alloc` into kernel-adjacent crates.
#[allow(non_camel_case_types)]
pub struct heapless_audio_toml {
    buf: [u8; 96],
    len: usize,
}

impl heapless_audio_toml {
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

pub fn render_persisted_buf(cfg: PersistedAudio) -> heapless_audio_toml {
    let mut out = heapless_audio_toml {
        buf: [0; 96],
        len: 0,
    };
    let _ = push_str(&mut out, "[audio]\nmaster_volume = ");
    let _ = push_u8(&mut out, clamp_volume(cfg.volume));
    let _ = push_str(&mut out, "\nmuted = ");
    let _ = push_str(&mut out, if cfg.muted { "true" } else { "false" });
    let _ = push_str(&mut out, "\nlast_nonzero = ");
    let last = if cfg.last_nonzero == 0 {
        DEFAULT_VOLUME
    } else {
        clamp_volume(cfg.last_nonzero)
    };
    let _ = push_u8(&mut out, last);
    let _ = push_str(&mut out, "\n");
    out
}

fn push_str(out: &mut heapless_audio_toml, s: &str) -> Result<(), ()> {
    let bytes = s.as_bytes();
    if out.len + bytes.len() > out.buf.len() {
        return Err(());
    }
    out.buf[out.len..out.len + bytes.len()].copy_from_slice(bytes);
    out.len += bytes.len();
    Ok(())
}

fn push_u8(out: &mut heapless_audio_toml, value: u8) -> Result<(), ()> {
    let mut tmp = [0u8; 3];
    let mut n = value;
    if n == 0 {
        return push_str(out, "0");
    }
    let mut i = 0;
    while n > 0 && i < tmp.len() {
        tmp[i] = b'0' + (n % 10);
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        if out.len >= out.buf.len() {
            return Err(());
        }
        out.buf[out.len] = tmp[i];
        out.len += 1;
    }
    Ok(())
}

fn parse_u8_assignment(value: &str) -> Option<u8> {
    let rest = value.trim().strip_prefix('=')?.trim();
    let mut n: u16 = 0;
    let mut seen = false;
    for b in rest.bytes() {
        if !b.is_ascii_digit() {
            break;
        }
        seen = true;
        n = n.saturating_mul(10).saturating_add((b - b'0') as u16);
        if n > 255 {
            return Some(255);
        }
    }
    seen.then_some(n as u8)
}

fn parse_bool_assignment(value: &str) -> Option<bool> {
    let rest = value.trim().strip_prefix('=')?.trim();
    if rest.starts_with("true") || rest.starts_with('1') {
        Some(true)
    } else if rest.starts_with("false") || rest.starts_with('0') {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_volume() {
        let mut v = MasterVolume::default_live();
        v.set_volume(200);
        assert_eq!(v.volume(), 100);
        v.set_volume(0);
        assert_eq!(v.volume(), 0);
        assert_eq!(v.last_nonzero(), 100);
    }

    #[test]
    fn mute_is_independent() {
        let mut v = MasterVolume::default_live();
        v.set_volume(40);
        v.set_muted(true);
        assert_eq!(v.volume(), 40);
        assert_eq!(v.effective(), 0);
        v.set_volume(80);
        assert!(v.muted());
        assert_eq!(v.volume(), 80);
        assert_eq!(v.effective(), 0);
        v.set_muted(false);
        assert_eq!(v.effective(), 80);
    }

    #[test]
    fn toggle_restores_last_nonzero() {
        let mut v = MasterVolume::default_live();
        v.set_volume(55);
        v.toggle_mute();
        assert!(v.muted());
        assert_eq!(v.last_nonzero(), 55);
        v.toggle_mute();
        assert!(!v.muted());
        assert_eq!(v.volume(), 55);
    }

    #[test]
    fn zero_is_effective_silence() {
        let mut v = MasterVolume::default_live();
        v.set_volume(0);
        assert_eq!(v.effective(), 0);
        v.set_muted(true);
        assert_eq!(v.effective(), 0);
    }

    #[test]
    fn icon_selection() {
        assert_eq!(volume_icon(80, false, true), VolumeIconKind::High);
        assert_eq!(volume_icon(50, false, true), VolumeIconKind::Medium);
        assert_eq!(volume_icon(10, false, true), VolumeIconKind::Low);
        assert_eq!(volume_icon(80, true, true), VolumeIconKind::Off);
        assert_eq!(volume_icon(0, false, true), VolumeIconKind::Off);
        assert_eq!(volume_icon(80, false, false), VolumeIconKind::Unavailable);
    }

    #[test]
    fn persisted_round_trip() {
        let text = render_persisted_buf(PersistedAudio {
            volume: 42,
            muted: true,
            last_nonzero: 70,
        });
        let parsed = parse_persisted(text.as_str());
        assert_eq!(parsed.volume, 42);
        assert!(parsed.muted);
        assert_eq!(parsed.last_nonzero, 70);
    }

    #[test]
    fn invalid_persisted_recovers() {
        let parsed = parse_persisted("not toml at all\nmaster_volume = 9999\n");
        assert_eq!(parsed.volume, DEFAULT_VOLUME);
        let parsed = parse_persisted("[audio]\nmaster_volume = 250\nmuted = maybe\n");
        assert_eq!(parsed.volume, 100);
        assert!(!parsed.muted);
        let live = MasterVolume::from_persisted(parsed);
        assert_eq!(live.volume(), 100);
        assert!(!live.muted());
    }
}
