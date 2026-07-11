use alloc::string::String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentParseState {
    NotStarted,
    Parsing,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct DocumentLifecycle {
    generation: u64,
    parse_attempts: usize,
    state: DocumentParseState,
}

impl Default for DocumentLifecycle {
    fn default() -> Self {
        Self {
            generation: 0,
            parse_attempts: 0,
            state: DocumentParseState::NotStarted,
        }
    }
}

impl DocumentLifecycle {
    pub fn begin_navigation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.parse_attempts = 0;
        self.state = DocumentParseState::NotStarted;
        self.generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn parse_attempts(&self) -> usize {
        self.parse_attempts
    }

    pub fn state(&self) -> &DocumentParseState {
        &self.state
    }

    pub fn start_parse(&mut self, generation: u64) -> bool {
        if generation != self.generation || self.state != DocumentParseState::NotStarted {
            return false;
        }
        self.parse_attempts = self.parse_attempts.saturating_add(1);
        self.state = DocumentParseState::Parsing;
        true
    }

    pub fn finish_ready(&mut self, generation: u64) -> bool {
        if generation != self.generation || self.state != DocumentParseState::Parsing {
            return false;
        }
        self.state = DocumentParseState::Ready;
        true
    }

    pub fn finish_failed(&mut self, generation: u64, error: String) -> bool {
        if generation != self.generation || self.state != DocumentParseState::Parsing {
            return false;
        }
        self.state = DocumentParseState::Failed(error);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_navigation_starts_parsing_once() {
        let mut lifecycle = DocumentLifecycle::default();
        let generation = lifecycle.begin_navigation();
        assert!(lifecycle.start_parse(generation));
        assert!(!lifecycle.start_parse(generation));
        assert!(lifecycle.finish_ready(generation));
        for _ in 0..100 {
            assert!(!lifecycle.start_parse(generation));
        }
        assert_eq!(lifecycle.parse_attempts(), 1);
    }

    #[test]
    fn failed_parse_is_not_retried_on_idle_ticks() {
        let mut lifecycle = DocumentLifecycle::default();
        let generation = lifecycle.begin_navigation();
        assert!(lifecycle.start_parse(generation));
        assert!(lifecycle.finish_failed(generation, String::from("bad markup")));
        for _ in 0..100 {
            assert!(!lifecycle.start_parse(generation));
        }
        assert_eq!(lifecycle.parse_attempts(), 1);
        assert!(matches!(
            lifecycle.state(),
            DocumentParseState::Failed(error) if error == "bad markup"
        ));
    }

    #[test]
    fn stale_generation_cannot_apply_results() {
        let mut lifecycle = DocumentLifecycle::default();
        let old_generation = lifecycle.begin_navigation();
        assert!(lifecycle.start_parse(old_generation));
        let new_generation = lifecycle.begin_navigation();
        assert!(!lifecycle.finish_ready(old_generation));
        assert!(lifecycle.start_parse(new_generation));
        assert!(lifecycle.finish_ready(new_generation));
    }
}
