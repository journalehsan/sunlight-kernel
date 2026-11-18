//! Presentation-only music policy and bounded PCM helpers. No World dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Track {
    Menu,
    Peace,
    Combat,
    Silent,
}
impl Track {
    pub fn filename(self) -> Option<&'static str> {
        match self {
            Self::Menu => Some("menu.wav"),
            Self::Peace => Some("peace.wav"),
            Self::Combat => Some("combat.wav"),
            Self::Silent => None,
        }
    }
}
pub struct Policy {
    pub track: Track,
    combat_until: u64,
    switched: u64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            track: Track::Menu,
            combat_until: 0,
            switched: 0,
        }
    }
}
impl Policy {
    /// Wall-clock hysteresis is deliberately outside authoritative game time.
    pub fn update(&mut self, now: u64, menu: bool, combat: bool, terminal: bool) -> Track {
        if combat {
            self.combat_until = now.saturating_add(12_000);
        }
        let desired = if menu {
            Track::Menu
        } else if terminal {
            Track::Silent
        } else if now < self.combat_until {
            Track::Combat
        } else {
            Track::Peace
        };
        if menu
            || terminal
            || self.track == Track::Menu
            || self.track == Track::Silent
            || now.saturating_sub(self.switched) >= 8_000
        {
            if desired != self.track {
                self.track = desired;
                self.switched = now;
            }
        }
        if menu || terminal {
            self.combat_until = 0;
        }
        self.track
    }
}
/// The reproducible asset converter emits canonical 44-byte RIFF PCM headers.
/// Other WAV variants are unsupported and fail closed, rather than misplaying.
pub fn pcm_bytes(header: &[u8; 44]) -> Option<u32> {
    let u16_at = |n| u16::from_le_bytes([header[n], header[n + 1]]);
    let u32_at = |n| u32::from_le_bytes(header[n..n + 4].try_into().unwrap());
    let size = u32_at(40);
    if &header[..4] != b"RIFF"
        || &header[8..16] != b"WAVEfmt "
        || u32_at(16) != 16
        || u16_at(20) != 1
        || u16_at(22) != 2
        || u32_at(24) != 48000
        || u32_at(28) != 192000
        || u16_at(32) != 4
        || u16_at(34) != 16
        || &header[36..40] != b"data"
        || size == 0
        || size % 4 != 0
        || size > 192000 * 600
        || u32_at(4) != size.checked_add(36)?
    {
        return None;
    }
    Some(size)
}
pub fn mix(sfx: &mut [u8], music: &[u8], master: u8, volume: u8) {
    let gain = i32::from(master.min(100)) * i32::from(volume.min(100));
    for (out, source) in sfx.chunks_exact_mut(2).zip(music.chunks_exact(2)) {
        let a = i32::from(i16::from_le_bytes([out[0], out[1]]));
        let b = i32::from(i16::from_le_bytes([source[0], source[1]]));
        out.copy_from_slice(&((a + b * gain / 10000).clamp(-32768, 32767) as i16).to_le_bytes());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hysteresis_and_menu_reset() {
        let mut p = Policy::default();
        assert_eq!(p.update(0, false, false, false), Track::Peace);
        assert_eq!(p.update(100, false, true, false), Track::Peace);
        assert_eq!(p.update(8000, false, true, false), Track::Combat);
        assert_eq!(p.update(19000, false, false, false), Track::Combat);
        assert_eq!(p.update(20000, false, false, false), Track::Peace);
        assert_eq!(p.update(20001, true, false, false), Track::Menu);
        assert_eq!(p.update(20002, false, false, false), Track::Peace);
        assert_eq!(p.update(20003, false, false, true), Track::Silent);
    }
    #[test]
    fn gain_and_saturation() {
        let mut a = 30000i16.to_le_bytes();
        mix(&mut a, &20000i16.to_le_bytes(), 100, 100);
        assert_eq!(i16::from_le_bytes(a), 32767);
        let mut a = [0; 2];
        mix(&mut a, &20000i16.to_le_bytes(), 50, 50);
        assert_eq!(i16::from_le_bytes(a), 5000);
        mix(&mut a, &20000i16.to_le_bytes(), 0, 100);
        assert_eq!(i16::from_le_bytes(a), 5000);
    }
    #[test]
    fn canonical_header_and_corruption() {
        let mut h = [0; 44];
        h[..4].copy_from_slice(b"RIFF");
        h[4..8].copy_from_slice(&40u32.to_le_bytes());
        h[8..16].copy_from_slice(b"WAVEfmt ");
        h[16..20].copy_from_slice(&16u32.to_le_bytes());
        h[20..22].copy_from_slice(&1u16.to_le_bytes());
        h[22..24].copy_from_slice(&2u16.to_le_bytes());
        h[24..28].copy_from_slice(&48000u32.to_le_bytes());
        h[28..32].copy_from_slice(&192000u32.to_le_bytes());
        h[32..34].copy_from_slice(&4u16.to_le_bytes());
        h[34..36].copy_from_slice(&16u16.to_le_bytes());
        h[36..40].copy_from_slice(b"data");
        h[40..44].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(pcm_bytes(&h), Some(4));
        for n in [0, 4, 8, 16, 20, 22, 24, 28, 32, 34, 36, 40] {
            let mut corrupt = h;
            corrupt[n] ^= 1;
            assert_eq!(pcm_bytes(&corrupt), None, "offset {n}");
        }
    }
}
