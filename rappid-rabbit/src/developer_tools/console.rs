use alloc::{format, string::String, vec::Vec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleSeverity {
    Quiet,
    Warn,
    Error,
}

impl ConsoleSeverity {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quiet => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleSource {
    Browser,
    Fetch,
    Parser,
    Network,
}

impl ConsoleSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Fetch => "fetch",
            Self::Parser => "parser",
            Self::Network => "network",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleEntry {
    pub severity: ConsoleSeverity,
    pub source: ConsoleSource,
    pub message: String,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ConsoleState {
    entries: Vec<ConsoleEntry>,
    scroll_offset: usize,
    rendered_text_cache: String,
    rendered_text_dirty: bool,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            scroll_offset: 0,
            rendered_text_cache: String::new(),
            rendered_text_dirty: true,
        }
    }
}

impl ConsoleState {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.scroll_offset = 0;
        self.rendered_text_cache.clear();
        self.rendered_text_dirty = true;
    }

    pub fn push(
        &mut self,
        severity: ConsoleSeverity,
        source: ConsoleSource,
        message: impl Into<String>,
    ) {
        self.entries.push(ConsoleEntry {
            severity,
            source,
            message: message.into(),
            timestamp: None,
        });
        self.rendered_text_dirty = true;
    }

    pub fn entries(&self) -> &[ConsoleEntry] {
        &self.entries
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn set_scroll_offset(&mut self, scroll_offset: usize) {
        self.scroll_offset = scroll_offset;
    }

    pub fn rendered_text(&mut self) -> &str {
        if !self.rendered_text_dirty {
            return self.rendered_text_cache.as_str();
        }
        if self.entries.is_empty() {
            self.rendered_text_cache = String::from("No console messages yet.");
            self.rendered_text_dirty = false;
            return self.rendered_text_cache.as_str();
        }

        let mut out = String::new();
        for entry in &self.entries {
            let prefix = if let Some(timestamp) = entry.timestamp {
                format!(
                    "[{}][{}][{timestamp}] ",
                    entry.severity.label(),
                    entry.source.label()
                )
            } else {
                format!("[{}][{}] ", entry.severity.label(), entry.source.label())
            };
            out.push_str(&prefix);
            out.push_str(&entry.message);
            out.push('\n');
        }
        self.rendered_text_cache = out;
        self.rendered_text_dirty = false;
        self.rendered_text_cache.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_console_text_is_cached_between_idle_frames() {
        let mut state = ConsoleState::default();
        state.push(ConsoleSeverity::Quiet, ConsoleSource::Browser, "ready");
        let first_ptr = state.rendered_text().as_ptr();
        for _ in 0..100 {
            assert_eq!(state.rendered_text().as_ptr(), first_ptr);
        }
    }
}
