//! Parse an HTML template string into the owned [`Node`] tree.
//!
//! Built on `html5gum`, a spec-compliant HTML tokenizer. We drive it
//! with a small custom callback that preserves attribute source order
//! (the default emitter sorts attributes into a `BTreeMap`), then
//! assemble the token stream into the owned, mutable [`Node`] tree the
//! rest of the renderer walks. `html5gum` tokenizes correctly where the
//! previous parser did not: unquoted attribute values containing `/`,
//! runs of bare boolean attributes, and `<style>`/`<script>` raw-text
//! bodies. A residual tree-construction pass ([`normalize`]) still
//! applies the insertion-mode rules a bare tokenizer doesn't (implicit
//! `<tbody>`, `<li>`/`<p>` auto-closing).

use html5gum::Tokenizer;
use html5gum::emitters::callback::{Callback, CallbackEmitter, CallbackEvent};

use crate::tree::{Element, Node, is_void_tag};

/// One token, with attributes kept in source order (a `Vec`, not the
/// default emitter's sorted `BTreeMap`).
enum RawToken {
    /// A start tag: name, ordered attributes, and the self-closing flag.
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    /// An end tag.
    End { name: String },
    /// A run of text.
    Text(String),
    /// A comment's inner text.
    Comment(String),
}

/// Collects tokenizer events into [`RawToken`]s with ordered attributes.
#[derive(Default)]
struct OrderedCallback {
    tag_name: String,
    attrs: Vec<(String, String)>,
}

impl Callback<RawToken> for OrderedCallback {
    fn handle_event(&mut self, event: CallbackEvent<'_>) -> Option<RawToken> {
        match event {
            CallbackEvent::OpenStartTag { name } => {
                self.tag_name = utf8(name);
                self.attrs.clear();
                None
            }
            CallbackEvent::AttributeName { name } => {
                // A new attribute begins; ignore a duplicate name (WHATWG
                // keeps the first) by skipping if already present.
                let name = utf8(name);
                if !self.attrs.iter().any(|(k, _)| k == &name) {
                    self.attrs.push((name, String::new()));
                }
                None
            }
            CallbackEvent::AttributeValue { value } => {
                if let Some(last) = self.attrs.last_mut() {
                    last.1.push_str(&utf8(value));
                }
                None
            }
            CallbackEvent::CloseStartTag { self_closing } => Some(RawToken::Start {
                name: std::mem::take(&mut self.tag_name),
                attrs: std::mem::take(&mut self.attrs),
                self_closing,
            }),
            CallbackEvent::EndTag { name } => Some(RawToken::End { name: utf8(name) }),
            CallbackEvent::String { value } => Some(RawToken::Text(utf8(value))),
            CallbackEvent::Comment { value } => Some(RawToken::Comment(utf8(value))),
            // Doctype / errors are not meaningful in a view template.
            _ => None,
        }
    }
}

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Parse `html` into a list of top-level [`Node`]s (a fragment can
/// have multiple roots).
///
/// `html5gum` is a tokenizer, so we build the tree here: an open-element
/// stack turns the start/end token stream into nesting. Void elements
/// (`<br>`, `<img>`, …) and self-closing tags never push a scope.
/// [`normalize`] then applies the tree-construction rules a tokenizer
/// leaves to the DOM builder.
pub fn parse_fragment(html: &str) -> Vec<Node> {
    let mut emitter = CallbackEmitter::new(OrderedCallback::default());
    // Switch the tokenizer into RAWTEXT/RCDATA for `<style>`, `<script>`,
    // `<title>`, etc. so their bodies are taken verbatim (the HTML spec
    // drives this from tree construction; a bare tokenizer needs telling).
    emitter.naively_switch_states(true);
    let tokenizer = Tokenizer::new_with_emitter(html, emitter);

    // The open-element stack: each frame is an element under
    // construction plus the siblings accumulated so far at the fragment
    // root level (frame 0).
    let mut roots: Vec<Node> = Vec::new();
    let mut stack: Vec<Element> = Vec::new();

    for token in tokenizer.flatten() {
        match token {
            RawToken::Start {
                name,
                attrs,
                self_closing,
            } => {
                let name = name.to_ascii_lowercase();
                let attrs = lowercase_attr_names(attrs);
                let void = is_void_tag(&name);
                let element = Element {
                    tag: name,
                    attrs,
                    children: Vec::new(),
                    void,
                };
                if void || self_closing {
                    // No scope: attach immediately.
                    push_node(&mut roots, &mut stack, Node::Element(element));
                } else {
                    stack.push(element);
                }
            }
            RawToken::End { name } => {
                let name = name.to_ascii_lowercase();
                close_tag(&mut roots, &mut stack, &name);
            }
            RawToken::Text(text) => {
                push_node(&mut roots, &mut stack, Node::Text(text));
            }
            RawToken::Comment(text) => {
                push_node(&mut roots, &mut stack, Node::Comment(text));
            }
        }
    }

    // Close any still-open elements (unbalanced template), innermost
    // first, so their accumulated children are preserved.
    while let Some(element) = stack.pop() {
        push_node(&mut roots, &mut stack, Node::Element(element));
    }

    normalize(&mut roots);
    roots
}

/// Append `node` to the current open element's children, or to the
/// fragment roots when the stack is empty.
fn push_node(roots: &mut Vec<Node>, stack: &mut [Element], node: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => roots.push(node),
    }
}

