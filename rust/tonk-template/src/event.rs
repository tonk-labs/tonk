//! The `event` concept: how a platform event fills a command's fields.
//!
//! An `event!:` declaration is the seam between a host's input model
//! and a command's domain shape. It carries the platform event name to
//! listen for, whether to suppress the platform's default handling, and
//! a `where:` map from **command field name** to **source**:
//!
//! ```yaml
//! event!: &on/click
//!   type: "click"
//!   prevent-default: false
//!   stop-propagation: false
//!   where:
//!     subject: "{this}"
//!     time: .timeStamp
//! ```
//!
//! ```html
//! <button on:click=increment>+</button>
//! ```
//!
//! Three things follow from putting the mapping here rather than in the
//! command's `with:` map, which is what `dom.event.*` identifiers did:
//!
//! - **The command is domain-shaped.** `increment.subject` is
//!   `io.gozala.increment/subject`, not a DOM path, so it means the same
//!   thing to a rule regardless of which host produced it.
//! - **The platform difference has one home.** A terminal declares its
//!   own `on/activate` against the same command; the command and every
//!   rule downstream are untouched.
//! - **Completeness is checkable before anything runs** ([`check`]):
//!   with the event and the command both in hand at view-lowering, a
//!   required field with no source is a diagnostic instead of a button
//!   that silently does nothing.
//!
//! This module is DOM-free so both the browser renderer and a native
//! one share one parser and one checker, the same way they already
//! share the binding planner.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Where one command field's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `{field}` — the same interpolation a template uses, resolved in
    /// the scope of the element the event fired on: a field of the
    /// enclosing repeat subject's conclusion. `{this}` is the subject
    /// itself.
    Field(String),
    /// `.a.b.c` — a property read off the live platform event. Segments
    /// are taken verbatim, so a DOM source is written the way JS spells
    /// it (`.currentTarget.dataset.todo`) with no case translation to
    /// get wrong.
    Property(Vec<String>),
    /// `"text"` — a constant, for defaults the interaction never
    /// supplies.
    Literal(String),
}

/// Why a `where:` value could not be read as a [`Source`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// Empty, or nothing but whitespace.
    Empty,
    /// `{` with no closing `}`, or text outside the braces.
    MalformedField(String),
    /// A `.` path with an empty segment (`.a..b`, `.`, `a.`).
    MalformedProperty(String),
    /// A bare word — neither `{field}`, nor `.path`, nor quoted.
    ///
    /// Rejected rather than guessed: `timeStamp` without its leading
    /// dot would otherwise be silently taken as the literal string
    /// `"timeStamp"`, which is exactly the class of typo this design
    /// exists to catch.
    Bare(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Empty => write!(f, "empty source"),
            SourceError::MalformedField(raw) => {
                write!(f, "`{raw}` is not a well-formed `{{field}}` reference")
            }
            SourceError::MalformedProperty(raw) => {
                write!(f, "`{raw}` has an empty path segment")
            }
            SourceError::Bare(raw) => write!(
                f,
                "`{raw}` is a bare word — write `.{raw}` to read it off the event, \
                 `{{{raw}}}` to read a field, or `\"{raw}\"` for a literal"
            ),
        }
    }
}

/// Read one `where:` value.
pub fn parse_source(raw: &str) -> Result<Source, SourceError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(SourceError::Empty);
    }

    if let Some(rest) = raw.strip_prefix('{') {
        let Some(name) = rest.strip_suffix('}') else {
            return Err(SourceError::MalformedField(raw.to_string()));
        };
        let name = name.trim();
        if name.is_empty() || name.contains('{') || name.contains('}') {
            return Err(SourceError::MalformedField(raw.to_string()));
        }
        return Ok(Source::Field(name.to_string()));
    }

    if let Some(rest) = raw.strip_prefix('.') {
        let segments: Vec<String> = rest.split('.').map(str::to_string).collect();
        if segments.iter().any(String::is_empty) {
            return Err(SourceError::MalformedProperty(raw.to_string()));
        }
        return Ok(Source::Property(segments));
    }

    // A quoted value is a literal. YAML usually strips the quotes
    // before we see the string, so an already-unquoted literal is
    // accepted only in that form; everything else is a bare word.
    if let Some(inner) = raw
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            raw.strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
    {
        return Ok(Source::Literal(inner.to_string()));
    }

    Err(SourceError::Bare(raw.to_string()))
}

