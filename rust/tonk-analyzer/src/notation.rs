//! The one module of tonk that names `dialog-notation`.
//!
//! Two halves:
//! - install: a library written in the notation, lowered to the claims
//!   tonk stores when it installs one, what `library_claims_after` in the
//!   worker gives for a YAML component ([`notation_library_claims`]);
//! - render: a view written in the notation, kept current on a space's
//!   branch by the notation's driver and handed back as patches of
//!   tonk's own ([`live`]).
//!
//! A later revision of the notation is meant to be a change here and
//! nowhere else.

pub mod live;
/// Demo (notation tour): one notebook cell in the notation, evaluated.
pub mod tour;

pub use dialog_notation::{Declaration, parse_written};

use std::collections::BTreeMap;

use dialog_corpus::{Changes, Instruction, Statement as _, Value};
use dialog_notation::{Analyzer, Concept, Segment, Step, Syntax, check_forms};
use dialog_query::ConceptConclusion;
use dialog_query::query::Output as _;
use dialog_repository::{QueryLayer, Transient};
use tonk_schema::concept::QueryEnv;

/// The top-level forms of a text in the notation, each with the
/// comments and blank lines above it.
pub fn split(text: &str) -> Vec<String> {
    let mut forms = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            ';' => {
                for (_, c) in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                while let Some((_, c)) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    forms.push(text[start..at + 1].to_string());
                    start = at + 1;
                }
            }
            _ => {}
        }
    }
    forms
}

/// A part of the notation and what it is.
pub fn notation_kind(tree: &Syntax, text: &str) -> &'static str {
    match tree.applies() {
        Some("concept") => {
            let head = tree.operand().and_then(|operand| operand.items.first());
            let entity = head
                .and_then(Syntax::operand)
                .and_then(|operand| operand.items.first());
            if matches!(entity, Some(Syntax::Variable(_))) {
                "concept"
            } else {
                "command"
            }
        }
        Some("rule") => "rule",
        Some("view") => "view",
        Some("assert") if text.contains("tonk:element/") || text.contains("id:notebook-save") => {
            "assert of an element"
        }
        Some("assert") => match tree
            .operand()
            .and_then(|operand| operand.items.first())
            .and_then(Syntax::applies)
        {
            Some("route") => "assert of a route",
            _ => "assert of other data",
        },
        _ => "other",
    }
}

/// What [`notation_library_claims`] gives: what `library_claims_after`
/// gives for a YAML component, and, for measuring, each form of the
/// library with what it writes, and what the notation declares.
pub struct NotationLibrary {
    /// The claims to assert, as `library_claims_after`'s durable set.
    pub durable: Vec<Instruction>,
    /// The commands to dispatch, as its transient set.
    pub transient: Vec<Instruction>,
    /// Each form of the library, with what it writes.
    pub forms: Vec<NotationForm>,
    /// What the library and core declare, in the order written.
    pub declarations: Vec<Declaration>,
}

/// One top-level form of a library in the notation, and what it writes.
#[derive(Debug, Clone)]
pub struct NotationForm {
    /// What kind of form it is: `concept`, `command`, `rule`, `view`,
    /// `event` (one a view binds), or an assert of a route, of an
    /// element or of other data.
    pub kind: &'static str,
    /// What pairs it with a form of a YAML library: a concept's or an
    /// event's name, or the kind and the place among forms of that kind.
    pub key: String,
    /// What it asserts.
    pub durable: Changes,
    /// What it dispatches.
    pub transient: Changes,
    /// Why it writes nothing, when the notation refuses it.
    pub refused: Option<String>,
}

