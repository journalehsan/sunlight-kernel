//! PCM format, gain, and a compact 440 Hz test-tone generator.

pub const NATIVE_RATE_HZ: u32 = 48_000;
pub const NATIVE_CHANNELS: u8 = 2;
pub const NATIVE_SAMPLE_BITS: u8 = 16;
pub const FRAME_BYTES: usize = 4; // S16LE stereo
pub const MAX_PCM_BYTES: usize = 64 * 1024;
pub const MAX_SUBMIT_FRAMES: usize = MAX_PCM_BYTES / FRAME_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub bits_per_sample: u8,
    pub signed: bool,
}

impl AudioFormat {
    pub const NATIVE: Self = Self {
        sample_rate_hz: NATIVE_RATE_HZ,
        channels: NATIVE_CHANNELS,
        bits_per_sample: NATIVE_SAMPLE_BITS,
        signed: true,
    };

    pub const fn frame_bytes(self) -> usize {
        (self.channels as usize) * (self.bits_per_sample as usize / 8)
    }

    pub const fn is_native(self) -> bool {
        self.sample_rate_hz == NATIVE_RATE_HZ
            && self.channels == NATIVE_CHANNELS
            && self.bits_per_sample == NATIVE_SAMPLE_BITS
            && self.signed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    UnsupportedFormat,
    BufferTooLarge,
    BufferUnaligned,
    EmptyBuffer,
    DeviceUnavailable,
    DeviceFailed,
    Timeout,
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmValidation {
    Ok { frames: usize },
    Err(AudioError),
}

/// A borrowed PCM view. Callers never hand physical DMA addresses through this.
#[derive(Clone, Copy, Debug)]
pub struct AudioBuffer<'a> {
    pub bytes: &'a [u8],
    pub format: AudioFormat,
}

pub fn validate_pcm(buf: AudioBuffer<'_>) -> PcmValidation {
    if !buf.format.is_native() {
        return PcmValidation::Err(AudioError::UnsupportedFormat);
    }
    if buf.bytes.is_empty() {
        return PcmValidation::Err(AudioError::EmptyBuffer);
    }
    if buf.bytes.len() > MAX_PCM_BYTES {
        return PcmValidation::Err(AudioError::BufferTooLarge);
    }
    if buf.bytes.len() % FRAME_BYTES != 0 {
        return PcmValidation::Err(AudioError::BufferUnaligned);
    }
    PcmValidation::Ok {
        frames: buf.bytes.len() / FRAME_BYTES,
    }
}

/// Integer gain: `sample * volume / 100`. Volume 0 or mute must pass 0.
pub fn apply_gain(sample: i16, volume: u8) -> i16 {
    if volume == 0 {
        return 0;
    }
    let vol = if volume > 100 { 100 } else { volume } as i32;
    ((sample as i32) * vol / 100) as i16
}

/// Apply gain in place to packed S16LE stereo.
pub fn apply_gain_s16le(bytes: &mut [u8], volume: u8) {
    let mut i = 0;
    while i + 1 < bytes.len() {
        let sample = i16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let scaled = apply_gain(sample, volume);
        let out = scaled.to_le_bytes();
        bytes[i] = out[0];
        bytes[i + 1] = out[1];
        i += 2;
    }
}

/// Quarter-wave table, 0..=π/2 inclusive. Amplitude 32767.
const QUARTER: [i16; 65] = [
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602, 6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530, 18204, 18868, 19519, 20159, 20787,
    21403, 22005, 22594, 23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790, 27245, 27683,
    28105, 28510, 28898, 29268, 29621, 29956, 30273, 30571, 30852, 31113, 31356, 31580, 31785,
    31971, 32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757, 32767,
];

/// Phase is a 32-bit fraction of a cycle (0 == 0, u32::MAX ~= 2π).
pub fn sine_s16(phase: u32) -> i16 {
    let idx = (phase >> 24) as usize; // 0..255
    let quad = idx >> 6;
    let mut pos = idx & 63;
    if quad == 1 || quad == 3 {
        pos = 64 - pos;
    }
    let sample = QUARTER[pos.min(64)];
    if quad >= 2 {
        sample.wrapping_neg()
    } else {
        sample
    }
}

pub fn phase_increment(freq_hz: u32, rate_hz: u32) -> u32 {
    if rate_hz == 0 {
        return 0;
    }
    (((freq_hz as u64) << 32) / rate_hz as u64) as u32
}

/// Fill `out` with interleaved S16LE stereo sine. Returns frames written.
pub fn generate_sine_s16le_stereo(
    out: &mut [u8],
    mut phase: u32,
    freq_hz: u32,
    rate_hz: u32,
    volume: u8,
) -> (u32, usize) {
    let inc = phase_increment(freq_hz, rate_hz);
    let frames = out.len() / FRAME_BYTES;
    for frame in 0..frames {
        let s = apply_gain(sine_s16(phase), volume);
        let bytes = s.to_le_bytes();
        let off = frame * FRAME_BYTES;
        out[off] = bytes[0];
        out[off + 1] = bytes[1];
        out[off + 2] = bytes[0];
        out[off + 3] = bytes[1];
        phase = phase.wrapping_add(inc);
    }
    (phase, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_format() {
        let bytes = [0u8; 8];
        let bad = AudioFormat {
            sample_rate_hz: 44100,
            ..AudioFormat::NATIVE
        };
        assert_eq!(
            validate_pcm(AudioBuffer {
                bytes: &bytes,
                format: bad
            }),
            PcmValidation::Err(AudioError::UnsupportedFormat)
        );
    }

    #[test]
    fn rejects_malformed_length() {
        let bytes = [0u8; 3];
        assert_eq!(
            validate_pcm(AudioBuffer {
                bytes: &bytes,
                format: AudioFormat::NATIVE
            }),
            PcmValidation::Err(AudioError::BufferUnaligned)
        );
        assert_eq!(
            validate_pcm(AudioBuffer {
                bytes: &[],
                format: AudioFormat::NATIVE
            }),
            PcmValidation::Err(AudioError::EmptyBuffer)
        );
        let huge = std::vec![0u8; MAX_PCM_BYTES + FRAME_BYTES];
        assert_eq!(
            validate_pcm(AudioBuffer {
                bytes: &huge,
                format: AudioFormat::NATIVE
            }),
            PcmValidation::Err(AudioError::BufferTooLarge)
        );
    }

    #[test]
    fn accepts_native_pcm() {
        let bytes = [0u8; 16];
        assert_eq!(
            validate_pcm(AudioBuffer {
                bytes: &bytes,
                format: AudioFormat::NATIVE
            }),
            PcmValidation::Ok { frames: 4 }
        );
    }

    #[test]
    fn gain_zero_is_silence() {
        assert_eq!(apply_gain(12345, 0), 0);
        assert_eq!(apply_gain(-20000, 100), -20000);
        assert_eq!(apply_gain(1000, 50), 500);
    }

    #[test]
    fn sine_is_bounded_and_nonzero() {
        let mut buf = [0u8; 480];
        let (phase, frames) = generate_sine_s16le_stereo(&mut buf, 0, 440, 48_000, 100);
        assert_eq!(frames, 120);
        assert_ne!(phase, 0);
        assert!(buf.iter().any(|&b| b != 0));
        let muted = {
            let mut silent = [0u8; 480];
            generate_sine_s16le_stereo(&mut silent, 0, 440, 48_000, 0);
            silent
        };
        assert!(muted.iter().all(|&b| b == 0));
    }
}
