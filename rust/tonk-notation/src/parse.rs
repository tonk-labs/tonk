//! Parser entry points.
//!
//! Two entry points produce the same [`Parsed`] shape from
//! different surface syntaxes:
//!
//! - [`parse`] takes a YAML document and walks it via
//!   [`saphyr`]. YAML's permissiveness justifies a *partial-parse*
//!   contract: when one statement is malformed the rest of the
//!   document still produces a [`Syntax`] tree, with diagnostics
//!   pointing at the offending nodes.
//!
//! - [`parse_json`] takes a JSON document and converts it via
//!   [`serde_json`] into a [`Syntax`] tree using
//!   `Syntax::try_from(&serde_json::Value)`. JSON's strict
//!   syntax means structural errors abort the whole parse —
//!   `Parsed::syntax` is `None` whenever any diagnostic was
//!   raised on the JSON path.
//!
//! Both flavours emit [`lsp_types::Diagnostic`]s. For the JSON
//! path source positions aren't available, so diagnostics
//! anchor at the document start; LSP clients render them as a
//! document-level annotation.

use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use saphyr::{LoadableYamlNode, MarkedYaml, Scalar as SaphyrScalar, ScanError, YamlData};
use serde_json::Value as Json;

use crate::syntax::{
    AttributeNode, ConceptField, ConceptNode, Context, DomainContext, DomainField, DomainValue,
    Reference, Scalar, Spanned, Statement, Subject, SubjectKind, Syntax, UserConceptNode,
};

/// Outcome of a parse: a [`Syntax`] tree (when the parser made
/// it through end-to-end) plus any diagnostics raised along the
/// way.
///
/// `syntax.is_some()` and `diagnostics.is_empty()` aren't
/// independent — they correlate, but the exact relationship
/// depends on the surface:
///
/// - **YAML** can produce `Some(syntax)` *with* diagnostics. A
///   partially-broken document still yields the well-formed
///   subset of statements so the language server can highlight
///   each issue separately rather than silencing everything
///   after the first one.
///
/// - **JSON** is all-or-nothing: any diagnostic means
///   `syntax = None`.
#[derive(Clone, Debug, Default)]
pub struct Parsed {
    /// The parsed [`Syntax`] when parsing reached the bottom.
    /// On the JSON path this is `Some` iff `diagnostics` is
    /// empty; on the YAML path it can carry partial results.
    pub syntax: Option<Syntax>,
    /// Diagnostics raised during the parse. Empty on success.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse `text` as a YAML document and convert it to a
/// [`Syntax`] tree.
///
/// Empty input is a no-op success — the editor freshly opens a
/// blank buffer all the time and we don't want to nag for
/// that.
pub fn parse(text: &str) -> Parsed {
    let documents = match MarkedYaml::load_from_str(text) {
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
    let mut statements = Vec::new();
    // Multi-document streams (`---`-separated) flatten — each
    // document contributes its statements to the same
    // top-level `Syntax`.
    let mut overall_range: Option<Range> = None;
    for doc in &documents {
        let doc_range = range_of(doc);
        overall_range = Some(match overall_range {
            None => doc_range,
            Some(existing) => extend_range(existing, doc_range),
        });
        walk_document(doc, &mut statements, &mut diagnostics);
    }

    let range = overall_range.unwrap_or_default();
    Parsed {
        syntax: Some(Syntax { statements, range }),
        diagnostics,
    }
}

/// Parse `text` as a JSON document and convert it to a
/// [`Syntax`] tree.
///
/// Returns `Parsed { syntax: None, diagnostics }` on any
/// failure — JSON's strict syntax means there's no useful
/// partial-parse contract on this side.
pub fn parse_json(text: &str) -> Parsed {
    let value: Json = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            return Parsed {
                syntax: None,
                diagnostics: vec![diagnostic_at_origin(format!("invalid JSON: {err}"))],
            };
        }
    };
    match Syntax::try_from(&value) {
        Ok(syntax) => Parsed {
            syntax: Some(syntax),
            diagnostics: Vec::new(),
        },
        Err(diagnostics) => Parsed {
            syntax: None,
            diagnostics,
        },
    }
}

