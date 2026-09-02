//! The terminal element vocabulary: resolved `Node` tree in, an
//! elm-ui-shaped `tonk_layout::Element` tree out.
//!
//! Two names are kept from HTML because the pipeline depends on them —
//! `<tonk-display>` and `<tonk-fallback>` (`plan/tui-views.md` §6.2) —
//! and everything else is a terminal word. An unrecognised tag lowers
//! to a plain container rather than erroring, so an unported view
//! degrades instead of exploding.

use std::collections::BTreeMap;

use tonk_layout::{AlignX, AlignY, Edges, Element, Emphasis, Kind, Length, Style};
use tonk_render::{Element as RenderElement, Node};

/// Lower a resolved fragment into one layout tree.
///
/// A fragment with several top-level nodes becomes a column, which is
/// the terminal reading of "these follow one another".
pub fn lower(nodes: &[Node]) -> Element {
    let mut children: Vec<Element> = nodes.iter().filter_map(lower_node).collect();
    if children.len() == 1 {
        children.pop().expect("length checked")
    } else {
        Element::column(children)
            .width(Length::Fill(1))
            .height(Length::Fill(1))
    }
}

fn lower_node(node: &Node) -> Option<Element> {
    match node {
        Node::Comment(_) => None,
        Node::Text(text) => {
            let trimmed = collapse(text);
            (!trimmed.is_empty()).then(|| Element::text(trimmed))
        }
        Node::Element(element) => lower_element(element),
    }
}

fn lower_element(element: &RenderElement) -> Option<Element> {
    let attrs: BTreeMap<String, String> = element
        .attrs
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();

    // `<style>` is inert here exactly as it is in the browser
    // collector: a terminal has no cascade to feed it.
    if element.tag == "style" || element.tag == "script" {
        return None;
    }

    let mut node = match element.tag.as_str() {
        "row" | "keybar" => Element::row(lower_children(element)),
        "column" | "col" => Element::column(lower_children(element)),
        "paragraph" | "p" => Element::paragraph(inner_text(element)),
        // A `text` element is a leaf even if the template nested
        // markup inside it: the terminal has no inline formatting.
        "text" | "label" | "key" => Element::text(inner_text(element)),
        "spacer" => Element::new(Kind::El),
        _ => {
            let children = lower_children(element);
            if children.is_empty() {
                let inner = inner_text(element);
                if inner.is_empty() {
                    Element::new(Kind::El)
                } else {
                    Element::text(inner)
                }
            } else {
                let mut container = Element::new(Kind::El);
                container.children = children;
                container
            }
        }
    };

    node.style = style_from(&attrs, &element.tag);
    node.attrs = attrs
        .into_iter()
        .filter(|(name, _)| !LAYOUT_ATTRS.contains(&name.as_str()))
        .collect();
    Some(node)
}

fn lower_children(element: &RenderElement) -> Vec<Element> {
    element.children.iter().filter_map(lower_node).collect()
}

/// Concatenate a subtree's text, collapsing whitespace the way a
/// terminal line does.
fn inner_text(element: &RenderElement) -> String {
    let mut out = String::new();
    walk_text(&element.children, &mut out);
    collapse(&out)
}

fn walk_text(nodes: &[Node], out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Element(element) => walk_text(&element.children, out),
            Node::Comment(_) => {}
        }
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Attributes layout consumes; everything else rides along in
/// `Element::attrs` for a painter or an event binder to read.
const LAYOUT_ATTRS: &[&str] = &[
    "width",
    "height",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "pad",
    "pad-x",
    "pad-y",
    "pad-top",
    "pad-right",
    "pad-bottom",
    "pad-left",
    "spacing",
    "spacing-x",
    "spacing-y",
    "align",
    "border",
    "wrap",
    "fg",
    "bg",
    "weight",
    "dim",
    "reverse",
    "underline",
];

