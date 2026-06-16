//! Phase 2: capability rule translation model for mock POSIX → broker flow.
//!
//! The schema is intentionally small and fixed-capacity; it models allow/deny
//! decisions for uid/gid identity against path-prefix ACLs and rights.

use heapless::{LinearMap, String, Vec};

/// Access rights accepted by broker-capability translation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessFlags {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl AccessFlags {
    pub const NONE: Self = Self {
        read: false,
        write: false,
        execute: false,
    };

    pub const READ: Self = Self {
        read: true,
        write: false,
        execute: false,
    };

    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        execute: false,
    };

    pub const EXECUTE: Self = Self {
        read: false,
        write: false,
        execute: true,
    };

    pub const READ_EXECUTE: Self = Self {
        read: true,
        write: false,
        execute: true,
    };

    pub const ALL: Self = Self {
        read: true,
        write: true,
        execute: true,
    };

    pub fn union(self, rhs: Self) -> Self {
        Self {
            read: self.read || rhs.read,
            write: self.write || rhs.write,
            execute: self.execute || rhs.execute,
        }
    }
}

impl Default for AccessFlags {
    fn default() -> Self {
        Self::NONE
    }
}

/// Single rule mapping a principal (uid or gid) to path-prefix + rights.
#[derive(Debug, Clone)]
pub struct PathRule {
    /// Prefix-matched filesystem path.
    pub allowed_prefix: String<64>,
    /// Rights granted for this prefix.
    pub flags: AccessFlags,
}

/// Rule-set associated with one principal and normalized into a fixed-size vector.
#[derive(Debug, Clone)]
pub struct RuleSet<const N: usize> {
    pub rules: Vec<PathRule, N>,
}

impl<const N: usize> RuleSet<N> {
    pub const fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(&mut self, rule: PathRule) -> Result<(), RuleError> {
        self.rules.push(rule).map_err(|_| RuleError::RuleStoreFull)
    }
}

/// Full user and group identity rule mapping table.
#[derive(Debug, Clone)]
pub struct RuleTable<const NU: usize, const NG: usize, const NR: usize> {
    pub uid_rules: LinearMap<u32, RuleSet<NR>, NU>,
    pub gid_rules: LinearMap<u32, RuleSet<NR>, NG>,
}

/// Explicit error set for rule lookup/management.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleError {
    MissingSubject,
    RuleStoreFull,
}

impl Default for RuleError {
    fn default() -> Self {
        Self::MissingSubject
    }
}

impl<const NU: usize, const NG: usize, const NR: usize> RuleTable<NU, NG, NR> {
    pub const fn new() -> Self {
        Self {
            uid_rules: LinearMap::new(),
            gid_rules: LinearMap::new(),
        }
    }

    pub fn add_uid_rule(
        &mut self,
        uid: u32,
        rule: PathRule,
    ) -> Result<(), RuleError> {
        match self.uid_rules.get_mut(&uid) {
            Some(set) => set.add_rule(rule),
            None => {
                let mut set = RuleSet::<NR>::new();
                set.add_rule(rule)?;
                self.uid_rules
                    .insert(uid, set)
                    .map_err(|_| RuleError::RuleStoreFull)?;
                Ok(())
            }
        }
    }

    pub fn add_gid_rule(
        &mut self,
        gid: u32,
        rule: PathRule,
    ) -> Result<(), RuleError> {
        match self.gid_rules.get_mut(&gid) {
            Some(set) => set.add_rule(rule),
            None => {
                let mut set = RuleSet::<NR>::new();
                set.add_rule(rule)?;
                self.gid_rules
                    .insert(gid, set)
                    .map_err(|_| RuleError::RuleStoreFull)?;
                Ok(())
            }
        }
    }

    pub fn find_rules<'a>(&'a self, uid: u32, gid: u32) -> impl Iterator<Item = &'a PathRule> {
        self.find_uid_rules(uid).chain(self.find_gid_rules(gid))
    }

    pub fn find_uid_rules<'a>(&'a self, uid: u32) -> impl Iterator<Item = &'a PathRule> {
        self.uid_rules
            .get(&uid)
            .map_or([].iter(), |set| set.rules.iter())
    }

    pub fn find_gid_rules<'a>(&'a self, gid: u32) -> impl Iterator<Item = &'a PathRule> {
        self.gid_rules
            .get(&gid)
            .map_or([].iter(), |set| set.rules.iter())
    }

    pub fn find_matching_rule(
        &self,
        uid: u32,
        gid: u32,
        path: &str,
    ) -> Option<(u32, PathRule)> {
        let mut merged = AccessFlags::NONE;
        let mut matched_prefix: Option<String<64>> = None;

        for r in self.find_rules(uid, gid) {
            if path.starts_with(r.allowed_prefix.as_str()) {
                merged = merged.union(r.flags);
                matched_prefix = Some(r.allowed_prefix.clone());
            }
        }

        matched_prefix.map(|prefix| (uid, PathRule {
            allowed_prefix: prefix,
            flags: merged,
        }))
    }

    pub fn has_access(&self, uid: u32, gid: u32, path: &str, required: AccessFlags) -> bool {
        for rule in self.find_rules(uid, gid) {
            if !path.starts_with(rule.allowed_prefix.as_str()) {
                continue;
            }

            if required.read && !rule.flags.read {
                continue;
            }
            if required.write && !rule.flags.write {
                continue;
            }
            if required.execute && !rule.flags.execute {
                continue;
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_rule_lookup() {
        let mut table: RuleTable<4, 4, 4> = RuleTable::new();
        let mut allowed = String::<64>::new();
        let _ = allowed.push_str("/home/alice");
        let _ = table.add_uid_rule(
            1000,
            PathRule {
                allowed_prefix: allowed.clone(),
                flags: AccessFlags::READ_WRITE,
            },
        );

        assert!(table.has_access(1000, 10, "/home/alice/docs/readme.txt", AccessFlags::READ));
        assert!(!table.has_access(1000, 10, "/var/secret", AccessFlags::READ));
    }
}