// -----------------------------------------------------------------
// YAML walker (saphyr → Syntax)
// -----------------------------------------------------------------

fn walk_document(doc: &MarkedYaml<'_>, statements: &mut Vec<Statement>, out: &mut Vec<Diagnostic>) {
    let YamlData::Mapping(top) = &doc.data else {
        out.push(error(
            range_of(doc),
            "Asserted notation expects a mapping at the document root \
             (subject → context → field). Found a scalar or sequence.",
        ));
        return;
    };
    for (subject_key, contexts_value) in top {
        if let Some(statement) = walk_statement(subject_key, contexts_value, out) {
            statements.push(statement);
        }
    }
}

fn walk_statement(
    key: &MarkedYaml<'_>,
    value: &MarkedYaml<'_>,
    out: &mut Vec<Diagnostic>,
) -> Option<Statement> {
    let key_text = match string_of(key) {
        Some(s) => s,
        None => {
            out.push(error(
                range_of(key),
                "Subject must be a string (a DID/URI, a bookmark name, \
                 `_`, or `?var`).",
            ));
            return None;
        }
    };

    let YamlData::Mapping(contexts) = &value.data else {
        out.push(error(
            range_of(value),
            "Subject value must be a mapping of context → fields. \
             Asserted notation has no representation for a subject \
             that is itself a scalar or sequence.",
        ));
        return None;
    };

    let subject = Subject {
        kind: classify_subject(key_text),
        source: key_text.to_owned(),
        range: range_of(key),
    };
    let mut context_nodes = Vec::new();
    for (context_key, context_value) in contexts {
        if let Some(context) = walk_context(context_key, context_value, out) {
            context_nodes.push(context);
        }
    }

    Some(Statement {
        subject,
        contexts: context_nodes,
        range: extend_range(range_of(key), range_of(value)),
    })
}

fn walk_context(
    key: &MarkedYaml<'_>,
    value: &MarkedYaml<'_>,
    out: &mut Vec<Diagnostic>,
) -> Option<Context> {
    let name = match string_of(key) {
        Some(s) => s,
        None => {
            out.push(error(
                range_of(key),
                "Context name must be a string (a domain like \
                 `io.gozala.person`, or a concept name like \
                 `attribute`).",
            ));
            return None;
        }
    };

    let YamlData::Mapping(fields) = &value.data else {
        out.push(error(
            range_of(value),
            "Context value must be a mapping of field → value.",
        ));
        return None;
    };

    let key_range = range_of(key);
    let block_range = extend_range(key_range, range_of(value));

    if name.contains('.') {
        let mut domain_fields = Vec::new();
        for (field_key, field_value) in fields {
            if let Some(field) = walk_domain_field(field_key, field_value, out) {
                domain_fields.push(field);
            }
        }
        return Some(Context::Domain(DomainContext {
            domain: name.to_owned(),
            key_range,
            fields: domain_fields,
            range: block_range,
        }));
    }

    match name {
        "attribute" => walk_attribute(fields, key_range, block_range, out).map(Context::Attribute),
        "concept" => walk_concept(fields, key_range, block_range, out).map(Context::Concept),
        _ => {
            // User-defined concept context — same field shape as
            // a domain context (each level-3 key is a field
            // name, value is a reference). Interpreter does the
            // schema lookup.
            let mut user_fields = Vec::new();
            for (field_key, field_value) in fields {
                if let Some(field) = walk_domain_field(field_key, field_value, out) {
                    user_fields.push(field);
                }
            }
            Some(Context::UserConcept(UserConceptNode {
                name: name.to_owned(),
                key_range,
                fields: user_fields,
                range: block_range,
            }))
        }
    }
}

