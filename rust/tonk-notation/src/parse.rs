//! YAML parser for asserted notation.
//!
//! [`parse`] takes a YAML document and walks it via [`saphyr`],
//! producing a [`Syntax`] tree plus a list of diagnostics. The
//! parser is permissive: a malformed expression yields a
//! diagnostic and the rest of the document still parses, so the
//! language server can underline several problems at once.
//!
//! The parser does **not** resolve names against a branch, derive
//! entity URIs, or know about the dialog meta-schema. Those
//! concerns live in [`tonk-schema`'s analyzer][analyze].
//!
//! [analyze]: https://github.com/dialog-db/tonk-workers/tree/main/rust/tonk-schema/src/interpret.rs

use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use saphyr::{MarkedYaml, Scalar as SaphyrScalar, ScanError, YamlData, YamlLoader};
use saphyr_parser::{Event, Marker, Parser, ScalarStyle, Span, SpannedEventReceiver, StrInput};

use crate::syntax::{
    Anchor, Application, Effectful, Expression, Field, FieldValue, HeadName, Predicate, Premise,
    Scalar, Spanned, Syntax,
};

/// Outcome of a parse.
#[derive(Clone, Debug, Default)]
pub struct Parsed {
    /// The parsed [`Syntax`] when parsing reached the bottom.
    pub syntax: Option<Syntax>,
    /// Diagnostics raised during the parse.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse `text` as a YAML document and convert it to a [`Syntax`]
/// tree.
pub fn parse(text: &str) -> Parsed {
    let documents = match parse_documents(text) {
        Ok(documents) => documents,
        Err(err) => {
            return Parsed {
                syntax: None,
                diagnostics: vec![diagnostic_for_scan_error(&err)],
            };
        }
    };

    if documents.is_empty() {
        return Parsed::default();
    }

    let mut diagnostics = Vec::new();
    let mut expressions = Vec::new();
    let mut overall_range: Option<Range> = None;
    for doc in &documents {
        let doc_range = range_from_span(doc.span);
        overall_range = Some(match overall_range {
            None => doc_range,
            Some(existing) => extend_range(existing, doc_range),
        });
        walk_document(doc, &mut expressions, &mut diagnostics);
    }

    let range = overall_range
        .map(|r| clamp_range(r, text))
        .unwrap_or_default();
    // Clamp every diagnostic so its range stays inside the
    // document — saphyr can hand us spans that point at
    // `(line_count, 0)`, one past the last real line, and that
    // crashes LSP clients that index into the buffer before
    // they consult the document version.
    for diagnostic in &mut diagnostics {
        diagnostic.range = clamp_range(diagnostic.range, text);
    }
    Parsed {
        syntax: Some(Syntax { expressions, range }),
        diagnostics,
    }
}

// -----------------------------------------------------------------
// Duplicate-preserving document loader
// -----------------------------------------------------------------
//
// saphyr's high-level `MarkedYaml::load_from_str` builds the
// document-root mapping into a `LinkedHashMap`, which silently
// collapses duplicate keys (last value wins). We want the opposite:
// two identical `head:` blocks at the document root should both
// produce expressions, so the user can spread one query across
// multiple blocks.

/// A document-root entry: head key, body value, and the head's
/// `&anchor` (recovered from the raw event spans in the loader,
/// while they're still reliable).
type RootPair<'input> = (MarkedYaml<'input>, MarkedYaml<'input>, Option<Anchor>);

/// One parsed YAML document with its root pairs in source order.
struct TopLevelDoc<'input> {
    pairs: Option<Vec<RootPair<'input>>>,
    span: Span,
    /// Spans of any `Event::Alias` events seen in this document.
    /// Aliases are not supported in this notation — saphyr's loader
    /// resolves them silently by substituting the anchor's content,
    /// so we have to remember the events ourselves to surface them
    /// as diagnostics.
    alias_spans: Vec<Span>,
}

fn parse_documents(text: &str) -> Result<Vec<TopLevelDoc<'_>>, ScanError> {
    let mut parser = Parser::new_from_str(text);
    let mut docs: Vec<TopLevelDoc<'_>> = Vec::new();
    let mut state = LoaderState::Idle;
    let mut current_aliases: Vec<Span> = Vec::new();
    while let Some(event) = parser.next_event() {
        let (event, span) = event?;
        if matches!(event, Event::Alias(_)) {
            current_aliases.push(span);
        }
        match &mut state {
            LoaderState::Idle => match event {
                Event::DocumentStart(_) => {
                    state = LoaderState::DocumentStarted { start: span.start };
                }
                Event::StreamStart | Event::StreamEnd | Event::Nothing => {}
                _ => {}
            },
            LoaderState::DocumentStarted { start } => match event {
                Event::MappingStart(_, _) => {
                    state = LoaderState::DocumentRoot {
                        start: *start,
                        pairs: Vec::new(),
                        pending_key: None,
                    };
                }
                Event::DocumentEnd => {
                    docs.push(TopLevelDoc {
                        pairs: None,
                        span: Span::new(*start, span.end),
                        alias_spans: std::mem::take(&mut current_aliases),
                    });
                    state = LoaderState::Idle;
                }
                _ => {
                    let mut depth = match &event {
                        Event::SequenceStart(_, _) | Event::MappingStart(_, _) => 1u32,
                        _ => 0,
                    };
                    let mut last_end = span.end;
                    while depth > 0 {
                        match parser.next_event() {
                            Some(Ok((ev, sp))) => {
                                last_end = sp.end;
                                match ev {
                                    Event::SequenceStart(_, _) | Event::MappingStart(_, _) => {
                                        depth += 1
                                    }
                                    Event::SequenceEnd | Event::MappingEnd => depth -= 1,
                                    _ => {}
                                }
                            }
                            Some(Err(err)) => return Err(err),
                            None => break,
                        }
                    }
                    let document_end = expect_document_end(&mut parser, last_end)?;
                    docs.push(TopLevelDoc {
                        pairs: None,
                        span: Span::new(*start, document_end),
                        alias_spans: std::mem::take(&mut current_aliases),
                    });
                    state = LoaderState::Idle;
                }
            },
            LoaderState::DocumentRoot {
                start,
                pairs,
                pending_key,
            } => match event {
                Event::MappingEnd => {
                    let pairs = std::mem::take(pairs);
                    let document_end = expect_document_end(&mut parser, span.end)?;
                    docs.push(TopLevelDoc {
                        pairs: Some(pairs),
                        span: Span::new(*start, document_end),
                        alias_spans: std::mem::take(&mut current_aliases),
                    });
                    state = LoaderState::Idle;
                }
                Event::Scalar(_, _, _, _) if pending_key.is_none() => {
                    let key = scalar_to_marked_yaml(event, span);
                    *pending_key = Some(key);
                }
                _ if pending_key.is_some() => {
                    let key = pending_key.take().unwrap();
                    // Recover the head's `&anchor` here, while we
                    // still hold the raw event spans. The value's
                    // first event carries a non-zero anchor id iff the
                    // head is anchored, and the `&name` token sits in
                    // the source gap between the key's end and the
                    // value's start. We must read it now: the spans on
                    // the loaded `MarkedYaml` value node drift (saphyr
                    // re-marks them, unreliably after block scalars),
                    // so a later source scan keyed off those spans
                    // silently mis-locates or drops the anchor.
                    let anchor = anchor_id_of(&event)
                        .and_then(|_| scan_anchor(text, key.span.end, span.start));
                    let value = load_subtree(&mut parser, event, span, &mut current_aliases)?;
                    pairs.push((key, value, anchor));
                }
                Event::Scalar(_, _, _, _) => {
                    unreachable!("scalar handling covered by pending_key arms");
                }
                _ => {
                    let synthetic_key = MarkedYaml {
                        span,
                        data: YamlData::Value(SaphyrScalar::Null),
                    };
                    let value = load_subtree(&mut parser, event, span, &mut current_aliases)?;
                    pairs.push((synthetic_key, value, None));
                }
            },
        }
    }
    Ok(docs)
}

enum LoaderState<'input> {
    Idle,
    DocumentStarted {
        start: Marker,
    },
    DocumentRoot {
        start: Marker,
        pairs: Vec<RootPair<'input>>,
        pending_key: Option<MarkedYaml<'input>>,
    },
}

/// The anchor id carried by a node's first event, or `None` when
/// the node is not anchored (id `0`). A non-zero id is the
/// reliable signal that a `&anchor` token precedes this value in
/// the source.
fn anchor_id_of(event: &Event<'_>) -> Option<usize> {
    let aid = match event {
        Event::MappingStart(a, _) | Event::SequenceStart(a, _) => *a,
        Event::Scalar(_, _, a, _) => *a,
        _ => 0,
    };
    (aid != 0).then_some(aid)
}

fn scalar_to_marked_yaml<'input>(event: Event<'input>, span: Span) -> MarkedYaml<'input> {
    let Event::Scalar(value, style, _, tag) = event else {
        unreachable!("scalar_to_marked_yaml called on non-scalar event");
    };
    let data = match style {
        ScalarStyle::Plain | ScalarStyle::DoubleQuoted | ScalarStyle::SingleQuoted
            if tag.as_ref().is_some_and(|t| !t.is_yaml_core_schema()) =>
        {
            YamlData::Representation(value, style, tag)
        }
        _ => {
            let yaml = saphyr::Yaml::value_from_cow_and_metadata(value, style, tag.as_ref());
            yaml_to_data(yaml)
        }
    };
    MarkedYaml { span, data }
}

