//! Serialize the [`Node`] tree back to an HTML string.

use crate::tree::{Node, is_raw_text_element, is_void_tag};

/// Serialize a list of nodes (a fragment) to HTML.
pub fn serialize_nodes(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        write_node(&mut out, node, false);
    }
    out
}

/// `raw` is true when the parent is a raw-text element
/// (`<style>`/`<script>`): its text children are emitted verbatim,
/// not HTML-escaped, since their content is CSS/JS (where `<`, `>`,
/// `&` are literal), matching the HTML serialization spec.
fn write_node(out: &mut String, node: &Node, raw: bool) {
    match node {
        Node::Text(t) => {
            if raw {
                out.push_str(t);
            } else {
                out.push_str(&escape_text(t));
            }
        }
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
            let child_raw = is_raw_text_element(node);
            for child in &el.children {
                write_node(out, child, child_raw);
            }
            out.push_str("</");
            out.push_str(&el.tag);
            out.push('>');
        }
    }
}

/// Escape text-node content: `&`, `<`, `>`. The parser (`html5gum`)
/// decodes entity references into characters, so the tree holds plain
/// text; escaping each special character once reproduces the browser's
/// decode-then-reencode round-trip (`&amp;` -> `&` -> `&amp;`).
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

/// Escape a double-quoted attribute value: `&` and `"`.
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

#[cfg(test)]
mod tests {
    use crate::{parse_fragment, serialize_nodes};

    #[test]
    fn it_does_not_escape_style_content() {
        let r = parse_fragment("<style>.a > .b { color: red }</style>");
        assert_eq!(serialize_nodes(&r), "<style>.a > .b { color: red }</style>");
    }

    #[test]
    fn it_does_not_double_escape_existing_entities() {
        let r = parse_fragment("<p>a &amp; b &lt; c</p>");
        assert_eq!(serialize_nodes(&r), "<p>a &amp; b &lt; c</p>");
    }

    #[test]
    fn it_still_escapes_a_bare_ampersand() {
        // A bare `&` not starting an entity is still escaped.
        let r = parse_fragment("<p>Tom &  Jerry</p>");
        assert_eq!(serialize_nodes(&r), "<p>Tom &amp;  Jerry</p>");
    }

    #[test]
    fn it_decodes_numeric_entities_to_characters() {
        // The parser decodes entities, so `&#169;`/`&#x41;` become the
        // characters `©`/`A` (the browser does the same in the DOM).
        let r = parse_fragment("<p>&#169; &#x41;</p>");
        assert_eq!(serialize_nodes(&r), "<p>© A</p>");
    }
}
