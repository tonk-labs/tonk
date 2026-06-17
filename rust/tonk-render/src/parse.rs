//! Parse an HTML template string into the owned [`Node`] tree.
//!
//! `tl` produces a borrowed `VDom`; we walk it once and build our
//! own owned tree so the rest of the renderer is `tl`-free and the
//! tree is mutable (the binding collector splits text nodes in
//! place).

use tl::{Node as TlNode, Parser, VDom};

use crate::tree::{Element, Node, is_void_tag};

/// Parse `html` into a list of top-level [`Node`]s (a fragment can
/// have multiple roots).
pub fn parse_fragment(html: &str) -> Vec<Node> {
    let dom = match tl::parse(html, tl::ParserOptions::default()) {
        Ok(dom) => dom,
        Err(_) => return Vec::new(),
    };
    let parser = dom.parser();
    convert_children(&dom, parser)
}

/// Convert the VDom's top-level children into owned nodes.
fn convert_children(dom: &VDom, parser: &Parser) -> Vec<Node> {
    dom.children()
        .iter()
        .filter_map(|h| convert_node(h.get(parser)?, parser))
        .collect()
}

/// Convert one `tl` node (and its subtree) into an owned [`Node`].
/// Comments are kept (they hold an index slot in the DOM, so the
/// binding paths must account for them).
fn convert_node(node: &TlNode, parser: &Parser) -> Option<Node> {
    match node {
        TlNode::Tag(tag) => {
            // Lowercase tag + attribute names to match how the browser
            // normalizes HTML names in the DOM (so void/raw-text checks
            // and attribute comparisons agree regardless of source case).
            let name = tag.name().as_utf8_str().to_ascii_lowercase();
            let attrs = tag
                .attributes()
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_ascii_lowercase(),
                        v.map(|v| v.to_string()).unwrap_or_default(),
                    )
                })
                .collect();
            let void = is_void_tag(&name);
            let children = if void {
                Vec::new()
            } else {
                tag.children()
                    .top()
                    .iter()
                    .filter_map(|h| convert_node(h.get(parser)?, parser))
                    .collect()
            };
            Some(Node::Element(Element {
                tag: name,
                attrs,
                children,
                void,
            }))
        }
        TlNode::Raw(bytes) => Some(Node::Text(bytes.as_utf8_str().to_string())),
        TlNode::Comment(bytes) => {
            // `tl` hands back the raw `<!-- ... -->`; strip the
            // delimiters to store just the inner text.
            let raw = bytes.as_utf8_str();
            let inner = raw
                .strip_prefix("<!--")
                .and_then(|s| s.strip_suffix("-->"))
                .unwrap_or(&raw)
                .to_string();
            Some(Node::Comment(inner))
        }
    }
}
