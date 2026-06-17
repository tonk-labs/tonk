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
///
/// `tl` is a lenient, non-spec parser; its tree does not always match
/// the browser's HTML tree construction. Since binding paths are
/// child-index paths shared with the browser planner, the tree must
/// match the browser DOM. [`normalize`] applies the two
/// tree-construction rules that bite real templates: tag-omission
/// auto-closing (`<li>`/`<p>`/table cells that `tl` nests but the
/// browser makes siblings) and the implicit `<tbody>` around table
/// rows.
pub fn parse_fragment(html: &str) -> Vec<Node> {
    let dom = match tl::parse(html, tl::ParserOptions::default()) {
        Ok(dom) => dom,
        Err(_) => return Vec::new(),
    };
    let parser = dom.parser();
    let mut roots = convert_children(&dom, parser);
    normalize(&mut roots);
    roots
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

/// Recursively reshape `nodes` to match the browser's HTML tree
/// construction for the cases `tl` gets wrong.
fn normalize(nodes: &mut [Node]) {
    for node in nodes.iter_mut() {
        if let Node::Element(el) = node {
            normalize(&mut el.children);
            unnest_auto_closed(&mut el.children);
        }
    }
    // The implicit-tbody rewrite is applied to each element's own
    // children below (a `<table>` wrapping its `<tr>` children); do it
    // after recursing so inner tables are handled too.
    for node in nodes.iter_mut() {
        if let Node::Element(el) = node
            && el.tag == "table"
        {
            wrap_table_rows(&mut el.children);
        }
    }
}

/// Tags whose end tag is optional and which therefore auto-close when
/// another element of the same "implied-end" set opens. `tl` instead
/// nests them; the browser makes them siblings. We handle the common
/// case: an element of one of these tags directly containing, as a
/// trailing child, another element that should have closed it.
fn auto_close_peers(tag: &str) -> &'static [&'static str] {
    match tag {
        // A new <li> closes an open <li>.
        "li" => &["li"],
        // A new block-level element closes an open <p>. (Common
        // template peers; not the full spec list.)
        "p" => &["p", "div", "ul", "ol", "table", "section", "article"],
        // Table cells/rows/groups close their same-level peers.
        "td" => &["td", "th"],
        "th" => &["td", "th"],
        "tr" => &["tr"],
        "thead" | "tbody" | "tfoot" => &["thead", "tbody", "tfoot"],
        "option" => &["option"],
        "dt" | "dd" => &["dt", "dd"],
        _ => &[],
    }
}

/// Lift wrongly-nested auto-closed elements to siblings. For each
/// child element whose tag has auto-close peers, split off the
/// trailing run starting at the first peer child and re-insert it
/// after the element, repeatedly until stable.
fn unnest_auto_closed(children: &mut Vec<Node>) {
    let mut i = 0;
    while i < children.len() {
        let Node::Element(el) = &children[i] else {
            i += 1;
            continue;
        };
        let peers = auto_close_peers(&el.tag);
        if peers.is_empty() {
            i += 1;
            continue;
        }
        // Find the first child of `el` that is a peer element.
        let split_at = el
            .children
            .iter()
            .position(|c| matches!(c, Node::Element(inner) if peers.contains(&inner.tag.as_str())));
        let Some(split_at) = split_at else {
            i += 1;
            continue;
        };
        // Move `el.children[split_at..]` out to become siblings after `el`.
        let Node::Element(el) = &mut children[i] else {
            unreachable!()
        };
        let lifted: Vec<Node> = el.children.split_off(split_at);
        for (offset, node) in lifted.into_iter().enumerate() {
            children.insert(i + 1 + offset, node);
        }
        // Re-examine `el` (more peers may remain nested) on the next
        // loop turn by NOT advancing `i` past it; advance to the first
        // lifted sibling, which we then process in turn.
        i += 1;
    }
}

