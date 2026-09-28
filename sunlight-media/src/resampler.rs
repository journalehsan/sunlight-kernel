//! Streaming linear conversion from mono/stereo S16 to the native stereo clock.

use crate::error::MediaError;

pub struct LinearResampler {
    input_rate: u32,
    output_rate: u32,
    input_frames: u64,
    output_frames: u64,
    previous: [i16; 2],
}

impl LinearResampler {
    pub const fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            input_rate,
            output_rate,
            input_frames: 0,
            output_frames: 0,
            previous: [0; 2],
        }
    }

    pub fn reset(&mut self) {
        self.input_frames = 0;
        self.output_frames = 0;
        self.previous = [0; 2];
    }

    pub fn process(
        &mut self,
        samples: &[i16],
        channels: usize,
        mut emit: impl FnMut([i16; 2]) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        for frame in samples.chunks_exact(channels) {
            let current = [frame[0], frame[if channels == 1 { 0 } else { 1 }]];
            let index = self.input_frames;
            if index != 0 {
                let end = index * self.output_rate as u64;
                while self.output_frames * (self.input_rate as u64) < end {
                    let numerator = self.output_frames * self.input_rate as u64
                        - (index - 1) * self.output_rate as u64;
                    emit(interpolate(
                        self.previous,
                        current,
                        numerator,
                        self.output_rate,
                    ))?;
                    self.output_frames += 1;
                }
            }
            self.previous = current;
            self.input_frames += 1;
        }
        Ok(())
    }

    /// Emit the final held frame for output positions within the source extent.
    pub fn finish(
        &mut self,
        mut emit: impl FnMut([i16; 2]) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        let end = self.input_frames * self.output_rate as u64;
        while self.output_frames * (self.input_rate as u64) < end {
            emit(self.previous)?;
            self.output_frames += 1;
        }
        Ok(())
    }
}

fn interpolate(a: [i16; 2], b: [i16; 2], numerator: u64, denominator: u32) -> [i16; 2] {
    let fraction = numerator as i64;
    let whole = denominator as i64;
    core::array::from_fn(|channel| {
        (a[channel] as i64 + (b[channel] as i64 - a[channel] as i64) * fraction / whole) as i16
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn converts_44100_to_48000_across_irregular_chunks() {
        let mut resampler = LinearResampler::new(44_100, 48_000);
        let input: Vec<i16> = (0..441).map(|i| i * 50).collect();
        let mut output = Vec::new();
        for chunk in input.chunks(37) {
            resampler
                .process(chunk, 1, |frame| {
                    output.push(frame);
                    Ok(())
                })
                .unwrap();
        }
        resampler
            .finish(|frame| {
                output.push(frame);
                Ok(())
            })
            .unwrap();
        assert_eq!(output.len(), 480);
        assert_eq!(output[0], [0, 0]);
        assert_eq!(output[479], [input[440], input[440]]);
        assert!(output.windows(2).all(|pair| pair[0][0] <= pair[1][0]));
    }

    #[test]
    fn native_rate_is_bit_exact_and_reset_discards_old_samples() {
        let mut resampler = LinearResampler::new(48_000, 48_000);
        let mut output = Vec::new();
        resampler
            .process(&[12, -34, 56, -78], 2, |frame| {
                output.push(frame);
                Ok(())
            })
            .unwrap();
        resampler
            .finish(|frame| {
                output.push(frame);
                Ok(())
            })
            .unwrap();
        assert_eq!(output, [[12, -34], [56, -78]]);
        resampler.reset();
        output.clear();
        resampler
            .process(&[90], 1, |frame| {
                output.push(frame);
                Ok(())
            })
            .unwrap();
        resampler
            .finish(|frame| {
                output.push(frame);
                Ok(())
            })
            .unwrap();
        assert_eq!(output, [[90, 90]]);
    }

    #[test]
    fn downsampling_preserves_duration_and_channel_order() {
        let mut resampler = LinearResampler::new(96_000, 48_000);
        let mut output = Vec::new();
        resampler
            .process(&[10, -10, 20, -20, 30, -30, 40, -40], 2, |frame| {
                output.push(frame);
                Ok(())
            })
            .unwrap();
        resampler
            .finish(|frame| {
                output.push(frame);
                Ok(())
            })
            .unwrap();
        assert_eq!(output, [[10, -10], [30, -30]]);
    }
}
