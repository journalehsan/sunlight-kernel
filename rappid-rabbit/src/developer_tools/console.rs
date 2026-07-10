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

#[derive(Debug, Clone, Default)]
pub struct ConsoleState {
    entries: Vec<ConsoleEntry>,
    scroll_offset: usize,
}

impl ConsoleState {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.scroll_offset = 0;
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

    pub fn rendered_text(&self) -> String {
        if self.entries.is_empty() {
            return String::from("No console messages yet.");
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
        out
    }
}