/// A parsed `event!:` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDescriptor {
    /// The platform event name to listen for (`click`, `submit`,
    /// `createsheet`). Distinct from the declaration's own identifier,
    /// so several declarations can read the same platform event
    /// differently — which is how a site-specific binding is expressed
    /// without a per-site override syntax.
    pub event_type: String,
    /// Suppress the platform's default handling.
    pub prevent_default: bool,
    /// Stop the event propagating further.
    pub stop_propagation: bool,
    /// Command field name -> source.
    pub sources: BTreeMap<String, Source>,
}

/// Why an `event!:` declaration could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventError {
    /// No `type:`, or it was not text.
    MissingType,
    /// A `where:` entry's value was unreadable.
    Source {
        /// The command field the entry was for.
        field: String,
        /// What was wrong with it.
        error: SourceError,
    },
}

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventError::MissingType => {
                write!(f, "event declaration has no `type:`")
            }
            EventError::Source { field, error } => {
                write!(f, "source for `{field}`: {error}")
            }
        }
    }
}

/// Build a descriptor from an event instance's already-projected
/// fields: `type`, `prevent-default`, `stop-propagation`, and the
/// `where` dictionary.
pub fn event_descriptor(
    event_type: Option<&str>,
    prevent_default: bool,
    stop_propagation: bool,
    where_entries: impl IntoIterator<Item = (String, String)>,
) -> Result<EventDescriptor, EventError> {
    let event_type = event_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(EventError::MissingType)?;

    let mut sources = BTreeMap::new();
    for (field, raw) in where_entries {
        let source = parse_source(&raw).map_err(|error| EventError::Source {
            field: field.clone(),
            error,
        })?;
        sources.insert(field, source);
    }

    Ok(EventDescriptor {
        event_type: event_type.to_string(),
        prevent_default,
        stop_propagation,
        sources,
    })
}

/// A command field an event cannot fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unfilled {
    /// The field's name on the command.
    pub field: String,
}

/// A source naming a field the command does not declare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unknown {
    /// The name the `where:` entry used.
    pub field: String,
}

/// The result of checking an event declaration against a command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mismatch {
    /// Required (`with:`) fields with no source. These are the ones
    /// that make an interaction silently do nothing today.
    pub unfilled: Vec<Unfilled>,
    /// Sources for fields the command does not have. Usually a typo,
    /// and harmless at runtime, which is why it is worth reporting.
    pub unknown: Vec<Unknown>,
}

impl Mismatch {
    /// True when nothing is wrong.
    pub fn is_empty(&self) -> bool {
        self.unfilled.is_empty() && self.unknown.is_empty()
    }
}

/// Check that `event` can fill `command`.
///
/// `required` is the command's `with:` field names, `optional` its
/// `maybe:` ones. This is the check that turns the guide's documented
/// "no warn, but the rule didn't fire" into a diagnostic: it needs only
/// the two declarations, not a resolvable source, so it works even
/// though a `.path` is opaque text.
pub fn check(
    event: &EventDescriptor,
    required: &BTreeSet<String>,
    optional: &BTreeSet<String>,
) -> Mismatch {
    let mut mismatch = Mismatch::default();

    for field in required {
        if !event.sources.contains_key(field) {
            mismatch.unfilled.push(Unfilled {
                field: field.clone(),
            });
        }
    }
    for field in event.sources.keys() {
        if !required.contains(field) && !optional.contains(field) {
            mismatch.unknown.push(Unknown {
                field: field.clone(),
            });
        }
    }
    mismatch
}

/// The attribute prefix that marks an event binding in a template.
pub const ON_PREFIX: &str = "on:";

/// The namespace every event declaration lives under.
pub const ON_NAMESPACE: &str = "on/";

/// Map a template attribute name to the event declaration it names.
///
/// `on:click` -> `on/click`. `/` is not a legal character in an HTML
/// attribute name (the spec excludes space, `"`, `'`, `>`, `/`, `=`)
/// while `:` is, so the identifier and the attribute differ by exactly
/// this substitution.
///
/// Only the single `on:` prefix is reserved, which leaves `bind:base`,
/// `xlink:href` and `xml:lang` alone — an attribute merely containing a
/// colon is not a binding.
pub fn event_name_for_attribute(attribute: &str) -> Option<String> {
    let rest = attribute.strip_prefix(ON_PREFIX)?;
    if rest.is_empty() || rest.contains(':') {
        return None;
    }
    Some(format!("{ON_NAMESPACE}{rest}"))
}