fn yaml_to_data<'input>(yaml: saphyr::Yaml<'input>) -> YamlData<'input, MarkedYaml<'input>> {
    match yaml {
        saphyr::Yaml::Value(scalar) => YamlData::Value(scalar),
        saphyr::Yaml::Representation(text, style, tag) => {
            YamlData::Representation(text, style, tag)
        }
        saphyr::Yaml::BadValue => YamlData::BadValue,
        _ => YamlData::BadValue,
    }
}

fn load_subtree<'input>(
    parser: &mut Parser<'input, StrInput<'input>>,
    first_event: Event<'input>,
    first_span: Span,
    alias_spans: &mut Vec<Span>,
) -> Result<MarkedYaml<'input>, ScanError> {
    // `early_parse(false)` keeps every scalar as
    // `YamlData::Representation(text, style, tag)` instead of
    // collapsing it to a typed `Value`. We need the original
    // `ScalarStyle` to tell quoted strings apart from plain
    // scalars (the latter classify into Symbol / Variable / Uri /
    // Blank, while the former are always literals).
    let mut loader: YamlLoader<MarkedYaml<'input>> = YamlLoader::default();
    loader.early_parse(false);
    let stream_mark = Span::empty(first_span.start);
    loader.on_event(Event::StreamStart, stream_mark);
    loader.on_event(Event::DocumentStart(false), stream_mark);

    let mut depth = match &first_event {
        Event::SequenceStart(_, _) | Event::MappingStart(_, _) => 1i32,
        _ => 0,
    };
    let mut last_end = first_span.end;
    if matches!(first_event, Event::Alias(_)) {
        alias_spans.push(first_span);
    }
    loader.on_event(first_event, first_span);
    while depth > 0 {
        match parser.next_event() {
            Some(Ok((ev, sp))) => {
                last_end = sp.end;
                if matches!(ev, Event::Alias(_)) {
                    alias_spans.push(sp);
                }
                match &ev {
                    Event::SequenceStart(_, _) | Event::MappingStart(_, _) => depth += 1,
                    Event::SequenceEnd | Event::MappingEnd => depth -= 1,
                    _ => {}
                }
                loader.on_event(ev, sp);
            }
            Some(Err(err)) => return Err(err),
            None => break,
        }
    }
    let end_mark = Span::empty(last_end);
    loader.on_event(Event::DocumentEnd, end_mark);
    loader.on_event(Event::StreamEnd, end_mark);

    let mut docs = loader.into_documents();
    Ok(docs.pop().unwrap_or(MarkedYaml {
        span: first_span,
        data: YamlData::BadValue,
    }))
}

fn expect_document_end<'input>(
    parser: &mut Parser<'input, StrInput<'input>>,
    fallback: Marker,
) -> Result<Marker, ScanError> {
    while let Some(event) = parser.next_event() {
        let (event, span) = event?;
        match event {
            Event::DocumentEnd => return Ok(span.end),
            Event::StreamEnd => return Ok(span.end),
            _ => {}
        }
    }
    Ok(fallback)
}

// -----------------------------------------------------------------
// YAML walker (saphyr → Syntax)
// -----------------------------------------------------------------

fn walk_document(
    doc: &TopLevelDoc<'_>,
    expressions: &mut Vec<Expression>,
    out: &mut Vec<Diagnostic>,
) {
    for span in &doc.alias_spans {
        out.push(error(
            range_from_span(*span),
            r#"YAML aliases (`*name`) are not supported in this notation. Use a `&anchor` plus the bare symbol name (`person-name`) to reference an in-document entity, or a `did:key:…` / `id:…` URI."#,
        ));
    }
    let Some(pairs) = &doc.pairs else {
        out.push(error(
            range_from_span(doc.span),
            r#"Asserted notation expects a mapping at the document root (head → body)."#,
        ));
        return;
    };
    for (head_key, body_value, anchor) in pairs {
        if let Some(expression) = walk_expression(head_key, body_value, anchor.clone(), out) {
            expressions.push(expression);
        }
    }
}

fn walk_expression(
    key: &MarkedYaml<'_>,
    value: &MarkedYaml<'_>,
    anchor: Option<Anchor>,
    out: &mut Vec<Diagnostic>,
) -> Option<Expression> {
    let key_text = match string_of(key) {
        Some(s) => s,
        None => {
            out.push(error(
                range_of(key),
                r#"Head must be a string (a concept name like `person` or `person!`, a claim domain like `xyz.tonk!`, or a URI like `db:concept!`)."#,
            ));
            return None;
        }
    };

    let key_range = range_of(key);
    let (head, effect) = parse_head(key_text, key_range, out)?;
    let block_range = extend_range(key_range, range_of(value));
    // `anchor` was recovered in the loader from the raw event spans
    // (see `parse_documents`), not scanned here — the `MarkedYaml`
    // value spans drift unreliably after block scalars.
    let rule_body = effect && is_rule_predicate(&head);

    // `rule!:` claims forbid `&anchor`: the rule has no single
    // subject entity to bind a name to (the rule's *effect entity*
    // is content-derived from the body).
    if rule_body && anchor.is_some() {
        out.push(error(
            block_range,
            r#"`&anchor` is not valid on a `rule!:` claim. Anchors publish a single entity's name; rules have no single subject entity (the effect's identity is derived from its rule body)."#,
        ));
    }

    // Body: null/empty (no-fields query or assertion), or a
    // mapping of fields. A bare `_` body is rejected — entity
    // selection requires a `this:` field, which requires a
    // mapping body. Per-field retraction lives inside the body
    // as `field: _` or `..: _`.
    if is_blank_scalar(value) {
        out.push(error(
            range_of(value),
            r#"A bare `_` body is not allowed. Retract attributes from inside the body (`field: _` for one attribute, `..: _` for the rest of the concept's `with:` map). The entity must be selected with `this:` in the same body."#,
        ));
        return None;
    }

    let field_nodes = match &value.data {
        YamlData::Mapping(fields) => {
            let mut nodes = Vec::new();
            for (field_key, field_value) in fields {
                if let Some(field) = walk_field(field_key, field_value, rule_body, out) {
                    nodes.push(field);
                }
            }
            nodes
        }
        // Empty body — saphyr surfaces this as either a typed
        // null (`Value::Null`) or, with `early_parse(false)`, a
        // plain-scalar `Representation` whose text is empty or
        // the YAML core-schema null token (`~`, `null`). Treat
        // any of those as "no fields", which is a query for any
        // entity matching the head, or an effect-free assertion.
        YamlData::Value(SaphyrScalar::Null) => Vec::new(),
        YamlData::Representation(text, _, _) if is_null_text(text.as_ref()) => Vec::new(),
        _ => {
            out.push(error(
                range_of(value),
                r#"Body must be either a mapping of `field: value`, empty, or `_` (retraction)."#,
            ));
            return None;
        }
    };

    let application = Application {
        predicate: head,
        fields: field_nodes,
        range: block_range,
    };

    if effect {
        Some(Expression::Claim(Effectful {
            anchor,
            inner: application,
        }))
    } else {
        if anchor.is_some() {
            // Anchors only make sense on assertions (one entity per
            // expression). Reject on queries.
            out.push(error(
                block_range,
                r#"`&anchor` is only allowed on assertion heads (`head!:`). Queries can match many entities, so an anchor would have no single target to point at."#,
            ));
        }
        Some(Expression::Query(application))
    }
}

/// `rule!:` claims have body fields whose values follow a richer
/// shape than the generic field-map (`when:` / `unless:` take
/// premise lists). [`walk_field`] dispatches on this when the
/// containing claim is over the `rule` predicate.
fn is_rule_predicate(head: &Predicate) -> bool {
    matches!(&head.name, HeadName::Concept(name) if name == "rule")
}

// ---------------------------------------------------------------- //
// Premise parsing — for `when:` / `unless:` values inside `rule!:`. //
// ---------------------------------------------------------------- //
//
// A premise list sits inside a `rule!:` claim body as the value of
// a `when:` or `unless:` field. The list shape is fixed:
//

// ```yaml
// - assert: counter
//   where: { this: ?c, count: ?n }
// - assert: increment
//   where: { subject: ?c }
// ```
//
// Premises are typed in the syntax tree (a [`Premise`] is a concept
// + bindings + range) rather than nested `Field`s so each premise's
// span survives into analyzer diagnostics. The analyzer reads
// [`FieldValue::Premises`] when projecting a `rule` claim into an
// [`tonk_schema::rule::Rule`] mutation.

/// Parse a `when:` or `unless:` value as a list of premises. Each
/// list item must be a `{assert: <concept>, where: { … }}` mapping.
fn parse_premise_list(value: &MarkedYaml<'_>, out: &mut Vec<Diagnostic>) -> Vec<Premise> {
    let YamlData::Sequence(items) = &value.data else {
        out.push(error(
            range_of(value),
            "`when:` / `unless:` must be a list (`-` items) of premises.",
        ));
        return Vec::new();
    };
    let mut premises = Vec::with_capacity(items.len());
    for item in items {
        if let Some(premise) = parse_premise(item, out) {
            premises.push(premise);
        }
    }
    premises
}

