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
//!
//! The other half of a template gets the same treatment. A `{field}`
//! interpolation is resolved against the row being rendered, and a
//! name that resolves to nothing renders as nothing — no gap, no
//! warning, just a value missing from the page. Since the view names
//! the concept it renders, the analyzer knows the legal names, so it
//! checks them here too: in the templates, and in the `where:` sources
//! of every declaration a template binds, which interpolate in the
//! same scope.
//!
//! Both checks work by *mirroring the renderer*, which is where the
//! subtlety lives. The renderer will not read a `<style>` or
//! `<script>` body (their braces are CSS and JS), will not read an
//! HTML comment, and will not interpolate a portal document at all.
//! Miss any of those and the check rejects the library it is meant to
//! protect. That is why the scan is shared with the binding scan
//! ([`tonk_template::scan`]) rather than written twice.

use std::collections::{BTreeMap, BTreeSet};

use tonk_notation::{Application as SyntaxApplication, Field, FieldValue, HeadName, Scalar};
use tonk_schema::resolution::ConceptDefinition;
use tonk_template::bindings::{Bindings, EventBinding, scan};
use tonk_template::event::{EventDescriptor, Source, event_descriptor};
use tonk_template::fields::{self, FieldReference};

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

/// Check a view's `show:` templates and resolve the bindings they
/// make into the artifact the view stores.
///
/// Three things are read out of one walk of the templates, because
/// they are three readings of the same text:
///
/// * every `{field}` must be one the model declares, or ambient
///   ([`check_interpolations`]);
/// * every `on:<name>` must name a declaration, and its command must
///   resolve and be fillable;
/// * a bound declaration's own `{field}` sources are interpolations in
///   the same scope, so they get the same check
///   ([`check_event_sources`]).
///
/// `Ok(None)` means the templates bind nothing, so the view carries no
/// `bindings` field at all — which is also what a view lowered before
/// this pass existed looks like, and what the display's fallback
/// handles.
pub(crate) fn compile_bindings(
    assertion: &SyntaxApplication,
    scope: &Scope,
) -> Result<Option<Bindings>, AnalyzeError> {
    // A portal view's templates are full HTML documents mounted into
    // a `<tonk-portal>` verbatim — the display checks the same `type`
    // entry and takes a different path entirely. Nothing interpolates
    // them and nothing delegates their events, so a `{` in one is a
    // brace and an `on:` in one is an attribute. Reading either as a
    // reference would fail the lowering over a document the renderer
    // never looks inside.
    if is_portal(assertion) {
        return Ok(None);
    }

    // The concept the view renders. Without it neither interpolation
    // check has anything to check against, so both are skipped: a view
    // whose `this:` is a variable, or names something that is not a
    // concept, is not one this pass can reason about.
    let model = model_concept(assertion, scope);

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
            if let Some(model) = &model {
                check_interpolations(&template, entry.value_range, model)?;
            }
            found.extend(scan(&template).into_iter().map(|binding| {
                let range = offset_range(
                    entry.value_range,
                    &template,
                    binding.offset,
                    binding.attribute.chars().count(),
                );
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
        if let Some(model) = &model {
            check_event_sources(&binding, &descriptor, &command, model, range)?;
        }
        events.insert(binding.event_name, descriptor);
    }

    if events.is_empty() {
        return Ok(None);
    }
    Ok(Some(Bindings::new(events)))
}

/// Where something inside a template is written, in the document's
/// own coordinates.
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
fn offset_range(
    value_range: lsp_types::Range,
    template: &str,
    offset: usize,
    width: usize,
) -> lsp_types::Range {
    let Some(head) = template.get(..offset) else {
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
        character: start.character + width as u32,
    };
    if (end.line, end.character) > (value_range.end.line, value_range.end.character) {
        return value_range;
    }
    lsp_types::Range { start, end }
}

/// Whether the view's `show:` declares a portal document: a `type`
/// entry reading `text/html`.
///
/// The same entry the display checks — see `handle_view_frame` in
/// `tonk-display` — so the two agree on which templates are markup to
/// read and which are documents to hand over untouched.
fn is_portal(assertion: &SyntaxApplication) -> bool {
    use tonk_template::resolve::TYPE_FACET;

    assertion
        .fields
        .iter()
        .filter(|field| field.name == SHOW_FIELD)
        .filter_map(|field| match &field.value {
            FieldValue::Nested(entries) => Some(entries),
            _ => None,
        })
        .flatten()
        .any(|entry| {
            entry.name == TYPE_FACET
                && field_text(&entry.value).as_deref() == Some(PORTAL_CONTENT_TYPE)
        })
}

/// The `type` value that marks a portal document.
const PORTAL_CONTENT_TYPE: &str = "text/html";

/// The concept a view renders, named by its `this:` field.
///
/// `None` when the field is absent, is a variable, or names something
/// that is not a concept — every case where there is no field list to
/// check a template against.
fn model_concept(assertion: &SyntaxApplication, scope: &Scope) -> Option<Model> {
    let field = assertion.fields.iter().find(|f| f.name == "this")?;
    let name = field_text(&field.value)?;
    let concept = resolve_concept(&name, scope)?;
    let fields: BTreeSet<String> = concept
        .descriptor
        .concept()
        .with()
        .iter()
        .map(|(field, _)| field.to_string())
        .collect();
    Some(Model { name, fields })
}

/// A view's model: what it is called, and what it declares.
struct Model {
    /// The name as the view writes it, for a diagnostic to quote.
    name: String,
    /// Every field the concept declares, required and optional alike.
    fields: BTreeSet<String>,
}

impl Model {
    /// Whether a template may interpolate this reference.
    ///
    /// `{this}` and `{dom.host/*}` are ambient — the subject and the
    /// outer host's attributes, which no concept declares. A
    /// `{block/key}` is legal exactly when `block` is, because the
    /// renderer puts the key into the same per-iteration scope as the
    /// field it belongs to.
    fn accepts(&self, reference: &FieldReference) -> bool {
        reference.is_ambient() || self.fields.contains(reference.declared_name())
    }

    /// The declared fields, rendered for a diagnostic.
    fn known(&self) -> String {
        if self.fields.is_empty() {
            return format!("`{}` declares no fields.", self.name);
        }
        let names: Vec<String> = self.fields.iter().map(|f| format!("`{f}`")).collect();
        format!("`{}` declares {}.", self.name, names.join(", "))
    }
}

/// Every `{field}` a template interpolates must be one the model
/// declares.
///
/// The renderer resolves a reference against the row it is rendering
/// and renders **nothing** when it misses — so a typo costs a value on
/// the page and produces no other symptom. This is the same failure the
/// `on:` checks catch, on the other half of the template.
fn check_interpolations(
    template: &str,
    value_range: lsp_types::Range,
    model: &Model,
) -> Result<(), AnalyzeError> {
    for reference in fields::scan(template) {
        if model.accepts(&reference) {
            continue;
        }
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::UnknownTemplateField {
                field: reference.name.clone(),
                model: model.name.clone(),
                known: model.known(),
            },
            offset_range(value_range, template, reference.offset, reference.len()),
        ));
    }
    Ok(())
}