/// The template attribute that would name `event_name`, for
/// diagnostics that want to point at the source.
pub fn attribute_for_event_name(event_name: &str) -> Option<String> {
    let rest = event_name.strip_prefix(ON_NAMESPACE)?;
    if rest.is_empty() {
        return None;
    }
    Some(format!("{ON_PREFIX}{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn required(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn a_braced_name_is_a_field_read() {
        assert_eq!(parse_source("{this}"), Ok(Source::Field("this".into())));
        assert_eq!(parse_source(" {title} "), Ok(Source::Field("title".into())));
        assert_eq!(
            parse_source("{dom.host/repo}"),
            Ok(Source::Field("dom.host/repo".into()))
        );
    }

    #[test]
    fn a_leading_dot_is_a_property_read() {
        assert_eq!(
            parse_source(".timeStamp"),
            Ok(Source::Property(vec!["timeStamp".into()]))
        );
        assert_eq!(
            parse_source(".currentTarget.dataset.todo"),
            Ok(Source::Property(vec![
                "currentTarget".into(),
                "dataset".into(),
                "todo".into()
            ]))
        );
    }

    #[test]
    fn property_segments_are_verbatim() {
        // The old identifier form kebab-cased then camel-cased, so
        // `name="like-seed"` had to be looked up as `likeSeed` and a
        // kebab control name silently failed. Written as JS spells it,
        // there is no transformation to get wrong.
        assert_eq!(
            parse_source(".currentTarget.elements.noteBody.value"),
            Ok(Source::Property(vec![
                "currentTarget".into(),
                "elements".into(),
                "noteBody".into(),
                "value".into()
            ]))
        );
    }

    #[test]
    fn a_quoted_value_is_a_literal() {
        assert_eq!(
            parse_source("\"Untitled\""),
            Ok(Source::Literal("Untitled".into()))
        );
        assert_eq!(parse_source("''"), Ok(Source::Literal(String::new())));
    }

    #[test]
    fn a_bare_word_is_rejected_rather_than_guessed() {
        // The whole point: `timeStamp` (missing its dot) must not be
        // silently accepted as the literal string "timeStamp".
        let error = parse_source("timeStamp").expect_err("bare word");
        assert_eq!(error, SourceError::Bare("timeStamp".into()));
        assert!(error.to_string().contains(".timeStamp"), "{error}");
    }

    #[test]
    fn malformed_sources_are_named() {
        assert_eq!(
            parse_source("{this"),
            Err(SourceError::MalformedField("{this".into()))
        );
        assert_eq!(
            parse_source(".a..b"),
            Err(SourceError::MalformedProperty(".a..b".into()))
        );
        assert_eq!(parse_source("   "), Err(SourceError::Empty));
    }

    #[test]
    fn a_descriptor_needs_a_type() {
        assert_eq!(
            event_descriptor(None, false, false, []),
            Err(EventError::MissingType)
        );
        assert_eq!(
            event_descriptor(Some("  "), false, false, []),
            Err(EventError::MissingType)
        );
    }

    #[test]
    fn a_bad_source_names_its_field() {
        let error = event_descriptor(
            Some("click"),
            false,
            false,
            [("time".to_string(), "timeStamp".to_string())],
        )
        .expect_err("bare source");
        let EventError::Source { field, .. } = &error else {
            panic!("expected a source error, got {error:?}");
        };
        assert_eq!(field, "time");
        assert!(error.to_string().contains("time"), "{error}");
    }

    #[test]
    fn a_required_field_with_no_source_is_reported() {
        // The failure this design exists to catch: today the command
        // posts without `subject`, the rule's premise matches nothing,
        // and the click is a silent no-op.
        let event = event_descriptor(
            Some("click"),
            false,
            false,
            [("time".to_string(), ".timeStamp".to_string())],
        )
        .expect("descriptor");
        let mismatch = check(&event, &required(&["subject", "time"]), &BTreeSet::new());
        assert_eq!(
            mismatch.unfilled,
            vec![Unfilled {
                field: "subject".into()
            }]
        );
        assert!(mismatch.unknown.is_empty());
    }

    #[test]
    fn an_optional_field_needs_no_source() {
        let event = event_descriptor(
            Some("click"),
            false,
            false,
            [("subject".to_string(), "{this}".to_string())],
        )
        .expect("descriptor");
        let mismatch = check(&event, &required(&["subject"]), &required(&["time"]));
        assert!(mismatch.is_empty(), "{mismatch:?}");
    }

    #[test]
    fn a_source_for_an_unknown_field_is_reported() {
        let event = event_descriptor(
            Some("click"),
            false,
            false,
            [
                ("subject".to_string(), "{this}".to_string()),
                ("tiem".to_string(), ".timeStamp".to_string()),
            ],
        )
        .expect("descriptor");
        let mismatch = check(&event, &required(&["subject"]), &required(&["time"]));
        assert_eq!(
            mismatch.unknown,
            vec![Unknown {
                field: "tiem".into()
            }]
        );
    }

    #[test]
    fn only_the_on_prefix_is_reserved() {
        assert_eq!(
            event_name_for_attribute("on:click").as_deref(),
            Some("on/click")
        );
        assert_eq!(
            event_name_for_attribute("on:createsheet").as_deref(),
            Some("on/createsheet")
        );
        // Not bindings: an attribute merely containing a colon.
        assert_eq!(event_name_for_attribute("bind:base"), None);
        assert_eq!(event_name_for_attribute("xlink:href"), None);
        assert_eq!(event_name_for_attribute("xml:lang"), None);
        // Nor the legacy form, which keeps working through its own path.
        assert_eq!(event_name_for_attribute("onclick"), None);
        // Nor a nested name: `on/` is one flat namespace for now.
        assert_eq!(event_name_for_attribute("on:table:createsheet"), None);
        assert_eq!(event_name_for_attribute("on:"), None);
    }

    #[test]
    fn attribute_and_event_name_round_trip() {
        for attribute in ["on:click", "on:submit", "on:createsheet"] {
            let name = event_name_for_attribute(attribute).expect("event name");
            assert_eq!(attribute_for_event_name(&name).as_deref(), Some(attribute));
        }
        assert_eq!(attribute_for_event_name("increment"), None);
    }
}

/// The event declarations a mounted template binds, indexed for
/// dispatch.
///
/// A fired event knows only its platform type, and several
/// declarations may share one — that is how a site-specific binding is
/// expressed. So dispatch is: for the fired type, which of this
/// element's attributes names a declaration of that type?
#[derive(Debug, Clone, Default)]
pub struct EventTable {
    /// Declaration name (`on/click`) -> descriptor.
    declarations: BTreeMap<String, EventDescriptor>,
}

/// One resolved binding: which declaration fired, and what it asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding<'a> {
    /// The declaration's name, e.g. `on/click`.
    pub event_name: &'a str,
    /// The declaration.
    pub event: &'a EventDescriptor,
    /// The command the attribute's value named.
    pub command: String,
}