/// Parse one premise: `{ assert: <concept>, where: { … } }`.
fn parse_premise(item: &MarkedYaml<'_>, out: &mut Vec<Diagnostic>) -> Option<Premise> {
    let Some(pairs) = mapping_of(item) else {
        out.push(error(
            range_of(item),
            r#"Premise must be a mapping with `assert:` and `where:` fields."#,
        ));
        return None;
    };

    let mut concept: Option<Spanned<String>> = None;
    let mut bindings: Vec<Field> = Vec::new();

    for (k, v) in pairs {
        let Some(key) = string_of(k) else {
            out.push(error(
                range_of(k),
                "Premise key must be a string (`assert:` or `where:`).",
            ));
            continue;
        };
        let key_range = range_of(k);
        match key {
            "assert" => {
                if concept.is_some() {
                    out.push(error(
                        key_range,
                        r#"Premise already declared `assert:`. Each premise names one concept."#,
                    ));
                    continue;
                }
                let Some(name) = string_of(v).map(str::to_owned) else {
                    out.push(error(
                        range_of(v),
                        "Premise `assert:` value must be a concept name.",
                    ));
                    continue;
                };
                concept = Some(Spanned::new(name, range_of(v)));
            }
            "where" => {
                if !bindings.is_empty() {
                    out.push(error(
                        key_range,
                        r#"Premise already declared `where:`. Combine bindings into one mapping."#,
                    ));
                    continue;
                }
                let Some(field_pairs) = mapping_of(v) else {
                    out.push(error(
                        range_of(v),
                        r#"Premise `where:` must be a mapping of `field: value`."#,
                    ));
                    continue;
                };
                // `where:` bindings are plain Field values — no rule
                // body recursion (no nested `when:`/`unless:` inside a
                // premise body).
                for (field_key, field_value) in field_pairs {
                    if let Some(field) = walk_field(field_key, field_value, false, out) {
                        bindings.push(field);
                    }
                }
            }
            other => {
                out.push(error(
                    key_range,
                    format!(r#"Unknown premise key `{other}`. Valid keys: `assert:`, `where:`."#),
                ));
            }
        }
    }

    let concept = concept.or_else(|| {
        out.push(error(
            range_of(item),
            "Premise must declare `assert: <concept-name>`.",
        ));
        None
    })?;

    Some(Premise {
        concept,
        bindings,
        range: range_of(item),
    })
}

/// Extract the mapping pairs from a [`MarkedYaml`], or `None`
/// if it isn't a mapping. Returns the underlying
/// `LinkedHashMap` (via the saphyr-exposed
/// `AnnotatedMapping<'_, MarkedYaml<'_>>` alias) so callers can
/// iterate `for (k, v) in pairs` — order matches source.
fn mapping_of<'a, 'b>(
    value: &'a MarkedYaml<'b>,
) -> Option<&'a saphyr::AnnotatedMapping<'b, MarkedYaml<'b>>> {
    if let YamlData::Mapping(pairs) = &value.data {
        Some(pairs)
    } else {
        None
    }
}

