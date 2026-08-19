pub const MAX_VISUALIZATION_BINS: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualizationFrame {
    pub bins: [u8; MAX_VISUALIZATION_BINS],
    pub len: u8,
}

impl VisualizationFrame {
    pub const fn empty() -> Self {
        Self {
            bins: [0; MAX_VISUALIZATION_BINS],
            len: 0,
        }
    }

    pub fn bins(&self) -> &[u8] {
        &self.bins[..self.len.min(MAX_VISUALIZATION_BINS as u8) as usize]
    }

    pub fn analyze_s16_stereo(&mut self, pcm: &[u8], requested_bins: usize) {
        let frames = pcm.len() / 4;
        let count = requested_bins
            .clamp(1, MAX_VISUALIZATION_BINS)
            .min(frames.max(1));
        self.len = count as u8;
        for index in 0..count {
            let start = index * frames / count;
            let end = ((index + 1) * frames / count).max(start + 1).min(frames);
            let mut peak = 0u32;
            for frame in start..end {
                let offset = frame * 4;
                let left = i16::from_le_bytes([pcm[offset], pcm[offset + 1]]);
                let right = i16::from_le_bytes([pcm[offset + 2], pcm[offset + 3]]);
                peak = peak
                    .max(left.unsigned_abs() as u32)
                    .max(right.unsigned_abs() as u32);
            }
            let target = (peak.saturating_mul(100) / i16::MAX as u32).min(100) as u8;
            let current = self.bins[index];
            self.bins[index] = if target > current {
                target
            } else {
                current.saturating_sub(6).max(target)
            };
        }
        self.bins[count..].fill(0);
    }
}

impl Default for VisualizationFrame {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_is_bounded_and_tracks_real_peak() {
        let mut frame = VisualizationFrame::empty();
        let pcm = [0xff, 0x7f, 0, 0, 0, 0, 0, 0];
        frame.analyze_s16_stereo(&pcm, 99);
        assert_eq!(frame.len, 2);
        assert_eq!(frame.bins[0], 100);
        assert_eq!(frame.bins[1], 0);
    }
}
