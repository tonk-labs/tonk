//! Parsing and classification of `the:` identifiers in the
//! `dom.event` namespace.
//!
//! A concept attribute that uses `dom.event` as the root of its
//! `the:` identifier is bound from a DOM event rather than from
//! dialog storage. This module knows nothing about the DOM — it
//! parses identifiers into a structured form so the wasm-only
//! traversal layer can follow the path through a `JsValue`.
//!
//! Two roles for `dom.event` identifiers:
//!
//! - **Read** — `dom.event/type`, `dom.event.target/value`,
//!   `dom.event.target.dataset/counter`. Reads a value from the
//!   event object at the named path.
//! - **Action** — `dom.event.do/prevent-default`,
//!   `dom.event.do/stop-propagation`. Calls a method on the event.
//!   The attribute carries no value; its presence in the concept's
//!   schema is the signal.
//!
//! Identifiers outside the `dom.event` family are not classified
//! here; the caller treats them as ordinary dialog attributes that
//! can't be filled from a DOM event.

/// Classification of a `the:` identifier for event-binding
/// purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    /// Identifier under `dom.event` (excluding `dom.event.do`) —
    /// a path to read against the JS event object.
    Read(EventPath),
    /// Identifier under `dom.event.do` — a method to call on the
    /// JS event object.
    Action(EventAction),
    /// Identifier addressed elsewhere — can't be filled from a
    /// DOM event. The caller is responsible for sourcing this
    /// value some other way, or refusing to bind the concept.
    Other,
}

/// A parsed read path. The segments name successive property
/// accesses to apply against the event object.
///
/// For example `dom.event.target.dataset/counter` parses to
/// segments `["target", "dataset", "counter"]`. Following those
/// against `event` yields `event.target.dataset.counter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventPath {
    /// Successive property names to apply, in order. Each name
    /// is the camelCase form ready for direct JS lookup
    /// (`shift-key` becomes `shiftKey`).
    pub segments: Vec<String>,
}

/// A parsed action — the JS method to invoke on the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAction {
    /// The method name in camelCase, ready for direct invocation
    /// via `js_sys::Reflect::get` + `Function::call0`.
    /// `prevent-default` becomes `preventDefault`.
    pub method: String,
}

/// Classify a `the:` identifier.
pub fn classify(identifier: &str) -> Classification {
    if let Some(rest) = identifier.strip_prefix("dom.event.do/") {
        // Action — single segment after the prefix, kebab → camel.
        // We deliberately reject multi-segment actions (`dom.event.do/foo/bar`)
        // since no JS event method needs nesting; treat as Other.
        if rest.is_empty() || rest.contains('/') || rest.contains('.') {
            Classification::Other
        } else {
            Classification::Action(EventAction {
                method: kebab_to_camel(rest),
            })
        }
    } else if let Some(rest) = identifier.strip_prefix("dom.event") {
        // Two cases: empty (the bare `dom.event` identifier, which
        // doesn't address a field) or a path starting with `.` or `/`.
        if rest.is_empty() {
            Classification::Other
        } else if let Some(parsed) = parse_event_path(rest) {
            Classification::Read(parsed)
        } else {
            Classification::Other
        }
    } else {
        Classification::Other
    }
}

/// Parse the suffix of a `dom.event…` identifier into path segments.
///
/// The suffix starts with either `.` or `/` (everything after the
/// `dom.event` prefix). Both characters separate path segments. An
/// empty segment (consecutive separators, leading/trailing
/// separator pair) makes the identifier invalid.
fn parse_event_path(suffix: &str) -> Option<EventPath> {
    let suffix = suffix.strip_prefix(['.', '/'])?;
    if suffix.is_empty() {
        return None;
    }
    let mut segments = Vec::new();
    for part in suffix.split(['.', '/']) {
        if part.is_empty() {
            return None;
        }
        segments.push(kebab_to_camel(part));
    }
    Some(EventPath { segments })
}