/// Wrap bare `<tr>` children of a `<table>` in an implicit `<tbody>`,
/// matching the browser. Existing `<thead>`/`<tbody>`/`<tfoot>` and
/// non-row nodes (caption, colgroup, whitespace) are left in place; a
/// contiguous run of bare `<tr>` is grouped into one `<tbody>`.
fn wrap_table_rows(children: &mut Vec<Node>) {
    let mut out: Vec<Node> = Vec::with_capacity(children.len());
    let mut pending_rows: Vec<Node> = Vec::new();
    for node in std::mem::take(children) {
        let is_bare_row = matches!(&node, Node::Element(e) if e.tag == "tr");
        if is_bare_row {
            pending_rows.push(node);
        } else {
            flush_tbody(&mut out, &mut pending_rows);
            out.push(node);
        }
    }
    flush_tbody(&mut out, &mut pending_rows);
    *children = out;
}

fn flush_tbody(out: &mut Vec<Node>, rows: &mut Vec<Node>) {
    if rows.is_empty() {
        return;
    }
    out.push(Node::Element(Element {
        tag: "tbody".to_string(),
        attrs: Vec::new(),
        children: std::mem::take(rows),
        void: false,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render the tree shape as `tag>tag` paths for comparison with
    /// the browser DOM captured via Chrome DevTools.
    fn shape(nodes: &[Node]) -> Vec<String> {
        fn walk(nodes: &[Node], prefix: &str, out: &mut Vec<String>) {
            for (i, n) in nodes.iter().enumerate() {
                let label = match n {
                    Node::Element(e) => e.tag.clone(),
                    Node::Text(t) => format!("#text({t:?})"),
                    Node::Comment(_) => "#comment".into(),
                };
                let path = format!("{prefix}[{i}] {label}");
                out.push(path.clone());
                walk(n.children(), &format!("{prefix}  "), out);
            }
        }
        let mut out = Vec::new();
        walk(nodes, "", &mut out);
        out
    }

    #[test]
    fn it_inserts_an_implicit_tbody_around_table_rows() {
        // Browser DOM: table > tbody > tr > td > #text
        let r = parse_fragment("<table><tr data-id={this}><td>{name}</td></tr></table>");
        assert_eq!(
            shape(&r),
            vec![
                "[0] table".to_string(),
                "  [0] tbody".to_string(),
                "    [0] tr".to_string(),
                "      [0] td".to_string(),
                "        [0] #text(\"{name}\")".to_string(),
            ]
        );
    }

    #[test]
    fn it_makes_tag_omitted_li_siblings() {
        // Browser DOM: ul > [li>{a}, li>{b}]
        let r = parse_fragment("<ul><li>{a}<li>{b}</ul>");
        assert_eq!(
            shape(&r),
            vec![
                "[0] ul".to_string(),
                "  [0] li".to_string(),
                "    [0] #text(\"{a}\")".to_string(),
                "  [1] li".to_string(),
                "    [0] #text(\"{b}\")".to_string(),
            ]
        );
    }

    #[test]
    fn it_makes_tag_omitted_p_siblings() {
        let r = parse_fragment("<div><p>{a}<p>{b}</div>");
        assert_eq!(
            shape(&r),
            vec![
                "[0] div".to_string(),
                "  [0] p".to_string(),
                "    [0] #text(\"{a}\")".to_string(),
                "  [1] p".to_string(),
                "    [0] #text(\"{b}\")".to_string(),
            ]
        );
    }

    #[test]
    fn it_leaves_well_formed_markup_unchanged() {
        let r = parse_fragment("<ul><li>{a}</li><li>{b}</li></ul>");
        assert_eq!(
            shape(&r),
            vec![
                "[0] ul".to_string(),
                "  [0] li".to_string(),
                "    [0] #text(\"{a}\")".to_string(),
                "  [1] li".to_string(),
                "    [0] #text(\"{b}\")".to_string(),
            ]
        );
    }

    #[test]
    fn it_keeps_an_explicit_tbody() {
        let r = parse_fragment("<table><tbody><tr><td>x</td></tr></tbody></table>");
        assert_eq!(
            shape(&r),
            vec![
                "[0] table".to_string(),
                "  [0] tbody".to_string(),
                "    [0] tr".to_string(),
                "      [0] td".to_string(),
                "        [0] #text(\"x\")".to_string(),
            ]
        );
    }
}
