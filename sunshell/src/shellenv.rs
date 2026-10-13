//! Shell-side environment variable store (Phase 6.5 Step 2).
//!
//! `sshl` keeps its own copy of the environment (seeded from the same
//! defaults the kernel attaches to the PCB) so builtins like `export`,
//! `env`, and `$VAR` expansion work without a syscall round-trip.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;

/// Mirrors `kernel::process::env::DEFAULT_PATH`.
pub const DEFAULT_PATH: &str = "/bin:/usr/bin";

#[derive(Clone)]
pub struct ShellEnv {
    vars: BTreeMap<String, String>,
    pub positional: alloc::vec::Vec<String>,
    pub arg0: String,
    pub pid: u64,
    exported: alloc::collections::BTreeSet<String>,
    readonly: alloc::collections::BTreeSet<String>,
}

impl ShellEnv {
    pub const fn new() -> Self {
        Self {
            vars: BTreeMap::new(),
            positional: alloc::vec::Vec::new(),
            arg0: String::new(),
            pid: 0,
            exported: alloc::collections::BTreeSet::new(),
            readonly: alloc::collections::BTreeSet::new(),
        }
    }

    /// Seed the standard variables for the logged-in user.
    pub fn load_defaults(&mut self, uid: u32, username: &str) {
        let user = if username.is_empty() {
            if uid == 0 {
                "root"
            } else {
                "user"
            }
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
        self.set("?", "0");
    }

    /// Overlay locale variables from a parsed configuration.
    /// Only known LANG/LC_* keys are applied. Empty values are skipped.
    /// This is called after load_defaults so that /etc/locale.conf wins.
    pub fn apply_locale(&mut self, pairs: &[(alloc::string::String, alloc::string::String)]) {
        for (k, v) in pairs {
            if !v.is_empty() {
                self.set(k, v);
            }
        }
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.vars.insert(String::from(key), String::from(value));
    }

    pub fn assign(&mut self, key: &str, value: &str) -> Result<(), &'static str> {
        if self.readonly.contains(key) {
            return Err("variable is readonly");
        }
        if value.len() > 16384 || (self.vars.len() >= 256 && !self.vars.contains_key(key)) {
            return Err("shell variable resource limit exceeded");
        }
        self.set(key, value);
        Ok(())
    }
    pub fn restore_variable(&mut self, key: &str, source: &Self) {
        self.vars.remove(key);
        self.exported.remove(key);
        self.readonly.remove(key);
        if let Some(value) = source.get(key) {
            self.set(key, value);
        }
        if source.exported.contains(key) {
            self.export(key);
        }
        if source.readonly.contains(key) {
            self.make_readonly(key);
        }
    }
    pub fn export(&mut self, key: &str) {
        self.exported.insert(String::from(key));
    }
    pub fn make_readonly(&mut self, key: &str) {
        self.readonly.insert(String::from(key));
    }
    pub fn is_readonly(&self, key: &str) -> bool {
        self.readonly.contains(key)
    }
    pub fn exported(&self) -> impl Iterator<Item = (&str, &str)> {
        self.iter().filter(|(key, _)| self.exported.contains(*key))
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn unset(&mut self, key: &str) -> bool {
        if self.readonly.contains(key) {
            return false;
        }
        self.exported.remove(key);
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
}