/// A library in the notation, with `core` before it, lowered to the
/// claims tonk stores when it installs it: what `library_claims_after`
/// returns for a YAML component.
///
/// A form the notation refuses writes nothing and is said to be
/// refused with the notation's first error; the rest of the document
/// is checked again without it, since a document with a refusal in it
/// compiles to nothing. A refusal in `core` is an error.
///
/// For each form that passes:
/// - a concept or a command declared writes tonk's declaration claims
///   ([`declaration_claims`]);
/// - a rule writes dialog's `dialog.rule/*` facts, as a branch
///   transaction writes it;
/// - an assert writes its facts, a command issued is dispatched, and
///   a write with a `when` writes once for each match of its `when`,
///   read over nothing but the write's own rule, through `env`;
/// - a query, a view among them, writes nothing: it is not stored on
///   the branch at this revision of the notation, and a view is kept
///   current through [`live`] from the library's text instead.
///
/// Nothing of `core` is written: it is read for its names only.
pub async fn notation_library_claims<Env: QueryEnv>(
    core: &str,
    library: &str,
    env: &Env,
) -> Result<NotationLibrary, String> {
    let core = split(core);
    let library = split(library);
    let parts: Vec<(bool, &str)> = core
        .iter()
        .map(|form| (true, form.as_str()))
        .chain(library.iter().map(|form| (false, form.as_str())))
        .collect();
    // The notation checks the document form by form: a refused form
    // gives nothing and says why, every other form gives its steps and
    // its declarations, each with the form it came of. A view is
    // refused by the Dialog-DB compiler and still declared.
    let text: String = parts.iter().map(|(_, part)| *part).collect();
    let checked = check_forms(
        &mut Analyzer::new().map_err(|error| error.to_string())?,
        &text,
    );
    let mut refused: BTreeMap<usize, String> = BTreeMap::new();
    for diagnostic in checked.refusals() {
        let at = match diagnostic.path.first() {
            Some(Segment::Item(at)) => *at,
            _ => return Err(format!("a refusal with no form: {}", diagnostic.message)),
        };
        if at >= parts.len() || parts[at].0 {
            return Err(format!("core is refused: {}", diagnostic.message));
        }
        refused
            .entry(at)
            .or_insert_with(|| diagnostic.message.clone());
    }
    let mut steps: BTreeMap<usize, Vec<Step>> = BTreeMap::new();
    for (step, at) in checked.steps.iter().zip(&checked.step_forms) {
        steps.entry(*at).or_default().push(step.clone());
    }
    let declared: BTreeMap<usize, &Declaration> = checked
        .declaration_forms
        .iter()
        .copied()
        .zip(&checked.declarations)
        .collect();

    let mut all = Changes::new();
    let mut all_transient = Changes::new();
    let mut ordinal: BTreeMap<&str, usize> = BTreeMap::new();
    let mut forms = Vec::new();
    for (at, (in_core, part)) in parts.iter().enumerate() {
        if *in_core {
            continue;
        }
        let (tree, _) = parse_written(part).map_err(|error| error.to_string())?;
        let tree = tree.first().ok_or("an empty part")?;
        let kind = notation_kind(tree, part);
        let name = tree
            .operand()
            .and_then(|operand| operand.items.first())
            .and_then(Syntax::applies)
            .unwrap_or_default()
            .to_string();
        let key = match kind {
            "concept" | "command" => name.clone(),
            "assert of an element" => "assert of an element 1".to_string(),
            _ => {
                let at = ordinal.entry(kind).or_default();
                *at += 1;
                format!("{kind} {at}")
            }
        };
        let mut changes = Changes::new();
        let mut transients = Changes::new();
        if !refused.contains_key(&at) {
            if let Some(Declaration::Concept(concept)) = declared.get(&at)
                && matches!(kind, "concept" | "command")
            {
                declaration_claims(concept, &mut changes)?;
            }
            for step in steps.remove(&at).unwrap_or_default() {
                match step {
                    Step::Assert(statement) => statement.assert(&mut changes),
                    Step::Retract(statement) => statement.retract(&mut changes),
                    Step::Issue(statement) => statement.assert(&mut transients),
                    Step::Deduce(rule) => rule.assert(&mut changes),
                    Step::Induce(rule) => rule.assert(&mut changes),
                    Step::Transient(concept) => Transient(concept).assert(&mut changes),
                    Step::Query(_) | Step::View(_) | Step::Select(_) => {}
                    Step::Write(write) => {
                        let mut layer = QueryLayer::new();
                        if let Some(rule) = &write.query.rule {
                            layer = layer.with(rule.clone());
                        }
                        let rows: Vec<ConceptConclusion> = layer
                            .select(write.query.select.clone())
                            .perform(env)
                            .try_vec()
                            .await
                            .map_err(|error| format!("a write's `when`: {error}"))?;
                        for row in &rows {
                            let statement =
                                write.statement(row).map_err(|error| error.to_string())?;
                            if write.retract {
                                statement.retract(&mut changes);
                            } else {
                                statement.assert(&mut changes);
                            }
                        }
                    }
                }
            }
        }
        all.merge(changes.clone());
        all_transient.merge(transients.clone());
        let form = NotationForm {
            kind,
            key,
            durable: changes,
            transient: transients,
            refused: refused.get(&at).cloned(),
        };
        // An element is many asserts in the notation and one form in
        // the YAML: its asserts are one form here.
        match forms
            .iter_mut()
            .find(|known: &&mut NotationForm| known.key == form.key)
        {
            Some(known) => {
                known.durable.merge(form.durable);
                known.transient.merge(form.transient);
                if known.refused.is_none() {
                    known.refused = form.refused;
                }
            }
            None => forms.push(form),
        }
    }
    Ok(NotationLibrary {
        durable: all.into_instructions(),
        transient: all_transient.into_instructions(),
        forms,
        declarations: checked.declarations,
    })
}

