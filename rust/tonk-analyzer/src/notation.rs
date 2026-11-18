//! A back end for `dialog-notation`: a library written in the notation,
//! lowered to the claims tonk stores when it installs one, what
//! `library_claims_after` in the worker gives for a YAML component.
//!
//! Three calls here are the managing session's opinion, each a small
//! function so it is easy to change: an event's name ([`event_name`]),
//! a variable's slot name ([`slots`]), and that no event flag is read
//! ([`view_claims`]).

use std::collections::BTreeMap;

use dialog_corpus::{Changes, Instruction, Statement as _, Value};
use dialog_notation::{
    Analyzer, Concept, Declaration, Segment, Step, Syntax, View, check_forms, parse_written,
};
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
/// - a view writes what tonk stores for it and for the events it binds
///   ([`view_claims`]), when `views` is set.
///
/// Nothing of `core` is written: it is read for its names only.
pub async fn notation_library_claims<Env: QueryEnv>(
    core: &str,
    library: &str,
    views: bool,
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
    let mut concepts: BTreeMap<String, &Concept> = BTreeMap::new();
    for declaration in &checked.declarations {
        if let Declaration::Concept(concept) = declaration {
            concepts.insert(concept.name.clone(), concept);
        }
    }

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
                    Step::Query(_) => {}
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
        // A view is a description the Dialog-DB back end refuses; tonk's
        // back end lowers it from the description, whichever way the
        // compile went.
        let mut events: Vec<NotationForm> = Vec::new();
        if views
            && kind == "view"
            && let Some(Declaration::View { view, .. }) = declared.get(&at)
        {
            let written = view_claims(view, &concepts)
                .map_err(|error| format!("view {}: {error}", view.concept))?;
            for statement in written.view {
                statement.assert(&mut changes);
            }
            for (name, statements) in written.events {
                let mut rows = Changes::new();
                for statement in statements {
                    statement.assert(&mut rows);
                }
                all.merge(rows.clone());
                events.push(NotationForm {
                    kind: "event",
                    key: name,
                    durable: rows,
                    transient: Changes::new(),
                    refused: None,
                });
            }
        }
        for event in events {
            if !forms
                .iter()
                .any(|known: &NotationForm| known.key == event.key)
            {
                forms.push(event);
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

/// What a view lowers to: its own facts, and each event it binds, by
/// the event's name, with its rows.
pub struct ViewClaims {
    /// The view's own facts.
    pub view: Vec<dialog_query::AttributeStatement>,
    /// Each event it binds, by name, with its rows.
    pub events: Vec<(String, Vec<dialog_query::AttributeStatement>)>,
}

fn fact(
    the: &str,
    of: dialog_corpus::Entity,
    is: Value,
) -> Result<dialog_query::AttributeStatement, String> {
    Ok(dialog_query::AttributeStatement {
        the: the.parse().map_err(|error| format!("{the}: {error:?}"))?,
        of,
        is,
        cause: None,
        cardinality: None,
        pick: Some(dialog_query::Pick::Last),
    })
}

/// The managing session's opinion: an event's name is `on/` and its
/// command's name with `/` turned into `-` (`block/edit` names
/// `on/block-edit`).
pub fn event_name(command: &str) -> String {
    format!("on/{}", command.replace('/', "-"))
}

/// The slot each variable of a view fills, named as tonk's templates
/// name them (`tonk-template/src/fields.rs:26` to `:33`, checked at
/// `tonk-analyzer/src/analyzer/view.rs:442`): `this` for the entity
/// rendered, the model's field label for a field's value, and
/// `<field>/key` for the key of a keyed collection's entry.
///
/// The view's conditions come expanded to reads of attributes
/// (`(xyz.tonk.notebook ?n title: (last (text ?title)) entry: …)`), not
/// as the concept read they were written as, so each read's attribute
/// is found among the concept's lines, and its label is the head label
/// the line's variable has.
pub fn slots(view: &View, concept: &Concept) -> Result<BTreeMap<String, String>, String> {
    fn innermost(term: &Syntax) -> Option<&str> {
        match term {
            Syntax::Variable(variable) => Some(variable),
            Syntax::Application(_, operand) => operand.items.first().and_then(innermost),
            _ => None,
        }
    }
    let domain = |attribute: &str| attribute.split('/').next().unwrap_or_default().to_string();
    let label = |line: &dialog_notation::Line| line.label.clone();
    let mut slots = BTreeMap::new();
    slots.insert(view.entity.clone(), "this".to_string());
    for condition in &view.conditions {
        let Syntax::Application(operator, operand) = condition else {
            continue;
        };
        let Syntax::Reference(read) = operator.as_ref() else {
            continue;
        };
        for (term_label, term) in &operand.terms {
            if term_label == "entry" {
                let line = concept
                    .lines
                    .iter()
                    .find(|line| line.key.is_some() && domain(&line.attribute) == *read)
                    .ok_or_else(|| format!("{} declares no collection in {read}", concept.name))?;
                let field =
                    label(line).ok_or_else(|| format!("no head label holds {}", line.attribute))?;
                if let Some(value) = innermost(term) {
                    slots.insert(value.to_string(), field.clone());
                }
                if let Syntax::Application(_, entry) = term
                    && let Some(Syntax::Variable(key)) = entry.terms.get("key")
                {
                    slots.insert(
                        key.clone(),
                        format!("{field}{}", tonk_template::fields::KEY_SUFFIX),
                    );
                }
                continue;
            }
            let attribute = format!("{read}/{term_label}");
            let Some(variable) = innermost(term) else {
                continue;
            };
            let line = concept
                .lines
                .iter()
                .find(|line| line.attribute == attribute)
                .ok_or_else(|| format!("{} has no line {attribute}", concept.name))?;
            let field = label(line).ok_or_else(|| format!("no head label holds {attribute}"))?;
            slots.insert(variable.to_string(), field);
        }
    }
    Ok(slots)
}

fn slot(slots: &BTreeMap<String, String>, variable: &str) -> Result<String, String> {
    slots
        .get(variable)
        .map(|slot| format!("{{{slot}}}"))
        .ok_or_else(|| format!("?{variable} fills no slot"))
}

fn piece(
    slots: &BTreeMap<String, String>,
    piece: &dialog_notation::view::Piece,
) -> Result<String, String> {
    use dialog_notation::view::Piece;
    match piece {
        Piece::Text(text) => Ok(text.clone()),
        Piece::Variable(variable) => slot(slots, variable),
        Piece::Value(value) => Ok(spelled(value)),
    }
}

fn spelled(value: &Syntax) -> String {
    match value {
        Syntax::Constant(Value::String(text)) => text.clone(),
        other => other.spelled(false),
    }
}

/// A template as tonk's renderer holds one (`tonk_render::Node`), so
/// its serializer writes the text tonk stores.
fn markup(
    node: &dialog_notation::Node,
    slots: &BTreeMap<String, String>,
) -> Result<tonk_render::Node, String> {
    use dialog_notation::{Attribute, Node};
    Ok(match node {
        Node::Text(text) => tonk_render::Node::Text(text.clone()),
        Node::Value(value) => tonk_render::Node::Text(spelled(value)),
        Node::Variable(variable) => tonk_render::Node::Text(slot(slots, variable)?),
        Node::View(nested) => {
            let mut attrs = Vec::new();
            for (key, pieces) in &nested.entries {
                let value: Result<String, String> =
                    pieces.iter().map(|part| piece(slots, part)).collect();
                attrs.push((key.clone(), value?));
            }
            if let Some(of) = &nested.of {
                attrs.push(("entity".into(), slot(slots, of)?));
            }
            if let Some(model) = &nested.model {
                attrs.push(("model".into(), model.clone()));
            }
            tonk_render::Node::Element(tonk_render::Element {
                tag: "tonk-display".into(),
                attrs,
                children: Vec::new(),
                void: false,
            })
        }
        Node::Element(element) => {
            let mut attrs = Vec::new();
            for (name, value) in &element.attributes {
                let value = match value {
                    Attribute::Text(text) => text.clone(),
                    Attribute::Value(value) => spelled(value),
                    Attribute::Variable(variable) => slot(slots, variable)?,
                    Attribute::Parts(pieces) => pieces
                        .iter()
                        .map(|part| piece(slots, part))
                        .collect::<Result<String, String>>()?,
                    Attribute::Present => String::new(),
                };
                attrs.push((name.clone(), value));
            }
            for event in &element.events {
                let name = event_name(&event.command);
                attrs.push((
                    format!("on:{}", name.trim_start_matches("on/")),
                    event.command.clone(),
                ));
            }
            let children = element
                .children
                .iter()
                .map(|child| markup(child, slots))
                .collect::<Result<Vec<_>, String>>()?;
            tonk_render::Node::Element(tonk_render::Element {
                void: tonk_render::tree::is_void_tag(&element.tag),
                tag: element.tag.clone(),
                attrs,
                children,
            })
        }
    })
}

/// What tonk stores for a view, from the notation's description:
/// - the facet text, `xyz.tonk.view/<name>`, on the model concept's
///   entity, printed by tonk's renderer (`tonk_render::serialize_nodes`);
/// - the `bindings` record of the events it binds
///   (`tonk_template::bindings::Bindings`), when it binds any;
/// - each event's rows, as an `event!:` stores them: `type`, one
///   `where` per field, and the name `id:on/<name>`, on the entity
///   tonk derives from that payload (`derive_this`).
///
/// No event flags are read: the notation has no spelling for
/// `prevent-default` or `stop-propagation` (the managing session's
/// opinion is to write both false).
pub fn view_claims(
    view: &View,
    concepts: &BTreeMap<String, &Concept>,
) -> Result<ViewClaims, String> {
    use tonk_template::event::{EventDescriptor, Source};

    let concept = concepts
        .get(&view.concept)
        .ok_or_else(|| format!("{} is not declared", view.concept))?;
    let [descriptor] = concept.descriptors.as_slice() else {
        return Err(format!("{} reads more than one descriptor", view.concept));
    };
    let model = descriptor.this();
    let slots = slots(view, concept)?;
    let text = tonk_render::serialize_nodes(&[markup(&view.template, &slots)?]);
    let mut statements = vec![fact(
        &format!("xyz.tonk.view/{}", view.name),
        model.clone(),
        Value::String(text),
    )?];

    let mut descriptors = BTreeMap::new();
    let mut events = Vec::new();
    let event_concept = tonk_schema::builtin::lookup_concept("event")
        .ok_or("tonk has no built-in `event`")?
        .entity;
    for event in view.events() {
        let name = event_name(&event.command);
        let mut sources = BTreeMap::new();
        let mut payload = tonk_core::claim::ValueMap::new();
        payload.insert("type".into(), Value::String(event.kind.clone()));
        let mut rows = Vec::new();
        for (field, input) in &event.fields {
            let (source, written) = match input {
                dialog_notation::Input::Path(steps) => (
                    Source::Property(steps.clone()),
                    format!(".{}", steps.join(".")),
                ),
                dialog_notation::Input::Variable(variable) => {
                    let slot = slot(&slots, variable)?;
                    (
                        Source::Field(slot.trim_matches(['{', '}']).to_string()),
                        slot,
                    )
                }
                dialog_notation::Input::Value(value) => {
                    let text = spelled(value);
                    (Source::Literal(text.clone()), format!("{text:?}"))
                }
            };
            sources.insert(field.clone(), source);
            payload.insert(format!("where.{field}"), Value::String(written.clone()));
            rows.push((format!("xyz.tonk.event.where/{field}"), written));
        }
        let entity = tonk_schema::transact::derive_this(&event_concept, &payload);
        let mut written = vec![fact(
            "xyz.tonk.event/type",
            entity.clone(),
            Value::String(event.kind.clone()),
        )?];
        for (the, value) in rows {
            written.push(fact(&the, entity.clone(), Value::String(value))?);
        }
        written.push(fact(
            "db.name/referent",
            format!("id:{name}")
                .parse()
                .map_err(|error| format!("{error}"))?,
            Value::Entity(entity),
        )?);
        events.push((name.clone(), written));
        descriptors.insert(
            name,
            EventDescriptor {
                event_type: event.kind.clone(),
                prevent_default: false,
                stop_propagation: false,
                sources,
            },
        );
    }
    if !descriptors.is_empty() {
        let record = tonk_template::bindings::Bindings::new(descriptors).encode()?;
        statements.push(fact(
            "xyz.tonk.view/bindings",
            model,
            Value::Record(record),
        )?);
    }
    Ok(ViewClaims {
        view: statements,
        events,
    })
}