/// Close the nearest open element matching `name`. If no match is on
/// the stack the end tag is stray and ignored (matching lenient HTML).
fn close_tag(roots: &mut Vec<Node>, stack: &mut Vec<Element>, name: &str) {
    let Some(idx) = stack.iter().rposition(|el| el.tag == name) else {
        return;
    };
    // Pop everything down to and including the match, nesting each
    // popped element into its parent so implicitly-closed elements keep
    // their children.
    while stack.len() > idx {
        let element = stack.pop().expect("stack non-empty above idx");
        push_node(roots, stack, Node::Element(element));
    }
}

fn lowercase_attr_names(attrs: Vec<(String, String)>) -> Vec<(String, String)> {
    attrs
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

/// Recursively reshape `nodes` to match the browser's HTML tree
/// construction (the insertion-mode rules a bare tokenizer leaves to
/// the DOM builder): implicit `<tbody>`, `<li>`/`<p>` auto-closing.
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

#[cfg(test)]
mod quote_tests {
    use super::*;

    #[test]
    fn it_parses_unquoted_brace_attribute_like_the_browser() {
        // Browser: <article data-concept="{dom.host/concept}"><h2>x</h2></article>
        let r = parse_fragment("<article data-concept={dom.host/concept}><h2>x</h2></article>");
        let Node::Element(article) = &r[0] else {
            panic!("expected <article>, got {r:?}");
        };
        assert_eq!(article.tag, "article");
        assert_eq!(
            article.attrs,
            vec![("data-concept".to_string(), "{dom.host/concept}".to_string())]
        );
        // <h2> is a child of <article>, not mis-parsed into it.
        assert!(matches!(&article.children[0], Node::Element(h) if h.tag == "h2"));
    }

    #[test]
    fn it_leaves_a_quoted_brace_attribute_alone() {
        let r = parse_fragment(r#"<a data-x="{v}">y</a>"#);
        let Node::Element(a) = &r[0] else {
            panic!("expected <a>");
        };
        assert_eq!(a.attrs, vec![("data-x".to_string(), "{v}".to_string())]);
    }

    /// The attributes of the first top-level element, for comparison
    /// against the browser DOM.
    fn first_attrs(html: &str) -> Vec<(String, String)> {
        match parse_fragment(html).into_iter().next() {
            Some(Node::Element(el)) => el.attrs,
            other => panic!("expected a leading element, got {other:?}"),
        }
    }

    // Regression: real `tonk/binder`-style templates use unquoted
    // attribute values containing `/` and `:`, bare boolean attributes,
    // and self-closing custom elements. `tl` mis-tokenizes all of
    // these; the browser DOM is the reference.

    #[test]
    fn it_keeps_consecutive_bare_boolean_attributes() {
        // Browser: autofocus="" required="" (tl drops the first char of
        // every bare attribute after the first: `required` -> `equired`).
        assert_eq!(
            first_attrs(r#"<wa-input name="remote" autofocus required></wa-input>"#),
            vec![
                ("name".to_string(), "remote".to_string()),
                ("autofocus".to_string(), String::new()),
                ("required".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn it_keeps_an_unquoted_value_with_a_slash() {
        // Browser: <form onsubmit="space/enable-sync"> with the <input>
        // as a child (tl destroys the tag on the unquoted `/`).
        let r = parse_fragment(r#"<form onsubmit=space/enable-sync><input name="x"></form>"#);
        let Node::Element(form) = &r[0] else {
            panic!("expected <form>, got {r:?}");
        };
        assert_eq!(form.tag, "form");
        assert_eq!(
            form.attrs,
            vec![("onsubmit".to_string(), "space/enable-sync".to_string())]
        );
        assert!(
            matches!(&form.children[0], Node::Element(i) if i.tag == "input"),
            "the <input> is a child of <form>: {:?}",
            form.children
        );
    }

    #[test]
    fn it_keeps_an_unquoted_value_with_a_colon() {
        // Browser: concept="tonk:repository".
        assert_eq!(
            first_attrs("<tonk-display this={subject} concept=tonk:repository></tonk-display>"),
            vec![
                ("this".to_string(), "{subject}".to_string()),
                ("concept".to_string(), "tonk:repository".to_string()),
            ]
        );
    }

    #[test]
    fn it_parses_a_self_closing_custom_element_with_unquoted_values() {
        // Browser: <tonk-display concept="workspace/sheet" data-active="{active}">
        // (the trailing /> is ignored for a non-void element).
        assert_eq!(
            first_attrs("<tonk-display concept=workspace/sheet data-active={active} />"),
            vec![
                ("concept".to_string(), "workspace/sheet".to_string()),
                ("data-active".to_string(), "{active}".to_string()),
            ]
        );
    }

    #[test]
    fn it_leaves_a_quoted_value_alone() {
        assert_eq!(
            first_attrs(r#"<a href="foo" data-x="{v}">z</a>"#),
            vec![
                ("href".to_string(), "foo".to_string()),
                ("data-x".to_string(), "{v}".to_string()),
            ]
        );
    }
}
