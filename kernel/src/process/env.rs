//! Per-process environment variable registry (Phase 6.5 Step 2).
//!
//! Sorted contiguous entries keep iteration deterministic and allow fallible
//! inheritance; BTreeMap node cloning cannot report allocation failure.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Default PATH handed to every freshly spawned process.
pub const DEFAULT_PATH: &str = "/bin:/usr/bin";

/// Key→value environment registry attached to a `Process`.
#[derive(Debug, Clone, Default)]
pub struct EnvMap {
    vars: Vec<(String, String)>,
}

impl EnvMap {
    pub const fn new() -> Self {
        Self { vars: Vec::new() }
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
        env.set("TERM", "vt100");
        // Locale foundation defaults (C.UTF-8). Actual user values come from
        // /etc/locale.conf via session shell / environment overlay so that
        // both kernel spawns and future installer sessions behave consistently.
        env.set("LANG", "C.UTF-8");
        env.set("LC_TIME", "C.UTF-8");
        env
    }

    /// Child environment inherited from a parent (fork/spawn semantics).
    pub fn inherit(parent: &EnvMap) -> Self {
        parent.clone()
    }

    /// Call only after checking the inherited byte and entry budget.
    pub fn try_inherit(parent: &Self) -> Result<Self, super::thread_admission::Error> {
        use super::thread_admission::{try_string, Error};
        super::thread_admission::checkpoint("environment-entries")?;
        let mut vars = Vec::new();
        vars.try_reserve_exact(parent.len())
            .map_err(|_| Error::NoMemory)?;
        for (key, value) in parent.iter() {
            vars.push((try_string(key)?, try_string(value)?));
        }
        Ok(Self { vars })
    }

    pub fn set(&mut self, key: &str, value: &str) {
        match self.vars.binary_search_by(|(k, _)| k.as_str().cmp(key)) {
            Ok(index) => self.vars[index].1 = String::from(value),
            Err(index) => self
                .vars
                .insert(index, (String::from(key), String::from(value))),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .binary_search_by(|(k, _)| k.as_str().cmp(key))
            .ok()
            .map(|index| self.vars[index].1.as_str())
    }

    pub fn unset(&mut self, key: &str) -> bool {
        if let Ok(index) = self.vars.binary_search_by(|(k, _)| k.as_str().cmp(key)) {
            self.vars.remove(index);
            true
        } else {
            false
        }
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

#[cfg(test)]
mod tests {
    use super::super::thread_admission::{host_fault, Error};
    use super::*;
    #[test]
    fn sorted_inheritance_and_all_allocation_failures() {
        let mut parent = EnvMap::new();
        parent.set("Z", "last");
        parent.set("A", "first");
        parent.set("A", "replacement");
        assert_eq!(
            parent.iter().collect::<Vec<_>>(),
            [("A", "replacement"), ("Z", "last")]
        );
        // Entry storage, then each owned key and value. Exercise every prefix
        // cleanup using the actual production clone, never a second algorithm.
        for ordinal in 1..=5 {
            host_fault::arm(ordinal);
            assert!(matches!(EnvMap::try_inherit(&parent), Err(Error::NoMemory)));
            assert_eq!(parent.get("A"), Some("replacement"));
            host_fault::arm(0);
        }
        let mut child = EnvMap::try_inherit(&parent).unwrap();
        assert!(child.unset("A"));
        assert!(!child.unset("A"));
        assert_eq!(parent.get("A"), Some("replacement"));
        assert_eq!(child.get("Z"), Some("last"));
    }
}