fn walk_attribute(
    fields: &saphyr::AnnotatedMapping<'_, MarkedYaml<'_>>,
    key_range: Range,
    block_range: Range,
    out: &mut Vec<Diagnostic>,
) -> Option<AttributeNode> {
    let mut node = AttributeNode {
        the: None,
        as_type: None,
        cardinality: None,
        description: None,
        key_range,
        range: block_range,
    };
    for (field_key, field_value) in fields {
        let Some(name) = string_of(field_key) else {
            out.push(error(range_of(field_key), "Field name must be a string."));
            continue;
        };
        let Some(value_str) = string_of(field_value) else {
            out.push(error(
                range_of(field_value),
                format!("Field `{name}` on `attribute:` expects a scalar value."),
            ));
            continue;
        };
        let value_range = range_of(field_value);
        match name {
            "the" => node.the = Some(Spanned::new(value_str.to_owned(), value_range)),
            "as" => node.as_type = Some(Spanned::new(value_str.to_owned(), value_range)),
            "cardinality" => {
                node.cardinality = Some(Spanned::new(value_str.to_owned(), value_range));
            }
            "description" => {
                node.description = Some(Spanned::new(value_str.to_owned(), value_range));
            }
            other => {
                out.push(warning(
                    range_of(field_key),
                    format!("Unknown field `{other}` on `attribute:` ignored."),
                ));
            }
        }
    }
    Some(node)
}

fn walk_concept(
    fields: &saphyr::AnnotatedMapping<'_, MarkedYaml<'_>>,
    key_range: Range,
    block_range: Range,
    out: &mut Vec<Diagnostic>,
) -> Option<ConceptNode> {
    let mut node = ConceptNode {
        description: None,
        with: Vec::new(),
        maybe: Vec::new(),
        key_range,
        range: block_range,
    };
    for (field_key, field_value) in fields {
        let Some(name) = string_of(field_key) else {
            out.push(error(range_of(field_key), "Field name must be a string."));
            continue;
        };
        match name {
            "description" => {
                if let Some(s) = string_of(field_value) {
                    node.description = Some(Spanned::new(s.to_owned(), range_of(field_value)));
                } else {
                    out.push(error(
                        range_of(field_value),
                        "Field `description` on `concept:` expects a scalar value.",
                    ));
                }
            }
            "with" | "maybe" => {
                let YamlData::Mapping(field_map) = &field_value.data else {
                    out.push(error(
                        range_of(field_value),
                        format!("`{name}:` on `concept:` expects a mapping of field → reference."),
                    ));
                    continue;
                };
                let mut collected = Vec::new();
                for (sub_key, sub_value) in field_map {
                    if let Some(field) = walk_concept_field(sub_key, sub_value, out) {
                        collected.push(field);
                    }
                }
                if name == "with" {
                    node.with = collected;
                } else {
                    node.maybe = collected;
                }
            }
            other => {
                out.push(warning(
                    range_of(field_key),
                    format!("Unknown field `{other}` on `concept:` ignored."),
                ));
            }
        }
    }
    Some(node)
}

fn walk_concept_field(
    key: &MarkedYaml<'_>,
    value: &MarkedYaml<'_>,
    out: &mut Vec<Diagnostic>,
) -> Option<ConceptField> {
    let Some(name) = string_of(key) else {
        out.push(error(range_of(key), "Field name must be a string."));
        return None;
    };
    let value_range = range_of(value);
    let reference = if let Some(s) = string_of(value) {
        if s.contains(':') {
            Reference::Uri(Spanned::new(s.to_owned(), value_range))
        } else {
            Reference::Bookmark(Spanned::new(s.to_owned(), value_range))
        }
    } else if let YamlData::Mapping(inline_fields) = &value.data {
        let inline = walk_attribute(inline_fields, value_range, value_range, out)?;
        Reference::Inline(Box::new(inline))
    } else {
        out.push(error(
            value_range,
            format!(
                "Field `{name}` value must be a string (bookmark or URI) \
                 or a mapping (inline attribute definition)."
            ),
        ));
        return None;
    };
    Some(ConceptField {
        name: name.to_owned(),
        name_range: range_of(key),
        value: reference,
        value_range,
    })
}

