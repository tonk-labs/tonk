//! The terminal focus model, driven through the real view pipeline.
//!
//! Every tree here comes from `pipeline::resolve`, not from a
//! hand-built `Node`: what makes focus work is that the repeat clone
//! carries the row stamp the renderer put there, and a hand-built tree
//! would let that assumption rot silently.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use tonk_render::Conclusion;
use tonk_template::event::{EventDescriptor, event_descriptor, parse_source};
use tonk_tui_poc::focus;

/// A declaration, as the analyzer would build it.
fn declaration(event_type: &str, sources: &[(&str, &str)]) -> EventDescriptor {
    event_descriptor(
        Some(event_type),
        false,
        false,
        sources
            .iter()
            .map(|(field, raw)| ((*field).to_string(), parse_source(raw))),
    )
    .expect("a declaration")
}

/// A compiled bindings table: declaration name -> descriptor.
fn events(entries: &[(&str, EventDescriptor)]) -> BTreeMap<String, EventDescriptor> {
    entries
        .iter()
        .map(|(name, descriptor)| ((*name).to_string(), descriptor.clone()))
        .collect()
}

fn row(this: &str, fields: &[(&str, Ipld)]) -> Conclusion {
    Conclusion {
        this: this.to_string(),
        fields: fields
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect(),
    }
}

fn resolve(template: &str, frame: &[Conclusion]) -> Vec<tonk_render::Node> {
    tonk_tui_poc::pipeline::resolve(template, frame, "test/model")
}

/// Only a binding whose declaration is in the artifact makes an element
/// focusable, and document order is tab order.
#[test]
fn bindings_in_the_artifact_are_the_tab_order() {
    let template = concat!(
        "<column>",
        "<text>chrome</text>",
        "<el on:click=demo/first>first</el>",
        "<el on:click=demo/second>second</el>",
        "</column>"
    );
    let table = events(&[("on/click", declaration("click", &[("subject", "{this}")]))]);
    let found = focus::collect(&resolve(template, &[]), &table);

    assert_eq!(
        found
            .iter()
            .map(|f| f.bindings[0].command.as_str())
            .collect::<Vec<_>>(),
        vec!["demo/first", "demo/second"],
        "document order is tab order",
    );
}

/// A dangling `on:` name is not a control.
///
/// The analyzer fails the lowering for one, so anything reaching a host
/// unresolved would render as an element that looks live and does
/// nothing — worse than not being focusable at all.
#[test]
fn a_binding_with_no_declaration_is_not_focusable() {
    let template = "<el on:hover=demo/act>hover me</el>";
    let table = events(&[("on/click", declaration("click", &[]))]);
    assert!(focus::collect(&resolve(template, &[]), &table).is_empty());
}

/// `focus` on its own makes an element reachable with no binding — a
/// pane the keyboard can enter but that posts nothing.
#[test]
fn an_explicit_focus_attribute_is_enough() {
    let template = "<el focus label=log>output</el>";
    let found = focus::collect(&resolve(template, &[]), &BTreeMap::new());
    assert_eq!(found.len(), 1);
    assert!(found[0].bindings.is_empty());
    assert_eq!(found[0].label, "log");
}

/// A focusable inside a repeat clone knows which conclusion it came
/// from, which is what a `{this}` source reads.
#[test]
fn a_focusable_in_a_repeat_clone_carries_its_row() {
    let template = "<column><row with={this}><el on:click=demo/pick>{title}</el></row></column>";
    let frame = [
        row("tonk:one", &[("title", Ipld::String("one".into()))]),
        row("tonk:two", &[("title", Ipld::String("two".into()))]),
    ];
    let table = events(&[("on/click", declaration("click", &[("subject", "{this}")]))]);
    let found = focus::collect(&resolve(template, &frame), &table);

    assert_eq!(
        found
            .iter()
            .map(|f| (f.subject.as_deref(), f.label.as_str()))
            .collect::<Vec<_>>(),
        vec![(Some("tonk:one"), "one"), (Some("tonk:two"), "two"),],
        "each clone's focusable reads its own conclusion",
    );
}

/// Chrome outside the repeat has no row of its own.
#[test]
fn a_focusable_outside_the_repeat_has_no_row() {
    let template = "<column><el on:click=demo/new>new</el><row with={this}>{title}</row></column>";
    let frame = [row("tonk:one", &[("title", Ipld::String("one".into()))])];
    let table = events(&[("on/click", declaration("click", &[]))]);
    let found = focus::collect(&resolve(template, &frame), &table);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].subject, None);
}

/// `data-` is stripped, so a `.currentTarget.dataset.x` source finds
/// what the browser would have found.
#[test]
fn attributes_are_keyed_the_way_dataset_reads_them() {
    let template = "<el on:click=demo/act data-remove=yes key=d label=done>x</el>";
    let table = events(&[("on/click", declaration("click", &[]))]);
    let found = focus::collect(&resolve(template, &[]), &table);

    assert_eq!(
        found[0].attributes.get("remove").map(String::as_str),
        Some("yes")
    );
    assert_eq!(found[0].key.as_deref(), Some("d"));
    assert_eq!(found[0].label, "done");
}

