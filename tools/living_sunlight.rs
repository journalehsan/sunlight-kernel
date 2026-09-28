//! Render a short, deterministic preview of "The Living Sunlight" for Melody Mina.
//!
//! Host build: rustc --edition=2021 -O tools/living_sunlight.rs -o /tmp/living-sunlight
//! Usage: /tmp/living-sunlight assets/sounds/the-living-sunlight-48k.wav
//!
//! The synth has no dependency on files or allocation in render_sample(). A
//! future audio worker can share Controls with a telemetry producer and call
//! render_sample() repeatedly without holding a lock.

use std::f32::consts::TAU;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

const RATE: u32 = 48_000;
const SECONDS: u32 = 18;
const FRAMES: u32 = RATE * SECONDS;

#[derive(Clone, Copy)]
struct SystemState {
    cpu_usage: f32,
    memory_pressure: f32,
    entropy_seed: u64,
}

struct Controls {
    cpu_bits: AtomicU32,
    memory_bits: AtomicU32,
    entropy_seed: AtomicU64,
}

impl Controls {
    fn new(state: SystemState) -> Self {
        Self {
            cpu_bits: AtomicU32::new(state.cpu_usage.clamp(0.0, 1.0).to_bits()),
            memory_bits: AtomicU32::new(state.memory_pressure.clamp(0.0, 1.0).to_bits()),
            entropy_seed: AtomicU64::new(state.entropy_seed),
        }
    }

