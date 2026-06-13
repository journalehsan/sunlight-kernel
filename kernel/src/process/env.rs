//! Per-process environment variable registry (Phase 6.5 Step 2).
//!
//! Lives on the Process Control Block. Backed by an ordered `BTreeMap`
//! (no_std + alloc) so `env` listings are deterministic and lookups stay
//! O(log n) without needing a hasher in the kernel.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Default PATH handed to every freshly spawned process.
pub const DEFAULT_PATH: &str = "/bin:/usr/bin";

/// Key→value environment registry attached to a `Process`.
#[derive(Debug, Clone, Default)]
pub struct EnvMap {
    vars: BTreeMap<String, String>,
}

impl EnvMap {
    pub const fn new() -> Self {
        Self {
            vars: BTreeMap::new(),
        }
    }

    /// Standard environment for a process spawned on behalf of `uid`.
    /// `username` comes from /etc/passwd (resolved by the caller); uid 0
    /// falls back to root conventions when no name is available.
    pub fn with_defaults(uid: u32, username: &str) -> Self {
        let mut env = Self::new();
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
        env.set("PATH", DEFAULT_PATH);
        env.set("USER", user);
        env.set("HOME", &home);
        env.set("SHELL", "/bin/sshl");
        env
    }

    /// Child environment inherited from a parent (fork/spawn semantics).
    pub fn inherit(parent: &EnvMap) -> Self {
        parent.clone()
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

    pub fn len(&self) -> usize {
        self.vars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Iterate the colon-separated entries of PATH, in order.
    pub fn path_entries(&self) -> impl Iterator<Item = &str> {
        self.get("PATH")
            .unwrap_or("")
            .split(':')
            .filter(|d| !d.is_empty())
    }

    /// Serialize to `KEY=VALUE` strings for SysV envp stack marshalling
    /// (consumed by `spawn::exec_into_process`).
    pub fn to_envp(&self) -> Vec<String> {
        self.vars
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect()
    }
}
