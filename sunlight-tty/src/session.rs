//! Minimal TOML-ish session config reader (no external parser dependencies).
//!
//! Parses key=value lines from /etc/sunlight/session.toml format.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMode {
    Terminal,
    Wayland,
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub mode: SessionMode,
    pub shell: [u8; 64],
    pub shell_len: usize,
    pub initial_tabs: u8,
    pub theme: [u8; 32],
    pub theme_len: usize,
    pub multi_user: bool,
    pub max_ttys: u8,
}

impl SessionConfig {
    pub fn default() -> Self {
        let shell = b"/bin/sh";
        let theme = b"sunlight-dark";
        let mut s = Self {
            mode: SessionMode::Terminal,
            shell: [0; 64],
            shell_len: shell.len(),
            initial_tabs: 1,
            theme: [0; 32],
            theme_len: theme.len(),
            multi_user: false,
            max_ttys: 6,
        };
        s.shell[..shell.len()].copy_from_slice(shell);
        s.theme[..theme.len()].copy_from_slice(theme);
        s
    }

    /// Parse session config from raw bytes. Line-by-line key=value, no
    /// dependency on external TOML crates.
    pub fn parse(data: &[u8]) -> Self {
        let mut cfg = Self::default();

        let text = match core::str::from_utf8(data) {
            Ok(s) => s,
            Err(_) => return cfg,
        };

        let mut current_section: &str = "";
        for line in text.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = &trimmed[1..trimmed.len() - 1];
                continue;
            }
            if let Some(eq) = trimmed.find('=') {
                let key = trimmed[..eq].trim();
                let val = &trimmed[eq + 1..].trim();
                let val = val.trim_matches('"').trim_matches('\'');
                match key {
                    "mode" => {
                        if current_section == "default" && val == "wayland" {
                            cfg.mode = SessionMode::Wayland;
                        }
                    }
                    "shell" => {
                        let bytes = val.as_bytes();
                        let len = bytes.len().min(cfg.shell.len());
                        cfg.shell[..len].copy_from_slice(&bytes[..len]);
                        cfg.shell_len = len;
                    }
                    "initial_tabs" => {
                        if let Ok(n) = val.parse::<u8>() {
                            cfg.initial_tabs = n.clamp(1, 10);
                        }
                    }
                    "theme" => {
                        let bytes = val.as_bytes();
                        let len = bytes.len().min(cfg.theme.len());
                        cfg.theme[..len].copy_from_slice(&bytes[..len]);
                        cfg.theme_len = len;
                    }
                    "enabled" => {
                        if current_section == "multi_user" {
                            cfg.multi_user = val == "true";
                        }
                    }
                    "max_ttys" => {
                        if let Ok(n) = val.parse::<u8>() {
                            cfg.max_ttys = n.clamp(1, 32);
                        }
                    }
                    _ => {}
                }
            }
        }

        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_sensible() {
        let cfg = SessionConfig::default();
        assert_eq!(cfg.mode, SessionMode::Terminal);
        assert_eq!(&cfg.shell[..cfg.shell_len], b"/bin/sh");
        assert_eq!(cfg.initial_tabs, 1);
        assert_eq!(&cfg.theme[..cfg.theme_len], b"sunlight-dark");
        assert!(!cfg.multi_user);
        assert_eq!(cfg.max_ttys, 6);
    }

    #[test]
    fn parse_session_toml() {
        let data = br#"
[default]
mode = "terminal"

[terminal]
shell = "/bin/sh"
initial_tabs = 3
theme = "dark"

[multi_user]
enabled = true
max_ttys = 8
"#;
        let cfg = SessionConfig::parse(data);
        assert_eq!(cfg.mode, SessionMode::Terminal);
        assert_eq!(&cfg.shell[..cfg.shell_len], b"/bin/sh");
        assert_eq!(cfg.initial_tabs, 3);
        assert_eq!(&cfg.theme[..cfg.theme_len], b"dark");
        assert!(cfg.multi_user);
        assert_eq!(cfg.max_ttys, 8);
    }
}