/// A bound declaration's `{field}` sources are interpolations too, and
/// resolve in the same scope a template's do.
///
/// Only a source filling a field the command actually declares is
/// checked. One that fills nothing is inert whatever it interpolates —
/// and is already what `E_EVENT_COMMAND_MISMATCH` reports — so adding a
/// second diagnostic for it would report the same mistake twice.
fn check_event_sources(
    binding: &EventBinding,
    descriptor: &EventDescriptor,
    command: &ConceptDefinition,
    model: &Model,
    range: lsp_types::Range,
) -> Result<(), AnalyzeError> {
    let concept = command.descriptor.concept();
    for (command_field, source) in &descriptor.sources {
        let Source::Field(name) = source else {
            continue;
        };
        if !concept.with().keys().any(|field| field == command_field) {
            continue;
        }
        let reference = FieldReference {
            name: name.clone(),
            offset: 0,
        };
        if model.accepts(&reference) {
            continue;
        }
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::UnknownEventSourceField {
                attribute: binding.attribute.clone(),
                field: name.clone(),
                detail: format!("`{command_field}`, which `{}` does not declare", model.name),
            },
            range,
        ));
    }
    Ok(())
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
    resolve_concept(&binding.command, scope).ok_or_else(unknown)
}

/// The concept a name refers to, by published name or by URI.
///
/// The two forms the notation writes for the same thing: a bare name
/// goes through the name table, a URI names the concept's entity
/// directly.
fn resolve_concept(name: &str, scope: &Scope) -> Option<ConceptDefinition> {
    if let Some(concept) = scope.concept(name) {
        return Some(concept);
    }
    let entity = name.parse().ok()?;
    scope
        .resolved_concept(&entity)
        .flatten()
        .or_else(|| scope.concept_by_entity(&entity))
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

    /// A model concept, a command over it, and a view that renders
    /// it. `$template` is the `show:` body and `$source` the
    /// declaration's source for `subject`, so a test can break either
    /// half against a model whose fields are known.
    fn modelled(template: &str, source: &str) -> String {
        format!(
            r#"
concept!: &counter/model
  description: A counter.
  with:
    count:
      description: How many.
      the: xyz.tonk.counter/count
      as: signed-integer
      cardinality: one

command!: &counter/increment
  description: Add one.
  with:
    subject:
      description: Which counter.
      the: xyz.tonk.counter.increment/subject
      as: entity

event!: &on/bump
  type: "click"
  where:
    subject: "{source}"

view!:
  this: counter/model
  show:
    ui: |
{}
"#,
            template
                .lines()
                .map(|line| format!("      {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// The gap the `on:` checks left open, on the other half of the
    /// template: `{counter}` is not a field of `counter/model`, and
    /// before this it lowered clean and rendered as nothing at all.
    #[dialog_common::test]
    fn a_template_interpolating_an_undeclared_field_fails_the_lowering() {
        let error = lower(&modelled("<p>{counter}</p>", "{this}"))
            .expect_err("an undeclared interpolation must not lower");
        assert_eq!(error.kind.code(), "E_UNKNOWN_TEMPLATE_FIELD", "{error}");
        assert!(
            error.to_string().contains("`count`"),
            "the report names what the model does declare: {error}",
        );
    }

    #[dialog_common::test]
    fn a_template_interpolating_a_declared_field_lowers() {
        lower(&modelled("<p>{count}</p>", "{this}")).expect("`count` is a field of the model");
    }

    /// `{this}` is the subject and `{dom.host/*}` is copied off the
    /// outer host element. No concept declares either, and rejecting
    /// them would reject most of the library.
    #[dialog_common::test]
    fn the_subject_and_host_attributes_are_not_model_fields() {
        lower(&modelled(
            "<a href=\"/c/{this}\" data-model=\"{dom.host/model}\">{count}</a>",
            "{this}",
        ))
        .expect("ambient references are not checked against the model");
    }

    /// The exclusion the check lives or dies on. A stylesheet's rule
    /// blocks are braces, not fields; so is a script's block syntax.
    #[dialog_common::test]
    fn css_and_script_braces_are_not_interpolations() {
        lower(&modelled(
            "<style>p { color: red; }</style>\n<script>if (x) { go(); }</script>\n<p>{count}</p>",
            "{this}",
        ))
        .expect("a rule block is not a field reference");
    }

    /// The report points at the reference, not at the block scalar.
    #[dialog_common::test]
    fn an_undeclared_interpolation_is_reported_where_it_is_written() {
        let source = modelled("<p>{counter}</p>", "{this}");
        let error = lower(&source).expect_err("must not lower");
        let range = error.range.expect("the report is placed");
        let line = source
            .lines()
            .nth(range.start.line as usize)
            .expect("the line is in the document");
        assert_eq!(
            &line[range.start.character as usize..range.end.character as usize],
            "{counter}",
        );
    }

    /// A declaration's `{field}` source is an interpolation too, in
    /// the same scope, so the same miss applies — the command posts
    /// with that field empty.
    #[dialog_common::test]
    fn an_event_sourcing_an_undeclared_field_fails_the_lowering() {
        let error = lower(&modelled(
            "<button on:bump=counter/increment>+</button>",
            "{tally}",
        ))
        .expect_err("an undeclared source must not lower");
        assert_eq!(error.kind.code(), "E_UNKNOWN_EVENT_SOURCE_FIELD", "{error}");
    }

    #[dialog_common::test]
    fn an_event_sourcing_a_declared_field_lowers() {
        lower(&modelled(
            "<button on:bump=counter/increment>{count}</button>",
            "{count}",
        ))
        .expect("`count` is a field of the model the view renders");
    }

    /// A source that fills a field the command does not declare is
    /// inert whatever it interpolates, and is already what the
    /// fill check reports. Reporting the interpolation too would name
    /// the same mistake twice, and point at the less useful half.
    #[dialog_common::test]
    fn a_source_the_command_cannot_use_is_not_an_interpolation_error() {
        let source = modelled("<button on:bump=counter/increment>+</button>", "{tally}")
            .replace("    subject: \"{tally}\"", "    tally: \"{tally}\"");
        let error = lower(&source).expect_err("the command still cannot be filled");
        assert_eq!(
            error.kind.code(),
            "E_EVENT_COMMAND_MISMATCH",
            "the fill check owns this, not the interpolation check: {error}",
        );
    }

    /// A portal document is mounted verbatim, so its braces are not
    /// interpolations and its `on:` attributes are not bindings. The
    /// display decides this from the same `type` entry.
    #[dialog_common::test]
    fn a_portal_documents_braces_are_not_interpolations() {
        let source = modelled("<p>{counter}</p>", "{this}")
            .replace("  show:\n", "  show:\n    type: \"text/html\"\n");
        lower(&source).expect("a portal document is not read as a template");
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
