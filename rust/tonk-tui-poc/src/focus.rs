//! Which elements a terminal can focus, in what order, and what an
//! activation on one of them fires.
//!
//! A browser hands you focus for free: it decides what is focusable
//! from tag semantics and `tabindex`, keeps tab order, paints a ring,
//! and turns `Enter` on a `<button>` into a `click`. A terminal supplies
//! none of it, so the host has to answer the same four questions itself
//! (`plan/tui-views.md` §5.1).
//!
//! The answers are now *decidable without a DOM*, which is what #904
//! bought. Focusability is "carries an `on:<name>` attribute whose
//! declaration is in the view's compiled bindings" — the host has the
//! declaration set from the artifact and the attribute names from the
//! parsed tree, so nothing has to be guessed from tag names. And an
//! activation maps onto the declaration's own `type:` rather than onto a
//! fixed `click`, which is what lets a `submit` binding written for a
//! browser form be reached from a keyboard with no `onactivate` twin.
//!
//! This module runs on the *resolved* node tree, before it is lowered
//! into the layout vocabulary, because that tree still carries the two
//! things focus needs and lowering drops: element tag names (for the
//! enclosing-form question) and the `with=` stamp that says which
//! conclusion a repeat clone came from.

use std::collections::BTreeMap;

use tonk_render::{Element, Node};
use tonk_template::event::{EventDescriptor, event_name_for_attribute};

/// The attribute an author writes to make an element focusable without
/// binding anything to it — a scroll container, a read-only pane.
const FOCUS_ATTRIBUTE: &str = "focus";

/// The attribute the *host* stamps on the one element that currently has
/// focus. Deliberately not the same word: `focus` is what an author
/// declares about an element for all time, `focused` is what the host
/// says about it this frame, and collapsing the two would make every
/// focusable element paint as though it were focused.
pub const FOCUSED_ATTRIBUTE: &str = "focused";

/// The attribute carrying a keybar chip's key (`key=g`), and the one
/// carrying its caption (`label=guide`). §5.4: a binding that names both
/// contributes a chip automatically, so the keybar is generated from the
/// bindings rather than maintained beside them.
const KEY_ATTRIBUTE: &str = "key";
const LABEL_ATTRIBUTE: &str = "label";

/// The attribute `tonk_render` stamps on each repeat clone, naming the
/// conclusion the clone was rendered from.
const ROW_ATTRIBUTE: &str = "with";

/// The `data-` prefix a browser strips to reach `dataset`.
const DATASET_PREFIX: &str = "data-";

/// One `on:<name>=<command>` binding carried by a focusable element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    /// The declaration it names (`on/click`).
    pub event_name: String,
    /// The command it posts.
    pub command: String,
}

/// One focusable element in the resolved tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focusable {
    /// Child indices from the fragment root, so the host can point back
    /// at the node — to stamp focus on it, or to find its rect.
    pub path: Vec<usize>,
    /// The `this` of the conclusion this element was rendered from, when
    /// it sits inside a repeat clone. Chrome outside the repeat has
    /// none, and reads against the lead conclusion.
    pub subject: Option<String>,
    /// The element's own attributes with any `data-` prefix stripped, in
    /// the shape a `.currentTarget.dataset.*` source reads.
    pub attributes: BTreeMap<String, String>,
    /// Its bindings, ordered as written.
    pub bindings: Vec<Bound>,
    /// Whether the element is inside a `<form>` subtree — or is one.
    pub in_form: bool,
    /// What a keybar chip would say: the `label=` attribute, else the
    /// element's own text.
    pub label: String,
    /// The `key=` attribute, when the author named one.
    pub key: Option<String>,
}

impl Focusable {
    /// Which binding `Enter` / `Space` fires, if any.
    ///
    /// Activation maps onto a declared platform type rather than onto a
    /// synthetic one: `click` normally, and `submit` inside a form,
    /// where that is the type the browser would have raised. Inside a
    /// form `submit` is tried first and `click` second, so an ordinary
    /// button inside a form stays activatable rather than being
    /// shadowed by the form's own binding.
    ///
    /// Every other declaration type is left alone. A key or focus
    /// declaration has its own trigger; firing it from `Enter` would
    /// post a command the author bound to something else.
    pub fn activation<'a>(
        &'a self,
        events: &BTreeMap<String, EventDescriptor>,
    ) -> Option<&'a Bound> {
        let order: &[&str] = if self.in_form {
            &["submit", "click"]
        } else {
            &["click"]
        };
        order.iter().find_map(|wanted| {
            self.bindings.iter().find(|bound| {
                events
                    .get(&bound.event_name)
                    .is_some_and(|descriptor| descriptor.event_type == *wanted)
            })
        })
    }
}

