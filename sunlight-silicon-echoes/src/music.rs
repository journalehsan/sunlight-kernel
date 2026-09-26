//! A small, deterministic 120 BPM soundtrack in audiod's native PCM format.
//! The composition loops every eight bars without storing a large audio asset.

use sunlight_audio::pcm::sine_s16;

pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAMES_PER_STEP: u64 = 12_000; // eighth note at 120 BPM
pub const LOOP_FRAMES: u64 = 64 * FRAMES_PER_STEP;

// Eight bars over C, G, Am, F, then a brighter second pass.
const MELODY: [u16; 64] = [
    659, 0, 784, 0, 880, 784, 659, 587, 784, 0, 659, 587, 494, 0, 587, 0, 659, 0, 880, 0, 784, 659,
    523, 0, 698, 0, 659, 587, 523, 0, 0, 0, 784, 0, 880, 0, 988, 880, 784, 659, 784, 0, 659, 587,
    784, 0, 587, 0, 880, 0, 784, 659, 523, 0, 659, 0, 698, 659, 587, 523, 0, 0, 0, 0,
];
const CHORDS: [[u16; 3]; 4] = [
    [262, 330, 392], // C
    [196, 247, 392], // G
    [220, 262, 330], // Am
    [175, 220, 349], // F
];
const BASS: [u16; 4] = [131, 98, 110, 87];
const ARPEGGIO: [usize; 8] = [0, 1, 2, 1, 0, 1, 2, 1];

fn note(freq: u16, frame: u64) -> i32 {
    let phase_step = ((freq as u64) << 32) / SAMPLE_RATE as u64;
    sine_s16(frame.wrapping_mul(phase_step) as u32) as i32
}

fn envelope(frame: u64, length: u64, attack: u64) -> i32 {
    if frame >= length {
        return 0;
    }
    let attack_level = (frame * 1024 / attack).min(1024);
    (attack_level * (length - frame) / length) as i32
}

fn sample(frame: u64) -> (i16, i16) {
    let loop_frame = frame % LOOP_FRAMES;
    let step = (loop_frame / FRAMES_PER_STEP) as usize;
    let local = loop_frame % FRAMES_PER_STEP;
    let chord = CHORDS[(step / 8) % 4];

    let melody = MELODY[step];
    let lead = if melody == 0 {
        0
    } else {
        let voice = note(melody, local) * 4 + note(melody * 2, local);
        voice * envelope(local, FRAMES_PER_STEP - 500, 240) / 1024 / 14
    };
    let arp = note(chord[ARPEGGIO[step % 8]], local) * envelope(local, FRAMES_PER_STEP - 300, 120)
        / 1024
        / 13;
    let beat_frame = loop_frame % (2 * FRAMES_PER_STEP);
    let bass = note(BASS[(step / 8) % 4], beat_frame)
        * envelope(beat_frame, 2 * FRAMES_PER_STEP - 800, 350)
        / 1024
        / 17;
    let kick = if step % 4 == 0 {
        note(70, local) * envelope(local, 4_000, 80) / 1024 / 18
    } else {
        0
    };
    // A short, quiet deterministic shaker marks each eighth note.
    let noise = (loop_frame as u32)
        .wrapping_mul(0x9e37_79b9)
        .rotate_left(13)
        .wrapping_mul(0x85eb_ca6b);
    let noise_sample = (noise >> 16) as i32 - 32_768;
    let hat = noise_sample * envelope(local, 650, 40) / 1024 / 40;
    let snare = if step % 4 == 2 {
        noise_sample * envelope(local, 3_000, 40) / 1024 / 22
    } else {
        0
    };

    let left = lead + arp * 3 / 4 + bass + kick + snare + hat;
    let right = lead * 4 / 5 + arp + bass + kick + snare - hat;
    (left as i16, right as i16)
}

/// Fill stereo S16LE frames. Callers advance `start_frame` only after audiod
/// accepts the entire chunk, so queue backpressure never skips a note.
pub fn render(out: &mut [u8], start_frame: u64) {
    assert_eq!(out.len() % 4, 0);
    for (index, frame) in out.chunks_exact_mut(4).enumerate() {
        let (left, right) = sample(start_frame + index as u64);
        frame[..2].copy_from_slice(&left.to_le_bytes());
        frame[2..].copy_from_slice(&right.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_and_loop_wrap_preserve_the_waveform() {
        let start = LOOP_FRAMES - 512;
        let mut whole = [0u8; 4096];
        let mut split = [0u8; 4096];
        render(&mut whole, start);
        render(&mut split[..2048], start);
        render(&mut split[2048..], start + 512);
        assert_eq!(whole, split);
        let mut beginning = [0u8; 2048];
        render(&mut beginning, 0);
        assert_eq!(&whole[2048..], &beginning);
        assert!(whole.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn mix_stays_below_clipping_and_returns_to_silence_at_loop_end() {
        let mut peak = 0i32;
        for frame in 0..LOOP_FRAMES {
            let (left, right) = sample(frame);
            peak = peak.max((left as i32).abs()).max((right as i32).abs());
        }
        assert!(peak > 1000);
        assert!(peak < 20_000);
        assert_eq!(sample(LOOP_FRAMES - 1), (0, 0));
        assert_eq!(sample(LOOP_FRAMES), sample(0));
    }
}
