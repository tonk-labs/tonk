//! Compile-time event bindings: what a view's templates bind, and the
//! declarations those bindings resolved to.
//!
//! A template binds an interaction with `on:<name>=<command>`. Until
//! now both halves were resolved at *render* time: the display scanned
//! the rendered fragment for `on:` attributes, then issued one query
//! per declaration name to rebuild the [`EventDescriptor`] and one per
//! command name to fetch its descriptor. A dangling reference produced
//! no listener and no diagnostic — the element was simply inert.
//!
//! Both halves are knowable at lowering: the analyzer has the template
//! text and the document's `event!:` / `command!:` declarations in
//! scope. So the scan moves here ([`scan`], DOM-free so it runs in the
//! analyzer), the resolution happens once, and the result is stored on
//! the view as one CBOR artifact ([`Bindings`]) the display decodes
//! instead of re-querying.
//!
//! What stays at runtime is resolving a *command* concept from its
//! name — the descriptor is the same one the display already resolves
//! for every model, so inlining it here would duplicate rather than
//! remove work. What the analyzer contributes for commands is the
//! check: a name that resolves to nothing fails the lowering.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::event::{EventDescriptor, event_name_for_attribute};

/// One `on:<name>=<command>` binding found in a template.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventBinding {
    /// The attribute as written (`on:click`), for diagnostics that
    /// point back at the template.
    pub attribute: String,
    /// The declaration it names (`on/click`).
    pub event_name: String,
    /// The command it posts — a bare name (`space/create`) or a URI
    /// (`tonk:invite`).
    pub command: String,
    /// Byte offset of the attribute name within the template, so a
    /// diagnostic can underline `on:click` rather than the whole
    /// template. When one binding is written twice, this is the first
    /// of them — the report has to point somewhere, and the earliest
    /// occurrence is the one an author reads first.
    pub offset: usize,
}

/// What every bindings artifact says it is, in its own `kind` field.
///
/// The value is stored as a `Record` — dialog's type for "one struct,
/// one fact", the same one a revision record uses — so it is already
/// distinguishable from a `Bytes` blob at the storage layer. But that
/// tag says only "structured"; it does not say *which* structure, and
/// it is flattened to plain bytes on every wire projection
/// (`Ipld::Bytes`, base64 in the claim API). So the payload identifies
/// itself too: a decoder handed some other CBOR fails loudly instead
/// of quietly reading an empty binding table, and a future format
/// change is a version bump rather than a silent misparse.
pub const KIND: &str = "tonk/view-bindings@1";

/// The bindings a view's templates make, resolved at lowering.
///
/// A struct rather than a bare map so the artifact can grow a field
/// without a format break. Encoded as dag-cbor, which is canonical:
/// re-lowering an unchanged view yields byte-identical output, so the
/// claim does not churn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bindings {
    /// Always [`KIND`]. Private so it cannot be set to anything else;
    /// checked on decode.
    kind: String,
    /// Declaration name (`on/click`) -> the descriptor it resolved to,
    /// with every bare-symbol source already rewritten to an entity.
    pub events: BTreeMap<String, EventDescriptor>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self::new(BTreeMap::new())
    }
}

impl Bindings {
    /// An artifact carrying `events`.
    pub fn new(events: BTreeMap<String, EventDescriptor>) -> Self {
        Self {
            kind: KIND.to_owned(),
            events,
        }
    }

    /// True when there is nothing to store.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Encode as dag-cbor.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        serde_ipld_dagcbor::to_vec(self).map_err(|error| error.to_string())
    }

    /// Decode from dag-cbor, refusing anything that is not this format.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let decoded: Self =
            serde_ipld_dagcbor::from_slice(bytes).map_err(|error| error.to_string())?;
        if decoded.kind != KIND {
            return Err(format!(
                "expected `{KIND}`, got `{}`",
                decoded.kind.escape_debug()
            ));
        }
        Ok(decoded)
    }
}

