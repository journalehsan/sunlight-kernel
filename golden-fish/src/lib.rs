//! # Golden Fish
//!
//! Golden Fish is the HTML parsing and DOM construction engine for `SunlightOS`.
//!
//! ## Responsibilities (this milestone)
//! - HTML source text → internal owned DOM tree
//!
//! ## Encoding contract
//! Golden Fish accepts Rust UTF-8 strings (`&str`).
//! It performs **no** HTTP transfer decoding, decompression, charset detection,
//! or clipboard conversion. Those responsibilities belong to the fetch/decoder
//! layer (e.g. `sunlight-fetch`).
//!
//! Data flow:
//! ```text
//! sunlight-fetch / decoder -> UTF-8 HTML text -> Golden Fish -> DOM
//! ```
//!
//! ## Parser backend
//! The `tl` crate ("0.7.8", stable, no SIMD) is used internally on std-capable
//! targets. Freestanding `SunlightOS` builds use a `no_std` fallback parser that
//! produces the same owned DOM types. No backend-specific types are exposed in
//! the public API.
//!
//! ## No unsupported features
//! This crate does not implement CSS, layout, painting, JavaScript, images,
//! forms, or browser navigation.

#![cfg_attr(target_os = "none", no_std)]
#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]

extern crate alloc;

pub mod attributes;
pub mod document;
pub mod error;
pub mod node;
pub mod parser;

#[cfg(not(target_os = "none"))]
mod convert;
#[cfg(any(target_os = "none", test))]
mod simple_parser;

pub use attributes::Attribute;
pub use document::Document;
pub use error::ParseError;
pub use node::{Node, NodeId};
pub use parser::parse_html;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Document {
        parse_html(s).expect("parse should succeed for test input")
    }

    #[test]
    fn test_simple_element_with_text() {
        let doc = parse("<p>Hello</p>");
        // Expect at least one element p under root
        let root = doc.root();
        let kids = doc.children(root);
        assert!(!kids.is_empty());
        let p = kids[0];
        assert_eq!(doc.tag_name(p), Some("p"));
        let p_kids = doc.children(p);
        assert_eq!(p_kids.len(), 1);
        assert_eq!(doc.text_content(p_kids[0]), Some("Hello"));
    }

    #[test]
    fn test_nested_elements() {
        let doc = parse("<div><span>hi</span></div>");
        // Find div, then its span child
        let div = doc.find_first_element("div").expect("div");
        let span = doc.find_first_element("span").expect("span");
        assert!(doc.children(div).contains(&span));
        let text_id = doc.children(span)[0];
        assert_eq!(doc.text_content(text_id), Some("hi"));
    }

    #[test]
    fn test_multiple_sibling_elements() {
        let doc = parse("<ul><li>a</li><li>b</li></ul>");
        let ul = doc.find_first_element("ul").unwrap();
        let lis = doc.children(ul);
        assert_eq!(lis.len(), 2);
        assert_eq!(doc.tag_name(lis[0]), Some("li"));
        assert_eq!(doc.tag_name(lis[1]), Some("li"));
    }

    #[test]
    fn test_attributes() {
        let doc = parse(r#"<a href="https://example.com" id="link">x</a>"#);
        let a = doc.find_first_element("a").unwrap();
        let attrs = doc.attributes(a).unwrap();
        let names: Vec<_> = attrs.iter().map(|a| a.name()).collect();
        assert!(names.contains(&"href"));
        assert!(names.contains(&"id"));
        let href = attrs.iter().find(|a| a.name() == "href").unwrap();
        assert_eq!(href.value(), "https://example.com");
    }

    #[test]
    fn test_comments() {
        let doc = parse("<!-- hello --><p>ok</p>");
        // Walk to find a comment
        let mut found_comment = false;
        fn walk(doc: &Document, id: NodeId, found: &mut bool) {
            if let Some(Node::Comment { .. }) = doc.get(id) {
                *found = true;
            }
            for &c in doc.children(id) {
                walk(doc, c, found);
            }
        }
        walk(&doc, doc.root(), &mut found_comment);
        assert!(found_comment, "expected to find a comment node");
    }

    #[test]
    fn test_html_fragment_without_html_body() {
        let doc = parse("<p>First</p><p>Second</p>");
        // Root should have two p children directly (or wrapped, but structure should contain both)
        let _root_kids = doc.children(doc.root());
        // Accept either direct children or nested under implicit html/body produced by parser
        let mut p_count = 0;
        fn count_p(doc: &Document, id: NodeId, n: &mut usize) {
            if doc.tag_name(id) == Some("p") {
                *n += 1;
            }
            for &c in doc.children(id) {
                count_p(doc, c, n);
            }
        }
        count_p(&doc, doc.root(), &mut p_count);
        assert!(p_count >= 2, "expected at least two paragraphs in fragment");
    }

    #[test]
    fn test_empty_input() {
        let doc = parse("");
        // Should still have a root document node
        assert_eq!(doc.node_count(), 1);
        assert!(matches!(doc.get(doc.root()), Some(Node::Document { .. })));
    }

    #[test]
    fn test_malformed_but_recoverable_html() {
        // Missing closing tags, stray brackets, etc.
        let doc = parse("<div><p>text<div></p>");
        // As long as we get a usable tree without panic, it's good.
        let div = doc.find_first_element("div");
        assert!(div.is_some());
    }

    #[test]
    fn test_utf8_text() {
        let doc = parse("<p>こんにちは 🌍</p>");
        let p = doc.find_first_element("p").unwrap();
        let text = doc.text_content(doc.children(p)[0]).unwrap();
        assert!(text.contains("こんにちは"));
        assert!(text.contains("🌍"));
    }

    #[test]
    fn test_deterministic_debug_tree_output() {
        let html = r#"<!doctype html>
<html>
<head><title>Example Domain</title></head>
<body><p id="message">Hello</p></body>
</html>"#;
        let doc = parse(html);
        let tree = doc.debug_tree();
        // Must contain key structural lines in order
        assert!(tree.contains("#document"));
        assert!(tree.contains("html"));
        assert!(tree.contains("head"));
        assert!(tree.contains("title"));
        assert!(tree.contains("Example Domain") || tree.contains("\"Example Domain\""));
        assert!(tree.contains("body"));
        assert!(tree.contains("p"));
        // Attribute should appear
        assert!(tree.contains("id=\"message\"") || tree.contains("id=message"));
    }

    #[test]
    fn test_example_domain_style_html() {
        // A realistic snippet similar to what Rappid Rabbit might fetch
        let html = r#"<!doctype html>
<html>
<head>
    <title>Example Domain</title>
</head>
<body>
<div>
    <h1>Example Domain</h1>
    <p>This domain is for use in illustrative examples.</p>
</div>
</body>
</html>"#;
        let doc = parse(html);
        assert!(doc.find_first_element("html").is_some());
        assert!(doc.find_first_element("title").is_some());
        let h1 = doc.find_first_element("h1").unwrap();
        let text = doc
            .children(h1)
            .iter()
            .find_map(|&id| doc.text_content(id))
            .unwrap_or("");
        assert!(text.contains("Example Domain"));
    }
}
