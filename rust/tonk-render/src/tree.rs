//! A DOM-independent node tree with the same child indexing the
//! browser's `child_nodes()` exposes, so the binding paths produced
//! by [`tonk_template`] navigate it identically.
//!
//! Children (element + text) are one ordered `Vec`, indexed
//! together exactly like DOM child nodes. That is what lets the
//! native renderer reuse the browser planner's `Vec<usize>` paths.

/// One node in the template tree.
///
/// Comments are kept as their own variant rather than dropped: the
/// browser's `child_nodes()` includes comment nodes, so omitting
/// them would shift sibling indices and break the `Vec<usize>` path
/// parity the whole design rests on. They never carry bindings, but
/// they hold an index slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// An element with a tag name, attributes (insertion order
    /// preserved), and child nodes.
    Element(Element),
    /// A text node.
    Text(String),
    /// A comment node (`<!-- ... -->`), holding its inner text.
    Comment(String),
}

/// An element node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Lowercased tag name.
    pub tag: String,
    /// Attributes in source order. A `Vec` of pairs (not a map) so
    /// duplicate handling and order match what serialization needs.
    pub attrs: Vec<(String, String)>,
    /// Child nodes, indexed together (text and element) like the
    /// DOM. A binding path's component selects into this list.
    pub children: Vec<Node>,
    /// Whether the element is void (no closing tag, e.g. `<br>`,
    /// `<img>`). Void elements never have children.
    pub void: bool,
}

impl Node {
    /// Borrow the child-node list, or an empty slice for non-elements.
    pub fn children(&self) -> &[Node] {
        match self {
            Node::Element(el) => &el.children,
            _ => &[],
        }
    }

    /// True for a text node.
    pub fn is_text(&self) -> bool {
        matches!(self, Node::Text(_))
    }

    /// The element's tag name, or `None` for a non-element node.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Node::Element(el) => Some(&el.tag),
            _ => None,
        }
    }
}

/// Elements whose content is verbatim (CSS/JS), not template
/// markup: `<style>` and `<script>`. The binding walk does not
/// descend into them, matching the browser collector.
pub fn is_raw_text_element(node: &Node) -> bool {
    matches!(node.tag(), Some("style") | Some("script"))
}

/// HTML void elements: self-closing, never carry children.
pub fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}