fn walk_domain_field(
    key: &MarkedYaml<'_>,
    value: &MarkedYaml<'_>,
    out: &mut Vec<Diagnostic>,
) -> Option<DomainField> {
    let Some(name) = string_of(key) else {
        out.push(error(range_of(key), "Field name must be a string."));
        return None;
    };
    let value_range = range_of(value);
    Some(DomainField {
        name: name.to_owned(),
        name_range: range_of(key),
        value: walk_domain_value(value, out)?,
        value_range,
    })
}

fn walk_domain_value(value: &MarkedYaml<'_>, out: &mut Vec<Diagnostic>) -> Option<DomainValue> {
    match &value.data {
        YamlData::Value(scalar) => Some(DomainValue::Scalar(scalar_from_saphyr(scalar))),
        YamlData::Representation(text, _, _) => Some(DomainValue::Scalar(Scalar::String(
            text.as_ref().to_owned(),
        ))),
        YamlData::Sequence(seq) => {
            let mut values = Vec::with_capacity(seq.len());
            for item in seq {
                if let Some(v) = walk_domain_value(item, out) {
                    values.push(v);
                }
            }
            Some(DomainValue::Sequence(values))
        }
        YamlData::Mapping(map) => {
            let mut nested = Vec::new();
            for (k, v) in map {
                if let Some(field) = walk_domain_field(k, v, out) {
                    nested.push(field);
                }
            }
            Some(DomainValue::Mapping(nested))
        }
        YamlData::Tagged(_, inner) => walk_domain_value(inner, out),
        YamlData::Alias(_) | YamlData::BadValue => {
            out.push(error(range_of(value), "Unsupported YAML node here."));
            None
        }
    }
}

fn scalar_from_saphyr(scalar: &SaphyrScalar<'_>) -> Scalar {
    match scalar {
        SaphyrScalar::Null => Scalar::Null,
        SaphyrScalar::Boolean(b) => Scalar::Boolean(*b),
        SaphyrScalar::Integer(i) => Scalar::Integer(i128::from(*i)),
        SaphyrScalar::FloatingPoint(f) => Scalar::Float(**f),
        SaphyrScalar::String(s) => Scalar::String(s.as_ref().to_owned()),
    }
}

// -----------------------------------------------------------------
// JSON conversion (serde_json::Value → Syntax)
// -----------------------------------------------------------------

impl TryFrom<&Json> for Syntax {
    type Error = Vec<Diagnostic>;