/// Parse the head text — `name` or `name!`.
///
/// The new grammar forbids inline bindings on the head; everything
/// about *which* entity an expression operates on lives in the
/// body via `this:` (or, for assertions, via a `&anchor` between
/// the colon and the body).
/// Parse the head text into a [`Predicate`] plus the trailing `!`
/// marker. The caller decides what to do with the marker — wrapping
/// the resulting [`Application`] in [`Effectful`] for claims, or
/// rejecting unexpected `!`s for queries.
fn parse_head(
    text: &str,
    key_range: Range,
    out: &mut Vec<Diagnostic>,
) -> Option<(Predicate, bool)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        out.push(error(key_range, "Head must not be empty."));
        return None;
    }

    // Detect old inline-binding shapes (`person ?p`, `person! alice`,
    // `person did:key:zX`) and emit a guiding diagnostic.
    if trimmed.contains(char::is_whitespace) {
        out.push(error(
            key_range,
            r#"Heads carry only `name[!]`. Move the entity binding into the body's `this:` field (e.g. `person:\
  this: ?p`) or, for assertions, use a `&anchor` between the colon and the body (e.g. `person!: &alice`)."#,
        ));
        return None;
    }

    let (name_str, effect) = if let Some(stripped) = trimmed.strip_suffix('!') {
        (stripped, true)
    } else {
        (trimmed, false)
    };

    if name_str.is_empty() {
        out.push(error(
            key_range,
            "Head name must not be empty (got `!` with no name).",
        ));
        return None;
    }

    let name = classify_head_name(name_str);

    Some((
        Predicate {
            name,
            range: key_range,
            source: name_str.to_owned(),
        },
        effect,
    ))
}

/// Decide whether a head name is a concept, a claim domain, or a
/// URI.
///
/// - Contains `:` (`db:concept`, `id:foo`, `did:key:…`) — a URI.
/// - An attribute identifier `<dotted-domain>/<attr>`
///   (`xyz.tonk.person/name`) — read as a URI head (it names a
///   specific attribute), since the part before the first `/` is a
///   reverse-dotted domain.
/// - Reverse-dotted with no `/` (`xyz.tonk`, `io.gozala.person`) —
///   a claim domain.
/// - Anything else, including a name with a `/` whose left side has
///   no dots (`demo/stuff`) — a concept. A `/` alone does not make
///   a name a URI; only a `:` (or a dotted domain before the `/`)
///   does, so concept names may contain `/`.
fn classify_head_name(name: &str) -> HeadName {
    if name.contains(':') || is_attribute_identifier(name) {
        HeadName::Uri(name.to_owned())
    } else if name.contains('.') {
        HeadName::Claim(name.to_owned())
    } else {
        HeadName::Concept(name.to_owned())
    }
}

/// `true` when `name` is an attribute identifier of the form
/// `<dotted-domain>/<attr>` — the segment before the first `/`
/// contains a `.` (e.g. `xyz.tonk.person/name`). A `/` whose left
/// side has no dots (`demo/stuff`) is a concept name, not an
/// attribute identifier.
fn is_attribute_identifier(name: &str) -> bool {
    name.split_once('/')
        .is_some_and(|(domain, _)| domain.contains('.'))
}

/// Recover a head's `&anchor` name by scanning the source gap
/// between the head key's end (`after_key`) and the value node's
/// start (`before_value`).
///
/// saphyr-parser registers anchors as numeric IDs and doesn't
/// expose the literal name on events, so we recover the name from
/// the source ourselves. **Call this from the loader with the raw
/// event spans**, never with the spans of a loaded `MarkedYaml`
/// node: saphyr re-marks `MarkedYaml` spans unreliably.
///
/// `Marker::index()` is a **char** index, not a byte offset (the
/// accessor's doc says "bytes" but the scanner increments it per
/// char — see saphyr's `scanner.rs`). We slice `source` by bytes,
/// so the markers must be converted first; skipping that made every
/// offset after multi-byte content run short, mis-locating or
/// dropping the anchor. (`anchor_id_of` is the reliable yes/no
/// signal for whether an anchor is present at all.)
fn scan_anchor(source: &str, after_key: Marker, before_value: Marker) -> Option<Anchor> {
    let start = char_index_to_byte_offset(source, after_key.index());
    let end = char_index_to_byte_offset(source, before_value.index());
    if end <= start {
        return None;
    }
    // Locate the `&` within the key→value gap. The gap is only used
    // to *find* the anchor token; the name itself is scanned from
    // the `&` forward through the full source, because saphyr's
    // value marker can land a few bytes inside the anchor name for
    // anchored block-mapping values (the value node "starts" partway
    // through the `&name` token). Bounding the name by `end` there
    // would truncate it; scanning the source to the first
    // non-anchor char recovers the whole name.
    let amp_pos = source[start..end].find('&')?;
    let amp_abs = start + amp_pos;
    let after_amp = &source[amp_abs + 1..];
    let name_len = after_amp
        .find(|c: char| !is_anchor_char(c))
        .unwrap_or(after_amp.len());
    if name_len == 0 {
        return None;
    }
    let name = &after_amp[..name_len];

    // Translate byte offsets back to LSP positions. We start from
    // the `&`'s line/column on the source — counting newlines from
    // the source start to the absolute offset.
    let amp_pos_lsp = byte_offset_to_position(source, amp_abs);
    let end_pos_lsp = byte_offset_to_position(source, amp_abs + 1 + name_len);
    Some(Anchor {
        name: name.to_owned(),
        range: Range {
            start: amp_pos_lsp,
            end: end_pos_lsp,
        },
    })
}

/// YAML anchor character predicate. Per the YAML 1.2 spec an
/// anchor name is `ns-char` minus the flow indicators (`,[]{}`),
/// so `/` is allowed — saphyr's scanner accepts it, and our
/// source-side name recovery must too (otherwise `&demo/stuff` is
/// truncated to `demo`, dropping the rest of the name). We accept
/// the practical subset used by concept names: alphanumerics and
/// `-`, `_`, `+`, `.`, `/`.
fn is_anchor_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '.' | '/')
}

/// Convert a saphyr `Marker::index()` (a **char** count) into a
/// byte offset into `source`, so it can be used to slice the
/// (byte-indexed) source string. An index past the end clamps to
/// `source.len()`.
fn char_index_to_byte_offset(source: &str, char_index: usize) -> usize {
    source
        .char_indices()
        .nth(char_index)
        .map(|(byte, _)| byte)
        .unwrap_or(source.len())
}

/// Convert an absolute byte offset into the source into an LSP
/// position. Linear scan; called at most once per anchor, so cost
/// is fine.
fn byte_offset_to_position(source: &str, offset: usize) -> Position {
    let clamped = offset.min(source.len());
    let prefix = &source[..clamped];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
    let line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let column = source[line_start..clamped].chars().count() as u32;
    Position {
        line,
        character: column,
    }
}

/// Walk one body field. `rule_body` flags whether the surrounding
/// claim is over the `rule` predicate, in which case `when:` and
/// `unless:` field values are parsed as premise lists rather than
/// rejected as generic sequences.
fn walk_field(
    key: &MarkedYaml<'_>,
    value: &MarkedYaml<'_>,
    rule_body: bool,
    out: &mut Vec<Diagnostic>,
) -> Option<Field> {
    let Some(name) = field_name_of(key) else {
        out.push(error(
            range_of(key),
            "Field name must be a string, or a bracketed key kind like `[position]`.",
        ));
        return None;
    };
    let value_range = range_of(value);
    let field_value = if rule_body && (name == "when" || name == "unless") {
        Some(FieldValue::Premises(parse_premise_list(value, out)))
    } else {
        walk_field_value(value, rule_body, out)
    }?;
    Some(Field {
        name,
        name_range: range_of(key),
        value: field_value,
        value_range,
    })
}

/// Walk a field value. `rule_body` propagates so that nested
/// `when:` / `unless:` inside a `rule!:` body (uncommon but
/// possible if a user nests rule mappings) reaches premise parsing.
fn walk_field_value(
    value: &MarkedYaml<'_>,
    rule_body: bool,
    out: &mut Vec<Diagnostic>,
) -> Option<FieldValue> {
    match &value.data {
        YamlData::Value(SaphyrScalar::String(s)) => {
            // Saphyr's `Value::String` covers both quoted strings
            // and plain scalars that look stringy. We can't tell
            // them apart from `YamlData` alone — but the
            // `Representation` arm below handles plain scalars,
            // and string values reaching `Value::String` are
            // either quoted or contain non-symbol characters
            // (uppercase, spaces, punctuation), which means they
            // are unambiguously string literals.
            Some(FieldValue::Literal(Scalar::String(s.as_ref().to_owned())))
        }
        YamlData::Representation(text, style, _) => {
            // A plain (unquoted) scalar can be a symbol, a
            // variable, a URI, or a literal that the YAML core
            // schema didn't promote to a typed value (e.g. all
            // letters). Quoted and block-style scalars are
            // unambiguously string literals — `"x"`, `'x'`,
            // `|`-literal, `>`-folded all signal "this is text,"
            // never a symbol or URI. Plain scalars run through
            // the classifier.
            match style {
                ScalarStyle::DoubleQuoted
                | ScalarStyle::SingleQuoted
                | ScalarStyle::Literal
                | ScalarStyle::Folded => Some(FieldValue::Literal(Scalar::String(
                    text.as_ref().to_owned(),
                ))),
                _ => Some(classify_plain_value(text.as_ref())),
            }
        }
        YamlData::Value(scalar) => Some(FieldValue::Literal(scalar_from_saphyr(scalar))),
        YamlData::Mapping(map) => {
            let mut nested = Vec::new();
            for (k, v) in map {
                if let Some(field) = walk_field(k, v, rule_body, out) {
                    nested.push(field);
                }
            }
            Some(FieldValue::Nested(nested))
        }
        YamlData::Sequence(_) => {
            out.push(error(
                range_of(value),
                r#"Sequence values are not supported in this notation. Use repeated assertions for cardinality-many writes."#,
            ));
            None
        }
        YamlData::Tagged(_, inner) => walk_field_value(inner, rule_body, out),
        YamlData::Alias(_) => {
            out.push(error(
                range_of(value),
                r#"YAML aliases (`*name`) are not supported in this notation. Use a `&anchor` plus the bare symbol name (`person-name`) to reference an in-document entity, or a `did:key:…` / `id:…` URI."#,
            ));
            None
        }
        YamlData::BadValue => {
            out.push(error(range_of(value), "Unsupported YAML node here."));
            None
        }
    }
}

/// Classify a plain (unquoted) scalar into the appropriate
/// [`FieldValue`].
///
/// - `_` → [`FieldValue::Blank`]
/// - `?name` → [`FieldValue::Variable`]
/// - URI shapes (`<scheme>:…` or `<dotted-domain>/name`) →
///   [`FieldValue::Uri`]
/// - bare lowercase identifier (`title`) or a `/`-qualified one
///   (`issue/title`, `space/route/view`) → [`FieldValue::Symbol`]
/// - everything else (uppercase, mixed case) → string literal
///
/// References require an explicit shape (no leading sigil for
/// symbols — bare lowercase is itself the marker). Quotes
/// distinguish string literals that would otherwise look like
/// symbols — `text/html` reads as a qualified symbol, so the MIME
/// literal must be written `"text/html"`.
pub fn classify_plain_value(text: &str) -> FieldValue {
    if text == "_" {
        return FieldValue::Blank;
    }
    if let Some(rest) = text.strip_prefix('?') {
        return FieldValue::Variable(rest.to_owned());
    }
    if looks_like_uri(text) {
        return FieldValue::Uri(text.to_owned());
    }
    // YAML core-schema typed values (numbers, booleans, null)
    // bypass the symbol/literal classification: a plain `28` is
    // an integer, not a symbol, even though `28` would not
    // satisfy `is_symbol` either.
    if let Some(scalar) = parse_typed_scalar(text) {
        return FieldValue::Literal(scalar);
    }
    if is_symbol(text) || is_qualified_symbol(text) {
        return FieldValue::Symbol(text.to_owned());
    }
    FieldValue::Literal(Scalar::String(text.to_owned()))
}

/// Does `text` look like a notation URI?
///
/// Two accepted shapes:
///
/// 1. `<scheme>:<rest>` — `did:key:…`, `id:foo`, `db:concept`,
///    `tonk-buffer:///x`. The scheme is a bare lowercase
///    identifier (letters / digits / `-` / `+`, starting with a
///    letter); everything before the first `:` must look like a
///    scheme so common values like `text/html` (no `:` at all)
///    or `12:34` (digit-leading) don't get hijacked.
///
/// 2. `<reverse-dotted-domain>/<name>` — attribute identifiers
///    (`xyz.tonk/foo`, `io.gozala.person/name`). Requires at
///    least one `.` *before* the first `/` so MIME-type-shaped
///    values (`text/html`, `application/json`) classify as
///    string literals instead.
///
/// Anything else — `text/html`, `Hello, World!`, `28` —
/// falls through to the symbol / typed-scalar / literal-string
/// rules.
fn looks_like_uri(text: &str) -> bool {
    if let Some((scheme, _)) = text.split_once(':')
        && is_uri_scheme(scheme)
    {
        return true;
    }
    if let Some((domain, rest)) = text.split_once('/')
        && !rest.is_empty()
        && is_reverse_dotted_domain(domain)
    {
        return true;
    }
    false
}

/// Bare lowercase identifier — letters / digits / `-` / `+`,
/// starting with a letter. Mirrors the URI-scheme shape from
/// RFC 3986 minus `.` (we use `.` to mean "domain segment
/// separator" in the reverse-dotted form).
fn is_uri_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '+')
}

/// Reverse-dotted domain — at least two segments separated by
/// `.`, each segment a bare lowercase identifier. Used to
/// recognize attribute-identifier domains like `xyz.tonk` or
/// `io.gozala.person`.
fn is_reverse_dotted_domain(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    parts.iter().all(|p| is_uri_scheme(p))
}

/// Try to parse `text` as a YAML core-schema typed scalar
/// (integer, float, bool, null). Returns `None` if the text
/// doesn't match any of those forms.
fn parse_typed_scalar(text: &str) -> Option<Scalar> {
    match text {
        "null" | "Null" | "NULL" | "~" => return Some(Scalar::Null),
        "true" | "True" | "TRUE" => return Some(Scalar::Boolean(true)),
        "false" | "False" | "FALSE" => return Some(Scalar::Boolean(false)),
        _ => {}
    }
    // Integer spelling picks the type: a leading sign (`+41`, `-7`)
    // is a signed integer, bare digits (`41`) are unsigned, and a
    // decimal point (`41.0`) is a float.
    if text.starts_with('+') || text.starts_with('-') {
        if let Ok(i) = text.parse::<i128>() {
            return Some(Scalar::Integer(i));
        }
    } else if let Ok(u) = text.parse::<u128>() {
        return Some(Scalar::UnsignedInteger(u));
    }
    if let Ok(f) = text.parse::<f64>() {
        return Some(Scalar::Float(f));
    }
    None
}

/// Recognise text that saphyr-parser produces for an empty or
/// explicit-null scalar value. With `early_parse(false)` the
/// `Yaml::value_from_cow_and_metadata` collapse never runs, so the
/// raw text reaches us — `""` for an implicit empty scalar (`key:`
/// with nothing after it), `"~"` / `"null"` / `"Null"` / `"NULL"`
/// for the explicit forms.
fn is_null_text(text: &str) -> bool {
    matches!(text, "" | "~" | "null" | "Null" | "NULL")
}

/// Symbol charset: starts with a-z, continues with a-z 0-9 - . +.
/// Mirrors the guide's *Symbols and strings* section.
fn is_symbol(text: &str) -> bool {
    let mut chars = text.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '+'))
}

/// Qualified symbol: one or more `/`-joined [`is_symbol`] segments
/// (`issue/title`, `space/route/view`). A name lookup against the
/// anchor table, same as a bare symbol — the slash is a namespace
/// separator, not a URI marker. Mirrors the anchor charset
/// ([`is_anchor_char`] allows `/`) so any anchor name can be
/// referenced back.
///
/// A `/` whose left side contains a `.` (`io.gozala.issue/title`)
/// is a URI, not a qualified symbol — [`looks_like_uri`] claims
/// those first, matching the head-name convention in
/// [`is_attribute_identifier`].
fn is_qualified_symbol(text: &str) -> bool {
    match text.split_once('/') {
        Some((first, rest)) => is_symbol(first) && rest.split('/').all(is_symbol),
        None => false,
    }
}