/// Activation maps onto the declaration's own `type:`: `click` outside a
/// form, and a form's `submit` from inside it. That is what makes a
/// binding written for a browser form reachable from a keyboard without
/// an `onactivate` twin.
#[test]
fn activation_maps_onto_the_declared_type() {
    let table = events(&[
        ("on/click", declaration("click", &[])),
        ("on/space-remove", declaration("submit", &[])),
    ]);

    let outside = focus::collect(&resolve("<el on:click=demo/open>open</el>", &[]), &table);
    assert_eq!(
        outside[0].activation(&table).map(|b| b.command.as_str()),
        Some("demo/open"),
    );

    let form = focus::collect(
        &resolve("<form on:space-remove=space/remove>leave</form>", &[]),
        &table,
    );
    assert!(form[0].in_form, "a form is inside itself");
    assert_eq!(
        form[0].activation(&table).map(|b| b.command.as_str()),
        Some("space/remove"),
        "the submit declaration is what Enter fires",
    );
}

/// A declaration that is neither `click` nor `submit` has its own
/// trigger; `Enter` must not fire it.
#[test]
fn activation_leaves_other_declaration_types_alone() {
    let table = events(&[("on/keydown", declaration("keydown", &[]))]);
    let found = focus::collect(&resolve("<el on:keydown=demo/type>x</el>", &[]), &table);
    assert_eq!(found.len(), 1, "it is still focusable");
    assert!(
        found[0].activation(&table).is_none(),
        "but Enter does not post a command bound to a key",
    );
}

/// `mark` stamps focus where `collect` said it was, which is the whole
/// contract between the focus model and the lowering.
///
/// It stamps `focused`, not `focus`: `focus` is the author's standing
/// claim that an element is reachable, `focused` is the host's claim
/// that it is reached *this frame*. Conflating them would paint every
/// focusable element as focused.
#[test]
fn mark_stamps_the_element_collect_pointed_at() {
    let template = "<column><text>chrome</text><el on:click=demo/act>go</el></column>";
    let table = events(&[("on/click", declaration("click", &[]))]);
    let mut tree = resolve(template, &[]);
    let found = focus::collect(&tree, &table);
    assert!(focus::mark(&mut tree, &found[0].path));

    let target = navigate(&tree, &found[0].path).expect("the marked element");
    assert!(
        target
            .attrs
            .iter()
            .any(|(name, _)| name == focus::FOCUSED_ATTRIBUTE),
        "the element collect pointed at is the one carrying the stamp",
    );
    assert!(
        !focus::collect(&tree, &BTreeMap::new())
            .iter()
            .any(|f| f.path == found[0].path),
        "and the stamp does not itself make anything focusable",
    );
}

fn navigate<'a>(
    nodes: &'a [tonk_render::Node],
    path: &[usize],
) -> Option<&'a tonk_render::Element> {
    let (index, rest) = path.split_first()?;
    let tonk_render::Node::Element(element) = nodes.get(*index)? else {
        return None;
    };
    if rest.is_empty() {
        Some(element)
    } else {
        navigate(&element.children, rest)
    }
}

/// An `on:` written inside a stylesheet is text, not a control.
///
/// Nothing here excludes it: the parser puts a raw-text element's body
/// in a single text node, so there is no element to find. Pinned as a
/// test because the focus walk *relies* on that and would otherwise need
/// an exclusion of its own.
#[test]
fn a_binding_inside_a_stylesheet_is_not_a_control() {
    let template = "<column><style>el { on:click=nope }</style></column>";
    let table = events(&[("on/click", declaration("click", &[]))]);
    assert!(focus::collect(&resolve(template, &[]), &table).is_empty());
}

/// The stamp is what makes `focused-*` paint, and only on the element
/// that carries it.
///
/// This is elm-ui's decoration model end to end: `focused-bg` is an
/// attribute bundle on the element, so the layout and the painter see an
/// ordinary resolved style and never learn that focus exists.
#[test]
fn the_stamp_promotes_the_focused_decorations() {
    let template = concat!(
        "<column>",
        "<el on:click=demo/one bg=surface focused-bg=accent>one</el>",
        "<el on:click=demo/two bg=surface focused-bg=accent>two</el>",
        "</column>"
    );
    let table = events(&[("on/click", declaration("click", &[]))]);
    let mut tree = resolve(template, &[]);
    let found = focus::collect(&tree, &table);
    assert!(focus::mark(&mut tree, &found[0].path));

    let lowered = tonk_tui_poc::vocabulary::lower(&tree);
    let backgrounds: Vec<Option<&str>> = lowered
        .children
        .iter()
        .map(|child| child.style.bg.as_deref())
        .collect();
    assert_eq!(
        backgrounds,
        vec![Some("accent"), Some("surface")],
        "the focused element takes its focused background, its sibling keeps its own",
    );
}

/// A state's decorations stay inert until its flag is set — otherwise
/// every focusable element would paint as though it were focused.
#[test]
fn an_unset_state_leaves_its_decorations_alone() {
    let template = "<el bg=surface focused-bg=accent hover-bg=hot>one</el>";
    let lowered = tonk_tui_poc::vocabulary::lower(&resolve(template, &[]));
    assert_eq!(lowered.style.bg.as_deref(), Some("surface"));
    assert!(
        !lowered.attrs.keys().any(|name| name.contains("focused-")),
        "and the prefixed attributes do not leak into the layout tree",
    );
}
