//! View bindings, resolved at lowering.
//!
//! A `show:` template binds an interaction with `on:<name>=<command>`.
//! Both halves used to be resolved at render time — one query per
//! declaration name to rebuild the event descriptor, one per command
//! name to fetch its shape — and a name that resolved to nothing
//! produced no listener and no diagnostic. The element was inert, and
//! the only way to find out was to click it.
//!
//! Both halves are knowable here: the analyzer has the template text
//! and the document's `event!:` / `command!:` declarations in scope.
//! So this module scans the templates, resolves each binding, fails
//! the lowering on anything that does not resolve, and encodes the
//! result as the view's `bindings` field — the same move `concept!:`
//! makes when it inlines its attributes' descriptors rather than
//! leaving names for the runtime to chase.
//!
//! What is *not* inlined is the command's descriptor. The display
//! already resolves a concept from a name for every model it renders,
//! so carrying a second copy here would duplicate work rather than
//! remove it. What the analyzer contributes for commands is the check.

use std::collections::{BTreeMap, BTreeSet};

use tonk_notation::{Application as SyntaxApplication, Field, FieldValue, HeadName, Scalar};
use tonk_schema::resolution::ConceptDefinition;
use tonk_template::bindings::{Bindings, EventBinding, scan};
use tonk_template::event::{EventDescriptor, event_descriptor};

use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::scope::Scope;

/// The field a view's compiled bindings are stored under, and the
/// keyed field its templates live under.
pub(crate) const BINDINGS_FIELD: &str = "bindings";
const SHOW_FIELD: &str = "show";

/// The built-in `view` concept's entity. A head resolving to it is a
/// view however it was spelled, and nothing else is.
const VIEW_ENTITY: &str = "db:view";

/// True when this resolved head is the built-in `view`.
pub(crate) fn is_view(concept: &ConceptDefinition) -> bool {
    concept.entity.to_string() == VIEW_ENTITY
}

/// Index every `event!:` declaration the document makes, so the
/// binding pass reads them synchronously.
///
/// Bare-symbol sources are resolved to entities here, once, using the
/// same name table every other reference goes through — so the
/// event-time path never performs a lookup, and a misspelled source is
/// caught at lowering rather than dropped silently at click time.
pub(crate) fn index_event_declarations(syntax: &tonk_notation::Syntax, scope: &Scope) {
    use tonk_notation::Expression;

    for expression in &syntax.expressions {
        let Expression::Claim(claim) = expression else {
            continue;
        };
        let Some(anchor) = &claim.anchor else {
            continue;
        };
        if !matches!(&claim.inner.predicate.name, HeadName::Concept(name) if name == "event") {
            continue;
        }
        let Some(mut descriptor) = parse_event_declaration(&claim.inner.fields) else {
            continue;
        };
        let resolved: BTreeMap<String, String> = descriptor
            .references()
            .into_iter()
            .filter_map(|name| scope.symbol(&name).map(|entity| (name, entity.to_string())))
            .collect();
        // Names left unresolved stay `Source::Reference`. They are not
        // this pass's to report — the binding pass reaches them only
        // if a template actually binds this declaration, and that is
        // where the diagnostic has a template to point at.
        let _ = descriptor.resolve_references(&resolved);
        scope.record_event_declaration(&anchor.name, descriptor);
    }
}