    // Independent relaxed stores are enough: each value is bounded and the
    // audio side smooths continuous parameters before applying them.
    fn update(&self, state: SystemState) {
        self.cpu_bits
            .store(state.cpu_usage.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        self.memory_bits.store(
            state.memory_pressure.clamp(0.0, 1.0).to_bits(),
            Ordering::Relaxed,
        );
        self.entropy_seed
            .store(state.entropy_seed, Ordering::Relaxed);
    }

    fn snapshot(&self) -> SystemState {
        SystemState {
            cpu_usage: f32::from_bits(self.cpu_bits.load(Ordering::Relaxed)),
            memory_pressure: f32::from_bits(self.memory_bits.load(Ordering::Relaxed)),
            entropy_seed: self.entropy_seed.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy)]
enum Wave {
    Sine,
    Square,
    Triangle,
    Noise,
}

#[derive(Clone, Copy)]
struct Adsr {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

impl Adsr {
    fn level(self, age: f32, gate: f32) -> f32 {
        let held = if age < self.attack {
            age / self.attack
        } else if age < self.attack + self.decay {
            1.0 - (1.0 - self.sustain) * (age - self.attack) / self.decay
        } else {
            self.sustain
        };
        if age <= gate {
            held
        } else if age >= gate + self.release {
            0.0
        } else {
            // All notes here hold longer than attack + decay, so release
            // starts at the sustain level and reaches exactly zero.
            self.sustain * (1.0 - (age - gate) / self.release).max(0.0)
        }
    }
}

#[derive(Clone, Copy)]
struct Voice {
    wave: Wave,
    phase: f32,
    mod_phase: f32,
    frequency: f32,
    amplitude: f32,
    pan: f32,
    pulse_width: f32,
    age: f32,
    gate: f32,
    envelope: Adsr,
    active: bool,
}

impl Voice {
    const fn silent() -> Self {
        Self {
            wave: Wave::Sine,
            phase: 0.0,
            mod_phase: 0.0,
            frequency: 0.0,
            amplitude: 0.0,
            pan: 0.5,
            pulse_width: 0.5,
            age: 0.0,
            gate: 0.0,
            envelope: Adsr {
                attack: 0.01,
                decay: 0.1,
                sustain: 0.5,
                release: 0.1,
            },
            active: false,
        }
    }

    fn start(
        &mut self,
        wave: Wave,
        midi: i32,
        amplitude: f32,
        pan: f32,
        gate: f32,
        envelope: Adsr,
    ) {
        *self = Self {
            wave,
            phase: 0.0,
            mod_phase: 0.0,
            frequency: 440.0 * 2.0_f32.powf((midi as f32 - 69.0) / 12.0),
            amplitude,
            pan,
            pulse_width: 0.43,
            age: 0.0,
            gate,
            envelope,
            active: true,
        };
    }

    fn sample(&mut self, memory: f32, noise: &mut u32) -> f32 {
        if !self.active {
            return 0.0;
        }
        let level = self.envelope.level(self.age, self.gate);
        if level <= 0.0 && self.age > self.gate {
            self.active = false;
            return 0.0;
        }
        // A phase in [0, 1) avoids large angles and stays stable indefinitely.
        self.phase = (self.phase + self.frequency / RATE as f32).fract();
        self.mod_phase = (self.mod_phase + 2.0 * self.frequency / RATE as f32).fract();
        let raw = match self.wave {
            Wave::Sine => {
                // Two-operator FM adds sidebands as memory pressure rises.
                (TAU * self.phase + memory * 0.85 * (TAU * self.mod_phase).sin()).sin()
            }
            Wave::Square => {
                let fundamental = if self.phase < self.pulse_width {
                    1.0
                } else {
                    -1.0
                };
                // A soft triangle layer keeps the lead clear at low pressure.
                let triangle = 1.0 - 4.0 * (self.phase - 0.5).abs();
                triangle * (1.0 - memory * 0.55) + fundamental * memory * 0.55
            }
            Wave::Triangle => 1.0 - 4.0 * (self.phase - 0.5).abs(),
            Wave::Noise => {
                // LCG white noise; high bits have better statistical quality.
                *noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((*noise >> 8) as f32 / 8_388_608.0) - 1.0
            }
        };
        self.age += 1.0 / RATE as f32;
        raw * level * self.amplitude
    }
}

struct Synth {
    controls: Arc<Controls>,
    voices: [Voice; 7],
    noise: u32,
    variation: u64,
    step: usize,
    samples_until_step: f32,
    cpu: f32,
    memory: f32,
    filter_l: f32,
    filter_r: f32,
}

impl Synth {
    fn new(controls: Arc<Controls>) -> Self {
        let state = controls.snapshot();
        Self {
            controls,
            voices: [Voice::silent(); 7],
            noise: state.entropy_seed as u32,
            variation: state.entropy_seed,
            step: 0,
            samples_until_step: 0.0,
            cpu: state.cpu_usage,
            memory: state.memory_pressure,
            filter_l: 0.0,
            filter_r: 0.0,
        }
    }

    fn next_random(&mut self) -> u64 {
        self.variation ^= self.variation << 13;
        self.variation ^= self.variation >> 7;
        self.variation ^= self.variation << 17;
        self.variation
    }

    fn advance_step(&mut self, state: SystemState) {
        // The first three notes retain Mozart 40's Eb-D-D (flat 6, 5, 5)
        // gesture in G minor. Later notes may move only to neighboring scale
        // degrees; F# is the harmonic-minor leading tone into G.
        const MOTIF: [i32; 12] = [75, 74, 74, 75, 74, 74, 79, 78, 79, 72, 74, 67];
        const SCALE: [i32; 11] = [67, 69, 70, 72, 74, 75, 78, 79, 81, 82, 84];
        const ROOTS: [i32; 4] = [43, 39, 36, 38]; // G2, Eb2, C2, D2
        let slot = self.step % MOTIF.len();
        if slot == 0 && state.entropy_seed != self.variation {
            self.variation ^= state.entropy_seed | 1;
        }
        let beat_seconds = 60.0 / (100.0 + 60.0 * self.cpu);
        let beats = if slot % 3 == 2 { 1.0 } else { 0.5 };
        let step_seconds = beat_seconds * beats;
        self.samples_until_step += step_seconds * RATE as f32;
        let mut note = MOTIF[slot];
        if slot >= 3 && self.next_random() % 5 == 0 {
            if let Some(index) = SCALE.iter().position(|&pitch| pitch == note) {
                let neighbor = if self.next_random() & 1 == 0 {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(SCALE.len() - 1)
                };
                note = SCALE[neighbor];
            }
        }
        self.voices[0].start(
            Wave::Square,
            note,
            0.23,
            0.38,
            step_seconds * 0.76,
            Adsr {
                attack: 0.006,
                decay: 0.08,
                sustain: 0.62,
                release: 0.08,
            },
        );

        let chord = (self.step / 3) % 4;
        let root = ROOTS[chord];
        if slot % 3 == 0 {
            self.voices[1].start(
                Wave::Triangle,
                root,
                0.22,
                0.52,
                beat_seconds * 1.55,
                Adsr {
                    attack: 0.01,
                    decay: 0.22,
                    sustain: 0.48,
                    release: 0.12,
                },
            );
            let chord_notes = match chord {
                0 => [55, 58, 62], // Gm: G, Bb, D
                1 => [51, 55, 58], // Eb: Eb, G, Bb
                2 => [48, 51, 55], // Cm: C, Eb, G
                _ => [50, 54, 57], // D: D, F#, A
            };
            for (voice, pitch) in self.voices[3..6].iter_mut().zip(chord_notes) {
                voice.start(
                    Wave::Sine,
                    pitch,
                    0.047,
                    0.5,
                    beat_seconds * 1.6,
                    Adsr {
                        attack: 0.15,
                        decay: 0.35,
                        sustain: 0.56,
                        release: 0.15,
                    },
                );
            }
            // A quiet, filtered noise tap marks the pulse without overpowering the motif.
            self.voices[6].start(
                Wave::Noise,
                0,
                0.07,
                0.5,
                0.022,
                Adsr {
                    attack: 0.001,
                    decay: 0.02,
                    sustain: 0.2,
                    release: 0.03,
                },
            );
        }
        let arpeggio = [root + 12, root + 19, root + 24][slot % 3];
        self.voices[2].start(
            Wave::Triangle,
            arpeggio,
            0.075,
            if slot % 2 == 0 { 0.24 } else { 0.76 },
            step_seconds * 0.5,
            Adsr {
                attack: 0.005,
                decay: 0.1,
                sustain: 0.28,
                release: 0.12,
            },
        );
        self.step += 1;
    }

    fn render_sample(&mut self) -> [f32; 2] {
        let state = self.controls.snapshot();
        // 20 ms smoothing avoids abrupt filter/gain changes at telemetry updates.
        let smoothing = 1.0 / (0.02 * RATE as f32);
        self.cpu += (state.cpu_usage - self.cpu) * smoothing;
        self.memory += (state.memory_pressure - self.memory) * smoothing;
        if self.samples_until_step <= 0.0 {
            self.advance_step(state);
        }
        self.samples_until_step -= 1.0;

        let mut left = 0.0;
        let mut right = 0.0;
        for voice in &mut self.voices {
            let sample = voice.sample(self.memory, &mut self.noise);
            left += sample * (1.0 - voice.pan);
            right += sample * voice.pan;
        }
        // One-pole RC low-pass: y[n] = y[n-1] + alpha*(x[n]-y[n-1]).
        // CPU load opens the filter, while memory pressure adds a gentle drive.
        let cutoff = 1_450.0 + self.cpu * 5_800.0;
        let alpha = (TAU * cutoff / RATE as f32) / (1.0 + TAU * cutoff / RATE as f32);
        self.filter_l += alpha * (left - self.filter_l);
        self.filter_r += alpha * (right - self.filter_r);
        let drive = 1.0 + self.memory * 0.7;
        [
            (self.filter_l * drive).tanh(),
            (self.filter_r * drive).tanh(),
        ]
    }
}

fn write_wav(path: &str) -> std::io::Result<()> {
    let controls = Arc::new(Controls::new(SystemState {
        cpu_usage: 0.32,
        memory_pressure: 0.24,
        entropy_seed: 0x51_4e_4c_49_47_48_54,
    }));
    let mut synth = Synth::new(Arc::clone(&controls));
    let mut out = BufWriter::new(File::create(path)?);
    let data_len = FRAMES * 4;
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16_u32.to_le_bytes())?;
    out.write_all(&1_u16.to_le_bytes())?; // integer PCM
    out.write_all(&2_u16.to_le_bytes())?;
    out.write_all(&RATE.to_le_bytes())?;
    out.write_all(&(RATE * 4).to_le_bytes())?;
    out.write_all(&4_u16.to_le_bytes())?;
    out.write_all(&16_u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    for frame in 0..FRAMES {
        if frame == 6 * RATE {
            controls.update(SystemState {
                cpu_usage: 0.78,
                memory_pressure: 0.7,
                entropy_seed: 0x51_4e_4c_49_47_48_55,
            });
        } else if frame == 12 * RATE {
            controls.update(SystemState {
                cpu_usage: 0.43,
                memory_pressure: 0.35,
                entropy_seed: 0x51_4e_4c_49_47_48_56,
            });
        }
        let [left, right] = synth.render_sample();
        // A short master fade prevents clicks at the WAV boundaries.
        let fade = (frame as f32 / (RATE as f32 * 0.06))
            .min((FRAMES - 1 - frame) as f32 / (RATE as f32 * 0.15))
            .clamp(0.0, 1.0);
        for sample in [left, right] {
            let pcm = (sample * fade * 0.88 * i16::MAX as f32).round() as i16;
            out.write_all(&pcm.to_le_bytes())?;
        }
    }
    out.flush()
}

fn main() -> std::io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: living-sunlight OUTPUT.wav");
    write_wav(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesizer_is_bounded_and_responds_to_telemetry() {
        let controls = Arc::new(Controls::new(SystemState {
            cpu_usage: 0.0,
            memory_pressure: 0.0,
            entropy_seed: 7,
        }));
        let mut synth = Synth::new(Arc::clone(&controls));
        let first = synth.render_sample();
        controls.update(SystemState {
            cpu_usage: 1.0,
            memory_pressure: 1.0,
            entropy_seed: 8,
        });
        for _ in 0..RATE {
            let sample = synth.render_sample();
            assert!(sample
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 1.0));
        }
        assert!(first.iter().all(|value| value.is_finite()));
        assert!(synth.cpu > 0.99 && synth.memory > 0.99);
    }

    #[test]
    fn adsr_releases_to_zero() {
        let env = Adsr {
            attack: 0.01,
            decay: 0.1,
            sustain: 0.5,
            release: 0.2,
        };
        assert_eq!(env.level(0.0, 0.5), 0.0);
        assert!(env.level(0.2, 0.5) > 0.0);
        assert_eq!(env.level(0.7, 0.5), 0.0);
    }
}