/// What tonk's analyzer writes for a `concept!:` or a `command!:`,
/// from what the notation declares: each field's attribute facts
/// (`attribute_statements`) with the comment above its line as its
/// description, the concept's `db.concept.with/*` facts and marker
/// (`AnonymousConcept`, or `TransientConcept` for a command) with the
/// comment above the form as its description, and the
/// `db.name/referent` an `&anchor` writes.
pub fn declaration_claims(concept: &Concept, changes: &mut Changes) -> Result<(), String> {
    use tonk_schema::concept::{AnonymousConcept, TransientConcept, attribute_statements};

    let [descriptor] = concept.descriptors.as_slice() else {
        return Err(format!(
            "{} reads {} descriptors, and tonk stores a concept as one",
            concept.name,
            concept.descriptors.len()
        ));
    };
    // The descriptor names a field by its attribute, and tonk by the
    // label the head gives it (what a view's `{field}` and a posted
    // command name): each field is found by the line that compiles to
    // its attribute, and takes that line's label.
    let mut relabelled = serde_json::Map::new();
    for (name, field) in descriptor.with().iter() {
        let uri = field.descriptor().to_uri();
        let line = concept.lines.iter().find(|line| {
            line.descriptor
                .as_ref()
                .is_some_and(|descriptor| descriptor.to_uri() == uri)
        });
        let label = line
            .and_then(|line| line.label.clone())
            .unwrap_or_else(|| name.to_string());
        let mut attribute =
            serde_json::to_value(field.descriptor()).map_err(|error| error.to_string())?;
        if let Some(description) = line.and_then(|line| line.description.clone()) {
            attribute["description"] = serde_json::Value::String(description);
        }
        let attribute: dialog_query::AttributeDescriptor =
            serde_json::from_value(attribute).map_err(|error| error.to_string())?;
        for statement in attribute_statements(&attribute)? {
            statement.assert(changes);
        }
        relabelled.insert(
            label,
            serde_json::to_value(field).map_err(|error| error.to_string())?,
        );
    }
    let mut shape = serde_json::to_value(descriptor).map_err(|error| error.to_string())?;
    shape["with"] = serde_json::Value::Object(relabelled);
    let descriptor: dialog_query::ConceptDescriptor =
        serde_json::from_value(shape).map_err(|error| error.to_string())?;
    let described = match &concept.description {
        Some(description) => descriptor.clone().with_description(description.clone()),
        None => descriptor.clone(),
    };
    let this = described.this();
    if concept.command {
        TransientConcept::new(described).assert(changes);
    } else {
        AnonymousConcept::new(described).assert(changes);
    }
    dialog_query::AttributeStatement {
        the: dialog_query::the!("db.name/referent"),
        of: format!("id:{}", concept.name)
            .parse()
            .map_err(|error| format!("{error}"))?,
        is: Value::Entity(this),
        cause: None,
        cardinality: None,
        pick: Some(dialog_query::Pick::Last),
    }
    .assert(changes);
    Ok(())
}
