#[cfg(feature = "dom")]
use alloc::vec;
use alloc::{format, string::String, vec::Vec};

use sunlight_http::ParsedUrl;

use crate::{
    format_url,
    resources::request::{ResourcePriority, ResourceType},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryClassification {
    RenderCriticalResource,
    EmbeddedResource,
    ExplicitPreload,
    ExplicitPrefetch,
    OrdinaryNavigationLink,
}

impl DiscoveryClassification {
    pub const fn label(self) -> &'static str {
        match self {
            Self::RenderCriticalResource => "render-critical",
            Self::EmbeddedResource => "embedded",
            Self::ExplicitPreload => "preload",
            Self::ExplicitPrefetch => "prefetch",
            Self::OrdinaryNavigationLink => "navigation",
        }
    }

    pub const fn priority(self) -> ResourcePriority {
        match self {
            Self::RenderCriticalResource => ResourcePriority::RenderCritical,
            Self::EmbeddedResource => ResourcePriority::Embedded,
            Self::ExplicitPreload => ResourcePriority::ExplicitPreload,
            Self::ExplicitPrefetch => ResourcePriority::ExplicitPrefetch,
            Self::OrdinaryNavigationLink => ResourcePriority::Navigation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCandidate {
    pub raw_url: String,
    pub resolved_url: String,
    pub resource_type: ResourceType,
    pub classification: DiscoveryClassification,
    pub enqueue_for_fetch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlResolutionError {
    Empty,
    InvalidAbsoluteUrl(String),
    UnsupportedScheme(String),
}

impl UrlResolutionError {
    pub fn message(&self) -> String {
        match self {
            Self::Empty => String::from("empty URL"),
            Self::InvalidAbsoluteUrl(value) => format!("invalid absolute URL: {value}"),
            Self::UnsupportedScheme(scheme) => format!("unsupported URL scheme: {scheme}"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceQueue {
    pending: Vec<ResourceCandidate>,
}

impl ResourceQueue {
    pub fn replace_from_candidates(&mut self, candidates: &[ResourceCandidate]) {
        self.pending.clear();
        self.pending.extend(
            candidates
                .iter()
                .filter(|candidate| candidate.enqueue_for_fetch)
                .cloned(),
        );
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    pub fn pending(&self) -> &[ResourceCandidate] {
        &self.pending
    }
}

#[cfg(feature = "dom")]
pub fn discover_resources(
    document: &golden_fish::Document,
    base_url: &ParsedUrl,
) -> Vec<ResourceCandidate> {
    use golden_fish::Node;

    let mut discovered = Vec::new();
    let mut stack = vec![document.root()];

    while let Some(node_id) = stack.pop() {
        let Some(node) = document.get(node_id) else {
            continue;
        };

        match node {
            Node::Document { children } => {
                for &child in children.iter().rev() {
                    stack.push(child);
                }
            }
            Node::Element {
                tag_name,
                attributes,
                children,
            } => {
                for candidate in candidates_for_element(tag_name, attributes, base_url) {
                    push_candidate(&mut discovered, candidate);
                }
                for &child in children.iter().rev() {
                    stack.push(child);
                }
            }
            Node::Text { .. } | Node::Comment { .. } => {}
        }
    }

    discovered
}

#[cfg(feature = "dom")]
fn candidates_for_element(
    tag_name: &str,
    attributes: &[golden_fish::Attribute],
    base_url: &ParsedUrl,
) -> Vec<ResourceCandidate> {
    let mut out = Vec::new();
    let lower_tag = tag_name.to_ascii_lowercase();

    match lower_tag.as_str() {
        "link" => {
            let Some(raw_url) = attribute_value(attributes, "href") else {
                return out;
            };
            let rel = attribute_value(attributes, "rel")
                .unwrap_or_default()
                .to_ascii_lowercase();

            let (resource_type, classification, enqueue_for_fetch) =
                if rel_contains(&rel, "stylesheet") {
                    (
                        ResourceType::Stylesheet,
                        DiscoveryClassification::RenderCriticalResource,
                        false,
                    )
                } else if rel_contains(&rel, "preload") {
                    (
                        ResourceType::Preload,
                        DiscoveryClassification::ExplicitPreload,
                        true,
                    )
                } else if rel_contains(&rel, "prefetch") {
                    (
                        ResourceType::Prefetch,
                        DiscoveryClassification::ExplicitPrefetch,
                        true,
                    )
                } else {
                    (
                        ResourceType::Other,
                        DiscoveryClassification::EmbeddedResource,
                        false,
                    )
                };

            if let Ok(resolved_url) = resolve_url(base_url, raw_url) {
                out.push(ResourceCandidate {
                    raw_url: String::from(raw_url),
                    resolved_url,
                    resource_type,
                    classification,
                    enqueue_for_fetch,
                });
            }
        }
        "img" => push_resolved_candidate(
            &mut out,
            base_url,
            attribute_value(attributes, "src"),
            ResourceType::Image,
            DiscoveryClassification::EmbeddedResource,
            false,
        ),
        "script" => push_resolved_candidate(
            &mut out,
            base_url,
            attribute_value(attributes, "src"),
            ResourceType::Script,
            DiscoveryClassification::EmbeddedResource,
            false,
        ),
        "iframe" => push_resolved_candidate(
            &mut out,
            base_url,
            attribute_value(attributes, "src"),
            ResourceType::Frame,
            DiscoveryClassification::EmbeddedResource,
            false,
        ),
        "source" => push_resolved_candidate(
            &mut out,
            base_url,
            attribute_value(attributes, "src"),
            ResourceType::Media,
            DiscoveryClassification::EmbeddedResource,
            false,
        ),
        "a" => push_resolved_candidate(
            &mut out,
            base_url,
            attribute_value(attributes, "href"),
            ResourceType::Navigation,
            DiscoveryClassification::OrdinaryNavigationLink,
            false,
        ),
        _ => {}
    }

    out
}

#[cfg(feature = "dom")]
fn push_resolved_candidate(
    out: &mut Vec<ResourceCandidate>,
    base_url: &ParsedUrl,
    raw_url: Option<&str>,
    resource_type: ResourceType,
    classification: DiscoveryClassification,
    enqueue_for_fetch: bool,
) {
    let Some(raw_url) = raw_url else {
        return;
    };
    if let Ok(resolved_url) = resolve_url(base_url, raw_url) {
        out.push(ResourceCandidate {
            raw_url: String::from(raw_url),
            resolved_url,
            resource_type,
            classification,
            enqueue_for_fetch,
        });
    }
}

#[cfg(feature = "dom")]
fn attribute_value<'a>(attributes: &'a [golden_fish::Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(golden_fish::Attribute::value)
}

#[cfg(feature = "dom")]
fn rel_contains(rel: &str, token: &str) -> bool {
    rel.split_ascii_whitespace()
        .any(|part| part.eq_ignore_ascii_case(token))
}

#[cfg(feature = "dom")]
fn push_candidate(out: &mut Vec<ResourceCandidate>, candidate: ResourceCandidate) {
    if out.iter().any(|existing| {
        existing.resource_type == candidate.resource_type
            && existing.resolved_url == candidate.resolved_url
    }) {
        return;
    }
    out.push(candidate);
}

pub fn resolve_url(base_url: &ParsedUrl, raw_url: &str) -> Result<String, UrlResolutionError> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(UrlResolutionError::Empty);
    }

    if let Some(scheme) = extract_scheme(trimmed) {
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return Err(UrlResolutionError::UnsupportedScheme(String::from(scheme)));
        }
        let sanitized = strip_fragment(trimmed);
        return ParsedUrl::parse(&sanitized)
            .map(|url| format_url(&url))
            .map_err(|_| UrlResolutionError::InvalidAbsoluteUrl(String::from(trimmed)));
    }

    if let Some(authority_relative) = trimmed.strip_prefix("//") {
        let absolute = format!("{}://{authority_relative}", scheme_name(base_url));
        let sanitized = strip_fragment(&absolute);
        return ParsedUrl::parse(&sanitized)
            .map(|url| format_url(&url))
            .map_err(|_| UrlResolutionError::InvalidAbsoluteUrl(String::from(trimmed)));
    }

    if trimmed.starts_with('#') {
        return Ok(strip_fragment(&format_url(base_url)));
    }

    let mut resolved_path = if trimmed.starts_with('/') {
        strip_fragment(trimmed)
    } else if trimmed.starts_with('?') {
        let base_path = strip_suffix_from_path(&base_url.path);
        format!("{base_path}{trimmed}")
    } else {
        let (base_path, _) = split_suffix_from_path(&base_url.path);
        let joined = join_relative_path(base_path, trimmed);
        strip_fragment(&joined)
    };

    if resolved_path.is_empty() {
        resolved_path = String::from("/");
    } else if !resolved_path.starts_with('/') {
        resolved_path.insert(0, '/');
    }

    Ok(format_base_with_path(base_url, &resolved_path))
}

fn format_base_with_path(base_url: &ParsedUrl, path: &str) -> String {
    let prefix = format!("{}://{}", scheme_name(base_url), base_url.host);
    let default_port = if base_url.uses_tls() { 443 } else { 80 };
    if base_url.port == default_port {
        format!("{prefix}{path}")
    } else {
        format!("{prefix}:{}{path}", base_url.port)
    }
}

fn scheme_name(base_url: &ParsedUrl) -> &'static str {
    if base_url.uses_tls() {
        "https"
    } else {
        "http"
    }
}

fn extract_scheme(value: &str) -> Option<&str> {
    let colon = value.find(':')?;
    let scheme = &value[..colon];
    if scheme.is_empty() {
        return None;
    }
    if scheme
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        Some(scheme)
    } else {
        None
    }
}

fn strip_fragment(value: &str) -> String {
    value
        .split_once('#')
        .map_or_else(|| String::from(value), |(prefix, _)| String::from(prefix))
}

fn strip_suffix_from_path(path: &str) -> String {
    let (path_only, _) = split_suffix_from_path(path);
    String::from(path_only)
}

fn split_suffix_from_path(path: &str) -> (&str, Option<&str>) {
    if let Some(index) = path.find(['?', '#']) {
        (&path[..index], path.get(index..))
    } else {
        (path, None)
    }
}

fn join_relative_path(base_path: &str, relative: &str) -> String {
    let (relative_path, suffix) = split_suffix_from_path(relative);

    let mut segments = Vec::new();
    let base_dir = if base_path.ends_with('/') {
        base_path
    } else {
        base_path
            .rsplit_once('/')
            .map_or("/", |(dir, _)| if dir.is_empty() { "/" } else { dir })
    };

    for segment in base_dir.split('/') {
        if !segment.is_empty() {
            segments.push(String::from(segment));
        }
    }

    for segment in relative_path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(String::from(value)),
        }
    }

    let mut joined = String::from("/");
    joined.push_str(&segments.join("/"));
    if relative_path.ends_with('/') && !joined.ends_with('/') {
        joined.push('/');
    }
    if let Some(suffix) = suffix {
        joined.push_str(suffix);
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "dom")]
    use golden_fish::parse_html;

    fn base_url() -> ParsedUrl {
        ParsedUrl::parse("https://example.com/docs/page.html?query=1").unwrap()
    }

    #[test]
    fn resolve_relative_urls_against_final_document_url() {
        let resolved = resolve_url(&base_url(), "../site.css").unwrap();
        assert_eq!(resolved, "https://example.com/site.css");
    }

    #[test]
    fn resolve_root_relative_urls() {
        let resolved = resolve_url(&base_url(), "/assets/app.js").unwrap();
        assert_eq!(resolved, "https://example.com/assets/app.js");
    }

    #[test]
    fn resolve_query_only_urls() {
        let resolved = resolve_url(&base_url(), "?updated=1").unwrap();
        assert_eq!(resolved, "https://example.com/docs/page.html?updated=1");
    }

    #[test]
    fn resolve_fragment_only_urls_without_panicking() {
        let resolved = resolve_url(&base_url(), "#intro").unwrap();
        assert_eq!(resolved, "https://example.com/docs/page.html?query=1");
    }

    #[test]
    fn reject_unsupported_schemes() {
        let err = resolve_url(&base_url(), "mailto:test@example.com").unwrap_err();
        assert_eq!(
            err,
            UrlResolutionError::UnsupportedScheme(String::from("mailto"))
        );
    }

    #[cfg(feature = "dom")]
    #[test]
    fn discover_stylesheet_image_script_and_prefetch_resources() {
        let document = parse_html(
            r#"
                <html>
                    <head>
                        <link rel="stylesheet" href="/site.css">
                        <link rel="prefetch" href="/next.html">
                    </head>
                    <body>
                        <img src="hero.png">
                        <script src="/app.js"></script>
                    </body>
                </html>
            "#,
        )
        .unwrap();

        let candidates = discover_resources(&document, &base_url());
        let summary: Vec<(ResourceType, DiscoveryClassification, &str, bool)> = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.resource_type,
                    candidate.classification,
                    candidate.resolved_url.as_str(),
                    candidate.enqueue_for_fetch,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    ResourceType::Stylesheet,
                    DiscoveryClassification::RenderCriticalResource,
                    "https://example.com/site.css",
                    false,
                ),
                (
                    ResourceType::Prefetch,
                    DiscoveryClassification::ExplicitPrefetch,
                    "https://example.com/next.html",
                    true,
                ),
                (
                    ResourceType::Image,
                    DiscoveryClassification::EmbeddedResource,
                    "https://example.com/docs/hero.png",
                    false,
                ),
                (
                    ResourceType::Script,
                    DiscoveryClassification::EmbeddedResource,
                    "https://example.com/app.js",
                    false,
                ),
            ]
        );
    }

    #[cfg(feature = "dom")]
    #[test]
    fn duplicate_resources_are_suppressed_per_type() {
        let document = parse_html(
            r#"
                <link rel="stylesheet" href="/site.css">
                <link rel="stylesheet" href="/site.css">
                <img src="/site.css">
            "#,
        )
        .unwrap();

        let candidates = discover_resources(&document, &base_url());
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].resource_type, ResourceType::Stylesheet);
        assert_eq!(candidates[1].resource_type, ResourceType::Image);
    }

    #[cfg(feature = "dom")]
    #[test]
    fn ordinary_anchor_links_are_not_queued_for_fetch() {
        let document = parse_html(r#"<a href="/docs/getting-started">Docs</a>"#).unwrap();
        let candidates = discover_resources(&document, &base_url());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].resource_type, ResourceType::Navigation);
        assert_eq!(
            candidates[0].classification,
            DiscoveryClassification::OrdinaryNavigationLink
        );
        assert!(!candidates[0].enqueue_for_fetch);
    }
}