    fn try_from(value: &Json) -> Result<Self, Self::Error> {
        let Json::Object(top) = value else {
            return Err(vec![diagnostic_at_origin(
                "Asserted notation expects a JSON object at the document root \
                 (subject → context → field). Found a non-object value.",
            )]);
        };
        let mut diagnostics = Vec::new();
        let mut statements = Vec::new();
        for (subject_key, body) in top {
            match try_statement_from_json(subject_key, body) {
                Ok(stmt) => statements.push(stmt),
                Err(mut errs) => diagnostics.append(&mut errs),
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        Ok(Syntax {
            statements,
            range: Range::default(),
        })
    }
}

fn try_statement_from_json(subject_key: &str, body: &Json) -> Result<Statement, Vec<Diagnostic>> {
    let Json::Object(contexts) = body else {
        return Err(vec![diagnostic_at_origin(format!(
            "Subject `{subject_key}` value must be a JSON object of `context: fields`."
        ))]);
    };
    let subject = Subject {
        kind: classify_subject(subject_key),
        source: subject_key.to_owned(),
        range: Range::default(),
    };
    let mut context_nodes = Vec::new();
    let mut diagnostics = Vec::new();
    for (context_key, context_value) in contexts {
        match try_context_from_json(subject_key, context_key, context_value) {
            Ok(ctx) => context_nodes.push(ctx),
            Err(mut errs) => diagnostics.append(&mut errs),
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(Statement {
        subject,
        contexts: context_nodes,
        range: Range::default(),
    })
}

fn try_context_from_json(
    subject_key: &str,
    context_key: &str,
    body: &Json,
) -> Result<Context, Vec<Diagnostic>> {
    let Json::Object(fields) = body else {
        return Err(vec![diagnostic_at_origin(format!(
            "Context `{subject_key}.{context_key}` value must be a JSON object of `field: value`."
        ))]);
    };

    if context_key.contains('.') {
        let mut diagnostics = Vec::new();
        let mut domain_fields = Vec::new();
        for (field_key, field_value) in fields {
            match try_domain_field_from_json(field_key, field_value) {
                Ok(f) => domain_fields.push(f),
                Err(mut errs) => diagnostics.append(&mut errs),
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        return Ok(Context::Domain(DomainContext {
            domain: context_key.to_owned(),
            key_range: Range::default(),
            fields: domain_fields,
            range: Range::default(),
        }));
    }

    match context_key {
        "attribute" => {
            let attr = try_attribute_from_json(subject_key, fields)?;
            Ok(Context::Attribute(attr))
        }
        "concept" => {
            let concept = try_concept_from_json(subject_key, fields)?;
            Ok(Context::Concept(concept))
        }
        _ => {
            let mut diagnostics = Vec::new();
            let mut user_fields = Vec::new();
            for (field_key, field_value) in fields {
                match try_domain_field_from_json(field_key, field_value) {
                    Ok(f) => user_fields.push(f),
                    Err(mut errs) => diagnostics.append(&mut errs),
                }
            }
            if !diagnostics.is_empty() {
                return Err(diagnostics);
            }
            Ok(Context::UserConcept(UserConceptNode {
                name: context_key.to_owned(),
                key_range: Range::default(),
                fields: user_fields,
                range: Range::default(),
            }))
        }
    }
}

fn try_attribute_from_json(
    subject_key: &str,
    fields: &serde_json::Map<String, Json>,
) -> Result<AttributeNode, Vec<Diagnostic>> {
    let mut node = AttributeNode {
        the: None,
        as_type: None,
        cardinality: None,
        description: None,
        key_range: Range::default(),
        range: Range::default(),
    };
    let mut diagnostics = Vec::new();
    for (name, value) in fields {
        let Some(s) = json_scalar_string(value) else {
            diagnostics.push(diagnostic_at_origin(format!(
                "Field `{subject_key}.attribute.{name}` expects a scalar value."
            )));
            continue;
        };
        match name.as_str() {
            "the" => node.the = Some(Spanned::new(s, Range::default())),
            "as" => node.as_type = Some(Spanned::new(s, Range::default())),
            "cardinality" => node.cardinality = Some(Spanned::new(s, Range::default())),
            "description" => node.description = Some(Spanned::new(s, Range::default())),
            other => diagnostics.push(diagnostic_at_origin(format!(
                "Unknown field `{other}` on `attribute:`."
            ))),
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(node)
}

fn try_concept_from_json(
    subject_key: &str,
    fields: &serde_json::Map<String, Json>,
) -> Result<ConceptNode, Vec<Diagnostic>> {
    let mut node = ConceptNode {
        description: None,
        with: Vec::new(),
        maybe: Vec::new(),
        key_range: Range::default(),
        range: Range::default(),
    };
    let mut diagnostics = Vec::new();
    for (name, value) in fields {
        match name.as_str() {
            "description" => match json_scalar_string(value) {
                Some(s) => node.description = Some(Spanned::new(s, Range::default())),
                None => diagnostics.push(diagnostic_at_origin(format!(
                    "Field `{subject_key}.concept.description` expects a scalar value."
                ))),
            },
            "with" | "maybe" => {
                let Json::Object(map) = value else {
                    diagnostics.push(diagnostic_at_origin(format!(
                        "`{subject_key}.concept.{name}` expects an object of field → reference."
                    )));
                    continue;
                };
                let mut collected = Vec::new();
                for (sub_key, sub_value) in map {
                    match try_concept_field_from_json(sub_key, sub_value) {
                        Ok(field) => collected.push(field),
                        Err(mut errs) => diagnostics.append(&mut errs),
                    }
                }
                if name == "with" {
                    node.with = collected;
                } else {
                    node.maybe = collected;
                }
            }
            other => diagnostics.push(diagnostic_at_origin(format!(
                "Unknown field `{other}` on `concept:`."
            ))),
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(node)
}

fn try_concept_field_from_json(name: &str, value: &Json) -> Result<ConceptField, Vec<Diagnostic>> {
    let reference = match value {
        Json::String(s) => {
            if s.contains(':') {
                Reference::Uri(Spanned::new(s.clone(), Range::default()))
            } else {
                Reference::Bookmark(Spanned::new(s.clone(), Range::default()))
            }
        }
        Json::Object(obj) => {
            let inline = try_attribute_from_json(name, obj)?;
            Reference::Inline(Box::new(inline))
        }
        _ => {
            return Err(vec![diagnostic_at_origin(format!(
                "Field `{name}` value must be a string (bookmark or URI) or an \
                 object (inline attribute definition)."
            ))]);
        }
    };
    Ok(ConceptField {
        name: name.to_owned(),
        name_range: Range::default(),
        value: reference,
        value_range: Range::default(),
    })
}

fn try_domain_field_from_json(name: &str, value: &Json) -> Result<DomainField, Vec<Diagnostic>> {
    Ok(DomainField {
        name: name.to_owned(),
        name_range: Range::default(),
        value: try_domain_value_from_json(value)?,
        value_range: Range::default(),
    })
}

fn try_domain_value_from_json(value: &Json) -> Result<DomainValue, Vec<Diagnostic>> {
    match value {
        Json::Null => Ok(DomainValue::Scalar(Scalar::Null)),
        Json::Bool(b) => Ok(DomainValue::Scalar(Scalar::Boolean(*b))),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(DomainValue::Scalar(Scalar::Integer(i128::from(i))))
            } else if let Some(u) = n.as_u64() {
                Ok(DomainValue::Scalar(Scalar::UnsignedInteger(u128::from(u))))
            } else if let Some(f) = n.as_f64() {
                Ok(DomainValue::Scalar(Scalar::Float(f)))
            } else {
                Err(vec![diagnostic_at_origin(format!(
                    "Number `{n}` is not representable."
                ))])
            }
        }
        Json::String(s) => Ok(DomainValue::Scalar(Scalar::String(s.clone()))),
        Json::Array(items) => {
            let mut diagnostics = Vec::new();
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                match try_domain_value_from_json(item) {
                    Ok(v) => values.push(v),
                    Err(mut errs) => diagnostics.append(&mut errs),
                }
            }
            if !diagnostics.is_empty() {
                return Err(diagnostics);
            }
            Ok(DomainValue::Sequence(values))
        }
        Json::Object(map) => {
            let mut diagnostics = Vec::new();
            let mut nested = Vec::new();
            for (k, v) in map {
                match try_domain_field_from_json(k, v) {
                    Ok(f) => nested.push(f),
                    Err(mut errs) => diagnostics.append(&mut errs),
                }
            }
            if !diagnostics.is_empty() {
                return Err(diagnostics);
            }
            Ok(DomainValue::Mapping(nested))
        }
    }
}

fn json_scalar_string(value: &Json) -> Option<String> {
    match value {
        Json::String(s) => Some(s.clone()),
        Json::Bool(b) => Some(b.to_string()),
        Json::Number(n) => Some(n.to_string()),
        Json::Null => None,
        _ => None,
    }
}

// -----------------------------------------------------------------
// Subject classification (shared)
// -----------------------------------------------------------------

fn classify_subject(text: &str) -> SubjectKind {
    if text == "_" {
        SubjectKind::Anonymous
    } else if text.starts_with('?') {
        SubjectKind::Variable
    } else if text.contains(':') {
        SubjectKind::Uri
    } else {
        SubjectKind::Bookmark
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

/// Convert a saphyr marker to an LSP position. Saphyr uses
/// 1-indexed lines and 0-indexed columns; LSP uses 0-indexed
/// for both.
pub(crate) fn position_at(marker: &saphyr::Marker) -> Position {
    Position {
        line: (marker.line() as u32).saturating_sub(1),
        character: marker.col() as u32,
    }
}

/// Convert a saphyr node's span to an LSP range. Zero-width
/// spans (e.g. at end-of-document) get widened to one column so
/// editor squiggles render visibly.
pub(crate) fn range_of(node: &MarkedYaml<'_>) -> Range {
    let start = position_at(&node.span.start);
    let mut end = position_at(&node.span.end);
    if start.line == end.line && start.character == end.character {
        end.character = end.character.saturating_add(1);
    }
    Range { start, end }
}

fn extend_range(start: Range, end: Range) -> Range {
    Range {
        start: start.start,
        end: end.end,
    }
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
    diagnostic(range, DiagnosticSeverity::ERROR, message)
}

fn warning(range: Range, message: impl Into<String>) -> Diagnostic {
    diagnostic(range, DiagnosticSeverity::WARNING, message)
}

fn diagnostic_at_origin(message: impl Into<String>) -> Diagnostic {
    error(
        Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 1,
            },
        },
        message,
    )
}

fn diagnostic(
    range: Range,
    severity: DiagnosticSeverity,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(severity),
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

    #[dialog_common::test]
    fn empty_input_is_clean() {
        let parsed = parse("");
        assert!(parsed.diagnostics.is_empty());
        assert!(parsed.syntax.is_none());
    }

    #[dialog_common::test]
    fn yaml_parse_error_surfaces() {
        let parsed = parse("a:\n  b: 1\n c: 2\n");
        assert!(parsed.syntax.is_none());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].source.as_deref(), Some("yaml"));
    }

    #[dialog_common::test]
    fn scalar_root_is_an_error() {
        let parsed = parse("just a string\n");
        assert_eq!(parsed.diagnostics.len(), 1);
        assert!(
            parsed.diagnostics[0]
                .message
                .contains("mapping at the document root")
        );
    }

    #[dialog_common::test]
    fn classify_subject_variants() {
        assert_eq!(classify_subject("_"), SubjectKind::Anonymous);
        assert_eq!(classify_subject("?meal"), SubjectKind::Variable);
        assert_eq!(classify_subject("did:key:zAlice"), SubjectKind::Uri);
        assert_eq!(classify_subject("alice"), SubjectKind::Bookmark);
    }

    #[dialog_common::test]
    fn attribute_node_round_trip() {
        let syntax = parse_clean(
            "person-name:\n\
             \x20 attribute:\n\
             \x20   description: The person's name\n\
             \x20   the: io.gozala.person/name\n\
             \x20   as: Text\n\
             \x20   cardinality: one\n",
        );
        assert_eq!(syntax.statements.len(), 1);
        let stmt = &syntax.statements[0];
        assert_eq!(stmt.subject.kind, SubjectKind::Bookmark);
        assert_eq!(stmt.subject.source, "person-name");
        assert_eq!(stmt.contexts.len(), 1);
        let Context::Attribute(attr) = &stmt.contexts[0] else {
            panic!("expected Attribute, got {:?}", stmt.contexts[0]);
        };
        assert_eq!(attr.the.as_ref().unwrap().value, "io.gozala.person/name");
        assert_eq!(attr.as_type.as_ref().unwrap().value, "Text");
        assert_eq!(attr.cardinality.as_ref().unwrap().value, "one");
        assert_eq!(
            attr.description.as_ref().unwrap().value,
            "The person's name"
        );
    }

    #[dialog_common::test]
    fn concept_with_bookmark_reference() {
        let syntax = parse_clean(
            "person:\n\
             \x20 concept:\n\
             \x20   description: A person\n\
             \x20   with:\n\
             \x20     name: person-name\n\
             \x20     age: person-age\n",
        );
        let Context::Concept(concept) = &syntax.statements[0].contexts[0] else {
            panic!("expected Concept");
        };
        assert_eq!(concept.with.len(), 2);
        match &concept.with[0].value {
            Reference::Bookmark(b) => assert_eq!(b.value, "person-name"),
            other => panic!("expected Bookmark, got {other:?}"),
        }
    }

    #[dialog_common::test]
    fn domain_context_classifies_correctly() {
        let syntax = parse_clean(
            "did:key:zAlice:\n\
             \x20 com.app.person:\n\
             \x20   name: Alice\n\
             \x20   age: 28\n",
        );
        let stmt = &syntax.statements[0];
        assert_eq!(stmt.subject.kind, SubjectKind::Uri);
        let Context::Domain(domain) = &stmt.contexts[0] else {
            panic!("expected Domain");
        };
        assert_eq!(domain.domain, "com.app.person");
        assert_eq!(domain.fields.len(), 2);
    }

    #[dialog_common::test]
    fn anonymous_subject_classifies() {
        let syntax = parse_clean(
            "_:\n\
             \x20 com.app.demo:\n\
             \x20   x: 1\n",
        );
        assert_eq!(syntax.statements[0].subject.kind, SubjectKind::Anonymous);
    }

    #[dialog_common::test]
    fn dialog_prefix_is_no_longer_an_error() {
        // Carry and the transact route legitimately write under
        // `dialog.meta` / `dialog.attribute` / etc. The reserved-
        // prefix rule was wrong; verify it stays gone.
        let parsed = parse(
            "did:key:zAlice:\n\
             \x20 dialog.attribute:\n\
             \x20   id: foo\n",
        );
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected diagnostics: {:#?}",
            parsed.diagnostics
        );
    }

    #[dialog_common::test]
    fn entity_with_scalar_value_is_an_error() {
        let parsed = parse("did:key:zAlice: hello\n");
        assert_eq!(parsed.diagnostics.len(), 1);
        assert!(parsed.diagnostics[0].message.contains("mapping of context"));
    }

    #[dialog_common::test]
    fn user_concept_context() {
        let syntax = parse_clean(
            "alice:\n\
             \x20 person:\n\
             \x20   name: Alice\n\
             \x20   age: 28\n",
        );
        let Context::UserConcept(user) = &syntax.statements[0].contexts[0] else {
            panic!("expected UserConcept");
        };
        assert_eq!(user.name, "person");
        assert_eq!(user.fields.len(), 2);
    }

    #[dialog_common::test]
    fn json_round_trip_parses_to_same_shape() {
        let json = r#"{
            "person-name": {
                "attribute": {
                    "the": "io.gozala.person/name",
                    "as": "Text",
                    "cardinality": "one"
                }
            }
        }"#;
        let parsed = parse_json(json);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        let syntax = parsed.syntax.unwrap();
        let Context::Attribute(attr) = &syntax.statements[0].contexts[0] else {
            panic!("expected Attribute");
        };
        assert_eq!(attr.the.as_ref().unwrap().value, "io.gozala.person/name");
    }

    #[dialog_common::test]
    fn json_invalid_root_aborts() {
        let parsed = parse_json("\"just a string\"");
        assert!(parsed.syntax.is_none());
        assert_eq!(parsed.diagnostics.len(), 1);
    }

    #[dialog_common::test]
    fn json_partial_failure_aborts_whole_parse() {
        // Even with one good statement, JSON path is all-or-
        // nothing: any per-statement error fails the whole
        // conversion (no Some(syntax) on the way out).
        let json = r#"{
            "ok": { "com.app.person": { "name": "Alice" } },
            "broken": "not an object"
        }"#;
        let parsed = parse_json(json);
        assert!(parsed.syntax.is_none());
        assert!(!parsed.diagnostics.is_empty());
    }
}
