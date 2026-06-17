//! Serialize the [`Node`] tree back to an HTML string.

use crate::tree::{Node, is_void_tag};

/// Serialize a list of nodes (a fragment) to HTML.
pub fn serialize_nodes(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        write_node(&mut out, node);
    }
    out
}

fn write_node(out: &mut String, node: &Node) {
    match node {
        Node::Text(t) => out.push_str(&escape_text(t)),
        Node::Comment(c) => {
            out.push_str("<!--");
            out.push_str(c);
            out.push_str("-->");
        }
        Node::Element(el) => {
            out.push('<');
            out.push_str(&el.tag);
            for (name, value) in &el.attrs {
                out.push(' ');
                out.push_str(name);
                out.push_str("=\"");
                out.push_str(&escape_attr(value));
                out.push('"');
            }
            out.push('>');
            if el.void || is_void_tag(&el.tag) {
                // Void elements have no closing tag and no children.
                return;
            }
            for child in &el.children {
                write_node(out, child);
            }
            out.push_str("</");
            out.push_str(&el.tag);
            out.push('>');
        }
    }
}

/// Escape text-node content: `&`, `<`, `>`.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Escape a double-quoted attribute value: `&`, `"`.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}
