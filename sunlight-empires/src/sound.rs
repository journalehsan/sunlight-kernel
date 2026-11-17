//! Bounded event boundary and original integer procedural effects. Playback is
//! an optional native adapter; settings and voices cannot affect world rules.
use crate::World;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Cue {
    Selection,
    Movement,
    Started,
    Completed,
    Trained,
    Invalid,
    Melee,
    Arrow,
    Death,
    Victory,
    Defeat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundEvent {
    pub cue: Cue,
    pub tick: u64,
}
impl World {
    pub(crate) fn emit(&mut self, cue: Cue) {
        if self.events.len() < 64 {
            self.events.push(SoundEvent {
                cue,
                tick: self.tick,
            });
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub master: u8,
    pub effects: u8,
    pub music: u8,
    pub mute: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            master: 70,
            effects: 75,
            music: 30,
            mute: false,
        }
    }
}
#[derive(Clone, Copy)]
struct Voice {
    cue: Cue,
    frame: u32,
}
pub struct Mixer {
    voices: [Option<Voice>; 8],
    pub settings: Settings,
}
impl Default for Mixer {
    fn default() -> Self {
        Self {
            voices: [None; 8],
            settings: Settings::default(),
        }
    }
}
impl Mixer {
    pub fn active(&self) -> bool {
        self.voices.iter().any(Option::is_some)
    }
    pub fn clear(&mut self) {
        self.voices.fill(None);
    }
    pub fn cue(&mut self, cue: Cue) {
        if self.settings.mute
            || self
                .voices
                .iter()
                .flatten()
                .any(|v| v.cue == cue && v.frame < 2400)
        {
            return;
        }
        if let Some(slot) = self.voices.iter_mut().find(|s| s.is_none()) {
            *slot = Some(Voice { cue, frame: 0 });
        }
    }
    /// 48 kHz, stereo signed 16-bit little-endian, matching audiod's native path.
    pub fn render(&mut self, out: &mut [u8]) {
        for frame in out.chunks_exact_mut(4) {
            let mut sum = 0i32;
            for voice in &mut self.voices {
                let Some(v) = voice else { continue };
                let length = match v.cue {
                    Cue::Victory | Cue::Defeat => 24000,
                    _ => 4800,
                };
                let frequency = match v.cue {
                    Cue::Selection => 720,
                    Cue::Movement => 520,
                    Cue::Started => 260,
                    Cue::Completed => 880,
                    Cue::Trained => 660,
                    Cue::Invalid => 150,
                    Cue::Melee => 90,
                    Cue::Arrow => 1100,
                    Cue::Death => 120,
                    Cue::Victory => 660 + v.frame / 4000 * 110,
                    Cue::Defeat => 330 - v.frame / 4000 * 25,
                };
                let wave = if (v.frame * frequency / 24000) % 2 == 0 {
                    1
                } else {
                    -1
                };
                let category = match v.cue {
                    Cue::Melee | Cue::Arrow | Cue::Death => 1800,
                    _ => 2800,
                };
                sum += wave * category * ((length - v.frame) as i32) / (length as i32);
                v.frame += 1;
                if v.frame >= length {
                    *voice = None;
                }
            }
            let gain = if self.settings.mute {
                0
            } else {
                i32::from(self.settings.master.min(100)) * i32::from(self.settings.effects.min(100))
            };
            let sample = (sum * gain / 10000).clamp(-32768, 32767) as i16;
            frame[..2].copy_from_slice(&sample.to_le_bytes());
            frame[2..].copy_from_slice(&sample.to_le_bytes());
        }
    }
}