/// Turn a kebab-case identifier into camelCase. `shift-key` →
/// `shiftKey`. Already-camelCase identifiers pass through.
fn kebab_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    for ch in s.chars() {
        if ch == '-' {
            upper_next = true;
        } else if upper_next {
            // First non-dash after a dash; uppercase it. Multi-byte
            // chars uppercase as themselves in most locales, which
            // is fine.
            for u in ch.to_uppercase() {
                out.push(u);
            }
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_segments(identifier: &str) -> Vec<String> {
        match classify(identifier) {
            Classification::Read(EventPath { segments }) => segments,
            other => panic!("expected Read, got {other:?} for {identifier:?}"),
        }
    }

    fn action_method(identifier: &str) -> String {
        match classify(identifier) {
            Classification::Action(EventAction { method }) => method,
            other => panic!("expected Action, got {other:?} for {identifier:?}"),
        }
    }

    #[dialog_common::test]
    fn it_classifies_a_top_level_field_as_read() {
        assert_eq!(read_segments("dom.event/type"), vec!["type"]);
        assert_eq!(read_segments("dom.event/key"), vec!["key"]);
        assert_eq!(read_segments("dom.event/pressure"), vec!["pressure"]);
    }

    #[dialog_common::test]
    fn it_camel_cases_kebab_segments() {
        assert_eq!(read_segments("dom.event/shift-key"), vec!["shiftKey"]);
        assert_eq!(read_segments("dom.event/client-x"), vec!["clientX"]);
        assert_eq!(
            read_segments("dom.event/some-very-long-name"),
            vec!["someVeryLongName"]
        );
    }

    #[dialog_common::test]
    fn it_traverses_nested_paths_via_dot_and_slash() {
        assert_eq!(
            read_segments("dom.event.target/value"),
            vec!["target", "value"]
        );
        assert_eq!(
            read_segments("dom.event.target.dataset/counter"),
            vec!["target", "dataset", "counter"]
        );
        assert_eq!(
            read_segments("dom.event.target.dataset/item-id"),
            vec!["target", "dataset", "itemId"]
        );
    }

    #[dialog_common::test]
    fn it_treats_dot_and_slash_uniformly_as_separators() {
        // Mixing dots and slashes in the path is allowed —
        // separator choice doesn't change the meaning.
        assert_eq!(
            read_segments("dom.event/target/value"),
            vec!["target", "value"]
        );
        assert_eq!(
            read_segments("dom.event/target.dataset/counter"),
            vec!["target", "dataset", "counter"]
        );
    }

    #[dialog_common::test]
    fn it_classifies_do_namespace_as_action() {
        assert_eq!(
            action_method("dom.event.do/prevent-default"),
            "preventDefault"
        );
        assert_eq!(
            action_method("dom.event.do/stop-propagation"),
            "stopPropagation"
        );
        assert_eq!(
            action_method("dom.event.do/stop-immediate-propagation"),
            "stopImmediatePropagation"
        );
    }

    #[dialog_common::test]
    fn it_rejects_bare_dom_event_identifier_as_other() {
        // No suffix → no field → not a binding.
        assert!(matches!(classify("dom.event"), Classification::Other));
    }

    #[dialog_common::test]
    fn it_rejects_empty_or_multi_segment_action_as_other() {
        assert!(matches!(classify("dom.event.do/"), Classification::Other));
        assert!(matches!(
            classify("dom.event.do/foo/bar"),
            Classification::Other
        ));
        assert!(matches!(
            classify("dom.event.do/foo.bar"),
            Classification::Other
        ));
    }

    #[dialog_common::test]
    fn it_rejects_paths_with_empty_segments_as_other() {
        assert!(matches!(classify("dom.event/"), Classification::Other));
        assert!(matches!(
            classify("dom.event/foo//bar"),
            Classification::Other
        ));
        assert!(matches!(classify("dom.event/.foo"), Classification::Other));
        assert!(matches!(classify("dom.event/foo."), Classification::Other));
    }

    #[dialog_common::test]
    fn it_classifies_non_dom_event_identifiers_as_other() {
        assert!(matches!(
            classify("xyz.tonk.counter/count"),
            Classification::Other
        ));
        assert!(matches!(
            classify("dialog.name/referent"),
            Classification::Other
        ));
        // Looks like a prefix match but the next char isn't a
        // separator — must not be treated as `dom.event`.
        assert!(matches!(
            classify("dom.eventually/something"),
            Classification::Other
        ));
    }

    #[dialog_common::test]
    fn it_camel_cases_consecutive_dashes_idiomatically() {
        // Two dashes in a row is unusual but well-defined: the
        // first dash sets the upper-next flag; the second char
        // (also a dash) re-sets it; the next non-dash gets
        // uppercased. The doubled dash collapses.
        assert_eq!(read_segments("dom.event/a--b"), vec!["aB"]);
    }
}