impl EventTable {
    /// Index a set of resolved declarations.
    pub fn new(declarations: BTreeMap<String, EventDescriptor>) -> Self {
        Self { declarations }
    }

    /// Look up one declaration.
    pub fn get(&self, event_name: &str) -> Option<&EventDescriptor> {
        self.declarations.get(event_name)
    }

    /// Every distinct platform event type to install a listener for.
    ///
    /// This is what closes the "which events does a template subscribe
    /// to?" question without scanning the DOM: it falls out of the
    /// declarations the view resolved.
    pub fn event_types(&self) -> BTreeSet<String> {
        self.declarations
            .values()
            .map(|event| event.event_type.clone())
            .collect()
    }

    /// Resolve the binding for a fired `event_type` against one
    /// element's attributes.
    ///
    /// `attributes` yields `(name, value)` pairs. Only `on:`-prefixed
    /// names resolving to a declaration of the fired type match, so an
    /// element carrying both `on:click=a` and `on:keydown=b` dispatches
    /// each to the right command, and `bind:base` is never considered.
    ///
    /// When several attributes on one element qualify, the
    /// lexicographically first declaration name wins, so dispatch does
    /// not depend on DOM attribute order.
    pub fn resolve<'a, I, N, V>(&'a self, event_type: &str, attributes: I) -> Option<Binding<'a>>
    where
        I: IntoIterator<Item = (N, V)>,
        N: AsRef<str>,
        V: AsRef<str>,
    {
        let mut best: Option<Binding<'a>> = None;
        for (name, value) in attributes {
            let Some(event_name) = event_name_for_attribute(name.as_ref()) else {
                continue;
            };
            let Some((key, event)) = self.declarations.get_key_value(&event_name) else {
                continue;
            };
            if event.event_type != event_type {
                continue;
            }
            let command = value.as_ref().trim();
            if command.is_empty() {
                continue;
            }
            let candidate = Binding {
                event_name: key.as_str(),
                event,
                command: command.to_string(),
            };
            if best
                .as_ref()
                .is_none_or(|b| candidate.event_name < b.event_name)
            {
                best = Some(candidate);
            }
        }
        best
    }
}

