//! Bounded, decoder-agnostic visualization presentation state.

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

    pub fn decay(&mut self, amount: u8) -> bool {
        let mut changed = false;
        for value in &mut self.bins[..self.len as usize] {
            let next = value.saturating_sub(amount);
            changed |= next != *value;
            *value = next;
        }
        changed
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
    fn decay_never_underflows_or_creates_animation() {
        let mut frame = VisualizationFrame::empty();
        frame.set_len(2);
        frame.set_bin(0, 8);
        assert!(frame.decay(6));
        assert_eq!(frame.bins(), &[2, 0]);
        assert!(frame.decay(6));
        assert_eq!(frame.bins(), &[0, 0]);
        assert!(!frame.decay(6));
    }
}