fn is_blank_scalar(value: &MarkedYaml<'_>) -> bool {
    matches!(string_of(value), Some("_"))
}

fn scalar_from_saphyr(scalar: &SaphyrScalar<'_>) -> Scalar {
    match scalar {
        SaphyrScalar::Null => Scalar::Null,
        SaphyrScalar::Boolean(b) => Scalar::Boolean(*b),
        // The sign spelling is lost on this path (saphyr already
        // typed the scalar), so the value decides: non-negative reads
        // as unsigned, matching the bare spelling it must have had.
        SaphyrScalar::Integer(i) if *i >= 0 => Scalar::UnsignedInteger(*i as u128),
        SaphyrScalar::Integer(i) => Scalar::Integer(i128::from(*i)),
        SaphyrScalar::FloatingPoint(f) => Scalar::Float(**f),
        SaphyrScalar::String(s) => Scalar::String(s.as_ref().to_owned()),
    }
}

// -----------------------------------------------------------------
// Saphyr helpers
// -----------------------------------------------------------------

fn diagnostic_for_scan_error(err: &ScanError) -> Diagnostic {
    let marker = err.marker();
    let line = (marker.line() as u32).saturating_sub(1);
    let column = marker.col() as u32;
    let start = Position {
        line,
        character: column,
    };
    let end = Position {
        line,
        character: column + 1,
    };
    Diagnostic {
        range: Range { start, end },
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("yaml".into()),
        message: err.info().to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

pub(crate) fn position_at(marker: &saphyr::Marker) -> Position {
    Position {
        line: (marker.line() as u32).saturating_sub(1),
        character: marker.col() as u32,
    }
}

pub(crate) fn range_of(node: &MarkedYaml<'_>) -> Range {
    range_from_span(node.span)
}

pub(crate) fn range_from_span(span: Span) -> Range {
    let start = position_at(&span.start);
    let mut end = position_at(&span.end);
    if start.line == end.line && start.character == end.character {
        end.character = end.character.saturating_add(1);
    }
    Range { start, end }
}

/// Clamp `range` so neither endpoint points past the last line
/// of `source`. Saphyr happily reports document-level spans that
/// end at `(line_count, 0)` — one past the final line — which is
/// a valid LSP `Position` per the spec but trips up clients
/// (notably `@codemirror/lsp-client`) that index into a `Text`
/// before they look at the version.
///
/// Conservative: when the end is past the last line we back it
/// up to the end of the last real line. When start is past the
/// last line (degenerate case) it gets the same treatment.
pub(crate) fn clamp_range(range: Range, source: &str) -> Range {
    // Same convention as `line_count`: split on `\n`, count
    // segments. `"a"` → 1 line, `"a\n"` → 2 lines (the trailing
    // empty line counts).
    let lines: Vec<&str> = source.split('\n').collect();
    let last_line = lines.len().saturating_sub(1) as u32;
    let clamp = |p: Position| -> Position {
        if (p.line as usize) < lines.len() {
            return p;
        }
        Position {
            line: last_line,
            character: lines.last().map(|l| l.len() as u32).unwrap_or(0),
        }
    };
    Range {
        start: clamp(range.start),
        end: clamp(range.end),
    }
}

fn extend_range(start: Range, end: Range) -> Range {
    Range {
        start: start.start,
        end: end.end,
    }
}

/// A field's name: a string key, or a one-element flow sequence
/// holding a string — `[position]` — which names a key *kind*
/// rather than a field. YAML reads `{[position]: entity}` as a
/// sequence-valued key; the analyzer reads the bracketed name as a
/// keyed-collection declaration.
fn field_name_of(node: &MarkedYaml<'_>) -> Option<String> {
    if let Some(name) = string_of(node) {
        return Some(name.to_owned());
    }
    if let YamlData::Sequence(items) = &node.data
        && let [item] = items.as_slice()
        && let Some(kind) = string_of(item)
    {
        return Some(format!("[{kind}]"));
    }
    None
}

fn string_of<'a>(node: &'a MarkedYaml<'_>) -> Option<&'a str> {
    match &node.data {
        YamlData::Value(SaphyrScalar::String(s)) => Some(s.as_ref()),
        YamlData::Representation(text, _, _) => Some(text.as_ref()),
        _ => None,
    }
}

// -----------------------------------------------------------------
// Diagnostic helpers
// -----------------------------------------------------------------

fn error(range: Range, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("asserted-notation".into()),
        message: message.into(),
        related_information: None,
        tags: None,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn parse_clean(text: &str) -> Syntax {
        let parsed = parse(text);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected diagnostics: {:#?}",
            parsed.diagnostics
        );
        parsed.syntax.expect("syntax should be Some on clean parse")
    }

    /// `{[position]: entity}` is a mapping whose key is a one-element
    /// flow sequence; it parses as a field named `[position]`, the
    /// keyed-collection declaration the analyzer reads.
    #[dialog_common::test]
    fn it_parses_a_bracketed_key_kind() {
        let syntax = parse_clean(
            "concept!: &x\n  with:\n    block:\n      the: xyz.test\n      as: {[position]: entity}\n",
        );
        let text = format!("{syntax:?}");
        assert!(
            text.contains("\"[position]\""),
            "the bracketed key survives as a field name: {text}"
        );
    }

    #[dialog_common::test]
    fn it_returns_clean_on_empty_input() {
        let parsed = parse("");
        assert!(parsed.diagnostics.is_empty());
        assert!(parsed.syntax.is_none());
    }

    #[dialog_common::test]
    fn it_surfaces_yaml_parse_errors() {
        let parsed = parse("a:\n  b: 1\n c: 2\n");
        assert!(parsed.syntax.is_none());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].source.as_deref(), Some("yaml"));
    }

    #[dialog_common::test]
    fn it_rejects_scalar_root() {
        let parsed = parse("just a string\n");
        assert_eq!(parsed.diagnostics.len(), 1);
        assert!(
            parsed.diagnostics[0]
                .message
                .contains("mapping at the document root")
        );
    }

    #[dialog_common::test]
    fn it_parses_concept_query_with_no_body() {
        let syntax = parse_clean("person:\n");
        assert_eq!(syntax.expressions.len(), 1);
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query, got {:?}", syntax.expressions[0]);
        };
        assert!(matches!(&q.predicate.name, HeadName::Concept(n) if n == "person"));
        // (no-effect implied by Expression::Query variant)
        assert!(q.fields.is_empty());
    }

    #[dialog_common::test]
    fn it_parses_query_with_this_variable_and_literal() {
        let syntax = parse_clean(
            r#"
person:
  this: ?alice
  name: "Alice"
  age: ?age
"#,
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        assert!(matches!(&q.predicate.name, HeadName::Concept(n) if n == "person"));
        assert_eq!(q.fields.len(), 3);
        assert_eq!(q.fields[0].name, "this");
        assert!(matches!(&q.fields[0].value, FieldValue::Variable(v) if v == "alice"));
        assert!(matches!(
            &q.fields[1].value,
            FieldValue::Literal(Scalar::String(s)) if s == "Alice"
        ));
        assert!(matches!(&q.fields[2].value, FieldValue::Variable(v) if v == "age"));
    }

    #[dialog_common::test]
    fn it_classifies_dotted_head_as_claim() {
        let syntax = parse_clean(
            r#"
xyz.tonk:
  this: ?p
  name: "Alice"
"#,
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        assert!(matches!(&q.predicate.name, HeadName::Claim(n) if n == "xyz.tonk"));
    }

    #[dialog_common::test]
    fn it_classifies_uri_head() {
        let syntax = parse_clean(
            r#"
db:concept!:
  description: "x"
  with:
    foo: bar
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(&a.predicate.name, HeadName::Uri(u) if u == "db:concept"));
    }

    #[dialog_common::test]
    fn it_parses_assertion_without_anchor() {
        let syntax = parse_clean(
            r#"
person!:
  name: "Nick"
  address: "Portland, OR"
"#,
        );
        let Expression::Claim(Effectful { anchor, inner: a }) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert!(matches!(&a.predicate.name, HeadName::Concept(n) if n == "person"));
        assert!(anchor.is_none());
    }

    #[dialog_common::test]
    fn it_captures_anchor_on_assertion() {
        let syntax = parse_clean(
            r#"
person!: &alice
  name: "Alice"
  age: 28
"#,
        );
        let Expression::Claim(Effectful { anchor, inner: _a }) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        let anchor = anchor.as_ref().expect("anchor present");
        assert_eq!(anchor.name, "alice");
    }

    #[dialog_common::test]
    fn it_captures_anchor_with_dashes() {
        let syntax = parse_clean(
            r#"
attribute!: &person-name
  description: "name"
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one
"#,
        );
        let Expression::Claim(Effectful { anchor, inner: _a }) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert_eq!(anchor.as_ref().unwrap().name, "person-name");
    }

    #[dialog_common::test]
    fn it_captures_anchor_with_slash() {
        // The YAML spec allows `/` in anchor names (it's `ns-char`
        // minus flow indicators). A concept named `demo/stuff` is
        // anchored as `&demo/stuff`; the whole name must be
        // recovered, not truncated at the `/`.
        let syntax = parse_clean(
            r#"
demo/stuff!: &demo/stuff
  stuff: 1
"#,
        );
        let Expression::Claim(Effectful { anchor, inner: _ }) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert_eq!(anchor.as_ref().expect("anchor present").name, "demo/stuff");
    }

    #[dialog_common::test]
    fn it_captures_anchor_after_multibyte_content() {
        // saphyr's `Marker::index()` returns a *char* index despite
        // its doc claiming bytes. A multi-byte char (here `é`) before
        // a later anchored head makes the char index run behind the
        // byte offset; slicing source by the raw index then mis-locates
        // the `&` and silently drops the anchor. The anchor must still
        // be recovered after multi-byte content earlier in the doc.
        let syntax = parse_clean(
            r#"
first!: &one
  label: "café"

second!: &two
  label: "plain"
"#,
        );
        let Expression::Claim(Effectful { anchor, inner: _ }) = &syntax.expressions[0] else {
            panic!("expected first assertion");
        };
        assert_eq!(anchor.as_ref().expect("first anchor").name, "one");
        let Expression::Claim(Effectful { anchor, inner: _ }) = &syntax.expressions[1] else {
            panic!("expected second assertion");
        };
        assert_eq!(
            anchor
                .as_ref()
                .expect("second anchor present after multi-byte content")
                .name,
            "two",
        );
    }

    #[dialog_common::test]
    fn it_rejects_anchor_on_query() {
        let parsed = parse(
            r#"
person: &alice
  name: "Alice"
"#,
        );
        assert!(!parsed.diagnostics.is_empty());
        assert!(parsed.diagnostics[0].message.contains("anchor"));
    }

    #[dialog_common::test]
    fn it_rejects_bare_blank_body_on_assertion() {
        let parsed = parse("person!: _\n");
        assert!(!parsed.diagnostics.is_empty());
        assert!(
            parsed.diagnostics[0]
                .message
                .to_lowercase()
                .contains("bare `_` body")
        );
    }

    #[dialog_common::test]
    fn it_parses_field_blank_for_field_retraction() {
        // `field: _` inside an assertion body means "retract this
        // field's attribute for the entity selected by `this:`".
        let syntax = parse_clean(
            r#"
person!:
  this: ?alice
  age: _
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert_eq!(a.fields.len(), 2);
        let age = a.fields.iter().find(|f| f.name == "age").unwrap();
        assert!(matches!(age.value, FieldValue::Blank));
    }

    #[dialog_common::test]
    fn it_parses_dotdot_rest_marker_with_other_fields() {
        // `..: _` retracts every attribute in the concept's
        // `with:` map that isn't named explicitly elsewhere.
        let syntax = parse_clean(
            r#"
person!:
  this: ?alice
  name: ?name
  ..: _
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let dotdot = a.fields.iter().find(|f| f.name == "..").unwrap();
        assert!(matches!(dotdot.value, FieldValue::Blank));
    }

    #[dialog_common::test]
    fn it_rejects_query_body_with_bare_blank() {
        let parsed = parse("person: _\n");
        assert!(!parsed.diagnostics.is_empty());
    }

    #[dialog_common::test]
    fn it_rejects_inline_head_binding() {
        // Old grammar: `person ?alice:` — now belongs in body.
        let parsed = parse(
            r#"
person ?alice:
  name: "Alice"
"#,
        );
        assert!(!parsed.diagnostics.is_empty());
        assert!(parsed.diagnostics[0].message.contains("this:"));
    }

    #[dialog_common::test]
    fn it_rejects_inline_bookmark_binding() {
        let parsed = parse(
            r#"
person! alice:
  name: "Alice"
"#,
        );
        assert!(!parsed.diagnostics.is_empty());
    }

    #[dialog_common::test]
    fn it_preserves_nested_map_with_symbol_references() {
        let syntax = parse_clean(
            r#"
concept!: &person
  description: "A person"
  with:
    name: person-name
    age:  person-age
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let with_field = a.fields.iter().find(|f| f.name == "with").unwrap();
        let FieldValue::Nested(inner) = &with_field.value else {
            panic!("expected nested map");
        };
        assert_eq!(inner.len(), 2);
        assert!(matches!(
            &inner[0].value,
            FieldValue::Symbol(s) if s == "person-name"
        ));
        assert!(matches!(
            &inner[1].value,
            FieldValue::Symbol(s) if s == "person-age"
        ));
    }

    #[dialog_common::test]
    fn it_joins_across_expressions_via_this() {
        let syntax = parse_clean(
            r#"
person:
  this: ?alice
  name: "Alice"
xyz.tonk:
  this: ?alice
  role: ?role
"#,
        );
        assert_eq!(syntax.expressions.len(), 2);
        let Expression::Query(q1) = &syntax.expressions[0] else {
            panic!("expected Query 1");
        };
        let Expression::Query(q2) = &syntax.expressions[1] else {
            panic!("expected Query 2");
        };
        assert!(matches!(&q1.predicate.name, HeadName::Concept(n) if n == "person"));
        assert!(matches!(&q2.predicate.name, HeadName::Claim(n) if n == "xyz.tonk"));
    }

    #[dialog_common::test]
    fn it_treats_quoted_value_as_literal_string() {
        let syntax = parse_clean(
            r#"
person!:
  address: "Portland, OR"
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "Portland, OR"
        ));
    }

    #[dialog_common::test]
    fn it_treats_bare_lowercase_value_as_symbol() {
        // Bare lowercase is now a symbol (resolves through name
        // table). Quotes are required for a literal string.
        let syntax = parse_clean(
            r#"
concept!:
  with:
    name: person-name
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let FieldValue::Nested(inner) = &a.fields[0].value else {
            panic!("expected nested");
        };
        assert!(matches!(
            &inner[0].value,
            FieldValue::Symbol(s) if s == "person-name"
        ));
    }

    #[dialog_common::test]
    fn it_treats_uri_value_as_uri_reference() {
        let syntax = parse_clean(
            r#"
name!:
  this:   id:alice
  entity: did:key:zHjKf
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let this = a.fields.iter().find(|f| f.name == "this").unwrap();
        assert!(matches!(&this.value, FieldValue::Uri(u) if u == "id:alice"));
        let entity = a.fields.iter().find(|f| f.name == "entity").unwrap();
        assert!(matches!(&entity.value, FieldValue::Uri(u) if u == "did:key:zHjKf"));
    }

    #[dialog_common::test]
    fn it_accepts_attribute_uri_in_field_position() {
        let syntax = parse_clean(
            r#"
attribute!: &person-name
  the: xyz.tonk.person/name
  as: text
  cardinality: one
  description: "name"
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let the = a.fields.iter().find(|f| f.name == "the").unwrap();
        assert!(matches!(&the.value, FieldValue::Uri(u) if u == "xyz.tonk.person/name"));
    }

    #[dialog_common::test]
    fn it_rejects_empty_head_after_bang() {
        let parsed = parse("!:\n  x: 1\n");
        assert!(!parsed.diagnostics.is_empty());
    }

    /// Integer spelling picks the type: bare digits are unsigned, a
    /// leading sign is signed, a decimal point is a float.
    #[dialog_common::test]
    fn it_types_integers_by_their_spelling() {
        let syntax = parse_clean(
            r#"
person:
  this: ?p
  bare: 41
  plus: +41
  minus: -7
  real: 41.5
"#,
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        let get = |name: &str| {
            q.fields
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.value.clone())
                .unwrap_or_else(|| panic!("{name}"))
        };
        assert!(matches!(
            get("bare"),
            FieldValue::Literal(Scalar::UnsignedInteger(41))
        ));
        assert!(matches!(
            get("plus"),
            FieldValue::Literal(Scalar::Integer(41))
        ));
        assert!(matches!(
            get("minus"),
            FieldValue::Literal(Scalar::Integer(-7))
        ));
        assert!(matches!(get("real"), FieldValue::Literal(Scalar::Float(f)) if f == 41.5));
    }

    #[dialog_common::test]
    fn it_parses_integer_field_value() {
        let syntax = parse_clean(
            r#"
person:
  this: ?alice
  age: 28
"#,
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        let age = q.fields.iter().find(|f| f.name == "age").unwrap();
        assert!(matches!(
            &age.value,
            FieldValue::Literal(Scalar::UnsignedInteger(28))
        ));
    }

    #[dialog_common::test]
    fn it_preserves_duplicate_top_level_keys() {
        let syntax = parse_clean(
            r#"
person:
  this: ?alice
  name: "Alice"
person:
  this: ?alice
  age: ?age
"#,
        );
        assert_eq!(syntax.expressions.len(), 2);
        let Expression::Query(q1) = &syntax.expressions[0] else {
            panic!("expected first Query");
        };
        let Expression::Query(q2) = &syntax.expressions[1] else {
            panic!("expected second Query");
        };
        assert_eq!(q1.fields.len(), 2);
        assert_eq!(q2.fields.len(), 2);
    }

    #[dialog_common::test]
    fn it_collapses_duplicate_field_keys() {
        let syntax = parse_clean(
            r#"
person:
  this: ?alice
  age: 28
  age: 29
"#,
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        let age = q.fields.iter().find(|f| f.name == "age").unwrap();
        assert!(matches!(
            &age.value,
            FieldValue::Literal(Scalar::UnsignedInteger(29))
        ));
    }

    #[dialog_common::test]
    fn it_rejects_sequence_value() {
        let parsed = parse(
            r#"
person!:
  name:
    - Alice
    - Bob
"#,
        );
        assert!(!parsed.diagnostics.is_empty());
        assert!(
            parsed.diagnostics[0]
                .message
                .to_lowercase()
                .contains("sequence")
        );
    }

    #[dialog_common::test]
    fn it_rejects_yaml_alias() {
        let parsed = parse(
            r#"
person!: &alice
  name: "Alice"
other!:
  thing: *alice
"#,
        );
        assert!(!parsed.diagnostics.is_empty());
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.message.to_lowercase().contains("alias"))
        );
    }

    #[dialog_common::test]
    fn it_parses_this_mapping_for_explicit_content_derivation() {
        let syntax = parse_clean(
            r#"
person!:
  name: "Alice"
  age: 23
  this:
    entropy: "Maybe Not"
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let this = a.fields.iter().find(|f| f.name == "this").unwrap();
        let FieldValue::Nested(inner) = &this.value else {
            panic!("expected nested mapping under `this:`");
        };
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].name, "entropy");
    }

    // -------- field-value classification --------

    #[dialog_common::test]
    fn it_parses_id_uri_in_head_position() {
        let syntax = parse_clean(
            r#"
id:person!:
  description: "x"
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(&a.predicate.name, HeadName::Uri(u) if u == "id:person"));
    }

    #[dialog_common::test]
    fn it_parses_did_key_uri_in_head_position() {
        let syntax = parse_clean(
            r#"
did:key:zHjKf!:
  ..: _
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(&a.predicate.name, HeadName::Uri(u) if u == "did:key:zHjKf"));
    }

    #[dialog_common::test]
    fn it_parses_this_with_did_key_value() {
        let syntax = parse_clean(
            r#"
person!:
  this: did:key:zHjKf
  age: 30
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let this = a.fields.iter().find(|f| f.name == "this").unwrap();
        assert!(matches!(&this.value, FieldValue::Uri(u) if u == "did:key:zHjKf"));
    }

    #[dialog_common::test]
    fn it_parses_this_with_id_uri() {
        let syntax = parse_clean(
            r#"
name!:
  this: id:alice
  entity: did:key:zX
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let this = a.fields.iter().find(|f| f.name == "this").unwrap();
        assert!(matches!(&this.value, FieldValue::Uri(u) if u == "id:alice"));
    }

    #[dialog_common::test]
    fn it_parses_this_with_bare_symbol_for_name_lookup() {
        let syntax = parse_clean(
            r#"
person!:
  this: alice
  ..: _
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let this = a.fields.iter().find(|f| f.name == "this").unwrap();
        assert!(matches!(&this.value, FieldValue::Symbol(s) if s == "alice"));
    }

    #[dialog_common::test]
    fn it_parses_float_literal() {
        let syntax = parse_clean(
            r#"
thing!:
  weight: 1.5
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        match &a.fields[0].value {
            FieldValue::Literal(Scalar::Float(f)) => assert!((f - 1.5).abs() < f64::EPSILON),
            other => panic!("expected float, got {:?}", other),
        }
    }

    #[dialog_common::test]
    fn it_parses_boolean_literals() {
        let syntax = parse_clean(
            r#"
thing!:
  yes: true
  no: false
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let yes = a.fields.iter().find(|f| f.name == "yes").unwrap();
        let no = a.fields.iter().find(|f| f.name == "no").unwrap();
        assert!(matches!(
            yes.value,
            FieldValue::Literal(Scalar::Boolean(true))
        ));
        assert!(matches!(
            no.value,
            FieldValue::Literal(Scalar::Boolean(false))
        ));
    }

    #[dialog_common::test]
    fn it_parses_null_literal_value() {
        let syntax = parse_clean(
            r#"
thing!:
  nope: null
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        // Plain `null` in field-value position is a Null literal,
        // not a blank or symbol.
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Literal(Scalar::Null)
        ));
    }

    #[dialog_common::test]
    fn it_treats_symbol_like_quoted_string_as_literal() {
        // `"alice"` is a literal string even though `alice` would
        // be a Symbol — quotes are load-bearing.
        let syntax = parse_clean(
            r#"
thing!:
  name: "alice"
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "alice"
        ));
    }

    #[dialog_common::test]
    fn it_treats_single_quoted_string_as_literal() {
        let syntax = parse_clean(
            r#"
thing!:
  name: 'alice'
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "alice"
        ));
    }

    #[dialog_common::test]
    fn it_classifies_dotted_bare_symbol() {
        // Symbol charset includes `.` — `xyz.tonk.person` is a
        // valid symbol (and a claim domain in head position, but
        // here it's a field value).
        let syntax = parse_clean(
            r#"
thing!:
  ref: xyz.tonk.person
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Symbol(s) if s == "xyz.tonk.person"
        ));
    }

    #[dialog_common::test]
    fn it_classifies_unquoted_value_with_space_as_string_literal() {
        // A plain scalar with an internal space contains a
        // non-symbol character (` `), so it must classify as a
        // string literal, not a Symbol.
        let syntax = parse_clean(
            r#"
thing!:
  greeting: hello world
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "hello world"
        ));
    }

    #[dialog_common::test]
    fn it_classifies_unquoted_value_with_underscore_as_string_literal() {
        // `_` is not in the symbol charset (only `-` `.` `+` are
        // allowed). A bare `name_alt` therefore is not a Symbol;
        // it should fall through to a string literal.
        let syntax = parse_clean(
            r#"
thing!:
  ref: name_alt
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "name_alt"
        ));
    }

    #[dialog_common::test]
    fn it_quoted_symbol_charset_value_is_literal_not_symbol() {
        // Three quoted forms of values whose unquoted shapes would
        // each be a Symbol. Quotes force a string literal.
        let syntax = parse_clean(
            r#"
thing!:
  bare:    person-name
  double: "person-name"
  single: 'person-name'
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let bare = a.fields.iter().find(|f| f.name == "bare").unwrap();
        let double = a.fields.iter().find(|f| f.name == "double").unwrap();
        let single = a.fields.iter().find(|f| f.name == "single").unwrap();
        assert!(matches!(
            &bare.value,
            FieldValue::Symbol(s) if s == "person-name"
        ));
        assert!(matches!(
            &double.value,
            FieldValue::Literal(Scalar::String(s)) if s == "person-name"
        ));
        assert!(matches!(
            &single.value,
            FieldValue::Literal(Scalar::String(s)) if s == "person-name"
        ));
    }

    #[dialog_common::test]
    fn it_classifies_uppercase_unquoted_value_as_string_literal() {
        // Uppercase first letter doesn't match the symbol charset;
        // saphyr keeps it as a plain scalar, and we surface it as
        // a string literal even without quotes (the user had no
        // way to mean a symbol with a capital).
        let syntax = parse_clean(
            r#"
thing!:
  name: Alice
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "Alice"
        ));
    }

    #[dialog_common::test]
    fn it_parses_symbol_with_digits_and_plus() {
        let syntax = parse_clean(
            r#"
thing!:
  ref: a-b1.c+d
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Symbol(s) if s == "a-b1.c+d"
        ));
    }

    // -------- structural / multi-expression --------

    #[dialog_common::test]
    fn it_accepts_empty_body_assertion() {
        // `person!:` with no fields is syntactically valid (no-op
        // semantically; the analyzer may flag it).
        let syntax = parse_clean("person!:\n");
        let Expression::Claim(Effectful { anchor, inner: a }) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert!(a.fields.is_empty());
        assert!(anchor.is_none());
    }

    #[dialog_common::test]
    fn it_parses_mixed_query_then_assertion() {
        let syntax = parse_clean(
            r#"
person:
  this: ?alice
  name: "Alice"
person!:
  this: ?alice
  age: 30
"#,
        );
        assert_eq!(syntax.expressions.len(), 2);
        assert!(matches!(syntax.expressions[0], Expression::Query(_)));
        assert!(matches!(syntax.expressions[1], Expression::Claim(_)));
    }

    #[dialog_common::test]
    fn it_records_anchor_range_pointing_at_ampersand() {
        let syntax = parse_clean("person!: &alice\n  name: \"Alice\"\n");
        let Expression::Claim(Effectful { anchor, inner: _a }) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        let anchor = anchor.as_ref().unwrap();
        // Anchor occupies the `&alice` token starting at column 9
        // (after `person!: `) on line 0.
        assert_eq!(anchor.range.start.line, 0);
        assert_eq!(anchor.range.start.character, 9);
        assert_eq!(anchor.range.end.character, 9 + "&alice".len() as u32);
    }

    #[dialog_common::test]
    fn it_classifies_attribute_identifier_in_head_position() {
        // `xyz.tonk.person/name` is an attribute identifier
        // (dotted domain + `/` + attr) — it reads as a URI head,
        // not Claim or Concept.
        let syntax = parse_clean(
            r#"
xyz.tonk.person/name:
  this: ?x
"#,
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        assert!(matches!(&q.predicate.name, HeadName::Uri(u) if u == "xyz.tonk.person/name"));
    }

    #[dialog_common::test]
    fn it_classifies_slash_name_without_dotted_domain_as_concept() {
        // `demo/stuff` has a `/` but the part before it (`demo`) has
        // no dots, so it is not an attribute identifier — it is a
        // concept name that happens to contain `/`. Only a `:` (or a
        // dotted domain before the `/`) makes a head a URI.
        let syntax = parse_clean(
            r#"
demo/stuff!:
  stuff: 1
"#,
        );
        let Expression::Claim(Effectful { inner: a, .. }) = &syntax.expressions[0] else {
            panic!("expected Claim");
        };
        assert!(
            matches!(&a.predicate.name, HeadName::Concept(n) if n == "demo/stuff"),
            "expected Concept(\"demo/stuff\"), got {:?}",
            a.predicate.name,
        );
    }

    /// `|`-style block scalars are unambiguously string literals,
    /// even when their content contains `:` or `/` (e.g.
    /// embedded HTML). They must not run through the
    /// classifier and end up as `FieldValue::Uri`.
    #[dialog_common::test]
    fn it_parses_literal_block_scalar_as_string() {
        let syntax = parse_clean(
            r#"
page!:
  content: |
    <html>
      <body>
        <h1>Hi</h1>
      </body>
    </html>
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let content = a
            .fields
            .iter()
            .find(|f| f.name == "content")
            .expect("content field");
        match &content.value {
            FieldValue::Literal(Scalar::String(s)) => {
                assert!(s.contains("<html>"), "got: {s:?}");
                assert!(s.contains("<h1>Hi</h1>"), "got: {s:?}");
            }
            other => panic!("expected literal string, got {other:?}"),
        }
    }

    /// A `/`-joined bare identifier (`issue/title`,
    /// `space/route/view`) classifies as a qualified
    /// [`FieldValue::Symbol`] — a name lookup against the anchor
    /// table, symmetric with the slash-bearing anchor charset.
    #[dialog_common::test]
    fn it_parses_namespaced_reference_as_symbol() {
        let syntax = parse_clean(
            r#"
concept!:
  with:
    title: issue/title
    body:  space/route/view
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let with = a
            .fields
            .iter()
            .find(|f| f.name == "with")
            .expect("with field");
        let FieldValue::Nested(fields) = &with.value else {
            panic!("expected nested with map, got {:?}", with.value);
        };
        let title = fields.iter().find(|f| f.name == "title").expect("title");
        assert!(
            matches!(&title.value, FieldValue::Symbol(s) if s == "issue/title"),
            "got {:?}",
            title.value,
        );
        let body = fields.iter().find(|f| f.name == "body").expect("body");
        assert!(
            matches!(&body.value, FieldValue::Symbol(s) if s == "space/route/view"),
            "got {:?}",
            body.value,
        );
    }

    /// MIME-type-shaped plain scalars (`text/html`) now read as
    /// qualified symbols (no `.` before the `/`), so the literal
    /// MIME string must be quoted. The quoted form stays a string.
    #[dialog_common::test]
    fn it_parses_quoted_mime_type_as_string_literal() {
        let syntax = parse_clean(
            r#"
page!:
  type: "text/html"
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let ty = a
            .fields
            .iter()
            .find(|f| f.name == "type")
            .expect("type field");
        match &ty.value {
            FieldValue::Literal(Scalar::String(s)) => assert_eq!(s, "text/html"),
            other => panic!("expected literal string, got {other:?}"),
        }
    }

    /// Reverse-dotted-domain forms (`xyz.tonk/foo`) still
    /// classify as URIs — that's the claim-attribute shape we
    /// need to keep working.
    #[dialog_common::test]
    fn it_parses_claim_attribute_path_as_uri() {
        let syntax = parse_clean(
            r#"
attribute!:
  the: xyz.tonk.person/name
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let the = a
            .fields
            .iter()
            .find(|f| f.name == "the")
            .expect("the field");
        match &the.value {
            FieldValue::Uri(u) => assert_eq!(u, "xyz.tonk.person/name"),
            other => panic!("expected URI, got {other:?}"),
        }
    }

    /// Scheme-prefixed URIs (`did:key:…`, `id:foo`,
    /// `db:concept`) classify as URIs.
    #[dialog_common::test]
    fn it_parses_scheme_prefixed_values_as_uri() {
        let syntax = parse_clean(
            r#"
person!:
  this: did:key:zMkAlice
  ref: id:foo
  scheme: db:concept
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        for name in ["this", "ref", "scheme"] {
            let f = a
                .fields
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("expected field {name}"));
            assert!(
                matches!(&f.value, FieldValue::Uri(_)),
                "field {name} should be URI, got {:?}",
                f.value,
            );
        }
    }

    /// `>`-style folded block scalars are also string literals.
    #[dialog_common::test]
    fn it_parses_folded_block_scalar_as_string() {
        let syntax = parse_clean(
            r#"
page!:
  description: >
    a long
    multi-line
    description
"#,
        );
        let Expression::Claim(Effectful {
            anchor: _anchor,
            inner: a,
        }) = &syntax.expressions[0]
        else {
            panic!("expected Assertion");
        };
        let desc = a
            .fields
            .iter()
            .find(|f| f.name == "description")
            .expect("description field");
        assert!(matches!(
            &desc.value,
            FieldValue::Literal(Scalar::String(_))
        ));
    }

    /// Saphyr reports document-level spans that end at
    /// `(line_count, 0)` — one past the final line — when the
    /// source has no trailing newline. Without clamping, an LSP
    /// client decoding the diagnostic indexes a `line_count`
    /// row that doesn't exist and crashes
    /// (`@codemirror/lsp-client` throws `Invalid line number N
    /// in M-line document`). The fix clamps the parser's own
    /// emissions; this test pins the bare-string single-line
    /// case that surfaced it.
    #[dialog_common::test]
    fn it_clamps_document_root_diagnostic_to_one_line_input() {
        let parsed = parse("a");
        assert_eq!(parsed.diagnostics.len(), 1);
        let range = &parsed.diagnostics[0].range;
        assert!(
            range.end.line == 0,
            "diagnostic end must stay on the only line, got {range:?}",
        );
    }

    // -------------------------------------------------------------
    // Rule expressions
    // -------------------------------------------------------------

    /// `rule!:` parses as a [`Claim`] over the `rule` predicate with
    /// its body fields preserved. The parser doesn't validate the
    /// shape of those fields (one polarity, non-empty `when:`,
    /// etc.) — that lives in the analyzer, where it can produce
    /// diagnostics with semantic context.
    #[dialog_common::test]
    fn it_parses_rule_claim_as_concept_with_body_fields() {
        let syntax =
            parse_clean("rule!:\n  assert!: pong\n  when:\n    - assert: ping\n      where: {}\n");
        assert_eq!(syntax.expressions.len(), 1);
        let Expression::Claim(Effectful { anchor, inner: app }) = &syntax.expressions[0] else {
            panic!("expected Claim, got {:?}", syntax.expressions[0]);
        };
        assert!(anchor.is_none());
        assert!(
            matches!(&app.predicate.name, HeadName::Concept(n) if n == "rule"),
            "predicate should be the `rule` concept",
        );

        // assert!: field carries the head concept as a symbol/literal.
        let polarity = app
            .fields
            .iter()
            .find(|f| f.name == "assert!")
            .expect("assert!: field present");
        assert!(matches!(&polarity.value, FieldValue::Symbol(s) if s == "pong"));

        // when: field is a premise list — typed, not a generic nested map.
        let when = app
            .fields
            .iter()
            .find(|f| f.name == "when")
            .expect("when: field present");
        let FieldValue::Premises(premises) = &when.value else {
            panic!("when: should be FieldValue::Premises, got {:?}", when.value);
        };
        assert_eq!(premises.len(), 1);
        assert_eq!(premises[0].concept.value, "ping");
        assert!(premises[0].bindings.is_empty());
    }

    /// `retract!:` polarity field carries through the same way.
    /// Premise bindings inside `where:` keep their typed value
    /// shape (variables stay [`FieldValue::Variable`]).
    #[dialog_common::test]
    fn it_parses_retract_polarity_rule() {
        let syntax = parse_clean(
            r#"rule!:
  retract!: message
  when:
    - assert: ack
      where: { target: ?m }
    - assert: message
      where: { this: ?m, body: ?b }
"#,
        );
        let Expression::Claim(Effectful { inner: app, .. }) = &syntax.expressions[0] else {
            panic!("expected Claim");
        };
        let polarity = app.fields.iter().find(|f| f.name == "retract!").unwrap();
        assert!(matches!(&polarity.value, FieldValue::Symbol(s) if s == "message"));
        let FieldValue::Premises(premises) =
            &app.fields.iter().find(|f| f.name == "when").unwrap().value
        else {
            panic!("when value must be Premises");
        };
        assert_eq!(premises.len(), 2);
        assert_eq!(premises[0].concept.value, "ack");
        assert_eq!(premises[1].bindings.len(), 2);
    }

    /// `unless:` parses to a premise list too, and `description:`
    /// is a plain string-literal field.
    #[dialog_common::test]
    fn it_parses_unless_and_description() {
        let syntax = parse_clean(
            r#"rule!:
  description: "increment counter on increment command"
  assert!: counter
  when:
    - assert: counter
      where: { this: ?c, count: ?prev }
  unless:
    - assert: counter-paused
      where: { this: ?c }
"#,
        );
        let Expression::Claim(Effectful { inner: app, .. }) = &syntax.expressions[0] else {
            panic!("expected Claim");
        };
        let FieldValue::Premises(unless) = &app
            .fields
            .iter()
            .find(|f| f.name == "unless")
            .unwrap()
            .value
        else {
            panic!("unless value must be Premises");
        };
        assert_eq!(unless.len(), 1);
        assert_eq!(unless[0].concept.value, "counter-paused");
        let desc = app.fields.iter().find(|f| f.name == "description").unwrap();
        assert!(
            matches!(&desc.value, FieldValue::Literal(Scalar::String(s)) if s.contains("increment counter")),
            "description must be a string literal",
        );
    }

    /// Anchors on `rule!:` heads are rejected — rules don't have a
    /// single subject entity to name. (Validation that lives in the
    /// parser because it's a syntactic restriction on the
    /// head-grammar slot, not a semantic property of the rule body.)
    #[dialog_common::test]
    fn it_rejects_anchor_on_rule_head() {
        let parsed = parse(
            r#"rule!: &mine
  assert!: pong
  when:
    - assert: ping
      where: {}
"#,
        );
        let messages: Vec<_> = parsed
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            messages
                .iter()
                .any(|m| m.contains("&anchor") && m.contains("`rule!:`")),
            "expected diagnostic about anchor on rule, got {messages:?}",
        );
    }
}
