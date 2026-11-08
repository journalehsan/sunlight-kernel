//! Compiled-in Rappid Rabbit start page and its tightly allow-listed routes.

use alloc::string::String;
use golden_fish::{parse_html_with_limits, Document, ParseLimits};
use std::sync::OnceLock;

const HTML_TEMPLATE: &str = include_str!("../assets/startpage.html");
const CSS: &str = include_str!("../assets/startpage.css");

static SOURCE: OnceLock<String> = OnceLock::new();
static DOCUMENT: OnceLock<Document> = OnceLock::new();

/// Recognize custom-scheme URLs so they can be routed before HTTP parsing.
pub fn is_custom_url(url: &str) -> bool {
    url.get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("rappid:"))
        || url
            .get(..9)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("rappid://"))
}

/// Resolve only the two currently supported spellings of the start page.
pub fn resolve_custom_url(url: &str) -> Option<&'static str> {
    if url.eq_ignore_ascii_case("rappid:startpage")
        || url.eq_ignore_ascii_case("rappid://startpage")
    {
        Some("rappid:startpage")
    } else {
        None
    }
}

/// Source with the separately maintained stylesheet inlined for Golden Fish.
pub fn source() -> &'static str {
    SOURCE
        .get_or_init(|| HTML_TEMPLATE.replace("<!-- STARTPAGE_CSS -->", CSS))
        .as_str()
}

/// Return a clone of the parsed immutable DOM. Styles and layout are rebuilt
/// by the caller for its current viewport, while HTML parsing is cached once.
pub fn document() -> Document {
    DOCUMENT
        .get_or_init(|| {
            parse_html_with_limits(source(), ParseLimits::default())
                .expect("compiled Rappid start page must parse")
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_supported_start_page_routes() {
        assert_eq!(
            resolve_custom_url("rappid:startpage"),
            Some("rappid:startpage")
        );
        assert_eq!(
            resolve_custom_url("rappid://startpage"),
            Some("rappid:startpage")
        );
        assert!(is_custom_url("rappid:unknown"));
        assert!(is_custom_url("RAPPID://unknown"));
        assert_eq!(resolve_custom_url("rappid:unknown"), None);
    }

    #[test]
    fn embedded_fixture_has_logo_search_and_quick_links() {
        let html = source();
        assert!(html.contains("SunlightOS"));
        assert!(html.contains("id=\"search-box\""));
        assert!(html.contains("Explore"));
        assert!(html.contains("DistroWatch"));
        assert!(html.contains("https://distrowatch.com/"));
        assert!(html.contains("https://github.com/journalehsan/sunlight-kernel"));
        assert!(!html.contains("wiki.archlinux.org"));
    }
}
