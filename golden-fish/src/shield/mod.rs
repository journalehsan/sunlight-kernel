//! Content Shield policy, independent of consumers' DOM, selector and transport engines.
#![allow(clippy::all, clippy::pedantic)]

extern crate alloc;
use alloc::{
    borrow::Cow,
    collections::BTreeSet,
    string::String,
    sync::Arc,
    vec::Vec,
};

mod network;
mod parser;
pub use network::{CanonicalUrl, Decision, ResourceType};
pub use parser::{compile, compile_static, CompileError, Diagnostics, RejectReason};

pub const MAX_LIST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RULES: usize = 100_000;
pub const MAX_LINE_BYTES: usize = 2048;
pub const MAX_LISTS: usize = 8;
pub const MAX_SITE_EXCEPTIONS: usize = 64;
pub const EASYLIST: &str = include_str!("../../filters/easylist.txt");

#[derive(Clone, Debug, Default)]
pub struct Domains {
    pub(crate) raw: Cow<'static, str>,
    pub(crate) separator: char,
    pub(crate) has_inclusions: bool,
}
impl Domains {
    pub fn is_generic(&self) -> bool {
        !self.has_inclusions
    }
    pub fn matches(&self, host: &str) -> bool {
        let mut best = 0;
        let mut allowed = !self.has_inclusions;
        for item in self.raw.split(self.separator).filter(|s| !s.is_empty()) {
            let (excluded, domain) = item.strip_prefix('~').map_or((false, item), |d| (true, d));
            if domain_matches(host, domain) && domain.len() >= best {
                if domain.len() > best || excluded {
                    allowed = !excluded;
                }
                best = domain.len();
            }
        }
        allowed
    }
    fn bytes(&self) -> usize {
        owned_bytes(&self.raw)
    }
}
fn owned_bytes(value: &Cow<'static, str>) -> usize {
    match value {
        Cow::Borrowed(_) => 0,
        Cow::Owned(s) => s.capacity(),
    }
}
pub fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.'))
}
pub fn canonical_host(host: &str) -> Option<String> {
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty() || host.len() > 253 || !host.is_ascii() {
        return None;
    }
    if host.split('.').count() == 4 && host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        let octets = host
            .split('.')
            .map(str::parse::<u8>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        return Some(alloc::format!(
            "{}.{}.{}.{}",
            octets[0],
            octets[1],
            octets[2],
            octets[3]
        ));
    }
    if !host.split('.').all(|l| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

#[derive(Clone, Debug)]
pub struct CosmeticRule {
    pub domains: Domains,
    pub selector: Cow<'static, str>,
    pub exception: bool,
}

#[derive(Clone, Debug)]
pub struct CompiledList {
    pub diagnostics: Diagnostics,
    pub version: Option<String>,
    pub checksum: Option<String>,
    pub cosmetic: Vec<CosmeticRule>,
    pub(crate) network: Vec<network::NetworkRule>,
    pub(crate) host_index: Vec<(Cow<'static, str>, usize)>,
    pub(crate) token_index: Vec<(u32, usize)>,
    pub(crate) fallback: Vec<usize>,
}
impl CompiledList {
    /// Owned vector/string capacities; excludes allocator metadata and immutable source text.
    pub fn compiled_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
            + self.network.capacity() * core::mem::size_of::<network::NetworkRule>()
            + self.cosmetic.capacity() * core::mem::size_of::<CosmeticRule>()
            + self
                .network
                .iter()
                .map(|r| {
                    owned_bytes(&r.pattern)
                        + r.domains.bytes()
                        + r.host.as_ref().map_or(0, owned_bytes)
                })
                .sum::<usize>()
            + self
                .cosmetic
                .iter()
                .map(|r| owned_bytes(&r.selector) + r.domains.bytes())
                .sum::<usize>()
            + self.host_index.capacity() * core::mem::size_of::<(Cow<'static, str>, usize)>()
            + self
                .host_index
                .iter()
                .map(|(key, _)| owned_bytes(key))
                .sum::<usize>()
            + self.token_index.capacity() * core::mem::size_of::<(u32, usize)>()
            + self.fallback.capacity() * core::mem::size_of::<usize>()
    }
}

#[derive(Clone, Debug)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub source: String,
    pub enabled: bool,
    pub last_successful_update: Option<u64>,
    pub compiled: Arc<CompiledList>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterSettings {
    pub enabled: bool,
    pub disabled_lists: BTreeSet<String>,
    /// Exact canonical host identities; subdomains do not inherit user exceptions.
    pub site_exceptions: BTreeSet<String>,
}
impl Default for FilterSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled_lists: BTreeSet::new(),
            site_exceptions: BTreeSet::new(),
        }
    }
}
impl FilterSettings {
    pub fn active_for(&self, document: &CanonicalUrl) -> bool {
        self.enabled && !self.site_exceptions.contains(&document.host)
    }
    pub fn set_site_exception(&mut self, host: &str, disabled: bool) -> bool {
        let Some(host) = canonical_host(host) else {
            return false;
        };
        if disabled {
            if self.site_exceptions.len() >= MAX_SITE_EXCEPTIONS
                && !self.site_exceptions.contains(&host)
            {
                return false;
            }
            self.site_exceptions.insert(host);
        } else {
            self.site_exceptions.remove(&host);
        }
        true
    }
    pub fn encode(&self) -> Option<String> {
        if self.disabled_lists.len() > MAX_LISTS
            || self.site_exceptions.len() > MAX_SITE_EXCEPTIONS
            || self.disabled_lists.iter().any(|s| {
                s.is_empty()
                    || s.len() > 32
                    || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            || self
                .site_exceptions
                .iter()
                .any(|s| canonical_host(s).as_ref() != Some(s))
        {
            return None;
        }
        let mut s = alloc::format!("shield-v1\nenabled={}\n", u8::from(self.enabled));
        for id in &self.disabled_lists {
            s.push_str(&alloc::format!("disabled={id}\n"));
        }
        for site in &self.site_exceptions {
            s.push_str(&alloc::format!("site={site}\n"));
        }
        (s.len() <= 4096).then_some(s)
    }
    pub fn decode(s: &str) -> Option<Self> {
        if s.len() > 4096 {
            return None;
        }
        let mut lines = s.lines();
        if lines.next() != Some("shield-v1") {
            return None;
        }
        let enabled = match lines.next()? {
            "enabled=1" => true,
            "enabled=0" => false,
            _ => return None,
        };
        let mut settings = Self {
            enabled,
            ..Self::default()
        };
        for line in lines {
            if let Some(id) = line.strip_prefix("disabled=") {
                if id.is_empty()
                    || id.len() > 32
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || settings.disabled_lists.len() >= MAX_LISTS
                {
                    return None;
                }
                settings.disabled_lists.insert(id.into());
            } else if let Some(host) = line.strip_prefix("site=") {
                if !settings.set_site_exception(host, true) {
                    return None;
                }
            } else {
                return None;
            }
        }
        Some(settings)
    }
}

/// Immutable compiled rules are shared by workers. Mutations replace the revision;
/// browser owners must reject completions carrying older revisions.
#[derive(Clone, Debug)]
pub struct FilterListManager {
    pub settings: FilterSettings,
    pub lists: Vec<Subscription>,
    pub revision: u64,
    suffixes: Arc<network::PublicSuffixes>,
}
impl FilterListManager {
    pub fn new() -> Self {
        Self {
            settings: FilterSettings::default(),
            lists: Vec::new(),
            revision: 1,
            suffixes: Arc::new(network::PublicSuffixes::bundled()),
        }
    }
    pub fn add(
        &mut self,
        id: &str,
        name: &str,
        source: &str,
        text: &str,
        selector_valid: fn(&str) -> bool,
    ) -> Result<(), CompileError> {
        self.add_compiled(id, name, source, compile(text, selector_valid)?)
    }
    pub fn add_static(
        &mut self,
        id: &str,
        name: &str,
        source: &str,
        text: &'static str,
        selector_valid: fn(&str) -> bool,
    ) -> Result<(), CompileError> {
        self.add_compiled(id, name, source, compile_static(text, selector_valid)?)
    }
    fn add_compiled(
        &mut self,
        id: &str,
        name: &str,
        source: &str,
        compiled: CompiledList,
    ) -> Result<(), CompileError> {
        if self.lists.len() >= MAX_LISTS
            || id.is_empty()
            || id.len() > 32
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || name.len() > 128
            || source.len() > 2048
            || self.lists.iter().any(|s| s.id == id)
        {
            return Err(CompileError::Limit);
        }
        let compiled = Arc::new(compiled);
        if compiled.compiled_bytes()
            + self
                .lists
                .iter()
                .map(|s| s.compiled.compiled_bytes())
                .sum::<usize>()
            > 48 * 1024 * 1024
        {
            return Err(CompileError::Limit);
        }
        self.lists.push(Subscription {
            id: id.into(),
            name: name.into(),
            source: source.into(),
            enabled: true,
            last_successful_update: None,
            compiled,
        });
        self.changed();
        Ok(())
    }
    pub fn replace(
        &mut self,
        id: &str,
        text: &str,
        updated: u64,
        selector_valid: fn(&str) -> bool,
    ) -> Result<(), CompileError> {
        let candidate = Arc::new(compile(text, selector_valid)?);
        if candidate.compiled_bytes()
            + self
                .lists
                .iter()
                .filter(|s| s.id != id)
                .map(|s| s.compiled.compiled_bytes())
                .sum::<usize>()
            > 48 * 1024 * 1024
        {
            return Err(CompileError::Limit);
        }
        let list = self
            .lists
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(CompileError::InvalidList)?;
        list.compiled = candidate;
        list.last_successful_update = Some(updated);
        self.changed();
        Ok(())
    }
    pub fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
    }
    pub fn active_lists(&self) -> impl Iterator<Item = &Subscription> {
        self.lists
            .iter()
            .filter(|s| s.enabled && !self.settings.disabled_lists.contains(&s.id))
    }
    pub fn evaluate(
        &self,
        request: &CanonicalUrl,
        document: &CanonicalUrl,
        kind: ResourceType,
    ) -> Decision {
        if kind == ResourceType::Document || !self.settings.active_for(document) {
            return Decision::Allow;
        }
        let party = self.suffixes.site(&request.host) != self.suffixes.site(&document.host);
        let mut budget = 1_000_000usize;
        let mut blocked = false;
        // Evaluate every eligible allow rule before publishing a block. A work
        // limit must never turn an unexamined exception into a broader block.
        for list in self.active_lists() {
            for index in list.compiled.candidates(request) {
                let rule = &list.compiled.network[index];
                if rule.cosmetic_exception != 0 {
                    continue;
                }
                match rule.matches(request, document, kind, party, &mut budget) {
                    Some(true) if rule.exception => return Decision::Exception,
                    Some(true) => blocked = true,
                    Some(false) => {}
                    None => return Decision::WorkLimit,
                }
            }
        }
        if blocked {
            Decision::Block
        } else {
            Decision::Allow
        }
    }
    /// `(disable all, disable generic)` page exceptions, including work-limit
    /// fallback. Consumers may use indexed rule candidates without copying all
    /// generic selectors into each document.
    pub fn cosmetic_modes(&self, document: &CanonicalUrl) -> (bool, bool) {
        if !self.settings.active_for(document) {
            return (true, true);
        }
        let mut all_disabled = false;
        let mut generic_disabled = false;
        let mut budget = 1_000_000;
        for list in self.active_lists() {
            for i in list.compiled.candidates(document) {
                let r = &list.compiled.network[i];
                if r.cosmetic_exception == 0 {
                    continue;
                }
                match r.matches(
                    document,
                    document,
                    ResourceType::Document,
                    false,
                    &mut budget,
                ) {
                    Some(true) => {
                        all_disabled |= r.cosmetic_exception == 1;
                        generic_disabled |= r.cosmetic_exception == 2;
                    }
                    None => return (true, true),
                    _ => {}
                }
            }
        }
        (all_disabled, generic_disabled)
    }
    pub fn cosmetic_selectors(&self, document: &CanonicalUrl) -> Vec<String> {
        let (all_disabled, generic_disabled) = self.cosmetic_modes(document);
        if all_disabled {
            return Vec::new();
        }
        let mut exceptions = BTreeSet::new();
        let mut hides = BTreeSet::new();
        for list in self.active_lists() {
            for rule in &list.compiled.cosmetic {
                if rule.domains.matches(&document.host) {
                    if rule.exception {
                        exceptions.insert(rule.selector.clone());
                    } else if !(generic_disabled && rule.domains.is_generic()) {
                        hides.insert(rule.selector.clone());
                    }
                }
            }
        }
        hides.retain(|s| !exceptions.contains(s));
        hides.into_iter().map(Cow::into_owned).collect()
    }
}
impl Default for FilterListManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
