//! Shell-side environment variable store (Phase 6.5 Step 2).
//!
//! Values and export/readonly attributes belong to an execution context.
//! Child environments are snapshots; exporting never mutates kernel globals.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;

/// Mirrors `kernel::process::env::DEFAULT_PATH`.
pub const DEFAULT_PATH: &str = "/bin:/usr/bin";
pub const MAX_VARIABLE_BYTES: usize = 64 * 1024;

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
        for key in ["PATH", "USER", "HOME", "SHELL"] {
            self.export(key);
        }
        self.set("?", "0");
    }

    /// Overlay locale variables from a parsed configuration.
    /// Only known LANG/LC_* keys are applied. Empty values are skipped.
    /// This is called after load_defaults so that /etc/locale.conf wins.
    pub fn apply_locale(&mut self, pairs: &[(alloc::string::String, alloc::string::String)]) {
        for (k, v) in pairs {
            if !v.is_empty() {
                self.set(k, v);
                self.export(k);
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
        if !sunlight_ipc::spawn_env::valid_name(key)
            || key.len() > sunlight_ipc::spawn_env::MAX_NAME_BYTES
            || value.as_bytes().contains(&0)
        {
            return Err("invalid shell variable name or value");
        }
        if value.len() > 16384 || (self.vars.len() >= 256 && !self.vars.contains_key(key)) {
            return Err("shell variable resource limit exceeded");
        }
        let existing: usize = self
            .iter()
            .filter(|(name, _)| *name != key)
            .map(|(name, value)| name.len() + value.len() + 2)
            .sum();
        if existing + key.len() + value.len() + 2 > MAX_VARIABLE_BYTES {
            return Err("shell variables exceed 64 KiB");
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
    /// Function side effects may consume space freed by a temporary prefix.
    /// If restoring the parent would exceed the bound, roll back the variable
    /// context and diagnose the command rather than accumulating unbounded state.
    pub fn restore_variables(
        &mut self,
        keys: &[String],
        source: &Self,
    ) -> Result<(), &'static str> {
        for key in keys {
            self.restore_variable(key, source);
        }
        let bytes: usize = self.iter().map(|(k, v)| k.len() + v.len() + 2).sum();
        if self.vars.len() > 256 || self.exported.len() > 256 || bytes > MAX_VARIABLE_BYTES {
            *self = source.clone();
            return Err("temporary assignment restoration exceeds shell variable limits");
        }
        Ok(())
    }
    pub fn export(&mut self, key: &str) {
        self.exported.insert(String::from(key));
    }
    pub fn mark_export(&mut self, key: &str) -> Result<(), &'static str> {
        if !sunlight_ipc::spawn_env::valid_name(key)
            || key.len() > sunlight_ipc::spawn_env::MAX_NAME_BYTES
        {
            return Err("invalid shell export name");
        }
        if self.exported.len() >= 256 && !self.exported.contains(key) {
            return Err("shell export attribute limit exceeded");
        }
        self.export(key);
        Ok(())
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
    /// Validate the complete child environment before opening descriptors or
    /// starting pipeline producers. Host and native use identical limits.
    pub fn export_size(&self) -> Result<usize, &'static str> {
        use sunlight_ipc::spawn_env::{entry_size, MAX_BYTES, MAX_ENTRIES};
        let mut size = 0;
        for (count, (key, value)) in self.exported().enumerate() {
            if count >= MAX_ENTRIES {
                return Err("exported environment exceeds 128 entries");
            }
            let entry = entry_size(key, value)
                .map_err(|_| "invalid or oversized exported environment entry")?;
            size += entry;
            if size > MAX_BYTES {
                return Err("exported environment exceeds 16 KiB");
            }
        }
        Ok(size)
    }
    pub fn export_blob(&self) -> Result<alloc::vec::Vec<u8>, &'static str> {
        let size = self.export_size()?;
        let mut blob = alloc::vec::Vec::new();
        blob.try_reserve_exact(size)
            .map_err(|_| "out of memory serializing environment")?;
        for (key, value) in self.exported() {
            blob.extend_from_slice(key.as_bytes());
            blob.push(b'=');
            blob.extend_from_slice(value.as_bytes());
            blob.push(0);
        }
        Ok(blob)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_restoration_rolls_back_variable_growth_on_limit_failure() {
        let mut parent = ShellEnv::new();
        for key in ["A", "B", "C"] {
            parent.assign(key, &"x".repeat(16000)).unwrap();
        }
        let mut child = parent.clone();
        child.assign("A", "").unwrap();
        child.assign("D", &"x".repeat(16000)).unwrap();
        child.assign("E", &"x".repeat(16000)).unwrap();
        assert!(child
            .restore_variables(&[String::from("A")], &parent)
            .is_err());
        assert_eq!(child.get("A"), parent.get("A"));
        assert_eq!(child.get("D"), None);
        child.assign("OK", "recovered").unwrap();
    }
    #[test]
    fn snapshots_include_only_exports_and_do_not_mutate_the_parent() {
        let mut env = ShellEnv::new();
        env.assign("LOCAL", "hidden").unwrap();
        env.assign("NAME", "parent").unwrap();
        env.export("NAME");
        env.mark_export("UNASSIGNED").unwrap();
        assert_eq!(env.get("UNASSIGNED"), None);
        assert_eq!(env.export_blob().unwrap(), b"NAME=parent\0");
        let mut child = env.clone();
        child.assign("NAME", "temporary").unwrap();
        assert_eq!(child.export_blob().unwrap(), b"NAME=temporary\0");
        assert_eq!(env.get("NAME"), Some("parent"));
        child.unset("NAME");
        assert_eq!(child.export_blob().unwrap(), b"");
    }
    #[test]
    fn oversized_export_fails_without_truncation_and_can_be_recovered() {
        let mut env = ShellEnv::new();
        env.assign("LARGE", &"x".repeat(4096)).unwrap();
        env.export("LARGE");
        assert!(env.export_blob().is_err());
        env.unset("LARGE");
        assert_eq!(env.export_blob().unwrap(), b"");
        for i in 0..129 {
            let key = format!("VAR_{i}");
            env.assign(&key, "").unwrap();
            env.export(&key);
        }
        assert!(env.export_size().is_err());
        env.unset("VAR_128");
        assert!(env.export_size().is_ok());
    }
    #[test]
    fn aggregate_variable_and_attribute_limits_preserve_existing_values() {
        let mut env = ShellEnv::new();
        for i in 0..6 {
            env.assign(&format!("V{i}"), &"x".repeat(10000)).unwrap();
        }
        assert!(env.assign("EXTRA", &"y".repeat(10000)).is_err());
        assert_eq!(env.get("EXTRA"), None);
        assert!(env.assign("V0", &"y".repeat(16384)).is_err());
        assert_eq!(env.get("V0").unwrap().len(), 10000);
        env.unset("V1");
        env.assign("EXTRA", "recovered").unwrap();
        assert!(env.assign("BAD", "x\0y").is_err());
        assert!(env.assign(&"A".repeat(256), "").is_err());
        for i in 0..256 {
            env.mark_export(&format!("E{i}")).unwrap();
        }
        assert!(env.mark_export("ONE_MORE").is_err());
    }
}