/// Assemble the `TransactRequest` body for one fired binding.
///
/// `descriptor` is the command's dialog descriptor and `parameters` the
/// already-resolved field values — resolving a [`Source`] needs the
/// live event and the rendered row, so it stays with the host, and
/// everything after it is shared.
///
/// The wire shape matches what the `dom.event.*` path did, so the
/// worker sees no difference between a command posted the old way and
/// the new one. That is what lets templates migrate one at a time.
pub fn transact_body(
    descriptor: &serde_json::Value,
    parameters: BTreeMap<String, serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": descriptor.clone(),
                },
                "parameters": parameters,
            },
        }],
    })
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    fn event(event_type: &str, sources: &[(&str, &str)]) -> EventDescriptor {
        event_descriptor(
            Some(event_type),
            false,
            false,
            sources
                .iter()
                .map(|(field, raw)| ((*field).to_string(), (*raw).to_string())),
        )
        .expect("descriptor")
    }

    fn table() -> EventTable {
        EventTable::new(BTreeMap::from([
            (
                "on/click".to_string(),
                event("click", &[("subject", "{this}")]),
            ),
            (
                "on/click-once".to_string(),
                event("click", &[("subject", "{this}"), ("time", ".timeStamp")]),
            ),
            (
                "on/submit".to_string(),
                event("submit", &[("subject", "{this}")]),
            ),
        ]))
    }

    #[test]
    fn listener_types_come_from_the_declarations() {
        // Deduplicated: two declarations share "click".
        assert_eq!(
            table().event_types(),
            BTreeSet::from(["click".to_string(), "submit".to_string()])
        );
    }

    #[test]
    fn an_attribute_dispatches_to_its_command() {
        let table = table();
        let binding = table
            .resolve("click", [("on:click", "increment")])
            .expect("binding");
        assert_eq!(binding.event_name, "on/click");
        assert_eq!(binding.command, "increment");
    }

    #[test]
    fn a_different_type_on_the_same_element_does_not_match() {
        let table = table();
        assert!(
            table
                .resolve("submit", [("on:click", "increment")])
                .is_none()
        );
        let binding = table
            .resolve("submit", [("on:click", "increment"), ("on:submit", "save")])
            .expect("binding");
        assert_eq!(binding.command, "save");
    }

    #[test]
    fn declarations_sharing_a_type_stay_distinct() {
        // Two declarations both read "click"; the attribute picks.
        let table = table();
        let binding = table
            .resolve("click", [("on:click-once", "publish")])
            .expect("binding");
        assert_eq!(binding.event_name, "on/click-once");
        assert!(binding.event.sources.contains_key("time"));
    }

    #[test]
    fn non_event_attributes_are_never_considered() {
        let table = table();
        assert!(
            table
                .resolve(
                    "click",
                    [
                        ("bind:base", "data-base"),
                        ("xlink:href", "#x"),
                        ("xml:lang", "en"),
                        ("onclick", "legacy"),
                        ("data-this", "id:1"),
                    ]
                )
                .is_none(),
            "only `on:` names bind"
        );
    }

    #[test]
    fn an_unresolvable_declaration_does_not_bind() {
        // `on:hover` names no declaration; it must not fall through to
        // some other binding on the element.
        let table = table();
        assert!(
            table
                .resolve("click", [("on:hover", "increment")])
                .is_none()
        );
    }

    #[test]
    fn an_empty_command_does_not_bind() {
        let table = table();
        assert!(table.resolve("click", [("on:click", "  ")]).is_none());
    }

    #[test]
    fn dispatch_does_not_depend_on_attribute_order() {
        let table = table();
        let forward = table
            .resolve("click", [("on:click", "a"), ("on:click-once", "b")])
            .expect("binding");
        let reverse = table
            .resolve("click", [("on:click-once", "b"), ("on:click", "a")])
            .expect("binding");
        assert_eq!(forward, reverse);
    }

    #[test]
    fn the_wire_shape_matches_the_legacy_path() {
        // Byte-identical to what the `dom.event.*` extractor built, so
        // the worker cannot tell which form posted the command — which
        // is what makes migrating one template at a time safe.
        let descriptor = serde_json::json!({ "with": { "subject": {} } });
        let body = transact_body(
            &descriptor,
            BTreeMap::from([("subject".to_string(), serde_json::json!("did:key:zCounter"))]),
        );
        assert_eq!(body["claims"][0]["op"], serde_json::json!("assert"));
        assert_eq!(
            body["claims"][0]["application"]["predicate"]["kind"],
            serde_json::json!("transient")
        );
        assert_eq!(
            body["claims"][0]["application"]["predicate"]["concept"],
            descriptor
        );
        assert_eq!(
            body["claims"][0]["application"]["parameters"]["subject"],
            serde_json::json!("did:key:zCounter")
        );
    }
}
