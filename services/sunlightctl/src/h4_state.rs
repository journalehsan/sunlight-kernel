//! The private prepare/release protocol never carries a manager capability.
#[derive(Debug, PartialEq)]
enum Phase {
    Starting,
    Ready,
    Submitted,
    Terminal,
}

pub(crate) struct Controller {
    fixture: u64,
    phase: Phase,
}

impl Controller {
    pub(crate) fn new(fixture: u64) -> Self {
        Self {
            fixture,
            phase: Phase::Starting,
        }
    }
    pub(crate) fn ready(&mut self, target: u64) -> bool {
        if self.fixture == 0 || target == 0 || self.phase != Phase::Starting {
            return false;
        }
        self.phase = Phase::Ready;
        true
    }
    pub(crate) fn release(&mut self, fixture: u64) -> bool {
        if fixture != self.fixture || self.phase != Phase::Ready {
            return false;
        }
        self.phase = Phase::Submitted;
        true
    }
    pub(crate) fn finish(&mut self, accepted: bool) -> bool {
        let success = accepted && self.phase == Phase::Submitted;
        self.phase = Phase::Terminal;
        success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepare_release_identity_and_exactly_once() {
        let mut c = Controller::new(7);
        assert!(!c.release(7));
        assert!(c.ready(22));
        assert!(!c.release(8));
        assert!(c.release(7));
        assert!(!c.release(7));
        assert!(c.finish(true));
        assert!(!c.release(7));
        assert!(!c.finish(true));
    }
    #[test]
    fn failed_stage_is_terminal_and_empty_identity_is_rejected() {
        assert!(!Controller::new(0).ready(22));
        let mut c = Controller::new(7);
        assert!(!c.ready(0));
        assert!(!c.finish(false));
        assert!(!c.ready(22));
        assert!(!c.release(7));
        assert!(!Controller::new(7).finish(true));
    }
}
