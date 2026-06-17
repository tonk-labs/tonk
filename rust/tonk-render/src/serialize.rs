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

/// Escape text-node content: `<`, `>`, and `&` (but not an `&` that
/// already begins a valid entity reference, so a literal `&amp;` in
/// the template round-trips to `&amp;` rather than `&amp;amp;` — the
/// browser parses entities into characters then re-encodes once, a
/// net identity round-trip).
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    for (i, ch) in s.char_indices() {
        match ch {
            '&' if starts_entity(&bytes[i..]) => out.push('&'),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Escape a double-quoted attribute value: `"` and `&` (same
/// entity-aware rule as [`escape_text`]).
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    for (i, ch) in s.char_indices() {
        match ch {
            '&' if starts_entity(&bytes[i..]) => out.push('&'),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// True if `rest` (starting at an `&`) begins a syntactically valid
/// HTML entity reference: `&name;` or `&#123;` / `&#xAB;`. Used so
/// already-encoded entities in template source aren't double-encoded.
fn starts_entity(rest: &[u8]) -> bool {
    debug_assert_eq!(rest.first(), Some(&b'&'));
    let after = &rest[1..];
    let body = if after.first() == Some(&b'#') {
        match after.get(1) {
            Some(b'x') | Some(b'X') => &after[2..],
            _ => &after[1..],
        }
    } else {
        after
    };
    // Find a `;` within a reasonable span, with only entity-name /
    // numeric chars before it.
    let numeric = after.first() == Some(&b'#');
    for (seen, &b) in body.iter().enumerate() {
        if b == b';' {
            return seen > 0;
        }
        let ok = if numeric {
            b.is_ascii_hexdigit()
        } else {
            b.is_ascii_alphanumeric()
        };
        if !ok || seen > 32 {
            return false;
        }
    }
    false
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
    fn it_escapes_a_numeric_entity_passthrough() {
        let r = parse_fragment("<p>&#169; &#x41;</p>");
        assert_eq!(serialize_nodes(&r), "<p>&#169; &#x41;</p>");
    }
}
