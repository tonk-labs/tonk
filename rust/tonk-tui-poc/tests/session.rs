//! The interaction model, driven with no terminal (`plan/tui-views.md`
//! §12).
//!
//! Every assertion here is about a decision that can be *wrong* — where
//! focus lands, which binding an activation fires, what the posted body
//! carries. None of it needs a tty, which is the point: the terminal
//! contributes reading a key and writing cells, and neither can be got
//! wrong in an interesting way.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use serde_json::{Value, json};
use tonk_render::Conclusion;
use tonk_template::event::{EventDescriptor, event_descriptor, parse_source};
use tonk_tui_poc::session::{Direction, Effect, Key, Session};

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

fn events(entries: &[(&str, EventDescriptor)]) -> BTreeMap<String, EventDescriptor> {
    entries
        .iter()
        .map(|(name, descriptor)| ((*name).to_string(), descriptor.clone()))
        .collect()
}

/// A command descriptor in the JSON shape a resolved concept takes.
fn command(fields: &[(&str, &str)]) -> Value {
    let with: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(name, as_type)| {
            (
                (*name).to_string(),
                json!({ "the": format!("xyz.tonk.test/{name}"), "as": as_type }),
            )
        })
        .collect();
    json!({ "with": with })
}

fn commands(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_string(), value.clone()))
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

fn session(
    template: &str,
    frame: Vec<Conclusion>,
    table: BTreeMap<String, EventDescriptor>,
    descriptors: BTreeMap<String, Value>,
) -> Session {
    let tree = tonk_tui_poc::pipeline::resolve(template, &frame, "test/model");
    Session::new(tree, frame, table, descriptors)
}

fn parameters(body: &Value) -> &serde_json::Map<String, Value> {
    body["claims"][0]["application"]["parameters"]
        .as_object()
        .expect("parameters")
}

/// Tab walks document order and wraps.
///
/// Wrapping rather than stopping: a terminal has no scrollbar to say
/// there is more, so a focus that silently refuses to move reads as a
/// hang.
#[test]
fn tab_walks_document_order_and_wraps() {
    let template = concat!(
        "<column>",
        "<el on:click=demo/a label=a>a</el>",
        "<el on:click=demo/b label=b>b</el>",
        "<el on:click=demo/c label=c>c</el>",
        "</column>"
    );
    let table = events(&[("on/click", declaration("click", &[]))]);
    let mut view = session(template, Vec::new(), table, BTreeMap::new());

    let mut visited = vec![view.focused().expect("a first focusable").label.clone()];
    for _ in 0..3 {
        assert_eq!(view.press(Key::Tab), Effect::Moved);
        visited.push(view.focused().expect("still focused").label.clone());
    }
    assert_eq!(visited, vec!["a", "b", "c", "a"]);

    assert_eq!(view.press(Key::BackTab), Effect::Moved);
    assert_eq!(
        view.focused().expect("focused").label,
        "c",
        "and wraps back"
    );
}

/// Arrows move only inside a container that asked for them.
///
/// A terminal user reaches for arrows constantly; making them move focus
/// everywhere would make focus jump under views that never asked.
#[test]
fn arrows_are_scoped_to_a_nav_container() {
    let template = concat!(
        "<column>",
        "<el on:click=demo/loose label=loose>loose</el>",
        "<column nav=vertical>",
        "<el on:click=demo/one label=one>one</el>",
        "<el on:click=demo/two label=two>two</el>",
        "</column>",
        "</column>"
    );
    let table = events(&[("on/click", declaration("click", &[]))]);
    let mut view = session(template, Vec::new(), table, BTreeMap::new());

    assert_eq!(
        view.press(Key::Arrow(Direction::Down)),
        Effect::Idle,
        "the first focusable is outside any nav container",
    );
    assert_eq!(view.focused().expect("focused").label, "loose");

    assert_eq!(view.press(Key::Tab), Effect::Moved);
    assert_eq!(view.focused().expect("focused").label, "one");
    assert_eq!(view.press(Key::Arrow(Direction::Down)), Effect::Moved);
    assert_eq!(view.focused().expect("focused").label, "two");
    assert_eq!(
        view.press(Key::Arrow(Direction::Right)),
        Effect::Idle,
        "and the other axis is not this container's",
    );
}

