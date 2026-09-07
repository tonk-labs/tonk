//! The terminal source reader, against real declarations.
//!
//! Every `EventDescriptor` here is built with the same constructor the
//! analyzer uses (`event_descriptor` over `parse_source`), from source
//! strings copied out of the shipped libraries — so a change to the
//! source grammar breaks these rather than silently changing what a
//! keypress posts.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use serde_json::{Value, json};
use tonk_render::Conclusion;
use tonk_template::event::{EventDescriptor, event_descriptor, parse_source};
use tonk_tui_poc::activate::{Activation, build_body};

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

/// A command descriptor, in the JSON shape a resolved concept takes.
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

fn row(this: &str, fields: &[(&str, Ipld)]) -> Conclusion {
    Conclusion {
        this: this.to_string(),
        fields: fields
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect(),
    }
}

fn attributes(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// The parameters a body carries, for assertions.
fn parameters(body: &Value) -> &serde_json::Map<String, Value> {
    body["claims"][0]["application"]["parameters"]
        .as_object()
        .expect("parameters")
}

/// `core.yaml`'s `on/click`: `subject: "{this}"`.
///
/// The commonest declaration in the library, and the one that proves
/// `{this}` needs no DOM: the browser reads a rendered `data-this`
/// attribute back out, the terminal has the conclusion.
#[test]
fn this_resolves_to_the_focused_rows_subject() {
    let event = declaration("click", &[("subject", "{this}")]);
    let subject = "did:key:z6MkExampleSubject";
    let conclusion = row(subject, &[]);
    let attrs = attributes(&[]);

    let body = build_body(
        &event,
        &command(&[("subject", "Entity")]),
        &Activation {
            row: &conclusion,
            attributes: &attrs,
        },
    )
    .expect("`{this}` resolves");

    assert_eq!(parameters(&body)["subject"], json!(subject));
}

/// `profile.yaml`'s `on/space-remove`: `subject:
/// ".currentTarget.dataset.remove"`, reading the confirm form's
/// `data-remove`. The exact declaration behind the Hub's delete row.
#[test]
fn a_current_target_dataset_read_resolves_to_the_focused_elements_attribute() {
    let event = declaration("submit", &[("subject", ".currentTarget.dataset.remove")]);
    let subject = "did:key:z6MkSpaceToRemove";
    let conclusion = row("did:key:z6MkSomethingElse", &[]);
    let attrs = attributes(&[("remove", subject)]);

    let body = build_body(
        &event,
        &command(&[("subject", "Entity")]),
        &Activation {
            row: &conclusion,
            attributes: &attrs,
        },
    )
    .expect("the dataset read resolves");

    assert_eq!(
        parameters(&body)["subject"],
        json!(subject),
        "the attribute wins over the row's own `this`",
    );
}

/// A named field reads off the conclusion, typed.
#[test]
fn a_named_field_reads_the_conclusion_directly() {
    let event = declaration("click", &[("count", "{count}")]);
    let conclusion = row("did:key:z6Mk", &[("count", Ipld::Integer(41))]);
    let attrs = attributes(&[]);

    let body = build_body(
        &event,
        &command(&[("count", "SignedInteger")]),
        &Activation {
            row: &conclusion,
            attributes: &attrs,
        },
    )
    .expect("`{count}` resolves");

    assert_eq!(parameters(&body)["count"], json!(41));
}

/// A source for a field the command does not declare is dropped, not
/// posted — an extra parameter is a wire error, and the analyzer already
/// reports the mismatch at lowering.
#[test]
fn a_source_the_command_does_not_declare_is_dropped() {
    let event = declaration("click", &[("subject", "{this}"), ("tally", "{count}")]);
    let conclusion = row("did:key:z6Mk", &[("count", Ipld::Integer(1))]);
    let attrs = attributes(&[]);

    let body = build_body(
        &event,
        &command(&[("subject", "Entity")]),
        &Activation {
            row: &conclusion,
            attributes: &attrs,
        },
    )
    .expect("the declared field still posts");

    let params = parameters(&body);
    assert!(params.contains_key("subject"));
    assert!(!params.contains_key("tally"), "got {params:?}");
}

/// Blank is "not provided" — omit the field and still post. A hard miss
/// is not: it abandons the assertion, so a binding that cannot be
/// filled never fires a half-formed command.
#[test]
fn blank_omits_the_field_but_a_miss_abandons_the_assertion() {
    let blank = declaration("click", &[("subject", "{this}"), ("name", "{name}")]);
    let conclusion = row("did:key:z6Mk", &[("name", Ipld::String(String::new()))]);
    let attrs = attributes(&[]);
    let body = build_body(
        &blank,
        &command(&[("subject", "Entity"), ("name", "Text")]),
        &Activation {
            row: &conclusion,
            attributes: &attrs,
        },
    )
    .expect("a blank field still posts");
    assert!(!parameters(&body).contains_key("name"));

    let missing = declaration("click", &[("subject", "{absent}")]);
    let empty = row("did:key:z6Mk", &[]);
    assert!(
        build_body(
            &missing,
            &command(&[("subject", "Entity")]),
            &Activation {
                row: &empty,
                attributes: &attrs
            },
        )
        .is_none(),
        "a field the row does not carry must abandon the assertion",
    );
}

/// The entity sanity-check the browser applies, applied here too: a
/// non-URI where an entity belongs is a miss, not a posted label.
#[test]
fn a_non_uri_is_refused_where_an_entity_is_declared() {
    let event = declaration("click", &[("subject", ".currentTarget.dataset.remove")]);
    let conclusion = row("did:key:z6Mk", &[]);
    let attrs = attributes(&[("remove", "not-a-uri")]);

    assert!(
        build_body(
            &event,
            &command(&[("subject", "Entity")]),
            &Activation {
                row: &conclusion,
                attributes: &attrs
            },
        )
        .is_none(),
    );
}

/// A browser-only path has no terminal meaning, and declines rather than
/// inventing one. This is the residual `Source::Property` gap §5.3 names.
#[test]
fn a_browser_only_property_path_declines() {
    let event = declaration("change", &[("subject", ".currentTarget.files")]);
    let conclusion = row("did:key:z6Mk", &[]);
    let attrs = attributes(&[]);

    assert!(
        build_body(
            &event,
            &command(&[("subject", "Entity")]),
            &Activation {
                row: &conclusion,
                attributes: &attrs
            },
        )
        .is_none(),
        "an unreadable path must not post a command with a wrong value",
    );
}

/// The wire shape is `tonk_template::event::transact_body`'s, not a
/// second hand-rolled one — one transient assertion carrying the
/// command's own descriptor.
#[test]
fn the_body_is_one_transient_assertion_carrying_the_command_descriptor() {
    let event = declaration("click", &[("subject", "{this}")]);
    let descriptor = command(&[("subject", "Entity")]);
    let conclusion = row("did:key:z6Mk", &[]);
    let attrs = attributes(&[]);

    let body = build_body(
        &event,
        &descriptor,
        &Activation {
            row: &conclusion,
            attributes: &attrs,
        },
    )
    .expect("resolves");

    assert_eq!(body["claims"].as_array().expect("claims").len(), 1);
    assert_eq!(body["claims"][0]["op"], json!("assert"));
    let predicate = &body["claims"][0]["application"]["predicate"];
    assert_eq!(predicate["kind"], json!("transient"));
    assert_eq!(predicate["concept"], descriptor);
}