/// Every `on:<name>=<command>` binding a template's raw HTML makes,
/// deduplicated, ordered by what the binding says.
///
/// The walk is [`crate::scan::walk`], shared with the interpolation
/// scan so the two cannot disagree about what a template contains —
/// only attribute positions inside a tag count, and a comment carries
/// none.
pub fn scan(template: &str) -> Vec<EventBinding> {
    let mut out: Vec<EventBinding> = Vec::new();
    crate::scan::walk(template, &mut |found| {
        let crate::scan::Found::Attribute {
            name,
            name_offset,
            value,
            ..
        } = found
        else {
            return;
        };
        let Some(event_name) = event_name_for_attribute(name) else {
            return;
        };
        let command = value.trim();
        if command.is_empty() {
            return;
        }
        out.push(EventBinding {
            attribute: name.to_owned(),
            event_name,
            command: command.to_owned(),
            offset: name_offset,
        });
    });

    // The derived order puts the offset last, so a repeated binding's
    // occurrences land next to each other and `dedup_by` — which keeps
    // the first of a run — keeps the earliest place it is written.
    out.sort();
    out.dedup_by(|left, right| {
        (&left.attribute, &left.event_name, &left.command)
            == (&right.attribute, &right.event_name, &right.command)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Source;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    #[dialog_common::test]
    fn it_finds_a_bare_and_a_quoted_binding() {
        let found = scan(
            r#"<form on:submit=space/create><button on:click="tonk:invite">go</button></form>"#,
        );
        assert_eq!(
            found
                .iter()
                .map(|b| (b.event_name.as_str(), b.command.as_str()))
                .collect::<Vec<_>>(),
            vec![("on/click", "tonk:invite"), ("on/submit", "space/create"),],
        );
    }

    #[dialog_common::test]
    fn it_ignores_a_binding_named_inside_a_comment() {
        let found = scan("<!-- bind it with on:click=nope --><div on:click=yes></div>");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "yes");
    }

    #[dialog_common::test]
    fn it_leaves_colon_attributes_that_are_not_bindings_alone() {
        let found = scan(r##"<use xlink:href="#x" bind:base=a on:click=go />"##);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].event_name, "on/click");
    }

    /// The offset points at the attribute name, which is what a
    /// diagnostic underlines. A repeated binding keeps the first one.
    #[dialog_common::test]
    fn it_records_where_each_attribute_is_written() {
        let template =
            "<p>hi</p>\n<button on:click=demo/act>go</button>\n<a on:click=demo/act>x</a>";
        let found = scan(template);
        assert_eq!(found.len(), 1, "the repeat is one binding, not two");
        assert_eq!(
            &template[found[0].offset..found[0].offset + found[0].attribute.len()],
            "on:click",
        );
        assert_eq!(
            found[0].offset,
            template.find("on:click").expect("it is in there"),
            "the earliest occurrence is the one reported",
        );
    }

    #[dialog_common::test]
    fn it_round_trips_through_dag_cbor() {
        let mut events = BTreeMap::new();
        events.insert(
            "on/click".to_string(),
            EventDescriptor {
                event_type: "click".into(),
                prevent_default: true,
                stop_propagation: false,
                sources: BTreeMap::from([
                    ("subject".to_string(), Source::Field("this".into())),
                    (
                        "time".to_string(),
                        Source::Property(vec!["timeStamp".into()]),
                    ),
                ]),
            },
        );
        let bindings = Bindings::new(events);
        let bytes = bindings.encode().expect("encodes");
        assert_eq!(Bindings::decode(&bytes).expect("decodes"), bindings);
        // Canonical: the same input encodes to the same bytes, so
        // re-lowering an unchanged view does not churn the claim.
        assert_eq!(bindings.encode().expect("encodes"), bytes);
    }

    /// Some other CBOR is refused, not read as an empty table.
    ///
    /// The `Record` type tag says "structured", not "these bindings",
    /// and it does not survive the wire at all — so the payload has to
    /// say what it is, or a decoder handed the wrong artifact silently
    /// installs no listeners.
    #[dialog_common::test]
    fn it_refuses_cbor_that_is_not_a_bindings_artifact() {
        #[derive(serde::Serialize)]
        struct Impostor {
            kind: &'static str,
            events: BTreeMap<String, EventDescriptor>,
        }
        let bytes = serde_ipld_dagcbor::to_vec(&Impostor {
            kind: "tonk/view-bindings@2",
            events: BTreeMap::new(),
        })
        .expect("encodes");
        let error = Bindings::decode(&bytes).expect_err("a foreign kind is refused");
        assert!(error.contains("tonk/view-bindings@2"), "{error}");
    }
}
