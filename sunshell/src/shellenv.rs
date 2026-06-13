//! Shell-side environment variable store (Phase 6.5 Step 2).
//!
//! `sshl` keeps its own copy of the environment (seeded from the same
//! defaults the kernel attaches to the PCB) so builtins like `export`,
//! `env`, and `$VAR` expansion work without a syscall round-trip.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;

/// Mirrors `kernel::process::env::DEFAULT_PATH`.
pub const DEFAULT_PATH: &str = "/bin:/usr/bin";

pub struct ShellEnv {
    vars: BTreeMap<String, String>,
}

impl ShellEnv {
    pub const fn new() -> Self {
        Self {
            vars: BTreeMap::new(),
        }
    }

    /// Seed the standard variables for the logged-in user.
    pub fn load_defaults(&mut self, uid: u32, username: &str) {
        let user = if username.is_empty() {
            if uid == 0 { "root" } else { "user" }
        } else {
            username
        };
        let home = if uid == 0 {
            String::from("/root")
        } else {
            format!("/home/{}", user)
        };
        self.set("PATH", DEFAULT_PATH);
        self.set("USER", user);
        self.set("HOME", &home);
        self.set("SHELL", "/bin/sshl");
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.vars.insert(String::from(key), String::from(value));
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn unset(&mut self, key: &str) -> bool {
        self.vars.remove(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Colon-separated PATH entries, in search order.
    pub fn path_entries(&self) -> impl Iterator<Item = &str> {
        self.get("PATH")
            .unwrap_or("")
            .split(':')
            .filter(|d| !d.is_empty())
    }

    /// Expand a single token: `$KEY` (or `${KEY}`) becomes its value,
    /// an unset variable becomes the empty string, anything else passes
    /// through unchanged.
    pub fn expand_token(&self, token: &str) -> String {
        if let Some(rest) = token.strip_prefix('$') {
            let key = rest
                .strip_prefix('{')
                .and_then(|k| k.strip_suffix('}'))
                .unwrap_or(rest);
            String::from(self.get(key).unwrap_or(""))
        } else {
            String::from(token)
        }
    }
}