/// Activating a control inside a repeat clone posts the row it belongs
/// to — the whole reason focus tracks the renderer's `with=` stamp.
#[test]
fn activation_posts_the_focused_rows_subject() {
    let template = "<column><row with={this}><el on:click=demo/pick>{title}</el></row></column>";
    let frame = vec![
        row("tonk:one", &[("title", Ipld::String("one".into()))]),
        row("tonk:two", &[("title", Ipld::String("two".into()))]),
    ];
    let table = events(&[(
        "on/click",
        declaration("click", &[("subject", "{this}"), ("name", "{title}")]),
    )]);
    let descriptors = commands(&[(
        "demo/pick",
        command(&[("subject", "Entity"), ("name", "Text")]),
    )]);
    let mut view = session(template, frame, table, descriptors);

    view.press(Key::Tab);
    let Effect::Post(body) = view.press(Key::Activate) else {
        panic!("the second row activates");
    };
    assert_eq!(parameters(&body)["subject"], json!("tonk:two"));
    assert_eq!(parameters(&body)["name"], json!("two"));
}

/// A `key=` accelerator fires an element without focusing it first,
/// which is what makes the keybar an affordance rather than a legend.
#[test]
fn an_accelerator_fires_without_focus() {
    let template = concat!(
        "<column>",
        "<el on:click=demo/first label=first>first</el>",
        "<el on:click=demo/new key=n label=new>new</el>",
        "</column>"
    );
    let table = events(&[("on/click", declaration("click", &[("name", "\"thing\"")]))]);
    let descriptors = commands(&[("demo/new", command(&[("name", "Text")]))]);
    let mut view = session(template, Vec::new(), table, descriptors);

    assert_eq!(view.focused().expect("focused").label, "first");
    let Effect::Post(body) = view.press(Key::Char('n')) else {
        panic!("the accelerator fires the element it names");
    };
    assert_eq!(parameters(&body)["name"], json!("thing"));
    assert_eq!(
        view.press(Key::Char('z')),
        Effect::Idle,
        "and only that key"
    );
}

/// The keybar is read off the bindings, so a chip cannot advertise
/// something the view does not handle (§5.4). One chip per key, however
/// many rows repeat it.
#[test]
fn the_keybar_is_generated_from_the_bindings() {
    let template = concat!(
        "<column>",
        "<el on:click=demo/new key=n label=new>+</el>",
        "<row with={this}><el on:click=demo/done key=d label=done>x</el></row>",
        "<el on:click=demo/plain>no chip</el>",
        "</column>"
    );
    let frame = vec![row("tonk:one", &[]), row("tonk:two", &[])];
    let table = events(&[("on/click", declaration("click", &[]))]);
    let view = session(template, frame, table, BTreeMap::new());

    assert_eq!(
        view.keybar()
            .iter()
            .map(|chip| (chip.key.as_str(), chip.label.as_str()))
            .collect::<Vec<_>>(),
        vec![("n", "new"), ("d", "done")],
        "one chip per key, and none for a binding that named no key",
    );
}

/// A source that does not resolve declines the activation rather than
/// posting a half-built command.
#[test]
fn an_unresolvable_source_declines() {
    let template = "<el on:click=demo/act>go</el>";
    let table = events(&[(
        "on/click",
        declaration("click", &[("subject", "{missing}")]),
    )]);
    let descriptors = commands(&[("demo/act", command(&[("subject", "Entity")]))]);
    let mut view = session(template, vec![row("tonk:one", &[])], table, descriptors);

    assert_eq!(view.press(Key::Activate), Effect::Declined);
}

/// The focused element is the one the frame stamps, and only it.
#[test]
fn the_frame_stamps_where_focus_is_now() {
    let template = concat!(
        "<column>",
        "<el on:click=demo/a bg=surface focused-bg=accent>a</el>",
        "<el on:click=demo/b bg=surface focused-bg=accent>b</el>",
        "</column>"
    );
    let table = events(&[("on/click", declaration("click", &[]))]);
    let mut view = session(template, Vec::new(), table, BTreeMap::new());

    let backgrounds = |view: &Session| {
        tonk_tui_poc::vocabulary::lower(&view.frame_tree())
            .children
            .iter()
            .map(|child| child.style.bg.clone().unwrap_or_default())
            .collect::<Vec<_>>()
    };
    assert_eq!(backgrounds(&view), vec!["accent", "surface"]);
    view.press(Key::Tab);
    assert_eq!(
        backgrounds(&view),
        vec!["surface", "accent"],
        "the stamp follows focus rather than accumulating behind it",
    );
}

/// A view with nothing focusable is inert, not broken.
#[test]
fn a_view_with_no_focusables_is_inert() {
    let mut view = session(
        "<column><text>read only</text></column>",
        Vec::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    );
    assert!(view.focused().is_none());
    assert_eq!(view.press(Key::Tab), Effect::Idle);
    assert_eq!(view.press(Key::Activate), Effect::Idle);
    assert_eq!(view.press(Key::Quit), Effect::Quit);
}
