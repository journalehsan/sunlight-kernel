//! Bounded visualization input and a lightweight demo producer.

pub const MAX_VISUALIZATION_BINS: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualizationFrame {
    bins: [u8; MAX_VISUALIZATION_BINS],
    len: u8,
}

impl VisualizationFrame {
    pub const fn empty() -> Self {
        Self {
            bins: [0; MAX_VISUALIZATION_BINS],
            len: 0,
        }
    }

    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(MAX_VISUALIZATION_BINS) as u8;
    }

    pub fn bins(&self) -> &[u8] {
        &self.bins[..self.len as usize]
    }

    pub fn set_bin(&mut self, index: usize, amplitude: u8) {
        if index < self.len as usize {
            self.bins[index] = amplitude.min(100);
        }
    }
}

pub trait VisualizationSource {
    fn next_frame(&mut self, bin_count: usize) -> VisualizationFrame;
}

/// Deterministic UI-only producer: quick attack, gradual decay, fixed storage.
pub struct DemoVisualizationSource {
    levels: [u8; MAX_VISUALIZATION_BINS],
    phase: u32,
    seed: u32,
}

impl DemoVisualizationSource {
    pub const fn new() -> Self {
        Self {
            levels: [10; MAX_VISUALIZATION_BINS],
            phase: 0,
            seed: 0x5EED_4D4D,
        }
    }

    fn random(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

impl Default for DemoVisualizationSource {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualizationSource for DemoVisualizationSource {
    fn next_frame(&mut self, bin_count: usize) -> VisualizationFrame {
        let count = bin_count.clamp(1, MAX_VISUALIZATION_BINS);
        let mut frame = VisualizationFrame::empty();
        frame.set_len(count);
        self.phase = self.phase.wrapping_add(1);
        for index in 0..count {
            let wave = ((self.phase.wrapping_mul(5) + index as u32 * 13) % 72) as u8;
            let mirrored = if wave > 36 { 72 - wave } else { wave };
            let noise = (self.random() % 25) as u8;
            let target = 14u8
                .saturating_add(mirrored.saturating_mul(2))
                .saturating_add(noise)
                .min(100);
            let current = self.levels[index];
            self.levels[index] = if target > current {
                current.saturating_add((target - current).min(24))
            } else {
                current.saturating_sub((current - target).min(5))
            };
            frame.set_bin(index, self.levels[index]);
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_bounded_and_clamps_amplitudes() {
        let mut frame = VisualizationFrame::empty();
        frame.set_len(99);
        frame.set_bin(0, 255);
        assert_eq!(frame.bins().len(), MAX_VISUALIZATION_BINS);
        assert_eq!(frame.bins()[0], 100);
    }

    #[test]
    fn demo_source_uses_fixed_capacity() {
        let mut source = DemoVisualizationSource::new();
        let frame = source.next_frame(32);
        assert_eq!(frame.bins().len(), 32);
        assert!(frame.bins().iter().all(|value| *value <= 100));
    }
}