/// Every focusable element in `nodes`, in document order — which is tab
/// order (§5.1).
///
/// `events` is the view's compiled binding table. An `on:` attribute
/// naming a declaration that is not in it does not make an element
/// focusable: the analyzer already failed the lowering for a dangling
/// name, so anything reaching here that is still unresolved would be a
/// control that looks live and does nothing.
pub fn collect(nodes: &[Node], events: &BTreeMap<String, EventDescriptor>) -> Vec<Focusable> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    walk(nodes, events, None, false, &mut path, &mut out);
    out
}

fn walk(
    nodes: &[Node],
    events: &BTreeMap<String, EventDescriptor>,
    subject: Option<&str>,
    in_form: bool,
    path: &mut Vec<usize>,
    out: &mut Vec<Focusable>,
) {
    for (index, node) in nodes.iter().enumerate() {
        let Node::Element(element) = node else {
            continue;
        };
        path.push(index);
        let subject = row_subject(element).or(subject);
        let in_form = in_form || element.tag == "form";
        if let Some(focusable) = describe(element, events, subject, in_form, path) {
            out.push(focusable);
        }
        walk(&element.children, events, subject, in_form, path, out);
        path.pop();
    }
}

/// The conclusion stamp `tonk_render` puts on a repeat clone, if this is
/// one.
fn row_subject(element: &Element) -> Option<&str> {
    element
        .attrs
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(ROW_ATTRIBUTE))
        .map(|(_, value)| value.as_str())
}

fn describe(
    element: &Element,
    events: &BTreeMap<String, EventDescriptor>,
    subject: Option<&str>,
    in_form: bool,
    path: &[usize],
) -> Option<Focusable> {
    let mut bindings = Vec::new();
    let mut attributes = BTreeMap::new();
    let mut explicit = false;
    for (name, value) in &element.attrs {
        let name = name.to_ascii_lowercase();
        if name == FOCUS_ATTRIBUTE {
            explicit = true;
        }
        if let Some(event_name) = event_name_for_attribute(&name) {
            let command = value.trim();
            if !command.is_empty() && events.contains_key(&event_name) {
                bindings.push(Bound {
                    event_name,
                    command: command.to_owned(),
                });
            }
        }
        let key = name
            .strip_prefix(DATASET_PREFIX)
            .unwrap_or(&name)
            .to_owned();
        attributes.insert(key, value.clone());
    }

    if bindings.is_empty() && !explicit {
        return None;
    }

    let key = attributes.get(KEY_ATTRIBUTE).cloned();
    let label = attributes
        .get(LABEL_ATTRIBUTE)
        .cloned()
        .unwrap_or_else(|| text_of(&element.children));

    Some(Focusable {
        path: path.to_vec(),
        subject: subject.map(str::to_owned),
        attributes,
        bindings,
        in_form,
        label,
        key,
    })
}

/// A subtree's text, collapsed onto one line the way a terminal caption
/// is. Deliberately the same reading `vocabulary::inner_text` gives, so
/// a generated keybar chip says what the element says.
fn text_of(nodes: &[Node]) -> String {
    let mut out = String::new();
    gather(nodes, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn gather(nodes: &[Node], out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Element(element) => gather(&element.children, out),
            Node::Comment(_) => {}
        }
    }
}

/// Stamp [`FOCUSED_ATTRIBUTE`] on the element at `path` so the lowering
/// can promote its `focused-*` attributes (§6.6).
///
/// Focus is expressed *in the tree* rather than carried alongside it
/// because that is elm-ui's own model: `focused` is an attribute bundle
/// on the element, not a selector matched against it. It also means the
/// layout and the painter need to know nothing about focus — they see a
/// resolved style, as they do for every other frame.
pub fn mark(nodes: &mut [Node], path: &[usize]) -> bool {
    let Some((index, rest)) = path.split_first() else {
        return false;
    };
    let Some(Node::Element(element)) = nodes.get_mut(*index) else {
        return false;
    };
    if rest.is_empty() {
        element
            .attrs
            .push((FOCUSED_ATTRIBUTE.to_owned(), String::new()));
        return true;
    }
    mark(&mut element.children, rest)
}
