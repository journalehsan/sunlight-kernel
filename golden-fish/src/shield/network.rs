use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalUrl {
    pub url: String,
    pub host: String,
    pub(crate) host_start: usize,
}
impl CanonicalUrl {
    /// ASCII HTTP(S) URLs only, matching the browser's current transport. Reject
    /// userinfo, encoded hosts, IPv6 and ambiguous authorities rather than
    /// guessing a hostname. Unicode must already be converted to A-labels.
    pub fn parse(input: &str) -> Option<Self> {
        if input.len() > 8192
            || input
                .bytes()
                .any(|b| b.is_ascii_control() || b == b' ' || b == b'\\')
        {
            return None;
        }
        let (scheme, rest) = input.split_once("://")?;
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return None;
        }
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
            (h, Some(p.parse::<u16>().ok()?))
        } else {
            (authority, None)
        };
        if port == Some(0) {
            return None;
        }
        let host = canonical_host(host)?;
        let mut url = alloc::format!("{}://{}", scheme.to_ascii_lowercase(), host);
        if let Some(port) = port.filter(|p| {
            *p != if scheme.eq_ignore_ascii_case("https") {
                443
            } else {
                80
            }
        }) {
            url.push_str(&alloc::format!(":{port}"));
        }
        let path = rest[end..].split('#').next().unwrap_or("");
        if !path.starts_with('/') {
            url.push('/');
        }
        url.push_str(path);
        Some(Self {
            host_start: scheme.len() + 3,
            url,
            host,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ResourceType {
    Document,
    Image,
    Stylesheet,
    Script,
    Frame,
    Media,
    Other,
    XmlHttpRequest,
    Font,
}
impl ResourceType {
    pub(crate) fn bit(self) -> u16 {
        1 << self as u8
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Exception,
    Block,
    WorkLimit,
}

#[derive(Clone, Debug)]
pub(crate) struct NetworkRule {
    pub pattern: Cow<'static, str>,
    pub host: Option<Cow<'static, str>>,
    pub domains: Domains,
    pub exception: bool,
    pub cosmetic_exception: u8,
    pub types: u16,
    pub party: Option<bool>,
    pub start: bool,
    pub end: bool,
    pub match_case: bool,
}
impl NetworkRule {
    pub fn matches(
        &self,
        url: &CanonicalUrl,
        document: &CanonicalUrl,
        kind: ResourceType,
        third_party: bool,
        budget: &mut usize,
    ) -> Option<bool> {
        if self.types & kind.bit() == 0
            || self.party.is_some_and(|p| p != third_party)
            || !self.domains.matches(&document.host)
        {
            return Some(false);
        }
        let text = if let Some(domain) = &self.host {
            if !domain_matches(&url.host, domain) {
                return Some(false);
            }
            &url.url[url.host_start + url.host.len() - domain.len()..]
        } else {
            &url.url
        };
        wildcard_match(
            self.pattern.as_bytes(),
            text.as_bytes(),
            self.start || self.host.is_some(),
            self.end,
            self.match_case,
            budget,
        )
    }
}

/// Non-recursive glob with one backtracking point. A shared per-request work
/// budget also bounds adversarial wildcard inputs and exception evaluation.
fn wildcard_match(
    pattern: &[u8],
    text: &[u8],
    start: bool,
    end: bool,
    case: bool,
    budget: &mut usize,
) -> Option<bool> {
    let mut p = 0;
    let mut t = 0;
    let mut star = if start { None } else { Some((0, 0)) };
    loop {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        if p == pattern.len() {
            if !end || t == text.len() {
                return Some(true);
            }
        } else if pattern[p] == b'*' {
            p += 1;
            star = Some((p, t));
            continue;
        } else if pattern[p] == b'^' && (t == text.len() || separator(text[t])) {
            p += 1;
            if t < text.len() {
                t += 1;
            }
            continue;
        } else if t < text.len()
            && if case {
                pattern[p] == text[t]
            } else {
                pattern[p].eq_ignore_ascii_case(&text[t])
            }
        {
            p += 1;
            t += 1;
            continue;
        }
        if let Some((retry, at)) = star {
            if at == text.len() {
                return Some(false);
            }
            t = at + 1;
            p = retry;
            star = Some((retry, t));
        } else {
            return Some(false);
        }
    }
}
fn separator(b: u8) -> bool {
    !b.is_ascii_alphanumeric() && !matches!(b, b'_' | b'-' | b'.' | b'%')
}
pub(crate) fn token(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([
        bytes[0].to_ascii_lowercase(),
        bytes[1].to_ascii_lowercase(),
        bytes[2].to_ascii_lowercase(),
        bytes[3].to_ascii_lowercase(),
    ])
}
impl CompiledList {
    pub(crate) fn candidates(&self, url: &CanonicalUrl) -> Vec<usize> {
        let mut ids = self.fallback.clone();
        let mut host = url.host.as_str();
        loop {
            let start = self
                .host_index
                .partition_point(|(key, _)| key.as_ref() < host);
            let end = self
                .host_index
                .partition_point(|(key, _)| key.as_ref() <= host);
            ids.extend(self.host_index[start..end].iter().map(|(_, i)| *i));
            let Some((_, rest)) = host.split_once('.') else {
                break;
            };
            host = rest;
        }
        let tokens: BTreeSet<u32> = url.url.as_bytes().windows(4).map(token).collect();
        for token in tokens {
            let start = self.token_index.partition_point(|(key, _)| *key < token);
            let end = self.token_index.partition_point(|(key, _)| *key <= token);
            ids.extend(self.token_index[start..end].iter().map(|(_, i)| *i));
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PublicSuffixes {
    rules: BTreeSet<&'static str>,
}
impl PublicSuffixes {
    pub fn bundled() -> Self {
        Self {
            rules: include_str!("../../filters/public_suffix_ascii.txt")
                .lines()
                .collect(),
        }
    }
    pub fn site<'a>(&self, host: &'a str) -> &'a str {
        if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return host;
        }
        let labels: Vec<usize> = core::iter::once(0)
            .chain(host.match_indices('.').map(|(i, _)| i + 1))
            .collect();
        let mut suffix_labels = 1usize;
        for (i, &start) in labels.iter().enumerate() {
            let suffix = &host[start..];
            let count = labels.len() - i;
            if self.rules.contains(alloc::format!("!{suffix}").as_str()) {
                suffix_labels = count.saturating_sub(1);
                break;
            }
            if self.rules.contains(suffix) {
                suffix_labels = suffix_labels.max(count);
            }
            if i > 0 && self.rules.contains(alloc::format!("*.{suffix}").as_str()) {
                suffix_labels = suffix_labels.max(count + 1);
            }
        }
        &host[labels[labels.len().saturating_sub(suffix_labels + 1)]..]
    }
}
