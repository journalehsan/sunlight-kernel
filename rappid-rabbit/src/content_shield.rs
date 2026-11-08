//! Golden Fish policy adapter. Reuses Rabbit's selector matcher; owns no DOM.
use crate::css::{parse_selector, selector_matches, Selector};
pub use golden_fish::shield::*;
use golden_fish::{Document, Node, NodeId};
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
    time::Instant,
};
mod settings;
pub use settings::{load_settings, save_settings};

#[derive(Debug, Default)]
pub struct ShieldStats {
    pub cancelled: AtomicBool,
    pub blocked: AtomicUsize,
    pub evaluated: AtomicUsize,
    pub work_limits: AtomicUsize,
    pub match_nanos: AtomicUsize,
}
pub fn check_request(
    manager: &FilterListManager,
    stats: &ShieldStats,
    url: &sunlight_http::ParsedUrl,
    document: &str,
    kind: ResourceType,
) -> Result<(), sunlight_fetch::FetchError> {
    if stats.cancelled.load(Ordering::Acquire) {
        return Err(sunlight_fetch::FetchError::Interrupted);
    }
    let started = Instant::now();
    let decision = request_decision(manager, &crate::format_url(url), document, kind);
    stats.evaluated.fetch_add(1, Ordering::Relaxed);
    stats.match_nanos.fetch_add(
        started.elapsed().as_nanos().min(usize::MAX as u128) as usize,
        Ordering::Relaxed,
    );
    match decision {
        Decision::Block => {
            stats.blocked.fetch_add(1, Ordering::Relaxed);
            sunlight_ipc::debug_log(&format!(
                "[SHIELD] blocked type={kind:?} revision={}",
                manager.revision
            ));
            Err(sunlight_fetch::FetchError::BlockedByPolicy)
        }
        Decision::WorkLimit => {
            stats.work_limits.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Filter selectors deliberately exclude pseudo classes (the CSS engine models
/// some dynamic states approximately), sibling combinators, escapes and lists.
/// Unsupported selectors are rejected in their entirety, never truncated.
pub fn filter_selector(text: &str) -> Option<Selector> {
    if !text.is_ascii()
        || text.len() > 256
        || text.trim_start().starts_with('>')
        || text.trim_end().ends_with('>')
    {
        return None;
    }
    let mut quote = None;
    let mut bracket = false;
    let mut prior_child = false;
    for ch in text.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        if bracket {
            match ch {
                '\'' | '"' => quote = Some(ch),
                ']' => bracket = false,
                '[' | '{' | '}' | ';' | '\\' => return None,
                _ => {}
            }
        } else {
            match ch {
                '[' => {
                    bracket = true;
                    prior_child = false;
                }
                '>' => {
                    if prior_child {
                        return None;
                    }
                    prior_child = true;
                }
                c if c.is_ascii_whitespace() => {}
                c if c.is_ascii_alphanumeric() || matches!(c, '.' | '#' | '*' | '_' | '-') => {
                    prior_child = false
                }
                _ => return None,
            }
        }
    }
    if bracket || quote.is_some() {
        return None;
    }
    // The author CSS parser is forgiving about unquoted attribute values.
    for body in text.split('[').skip(1) {
        let attr = body.split(']').next()?;
        if let Some((_, value)) = attr.split_once('=') {
            let value = value
                .trim()
                .strip_suffix(" i")
                .unwrap_or(value.trim())
                .trim();
            if value.is_empty() {
                return None;
            }
            if let Some(quote) = value.chars().next().filter(|c| matches!(c, '\'' | '"')) {
                if value.len() < 2
                    || !value.ends_with(quote)
                    || value[1..value.len() - 1].contains(quote)
                {
                    return None;
                }
            } else if !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            {
                return None;
            }
        }
    }
    let selector = parse_selector(text)?;
    (selector.parts.len() <= 8).then_some(selector)
}
pub fn selector_valid(text: &str) -> bool {
    filter_selector(text).is_some()
}

pub fn bundled_manager() -> FilterListManager {
    static BUNDLED: OnceLock<FilterListManager> = OnceLock::new();
    BUNDLED
        .get_or_init(|| {
            let started = Instant::now();
            let mut manager = FilterListManager::new();
            manager
                .add_static(
                    "easylist",
                    "EasyList",
                    "https://easylist-downloads.adblockplus.org/easylist.txt",
                    EASYLIST,
                    selector_valid,
                )
                .expect("validated bundled EasyList");
            let list = &manager.lists[0].compiled;
            sunlight_ipc::debug_log(&format!(
                "[SHIELD] parse_us={} supported={} unsupported={} compiled_bytes={}",
                started.elapsed().as_micros(),
                list.diagnostics.supported,
                list.diagnostics.unsupported,
                list.compiled_bytes()
            ));
            manager
        })
        .clone()
}

#[derive(Clone, Debug, Default)]
pub struct CosmeticPolicy {
    pub revision: u64,
    host: String,
    lists: Vec<(Arc<CompiledList>, Arc<CosmeticIndex>)>,
    exceptions: BTreeSet<Cow<'static, str>>,
    generic_disabled: bool,
}
#[derive(Debug, Default)]
struct CosmeticIndex {
    index: Vec<(Cow<'static, str>, usize)>,
    fallback: Vec<usize>,
    exceptions: Vec<usize>,
}
struct IndexEntry {
    list: std::sync::Weak<CompiledList>,
    index: Arc<CosmeticIndex>,
}
fn cosmetic_index(list: &Arc<CompiledList>) -> Arc<CosmeticIndex> {
    static CACHE: OnceLock<std::sync::Mutex<Vec<IndexEntry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    {
        let mut entries = cache.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|e| e.list.strong_count() > 0);
        if let Some(entry) = entries
            .iter()
            .find(|e| core::ptr::eq(e.list.as_ptr(), Arc::as_ptr(list)))
        {
            return entry.index.clone();
        }
    }
    let start = Instant::now();
    let mut index = CosmeticIndex::default();
    for (i, rule) in list.cosmetic.iter().enumerate() {
        if rule.exception {
            index.exceptions.push(i);
            continue;
        }
        let key = match &rule.selector {
            Cow::Borrowed(text) => selector_hint(text),
            Cow::Owned(text) => selector_hint(text).map(|k| Cow::Owned(k.into_owned())),
        };
        if let Some(key) = key {
            index.index.push((key, i));
        } else {
            index.fallback.push(i);
        }
    }
    index.index.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let index = Arc::new(index);
    let mut entries = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = entries
        .iter()
        .find(|e| core::ptr::eq(e.list.as_ptr(), Arc::as_ptr(list)))
    {
        return entry.index.clone();
    }
    if entries.len() >= MAX_LISTS {
        entries.remove(0);
    }
    entries.push(IndexEntry {
        list: Arc::downgrade(list),
        index: index.clone(),
    });
    sunlight_ipc::debug_log(&format!(
        "[SHIELD] cosmetic_index_us={} rules={}",
        start.elapsed().as_micros(),
        list.cosmetic.len()
    ));
    index
}
impl CosmeticPolicy {
    pub fn for_document(manager: &FilterListManager, url: &str) -> Arc<Self> {
        let mut policy = Self {
            revision: manager.revision,
            ..Self::default()
        };
        if let Some(url) = CanonicalUrl::parse(url) {
            let (disabled, generic_disabled) = manager.cosmetic_modes(&url);
            if disabled {
                return Arc::new(policy);
            }
            policy.host = url.host;
            policy.generic_disabled = generic_disabled;
            for subscription in manager.active_lists() {
                let list = subscription.compiled.clone();
                let index = cosmetic_index(&list);
                for &i in &index.exceptions {
                    let rule = &list.cosmetic[i];
                    if rule.domains.matches(&policy.host) {
                        policy.exceptions.insert(rule.selector.clone());
                    }
                }
                policy.lists.push((list, index));
            }
        }
        Arc::new(policy)
    }
    pub fn hidden_nodes(&self, document: &Document) -> BTreeSet<NodeId> {
        if self.lists.is_empty() {
            return BTreeSet::new();
        }
        let started = Instant::now();
        let mut hidden = BTreeSet::new();
        let mut budget = 1_000_000usize;
        let mut compiled = BTreeMap::<(usize, usize), Option<Selector>>::new();
        let max_depth = document.stats().max_depth.saturating_add(1);
        // Attribute values can be much larger than a selector. Account for
        // their potential scan cost too, and bound feature-key allocation.
        let mut attribute_bytes = BTreeMap::<String, usize>::new();
        let mut max_attributes = 0usize;
        for node in 0..document.node_count() {
            if let Some(Node::Element { attributes, .. }) = document.get(node) {
                let bytes: usize = attributes
                    .iter()
                    .map(|a| a.name().len() + a.value().len())
                    .sum();
                if attributes.len() > 128
                    || bytes > 64 * 1024
                    || budget < bytes
                    || attributes.iter().any(|a| a.name().len() > 128)
                {
                    sunlight_ipc::debug_log("[SHIELD] cosmetic feature limit; policy not applied");
                    return BTreeSet::new();
                }
                budget -= bytes;
                max_attributes = max_attributes.max(attributes.len());
                for attr in attributes {
                    let entry = attribute_bytes
                        .entry(attr.name().to_ascii_lowercase())
                        .or_default();
                    *entry = (*entry).max(attr.value().len());
                    if attribute_bytes.len() > 256 {
                        sunlight_ipc::debug_log(
                            "[SHIELD] cosmetic feature limit; policy not applied",
                        );
                        return BTreeSet::new();
                    }
                }
            }
        }
        for node in 0..document.node_count() {
            let Some(Node::Element {
                tag_name,
                attributes,
                ..
            }) = document.get(node)
            else {
                continue;
            };
            let mut keys = vec![tag_name.to_ascii_lowercase()];
            for attr in attributes {
                keys.push(format!("[{}", attr.name().to_ascii_lowercase()));
                if attr.name().eq_ignore_ascii_case("id") {
                    keys.push(format!("#{}", attr.value()));
                }
                if attr.name().eq_ignore_ascii_case("class") {
                    for c in attr.value().split_ascii_whitespace() {
                        if keys.len() >= 256 {
                            sunlight_ipc::debug_log(
                                "[SHIELD] cosmetic feature limit; policy not applied",
                            );
                            return BTreeSet::new();
                        }
                        keys.push(format!(".{c}"));
                    }
                }
            }
            'lists: for (list_number, (list, index)) in self.lists.iter().enumerate() {
                let mut candidates = index.fallback.clone();
                for key in &keys {
                    let start = index
                        .index
                        .partition_point(|(k, _)| k.as_ref() < key.as_str());
                    let end = index
                        .index
                        .partition_point(|(k, _)| k.as_ref() <= key.as_str());
                    candidates.extend(index.index[start..end].iter().map(|(_, i)| *i));
                }
                candidates.sort_unstable();
                candidates.dedup();
                for i in candidates {
                    if budget == 0 {
                        return BTreeSet::new();
                    }
                    budget -= 1;
                    let rule = &list.cosmetic[i];
                    if !rule.domains.matches(&self.host)
                        || self.generic_disabled && rule.domains.is_generic()
                        || self.exceptions.contains(&rule.selector)
                    {
                        continue;
                    }
                    let cache_key = (list_number, i);
                    if !compiled.contains_key(&cache_key) {
                        if budget < rule.selector.len() {
                            return BTreeSet::new();
                        }
                        budget -= rule.selector.len();
                        compiled.insert(cache_key, filter_selector(&rule.selector));
                    }
                    let Some(selector) = compiled[&cache_key].as_ref() else {
                        continue;
                    };
                    let part_work: usize = selector
                        .parts
                        .iter()
                        .map(|part| {
                            1 + usize::from(part.id.is_some())
                                * attribute_bytes.get("id").copied().unwrap_or(0)
                                + part.classes.len()
                                    * attribute_bytes.get("class").copied().unwrap_or(0)
                                + part
                                    .attributes
                                    .iter()
                                    .filter(|a| !a.1.is_empty())
                                    .map(|a| {
                                        attribute_bytes
                                            .get(&a.0.to_ascii_lowercase())
                                            .copied()
                                            .unwrap_or(0)
                                    })
                                    .sum::<usize>()
                        })
                        .sum();
                    let work = max_depth
                        .saturating_mul(part_work)
                        .saturating_mul(max_attributes + 1);
                    if budget < work {
                        sunlight_ipc::debug_log("[SHIELD] cosmetic work limit; policy not applied");
                        return BTreeSet::new();
                    }
                    budget -= work;
                    if selector_matches(document, node, selector) {
                        hidden.insert(node);
                        break 'lists;
                    }
                }
            }
        }
        sunlight_ipc::debug_log(&format!(
            "[SHIELD] cosmetic_us={} hidden={} revision={}",
            started.elapsed().as_micros(),
            hidden.len(),
            self.revision
        ));
        hidden
    }
}

/// A conservative dispatch hint for the already-validated selector subset.
/// This does not match selectors: all decisions still use the CSS matcher.
/// Keeping strings/indexes avoids constructing tens of thousands of selector
/// ASTs for a document that has only a few relevant IDs/classes/attributes.
fn selector_hint(text: &str) -> Option<Cow<'_, str>> {
    let mut quote = None;
    let mut bracket = false;
    let mut start = 0;
    for (at, ch) in text.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' if bracket => quote = Some(ch),
            '[' => bracket = true,
            ']' => bracket = false,
            c if !bracket && (c.is_ascii_whitespace() || c == '>') => start = at + c.len_utf8(),
            _ => {}
        }
    }
    let part = &text[start..];
    let bytes = part.as_bytes();
    let mut i = 0;
    let mut class = None;
    let mut attribute = None;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let end = part[i + 1..].find(']')? + i + 1;
            let name = part[i + 1..end]
                .trim_start()
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                .next()?;
            if attribute.is_none() {
                let raw = &part[i..i + 1 + name.len()];
                attribute = Some(if raw == format!("[{}", name.to_ascii_lowercase()) {
                    Cow::Borrowed(raw)
                } else {
                    Cow::Owned(format!("[{}", name.to_ascii_lowercase()))
                });
            }
            i = end + 1;
            continue;
        }
        if matches!(bytes[i], b'#' | b'.') {
            let marker = bytes[i];
            let from = i + 1;
            i = from;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_'))
            {
                i += 1;
            }
            if marker == b'#' {
                return Some(Cow::Borrowed(&part[from - 1..i]));
            }
            if class.is_none() {
                class = Some(Cow::Borrowed(&part[from - 1..i]));
            }
        } else {
            i += 1;
        }
    }
    class.or(attribute).or_else(|| {
        let name = part
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .next()?;
        (!name.is_empty()).then(|| {
            if name.bytes().any(|b| b.is_ascii_uppercase()) {
                Cow::Owned(name.to_ascii_lowercase())
            } else {
                Cow::Borrowed(name)
            }
        })
    })
}

