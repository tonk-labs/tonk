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
}

/// The bindings a view's templates make, resolved at lowering.
///
/// A struct rather than a bare map so the artifact can grow a field
/// without a format break. Encoded as dag-cbor, which is canonical:
/// re-lowering an unchanged view yields byte-identical output, so the
/// claim does not churn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bindings {
    /// Declaration name (`on/click`) -> the descriptor it resolved to,
    /// with every bare-symbol source already rewritten to an entity.
    pub events: BTreeMap<String, EventDescriptor>,
}

impl Bindings {
    /// True when there is nothing to store.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Encode as dag-cbor.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        serde_ipld_dagcbor::to_vec(self).map_err(|error| error.to_string())
    }

    /// Decode from dag-cbor.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        serde_ipld_dagcbor::from_slice(bytes).map_err(|error| error.to_string())
    }
}

/// Every `on:<name>=<command>` binding a template's raw HTML makes, in
/// document order, deduplicated.
///
/// A text scan rather than a DOM walk, because the analyzer has no DOM
/// and the browser's `preprocess` walk is not available to it. It
/// mirrors that walk on the two points that matter: only attribute
/// positions inside a tag count, and an HTML comment carries none —
/// the parser drops comment content, so a binding mentioned in prose
/// inside `<!-- -->` must not become a dangling reference here.
pub fn scan(template: &str) -> Vec<EventBinding> {
    let bytes = template.as_bytes();
    let mut out: Vec<EventBinding> = Vec::new();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        if template[index..].starts_with("<!--") {
            index = match template[index + 4..].find("-->") {
                Some(offset) => index + 4 + offset + 3,
                None => bytes.len(),
            };
            continue;
        }
        // `<` that opens no tag (a stray less-than in text) is not a
        // tag start; only a name or a closing slash follows one.
        let after = index + 1;
        if after >= bytes.len() || !(bytes[after].is_ascii_alphabetic() || bytes[after] == b'/') {
            index += 1;
            continue;
        }
        index = scan_tag(template, after, &mut out);
    }

    out.sort();
    out.dedup();
    out
}

/// Read one tag's attributes starting just after its `<`. Returns the
/// offset just past the tag's `>` (or the end of input).
fn scan_tag(template: &str, mut index: usize, out: &mut Vec<EventBinding>) -> usize {
    let bytes = template.as_bytes();
    // Skip the tag name (and a closing tag's slash).
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>' {
        index += 1;
    }

    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'>' {
            return (index + 1).min(bytes.len());
        }
        // A self-closing `/` before `>` is not an attribute name.
        if bytes[index] == b'/' {
            index += 1;
            continue;
        }

        let name_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && !matches!(bytes[index], b'=' | b'>' | b'/')
        {
            index += 1;
        }
        let name = &template[name_start..index];

        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            // A valueless attribute — nothing to bind.
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            return bytes.len();
        }

        let value_start;
        let value_end;
        match bytes[index] {
            quote @ (b'"' | b'\'') => {
                index += 1;
                value_start = index;
                while index < bytes.len() && bytes[index] != quote {
                    index += 1;
                }
                value_end = index;
                index = (index + 1).min(bytes.len());
            }
            _ => {
                value_start = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && bytes[index] != b'>'
                {
                    index += 1;
                }
                value_end = index;
            }
        }

        let Some(event_name) = event_name_for_attribute(name) else {
            continue;
        };
        let command = template[value_start..value_end].trim();
        if command.is_empty() {
            continue;
        }
        out.push(EventBinding {
            attribute: name.to_owned(),
            event_name,
            command: command.to_owned(),
        });
    }
    bytes.len()
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
        let bindings = Bindings { events };
        let bytes = bindings.encode().expect("encodes");
        assert_eq!(Bindings::decode(&bytes).expect("decodes"), bindings);
        // Canonical: the same input encodes to the same bytes, so
        // re-lowering an unchanged view does not churn the claim.
        assert_eq!(bindings.encode().expect("encodes"), bytes);
    }
}
