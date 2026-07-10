//! Conversion from the tl parser backend into Golden Fish's owned DOM.
//!
//! This module is the only place that knows about `tl` types. All public
//! APIs of Golden Fish return only Golden Fish owned types.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use tl::{ParserOptions, VDom};

use crate::attributes::Attribute;
use crate::document::Document as GoldenDoc;
use crate::error::ParseError;
use crate::node::{Node, NodeId};

/// Parse source HTML using tl and convert the result into a Golden Fish `Document`.
///
/// Always produces a Golden Fish `Document` with a root `Node::Document`.
/// Child content from the parser (including fragments) becomes children of the root.
///
/// On genuinely unrecoverable parse failure, returns `ParseError`.
/// Partial / malformed but parsable input yields the best-effort tree.
pub fn parse_with_tl(source: &str) -> Result<GoldenDoc, ParseError> {
    // Use the stable parser only (no simd feature).
    let dom = match tl::parse(source, ParserOptions::default()) {
        Ok(d) => d,
        Err(e) => {
            // tl::ParseError is just InvalidLength today.
            return Err(ParseError::new(alloc::format!("parse error: {e}")));
        }
    };

    Ok(convert_vdom_to_golden(&dom, source))
}

fn convert_vdom_to_golden(dom: &VDom<'_>, _source: &str) -> GoldenDoc {
    let mut golden = GoldenDoc::new();
    let root_id = golden.root();

    // VDom::children() returns &[NodeHandle] for top-level nodes.
    for handle in dom.children() {
        if let Some(tl_node) = handle.get(dom.parser()) {
            let child_id = convert_node(&mut golden, tl_node, dom.parser());
            golden.append_child(root_id, child_id);
            golden.set_parent(child_id, root_id);
        }
    }

    golden
}

/// Recursively convert a tl node into a Golden Fish node and return its id.
fn convert_node(golden: &mut GoldenDoc, tl_node: &tl::Node<'_>, parser: &tl::Parser<'_>) -> NodeId {
    match tl_node {
        tl::Node::Tag(tag) => {
            let tag_name = tag.name().as_utf8_str().into_owned();

            // Attributes: iter() yields (Cow<str>, Option<Cow<str>>)
            let mut attrs: Vec<Attribute> = Vec::new();
            for (k, v_opt) in tag.attributes().iter() {
                let name = k.into_owned();
                let value = v_opt.map(Cow::into_owned).unwrap_or_default();
                attrs.push(Attribute::new(name, value));
            }

            let elem_id = golden.alloc_node(Node::Element {
                tag_name,
                attributes: attrs,
                children: Vec::new(),
            });

            // Children via tag.children().top() -> &RawChildren (InlineVec)
            for child_handle in tag.children().top().iter() {
                if let Some(child_tl) = child_handle.get(parser) {
                    let child_id = convert_node(golden, child_tl, parser);
                    golden.append_child(elem_id, child_id);
                    golden.set_parent(child_id, elem_id);
                }
            }

            elem_id
        }
        tl::Node::Raw(bytes) => {
            // Raw text content
            let content = bytes.as_utf8_str().into_owned();
            golden.alloc_node(Node::Text { content })
        }
        tl::Node::Comment(bytes) => {
            let content = bytes.as_utf8_str().into_owned();
            golden.alloc_node(Node::Comment { content })
        }
    }
}