pub fn request_decision(
    manager: &FilterListManager,
    url: &str,
    document: &str,
    kind: ResourceType,
) -> Decision {
    match (CanonicalUrl::parse(url), CanonicalUrl::parse(document)) {
        (Some(url), Some(document)) => manager.evaluate(&url, &document, kind),
        _ => Decision::Allow, // Transport validation still decides unsupported URLs.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_does_not_retain_replaced_lists_or_misread_attribute_values() {
        let mut manager = FilterListManager::new();
        manager
            .add(
                "test",
                "Test",
                "bundled:test",
                "[Adblock Plus 2.0]\n##[data-ad=\"a#b\"]",
                selector_valid,
            )
            .unwrap();
        let weak = Arc::downgrade(&manager.lists[0].compiled);
        let policy = CosmeticPolicy::for_document(&manager, "https://page.test/");
        let dom =
            golden_fish::parse_html("<div data-ad='a#b'>ad</div><p id='b'>normal</p>").unwrap();
        let hidden = policy.hidden_nodes(&dom);
        assert!(hidden.contains(&dom.find_first_element("div").unwrap()));
        assert!(!hidden.contains(&dom.find_first_element("p").unwrap()));
        manager
            .replace("test", "[Adblock Plus 2.0]\n##.new", 1, selector_valid)
            .unwrap();
        assert!(weak.upgrade().is_some());
        drop(policy);
        assert!(weak.upgrade().is_none());
        for n in 0..16 {
            let old = Arc::downgrade(&manager.lists[0].compiled);
            let policy = CosmeticPolicy::for_document(&manager, "https://page.test/");
            manager
                .replace(
                    "test",
                    &format!("[Adblock Plus 2.0]\n##.rule{n}"),
                    n,
                    selector_valid,
                )
                .unwrap();
            drop(policy);
            assert!(old.upgrade().is_none());
        }
    }
}