fn style_from(attrs: &BTreeMap<String, String>, tag: &str) -> Style {
    let mut style = Style::default();

    // A `spacer` is the one element whose whole job is to grow.
    if tag == "spacer" {
        style.width = Length::Fill(1);
        style.height = Length::Fill(1);
    }
    // A `box` is an `el` that draws its inset; a `key` is a chip, and
    // chips are fixed cells, so it never grows.
    if tag == "box" {
        style.border = true;
        // A drawn box that its contents can spill out of is a lie, so a
        // `box` clips by default. Any element can opt in with `clip`.
        style.clip = true;
    }
    if tag == "key" {
        style.emphasis.reverse = true;
        style.pad = Edges::xy(1, 0);
    }
    if tag == "keybar" {
        style.spacing = (2, 0);
    }

    if let Some(value) = attrs.get("width") {
        style.width = parse_length(value);
    }
    if let Some(value) = attrs.get("height") {
        style.height = parse_length(value);
    }
    style.min_width = attrs.get("min-width").and_then(|v| v.parse().ok());
    style.max_width = attrs.get("max-width").and_then(|v| v.parse().ok());
    style.min_height = attrs.get("min-height").and_then(|v| v.parse().ok());
    style.max_height = attrs.get("max-height").and_then(|v| v.parse().ok());

    apply_padding(&mut style, attrs);
    apply_spacing(&mut style, attrs);
    apply_alignment(&mut style, attrs);

    if attrs.contains_key("border") {
        style.border = !matches!(attrs["border"].as_str(), "false" | "0" | "none");
    }
    style.wrap = attrs.contains_key("wrap");
    style.clip = style.clip || attrs.contains_key("clip");

    style.fg = attrs.get("fg").cloned();
    style.bg = attrs.get("bg").cloned();
    style.emphasis = Emphasis {
        bold: attrs.get("weight").map(String::as_str) == Some("bold"),
        dim: attrs.contains_key("dim") || attrs.get("weight").map(String::as_str) == Some("dim"),
        reverse: style.emphasis.reverse || attrs.contains_key("reverse"),
        underline: attrs.contains_key("underline"),
    };
    style
}

fn apply_padding(style: &mut Style, attrs: &BTreeMap<String, String>) {
    // No uniform `pad=n` shorthand on purpose: a terminal cell is about
    // 1:2, so one number cannot mean the same inset on both axes
    // (`plan/tui-views.md` §6.4). Authors say which axis they mean.
    if let Some(x) = attrs.get("pad-x").and_then(|v| v.parse().ok()) {
        style.pad.left = x;
        style.pad.right = x;
    }
    if let Some(y) = attrs.get("pad-y").and_then(|v| v.parse().ok()) {
        style.pad.top = y;
        style.pad.bottom = y;
    }
    for name in ["pad-top", "pad-right", "pad-bottom", "pad-left"] {
        let Some(value) = attrs.get(name).and_then(|v| v.parse().ok()) else {
            continue;
        };
        match name {
            "pad-top" => style.pad.top = value,
            "pad-right" => style.pad.right = value,
            "pad-bottom" => style.pad.bottom = value,
            _ => style.pad.left = value,
        }
    }
}

fn apply_spacing(style: &mut Style, attrs: &BTreeMap<String, String>) {
    if let Some(both) = attrs.get("spacing").and_then(|v| v.parse().ok()) {
        style.spacing = (both, both);
    }
    if let Some(x) = attrs.get("spacing-x").and_then(|v| v.parse().ok()) {
        style.spacing.0 = x;
    }
    if let Some(y) = attrs.get("spacing-y").and_then(|v| v.parse().ok()) {
        style.spacing.1 = y;
    }
}

fn apply_alignment(style: &mut Style, attrs: &BTreeMap<String, String>) {
    let Some(value) = attrs.get("align") else {
        return;
    };
    for token in value.split_whitespace() {
        match token {
            "left" => style.align_x = Some(AlignX::Left),
            "center-x" | "centre-x" | "center" | "centre" => style.align_x = Some(AlignX::Center),
            "right" => style.align_x = Some(AlignX::Right),
            "top" => style.align_y = Some(AlignY::Top),
            "center-y" | "centre-y" | "middle" => style.align_y = Some(AlignY::Center),
            "bottom" => style.align_y = Some(AlignY::Bottom),
            _ => {}
        }
    }
}

/// `fill`, `fill:3`, `shrink`, or a cell count.
fn parse_length(value: &str) -> Length {
    let value = value.trim();
    if value == "shrink" {
        return Length::Shrink;
    }
    if let Some(portion) = value.strip_prefix("fill") {
        let portion = portion.trim_start_matches(':').trim();
        return Length::Fill(portion.parse().unwrap_or(1));
    }
    value.parse().map(Length::Px).unwrap_or(Length::Shrink)
}
