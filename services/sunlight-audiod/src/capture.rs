//! Bounded capture periods. DMA owns its ring; this queue owns copies until
//! the client receives one through its own shared page.

pub const CAPTURE_PERIOD_BYTES: usize = 4096;
pub const CAPTURE_QUEUE_PERIODS: usize = 8;
pub const CAPTURE_FORMAT_WORD: u64 = 48_000u64 | (2u64 << 32) | (16u64 << 40);

pub fn valid_open_request(msg: &sunlight_ipc::IpcMsg) -> bool {
    msg.word_count == 2
        && msg.cap_count == 0
        && msg.words[0] == CAPTURE_FORMAT_WORD
        && msg.words[1] <= 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureState {
    Closed,
    Open,
    Running,
}

/// A stopped stream retains completed periods for a client's final drain.
pub fn can_read(state: CaptureState, monitor_only: bool) -> bool {
    !monitor_only && matches!(state, CaptureState::Open | CaptureState::Running)
}

pub struct CaptureQueue {
    periods: [[u8; CAPTURE_PERIOD_BYTES]; CAPTURE_QUEUE_PERIODS],
    head: usize,
    len: usize,
    pub dropped_periods: u64,
    pub received_periods: u64,
    pub peak: u16,
}

impl CaptureQueue {
    pub const fn new() -> Self {
        Self {
            periods: [[0; CAPTURE_PERIOD_BYTES]; CAPTURE_QUEUE_PERIODS],
            head: 0,
            len: 0,
            dropped_periods: 0,
            received_periods: 0,
            peak: 0,
        }
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
        self.peak = 0;
    }
    pub fn reset(&mut self) {
        self.clear();
        self.dropped_periods = 0;
        self.received_periods = 0;
    }
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if bytes.len() != CAPTURE_PERIOD_BYTES {
            return;
        }
        self.observe_level(bytes);
        if self.len == CAPTURE_QUEUE_PERIODS {
            // Oldest unread period is discarded; DMA and a returned client
            // page are never touched by this overflow policy.
            self.head = (self.head + 1) % CAPTURE_QUEUE_PERIODS;
            self.len -= 1;
            self.dropped_periods = self.dropped_periods.saturating_add(1);
        }
        let tail = (self.head + self.len) % CAPTURE_QUEUE_PERIODS;
        self.periods[tail].copy_from_slice(bytes);
        self.len += 1;
    }

    pub fn observe_level(&mut self, bytes: &[u8]) {
        if bytes.len() != CAPTURE_PERIOD_BYTES {
            return;
        }
        self.received_periods = self.received_periods.saturating_add(1);
        let mut peak = 0u16;
        for sample in bytes.chunks_exact(2) {
            let value = i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs();
            peak = peak.max(value);
        }
        self.peak = peak;
    }

    pub fn pop_into(&mut self, dst: &mut [u8]) -> bool {
        if self.len == 0 || dst.len() < CAPTURE_PERIOD_BYTES {
            return false;
        }
        dst[..CAPTURE_PERIOD_BYTES].copy_from_slice(&self.periods[self.head]);
        self.head = (self.head + 1) % CAPTURE_QUEUE_PERIODS;
        self.len -= 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_capture_can_drain_but_closed_and_monitor_cannot() {
        assert!(can_read(CaptureState::Open, false));
        assert!(can_read(CaptureState::Running, false));
        assert!(!can_read(CaptureState::Closed, false));
        assert!(!can_read(CaptureState::Open, true));
    }
    #[test]
    fn overrun_discards_only_oldest_unread_period() {
        let mut q = CaptureQueue::new();
        for value in 0..9u8 {
            q.push(&[value; CAPTURE_PERIOD_BYTES]);
        }
        assert_eq!(q.dropped_periods, 1);
        let mut out = [0; CAPTURE_PERIOD_BYTES];
        assert!(q.pop_into(&mut out));
        assert_eq!(out[0], 1);
        assert_eq!(q.len(), 7);
    }
    #[test]
    fn peak_handles_negative_full_scale() {
        let mut q = CaptureQueue::new();
        let mut p = [0; CAPTURE_PERIOD_BYTES];
        p[..2].copy_from_slice(&i16::MIN.to_le_bytes());
        q.push(&p);
        assert_eq!(q.peak, 32768);
    }
    #[test]
    fn monitor_does_not_fill_recording_fifo() {
        let mut q = CaptureQueue::new();
        for _ in 0..100 {
            q.observe_level(&[1; CAPTURE_PERIOD_BYTES]);
        }
        assert_eq!(q.received_periods, 100);
        assert_eq!(q.dropped_periods, 0);
        assert_eq!(q.len(), 0);
    }
    #[test]
    fn malformed_open_requests_are_rejected() {
        use sunlight_ipc::{AudiodMsg, IpcMsg};
        let valid = IpcMsg::with_label(AudiodMsg::CAPTURE_OPEN)
            .word(0, CAPTURE_FORMAT_WORD)
            .word(1, 0);
        assert!(valid_open_request(&valid));
        assert!(!valid_open_request(&IpcMsg::with_label(
            AudiodMsg::CAPTURE_OPEN
        )));
        assert!(!valid_open_request(&valid.word(1, 2)));
        assert!(!valid_open_request(&valid.word(0, 44_100)));
    }
}