/// Read an `event!:` body into a descriptor. `None` when the body
/// carries no usable `type:` — the one thing a declaration cannot do
/// without.
fn parse_event_declaration(fields: &[Field]) -> Option<EventDescriptor> {
    let mut event_type = None;
    let mut prevent_default = false;
    let mut stop_propagation = false;
    let mut sources: Vec<(String, String)> = Vec::new();

    for field in fields {
        match field.name.as_str() {
            "type" => event_type = field_text(&field.value),
            "prevent-default" => prevent_default = field_flag(&field.value),
            "stop-propagation" => stop_propagation = field_flag(&field.value),
            "where" => {
                if let FieldValue::Nested(entries) = &field.value {
                    for entry in entries {
                        if let Some(text) = field_text(&entry.value) {
                            sources.push((entry.name.clone(), text));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    event_descriptor(
        event_type.as_deref(),
        prevent_default,
        stop_propagation,
        sources,
    )
    .ok()
}

/// A field value as the source text `parse_source` classifies.
///
/// A bare symbol stays a bare symbol and a URI stays a URI, so the
/// classification a source gets here is the one it would get anywhere
/// else in the notation.
fn field_text(value: &FieldValue) -> Option<String> {
    match value {
        FieldValue::Literal(Scalar::String(text)) => Some(text.clone()),
        FieldValue::Literal(Scalar::Integer(number)) => Some(number.to_string()),
        FieldValue::Literal(Scalar::UnsignedInteger(number)) => Some(number.to_string()),
        FieldValue::Literal(Scalar::Float(number)) => Some(number.to_string()),
        FieldValue::Literal(Scalar::Boolean(flag)) => Some(flag.to_string()),
        FieldValue::Symbol(name) => Some(name.clone()),
        FieldValue::Uri(uri) => Some(uri.clone()),
        _ => None,
    }
}

/// A field value as a flag. Anything that is not an explicit `true`
/// reads as `false`, matching the runtime's "absent means no".
fn field_flag(value: &FieldValue) -> bool {
    matches!(value, FieldValue::Literal(Scalar::Boolean(true)))
}

/// Resolve every binding a view's `show:` templates make, into the
/// artifact the view stores.
///
/// `Ok(None)` means the templates bind nothing, so the view carries no
/// `bindings` field at all — which is also what a view lowered before
/// this pass existed looks like, and what the display's fallback
/// handles.
pub(crate) fn compile_bindings(
    assertion: &SyntaxApplication,
    scope: &Scope,
) -> Result<Option<Bindings>, AnalyzeError> {
    let mut found: Vec<(EventBinding, lsp_types::Range)> = Vec::new();
    for field in &assertion.fields {
        if field.name != SHOW_FIELD {
            continue;
        }
        let FieldValue::Nested(entries) = &field.value else {
            continue;
        };
        for entry in entries {
            let Some(template) = field_text(&entry.value) else {
                continue;
            };
            found.extend(scan(&template).into_iter().map(|binding| {
                let range = attribute_range(entry.value_range, &template, &binding);
                (binding, range)
            }));
        }
    }
    if found.is_empty() {
        return Ok(None);
    }

    let mut events: BTreeMap<String, EventDescriptor> = BTreeMap::new();
    for (binding, range) in found {
        // Every binding's command must resolve, inlined or not: an
        // `on:` that posts nothing is the failure this pass exists to
        // catch, and it is checkable wherever the command lives.
        let command = resolve_command(&binding, scope, range)?;

        let Some(descriptor) = scope.event_declaration(&binding.event_name) else {
            // Not declared in this document. A declaration seeded by
            // an earlier one (a component library binding a core
            // event) still resolves as a name, and the display's
            // per-name query reads it at render time — so this is a
            // binding that is not inlined, not a broken one. Only a
            // name that resolves to nothing at all is an error.
            if scope.symbol(&binding.event_name).is_some() {
                continue;
            }
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::UnknownEventDeclaration {
                    attribute: binding.attribute.clone(),
                    name: binding.event_name.clone(),
                },
                range,
            ));
        };
        check_fills(&binding, &descriptor, &command, range)?;
        events.insert(binding.event_name, descriptor);
    }

    if events.is_empty() {
        return Ok(None);
    }
    Ok(Some(Bindings::new(events)))
}

/// Where a binding's attribute is written, in the document's own
/// coordinates.
///
/// A template is a block scalar: its value range starts at the first
/// content character, and every content line carries that same
/// indentation. So a position inside the template maps to the source
/// by adding the range's start — line offset by line offset, and the
/// indentation added to the column. That is exact for the block form,
/// which is how every template in the corpus is written.
///
/// Anything else — a flow scalar whose escapes shift the columns, a
/// computed position past the end of the value — falls back to the
/// whole value. A range that is merely wide is a worse report; one
/// that points outside the value is a wrong one.
fn attribute_range(
    value_range: lsp_types::Range,
    template: &str,
    binding: &EventBinding,
) -> lsp_types::Range {
    let Some(head) = template.get(..binding.offset) else {
        return value_range;
    };
    let line = head.matches('\n').count() as u32;
    // Every content line of a block scalar starts at the same column
    // the value range does, so the same offset applies to all of them.
    let column = head.rsplit('\n').next().unwrap_or(head).chars().count() as u32;
    let start = lsp_types::Position {
        line: value_range.start.line + line,
        character: value_range.start.character + column,
    };
    let end = lsp_types::Position {
        line: start.line,
        character: start.character + binding.attribute.chars().count() as u32,
    };
    if (end.line, end.character) > (value_range.end.line, value_range.end.character) {
        return value_range;
    }
    lsp_types::Range { start, end }
}

/// The concept a binding's command half names — by bare name or by
/// URI, the two forms a template writes.
fn resolve_command(
    binding: &EventBinding,
    scope: &Scope,
    range: lsp_types::Range,
) -> Result<ConceptDefinition, AnalyzeError> {
    let unknown = || {
        AnalyzeError::at(
            AnalyzeErrorKind::UnknownBoundCommand {
                attribute: binding.attribute.clone(),
                command: binding.command.clone(),
            },
            range,
        )
    };
    if let Some(concept) = scope.concept(&binding.command) {
        return Ok(concept);
    }
    // A URI-spelled command (`on:invite=tonk:invite`) names the
    // concept's entity rather than its published name.
    let entity = binding.command.parse().map_err(|_| unknown())?;
    scope
        .resolved_concept(&entity)
        .flatten()
        .or_else(|| scope.concept_by_entity(&entity))
        .ok_or_else(unknown)
}

/// The declaration must fill the command it is bound to.
///
/// This is the check the whole `event!:` split exists to make
/// possible: a required field with no source posts a command no rule
/// premise matches, and before this the only symptom was a button that
/// did nothing.
fn check_fills(
    binding: &EventBinding,
    descriptor: &EventDescriptor,
    command: &ConceptDefinition,
    range: lsp_types::Range,
) -> Result<(), AnalyzeError> {
    let concept = command.descriptor.concept();
    let mut required: BTreeSet<String> = BTreeSet::new();
    let mut optional: BTreeSet<String> = BTreeSet::new();
    for (name, attribute) in concept.with().iter() {
        if attribute.is_optional() {
            optional.insert(name.to_string());
        } else {
            required.insert(name.to_string());
        }
    }
    let mismatch = tonk_template::event::check(descriptor, &required, &optional);
    if mismatch.is_empty() {
        return Ok(());
    }
    let mut detail = Vec::new();
    if !mismatch.unfilled.is_empty() {
        let fields: Vec<&str> = mismatch.unfilled.iter().map(|u| u.field.as_str()).collect();
        detail.push(format!("nothing fills {}", fields.join(", ")));
    }
    if !mismatch.unknown.is_empty() {
        let fields: Vec<&str> = mismatch.unknown.iter().map(|u| u.field.as_str()).collect();
        detail.push(format!(
            "the declaration sources {}, which the command does not declare",
            fields.join(", ")
        ));
    }
    Err(AnalyzeError::at(
        AnalyzeErrorKind::EventCommandMismatch {
            attribute: binding.attribute.clone(),
            command: binding.command.clone(),
            detail: detail.join("; "),
        },
        range,
    ))
}

#[cfg(test)]
mod tests {
    use dialog_artifacts::Value;
    use dialog_query::Term;
    use tonk_schema::transact::{Application, Statement};
    use tonk_template::bindings::Bindings;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// A command, the declaration that fills it, and a view binding
    /// the two. `$binding` is what the template's `on:` attribute
    /// carries, so a test can break exactly one half.
    fn document(binding: &str, source: &str) -> String {
        format!(
            r#"
command!: &demo/act
  description: A demo command.
  with:
    subject:
      description: What was acted on.
      the: xyz.tonk.demo.act/subject
      as: entity

event!: &on/demo
  type: "click"
  where:
    subject: "{source}"

view!:
  this: tonk:demo
  show:
    ui: |
      <button {binding}>go</button>
"#
        )
    }

    /// Lower a document, returning the analyzer's verdict.
    fn lower(source: &str) -> Result<Vec<Statement>, crate::analyzer::AnalyzeError> {
        let parsed = tonk_notation::parse(source);
        assert!(
            parsed.diagnostics.is_empty(),
            "the fixture must parse: {:#?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("a syntax tree");
        let tree = crate::analyzer::analyze_local(&syntax)?;
        Ok(tree
            .analysis
            .statements()
            .into_iter()
            .map(|planned| planned.statement)
            .collect())
    }

    /// The `bindings` artifact the lowered document carries, if any.
    fn bindings(statements: &[Statement]) -> Option<Bindings> {
        for statement in statements {
            let Statement::Assert(Application::Concept { query, .. }) = statement else {
                continue;
            };
            if let Some(Term::Constant(Value::Record(bytes))) = query.terms.get("bindings") {
                return Some(Bindings::decode(bytes).expect("the artifact decodes"));
            }
        }
        None
    }

    #[dialog_common::test]
    fn it_inlines_the_declaration_a_binding_names() {
        let statements = lower(&document("on:demo=demo/act", "{this}")).expect("lowers");
        let bindings = bindings(&statements).expect("the view carries its bindings");
        let descriptor = bindings
            .events
            .get("on/demo")
            .expect("`on/demo` is inlined under the name the binding uses");
        assert_eq!(descriptor.event_type, "click");
        assert_eq!(
            descriptor.sources.get("subject"),
            Some(&tonk_template::event::Source::Field("this".into())),
            "the `where:` source rides along, so nothing is queried per name at render time",
        );
    }

    #[dialog_common::test]
    fn a_view_that_binds_nothing_carries_no_artifact() {
        let source = r#"
view!:
  this: tonk:demo
  show:
    ui: |
      <p>nothing to click</p>
"#;
        let statements = lower(source).expect("lowers");
        assert!(
            bindings(&statements).is_none(),
            "a view with no bindings must not grow an empty artifact",
        );
    }

    #[dialog_common::test]
    fn a_binding_naming_no_declaration_fails_the_lowering() {
        let error = lower(&document("on:missing=demo/act", "{this}"))
            .expect_err("a dangling declaration reference must not lower");
        assert_eq!(error.kind.code(), "E_UNKNOWN_EVENT_DECLARATION", "{error}");
    }

    #[dialog_common::test]
    fn a_binding_naming_no_command_fails_the_lowering() {
        let error = lower(&document("on:demo=demo/nope", "{this}"))
            .expect_err("a dangling command reference must not lower");
        assert_eq!(error.kind.code(), "E_UNKNOWN_BOUND_COMMAND", "{error}");
    }

    #[dialog_common::test]
    fn a_declaration_that_cannot_fill_its_command_fails_the_lowering() {
        // The declaration sources `topic`, which the command does not
        // declare, and leaves `subject` — which it requires —
        // unfilled. Both halves of the mismatch, one diagnostic.
        let source = document("on:demo=demo/act", "{this}").replace("subject: \"{", "topic: \"{");
        let error = lower(&source).expect_err("an unfillable command must not lower");
        assert_eq!(error.kind.code(), "E_EVENT_COMMAND_MISMATCH", "{error}");
    }

    /// The report points at the attribute, not at the template.
    ///
    /// A block scalar is many lines wide; underlining all of it says
    /// "something in here is wrong" when the analyzer knows exactly
    /// which eight characters are.
    #[dialog_common::test]
    fn a_dangling_binding_is_reported_where_it_is_written() {
        let source = document("on:missing=demo/act", "{this}");
        let error = lower(&source).expect_err("a dangling declaration must not lower");
        let range = error.range.expect("the report is placed");
        let line = source
            .lines()
            .nth(range.start.line as usize)
            .expect("the line is in the document");
        let start = range.start.character as usize;
        let end = range.end.character as usize;
        assert_eq!(
            &line[start..end],
            "on:missing",
            "the range covers the attribute alone, in `{line}`",
        );
        assert_eq!(range.start.line, range.end.line, "one line, not the block");
    }

    /// The `bindings` field's type is spellable in an author's own
    /// `concept!:`, not only in the hand-built built-in.
    ///
    /// Lives with the view tests because the built-in is the reason
    /// `record` entered the `as:` vocabulary: a type the analyzer
    /// writes but cannot parse would be a schema only the compiler
    /// could author.
    #[dialog_common::test]
    fn record_is_a_spellable_value_type() {
        let source = r#"
attribute!: &demo/blob
  the: xyz.tonk.demo/blob
  as: record
  cardinality: one
  description: A compiled artifact.
"#;
        lower(source).expect("`as: record` is a declarable value type");
    }

    /// A binding mentioned in an HTML comment is not a binding: the
    /// parser drops comment content, so treating it as one would fail
    /// the lowering over prose.
    #[dialog_common::test]
    fn a_binding_named_in_a_comment_is_not_one() {
        let source = r#"
view!:
  this: tonk:demo
  show:
    ui: |
      <!-- bind it with on:nowhere=nothing -->
      <p>prose</p>
"#;
        let statements = lower(source).expect("lowers");
        assert!(bindings(&statements).is_none());
    }
}
