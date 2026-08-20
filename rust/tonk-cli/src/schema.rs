//! `tonk schema` — read every attribute and concept asserted on
//! the branch, emit a notation document that re-submits cleanly.
//!
//! Two query passes:
//!
//! 1. The built-in `attribute` concept enumerates every attribute
//!    (named or not). For each, a separate lookup against
//!    `db.name/referent` recovers the bookmark name where one
//!    exists.
//! 2. The built-in `concept` concept enumerates every concept —
//!    named or not. Each row's `source` is the JSON-encoded
//!    [`ConceptDescriptor`], deserialized to recover the full
//!    `with:`/`maybe:` maps and the concept description (the
//!    query layer folds the `db.meta/description` claim into the
//!    synthesised descriptor). The `transient` field decides the
//!    head: transient concepts re-emit as `command!:`, which the
//!    analyzer derives to the same entity as `concept!:` +
//!    `transient:`.
//!
//! A concept whose stored entity differs from its descriptor's
//! content address (a pinned `this:`, e.g. `tonk:view`) re-emits
//! the pin; anonymous concepts emit without an anchor and rely on
//! the pin or the content address for identity.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use anyhow::{Context, Result, anyhow};
use dialog_artifacts::Entity;
use dialog_query::{
    AttributeQuery, Cardinality, ConceptDescriptor, ConceptFieldDescriptor, Output as _, Term,
    Type, attribute,
};
use serde_json::Value as Json;
use tonk_evaluator::evaluate::{QueryMatchBlock, SyntaxEvaluateExt};
use tonk_notation::parse;
use tonk_schema::rule::{DeductiveRule, InductiveRule, Rule};
use tonk_schema::rule_notation;

use crate::output::EvaluateResponse;

use crate::site::TonkSite;

/// Slim summary of one named concept on the branch — just enough
/// for `tonk concept ls` to print. The full descriptor stays internal
/// to [`render`] (which needs it to re-emit the `with:` map as
/// notation).
#[derive(Debug, Clone)]
pub struct ConceptSummary {
    /// Bookmark name published via `db.name/referent` on
    /// the matching `id:<name>` entity.
    pub name: String,
    /// Entity identifier of the concept descriptor.
    pub entity: String,
    /// Human description claim (`db.meta/description`), if
    /// asserted. Concept descriptions are optional in the
    /// analyzer.
    pub description: Option<String>,
    /// Field names from the concept's `with:` map, in the order
    /// the descriptor yields them.
    pub fields: Vec<String>,
    /// Typed field details for workflow-oriented clients.
    pub field_specs: Vec<FieldSummary>,
}

/// Agent-facing summary of one concept field.
#[derive(Debug, Clone)]
pub struct FieldSummary {
    /// Flag name accepted by `tonk assert`.
    pub name: String,
    /// Asserted-notation value type.
    pub value_type: String,
    /// `one` or `many`.
    pub cardinality: String,
    /// Whether minting a new instance requires this field.
    pub required: bool,
    /// Human description from the attribute descriptor.
    pub description: String,
}

/// Enumerate every user-defined concept on the meta branch,
/// returning a slim per-concept summary. Built-in concepts
/// (`attribute`, `concept`, etc.) are filtered out — see
/// [`is_builtin_concept`].
pub async fn list_concepts(site: &TonkSite) -> Result<Vec<ConceptSummary>> {
    let infos = enumerate_concepts(site).await?;
    Ok(infos
        .into_iter()
        .filter_map(|info| {
            let name = info.name.clone()?;
            Some((name, info))
        })
        .map(|(name, info)| ConceptSummary {
            name,
            entity: info.entity,
            description: info.description,
            fields: info
                .descriptor
                .with()
                .iter()
                .map(|(field, _)| field.to_string())
                .collect(),
            field_specs: info
                .descriptor
                .with()
                .iter()
                .map(|(field, descriptor)| FieldSummary {
                    name: field.to_string(),
                    value_type: descriptor
                        .content_type()
                        .map(|value_type| type_to_notation(&value_type).to_ascii_lowercase())
                        .unwrap_or_else(|| "value".to_string()),
                    cardinality: match descriptor.cardinality() {
                        Cardinality::One => "one",
                        Cardinality::Many => "many",
                    }
                    .to_string(),
                    required: !descriptor.is_optional(),
                    description: descriptor.description().to_string(),
                })
                .collect(),
        })
        .collect())
}

/// Render the site's full schema as a re-submittable notation
/// document. Output is a sequence of `attribute! …:` heads
/// followed by `concept! …:` heads — attributes first so concept
/// `with:` references resolve in document scope.
pub async fn render(site: &TonkSite) -> Result<String> {
    let attrs = enumerate_attributes(site).await?;
    let concepts = enumerate_concepts(site).await?;

    // URI → bookmark name, used to render `with: { field: name }`
    // when a referenced attribute has a published name.
    let uri_to_name: HashMap<String, String> = attrs
        .iter()
        .filter_map(|a| a.name.as_ref().map(|n| (a.the.clone(), n.clone())))
        .collect();

    let mut out = String::new();
    for attr in &attrs {
        render_attribute(&mut out, attr);
    }
    for concept in &concepts {
        render_concept(&mut out, concept, &uri_to_name);
    }
    Ok(out)
}

/// Render every rule installed on the branch — inductive and
/// deductive — as `rule!:` notation, appended after the schema the
/// rules' concept references resolve against.
///
/// A rule that cannot be expressed in notation (a `reduce` fold, a
/// foreign body, a concept with no published name) is emitted as a
/// comment naming the rule entity and the reason, so the document
/// stays honest about what it dropped.
pub async fn render_rules(site: &TonkSite) -> Result<String> {
    let rules = enumerate_rules(site).await?;
    let mut names = BranchNames::load(site).await?;
    let mut out = String::new();
    for (entity, rule) in &rules {
        match rule_notation::render_rule(rule, &mut names) {
            Ok(block) => {
                out.push_str(&block);
                out.push('\n');
            }
            Err(reason) => {
                let _ = writeln!(
                    out,
                    "# rule {entity}: not expressible in notation: {reason}\n"
                );
            }
        }
    }
    Ok(out)
}

/// Render one named concept's schema subset — the `attribute!:`
/// declarations it references followed by its `concept!:` block —
/// in the same re-submittable notation as [`render`]. Returns
/// `Ok(None)` when no user concept has that name.
pub async fn render_one(site: &TonkSite, name: &str) -> Result<Option<String>> {
    let attrs = enumerate_attributes(site).await?;
    let concepts = enumerate_concepts(site).await?;
    let Some(concept) = concepts.iter().find(|c| c.name.as_deref() == Some(name)) else {
        return Ok(None);
    };
    let uri_to_name: HashMap<String, String> = attrs
        .iter()
        .filter_map(|a| a.name.as_ref().map(|n| (a.the.clone(), n.clone())))
        .collect();
    let referenced: std::collections::HashSet<String> = concept
        .descriptor
        .with()
        .iter()
        .map(|(_, ad)| ad.the().to_string())
        .collect();
    let mut out = String::new();
    for attr in attrs.iter().filter(|a| referenced.contains(&a.the)) {
        render_attribute(&mut out, attr);
    }
    render_concept(&mut out, concept, &uri_to_name);
    Ok(Some(out))
}

// ---------------------------------------------------------------- //
// Data shapes                                                      //
// ---------------------------------------------------------------- //

#[derive(Debug)]
struct AttributeInfo {
    /// Bookmark name (the `<n>` from an `id:<n>` `db.name/referent`
    /// claim pointing at this entity), when one is published.
    name: Option<String>,
    /// Attribute URI (`xyz.tonk.task/title`).
    the: String,
    /// Value type tag (`Text`, `UnsignedInteger`, `Boolean`,
    /// `Entity`, …) — the same string the analyzer accepts as
    /// `as:`. Empty if the attribute carries no type constraint.
    type_name: String,
    /// `one` / `many`, matching the analyzer's accepted values.
    cardinality: String,
    /// Human description claim (`db.meta/description`).
    description: String,
}

/// A single concept's schema — fields, types, cardinalities,
/// and description — as read off the branch.
#[derive(Debug)]
pub struct ConceptInfo {
    /// Bookmark name, when one is published.
    pub name: Option<String>,
    /// Entity identifier of the concept descriptor.
    pub entity: String,
    /// `db.meta/description` when present.
    pub description: Option<String>,
    /// Decoded descriptor — the source of truth for the rendered
    /// `with:`/`maybe:` maps.
    pub descriptor: ConceptDescriptor,
    /// `dialog.concept/transient` marker — `true` means this is a
    /// command and re-emits as `command!:`.
    pub transient: bool,
}

/// Find a single user-defined concept by its bookmark name,
/// returning its full descriptor (fields, types, cardinalities,
/// descriptions) or `None`. Built-in concepts are excluded, matching
/// `enumerate_concepts`.
pub async fn find_concept(site: &TonkSite, name: &str) -> Result<Option<ConceptInfo>> {
    Ok(enumerate_concepts(site)
        .await?
        .into_iter()
        .find(|c| c.name.as_deref() == Some(name)))
}

// ---------------------------------------------------------------- //
// Attribute enumeration                                            //
// ---------------------------------------------------------------- //

/// Run the built-in `attribute` query plus a name-claim lookup
/// and merge the two by entity.
async fn enumerate_attributes(site: &TonkSite) -> Result<Vec<AttributeInfo>> {
    const QUERY: &str = r#"attribute:
  this:        ?a
  id:          ?the
  type:        ?type
  cardinality: ?card
  description: ?desc
"#;
    let response = run_query(site, QUERY).await?;
    let names = name_claims_by_entity(site).await?;

    let block = expect_block(&response, "attribute")?;
    let mut out: Vec<AttributeInfo> = Vec::with_capacity(block.results.len());
    for row in &block.results {
        let entity: Entity = row
            .this
            .parse()
            .with_context(|| format!("attribute row had unparseable entity: {}", row.this))?;
        let the = take_string(&row.fields, "id");
        // Runtime-owned vocabulary (`dialog.*` replica bindings,
        // `db.*` schema bookkeeping) is regenerated by the
        // destination branch and often carries no description,
        // which a re-submitted `attribute!:` block must have.
        // Concepts referencing one of these render it inline from
        // their own descriptor instead.
        if the.starts_with("dialog.") || the.starts_with("db.") {
            continue;
        }
        out.push(AttributeInfo {
            name: names.get(&entity).cloned(),
            the,
            type_name: take_string(&row.fields, "type"),
            cardinality: take_string(&row.fields, "cardinality"),
            description: take_string(&row.fields, "description"),
        });
    }
    out.sort_by(|a, b| {
        // Named attrs first (alphabetical), anonymous after (by URI).
        match (&a.name, &b.name) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.the.cmp(&b.the),
        }
    });
    Ok(out)
}

/// Pull every name-publication claim and return a `target →
/// name` map. Used to recover bookmark names for attributes
/// (the built-in `attribute` concept descriptor doesn't carry
/// `name`).
///
/// Names are stored inverted under `db.name/referent`: each
/// anchor `&foo` publishes `(db.name/referent, id:foo,
/// <target-entity>)`. The *name* lives in the claim's subject as
/// `id:<name>`; the *target* is the value. We invert that mapping
/// here so callers can ask "what's this entity's display name?"
/// in one lookup.
async fn name_claims_by_entity(site: &TonkSite) -> Result<HashMap<Entity, String>> {
    let name_attr: dialog_artifacts::Attribute = "db.name/referent"
        .parse()
        .context("db.name/referent should be a valid attribute URI")?;
    let the_term: attribute::The = name_attr.into();
    let session = site.branch().await?;
    let claims: Vec<dialog_query::Claim> = session
        .handle()
        .query()
        .select(AttributeQuery::new(
            Term::from(the_term),
            Term::<Entity>::var("of"),
            Term::<dialog_query::Any>::var("is"),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|e| anyhow!("db.name/referent query failed: {e:?}"))?;

    let mut out = HashMap::with_capacity(claims.len());
    for claim in claims {
        let Some(name) = claim.of.to_string().strip_prefix("id:").map(str::to_owned) else {
            continue;
        };
        if let dialog_artifacts::Value::Entity(target) = claim.is {
            out.insert(target, name);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- //
// Concept enumeration                                              //
// ---------------------------------------------------------------- //

async fn enumerate_concepts(site: &TonkSite) -> Result<Vec<ConceptInfo>> {
    const QUERY: &str = r#"concept:
  this:      ?c
  concept:   ?cc
  name:      ?name
  source:    ?source
  transient: ?transient
"#;
    let response = run_query(site, QUERY).await?;
    let block = expect_block(&response, "concept")?;
    // Built-in concepts are folded into the query results from the
    // registry; recognize them by their well-known entities rather
    // than a name list, so a registry addition can't leak into the
    // rendered document (the `command` sentinel once did).
    let builtin_entities: std::collections::HashSet<String> =
        tonk_schema::builtin::concept_registry()
            .iter()
            .map(|(_, definition)| definition.entity.to_string())
            .collect();
    let mut out: Vec<ConceptInfo> = Vec::with_capacity(block.results.len());
    for row in &block.results {
        if builtin_entities.contains(&row.this) {
            continue;
        }
        let source = match row.fields.get("source") {
            Some(Json::String(s)) => s.clone(),
            _ => continue, // skip rows lacking a parsable source
        };
        let descriptor: ConceptDescriptor = serde_json::from_str(&source).with_context(|| {
            format!(
                "failed to deserialize concept source for entity {}",
                row.this
            )
        })?;
        let name = match row.fields.get("name") {
            Some(Json::String(s)) => Some(s.clone()),
            _ => None,
        };
        // Built-in concepts are baked into the analyzer's
        // registry — every branch already resolves them without
        // needing them in the document. Skipping them keeps the
        // emitted schema portable to a fresh branch (no
        // duplicate-shadow attempts) and short.
        if let Some(name) = &name
            && is_builtin_concept(name)
        {
            continue;
        }
        let transient = matches!(row.fields.get("transient"), Some(Json::Bool(true)));
        // The query layer folds the concept's `db.meta/description`
        // claim into the synthesised descriptor, so the descriptor
        // is the read path for it.
        let description = descriptor
            .description()
            .filter(|d| !d.is_empty())
            .map(str::to_owned);
        out.push(ConceptInfo {
            name,
            entity: row.this.to_string(),
            description,
            descriptor,
            transient,
        });
    }
    out.sort_by(|a, b| match (&a.name, &b.name) {
        // Named concepts first (alphabetical), anonymous after
        // (by entity), matching the attribute ordering.
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.entity.cmp(&b.entity),
    });
    Ok(out)
}

/// Render every view instance on the branch as `view!:` blocks.
///
/// A view is ordinary data — three claims of the stdlib `view`
/// concept (`model`, `display`) — so unlike concepts and rules it
/// needs no descriptor reconstruction, just enumeration. The
/// stored `this` is emitted verbatim: most stdlib views pin
/// entities like `id:tonk:blank/view` that an anchor-derived
/// `id:{anchor}` would silently move.
pub async fn render_views(site: &TonkSite) -> Result<String> {
    const QUERY: &str = r#"view:
  this:    ?v
  model:   ?model
  display: ?display
"#;
    let response = run_query(site, QUERY).await?;
    let block = expect_block(&response, "view")?;
    let names = name_claims_by_entity(site).await?;

    let mut views: Vec<(String, Option<String>, String, String)> = Vec::new();
    for row in &block.results {
        let model = take_string(&row.fields, "model");
        let display = take_string(&row.fields, "display");
        let name = row
            .this
            .parse::<Entity>()
            .ok()
            .and_then(|entity| names.get(&entity).cloned());
        views.push((row.this.clone(), name, model, display));
    }
    // Named views first (alphabetical), anonymous after (by
    // entity), matching the attribute and concept ordering.
    views.sort_by(|a, b| match (&a.1, &b.1) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });

    let mut out = String::new();
    for (this, name, model, display) in &views {
        match name {
            Some(name) => {
                let _ = writeln!(out, "view!: &{name}");
            }
            None => out.push_str("view!:\n"),
        }
        let _ = writeln!(out, "  this: {this}");
        let _ = writeln!(out, "  model: {model}");
        out.push_str("  display: |\n");
        for line in display.lines() {
            let _ = writeln!(out, "    {line}");
        }
        out.push('\n');
    }
    Ok(out)
}

// ---------------------------------------------------------------- //
// Rule enumeration                                                 //
// ---------------------------------------------------------------- //

/// Read every rule stored on the branch, both kinds, sorted by
/// entity for stable output.
///
/// This is the seam a dialog-native enumeration API will replace:
/// it sweeps the `dialog.rule/source` claims directly and decodes
/// each canonical body, verifying the content address the way
/// dialog's own readers do — an entry whose body does not hash
/// back to its entity is inert and skipped.
pub async fn enumerate_rules(site: &TonkSite) -> Result<Vec<(Entity, Rule)>> {
    let the: attribute::The = "dialog.rule/source"
        .parse()
        .expect("`dialog.rule/source` is a valid attribute URI");
    let session = site.branch().await?;
    let claims: Vec<dialog_query::Claim> = session
        .handle()
        .query()
        .select(AttributeQuery::new(
            Term::from(the),
            Term::<Entity>::var("of"),
            Term::<Vec<u8>>::var("is").into(),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|e| anyhow!("dialog.rule/source query failed: {e:?}"))?;

    let mut out: Vec<(Entity, Rule)> = Vec::with_capacity(claims.len());
    for claim in claims {
        let dialog_artifacts::Value::Bytes(source) = claim.is else {
            continue;
        };
        // The two kinds share the source attribute; the decoded
        // descriptor's head field decides which one this is.
        let rule = if let Ok(rule) = InductiveRule::decode(&source) {
            Rule::Inductive(rule)
        } else if let Ok(rule) = DeductiveRule::decode(&source) {
            Rule::Deductive(rule)
        } else {
            continue;
        };
        let verified = match &rule {
            Rule::Inductive(rule) => rule.this() == claim.of,
            Rule::Deductive(rule) => rule.this() == claim.of,
        };
        if !verified {
            continue;
        }
        out.push((claim.of, rule));
    }
    out.sort_by_key(|(entity, _)| entity.to_string());
    Ok(out)
}

/// Concept-name table for rule rendering: the branch's published
/// concepts plus the analyzer's built-in registry, matched by
/// descriptor serde form as [`rule_notation::ConceptNames`]
/// requires.
struct BranchNames {
    entries: Vec<(Json, String)>,
}

impl BranchNames {
    async fn load(site: &TonkSite) -> Result<Self> {
        let mut entries = Vec::new();
        for concept in enumerate_concepts(site).await? {
            let Some(name) = concept.name else {
                continue;
            };
            entries.push((serde_json::to_value(&concept.descriptor)?, name));
        }
        for (name, definition) in tonk_schema::builtin::concept_registry() {
            entries.push((
                serde_json::to_value(definition.descriptor.concept())?,
                (*name).to_string(),
            ));
        }
        Ok(Self { entries })
    }
}

impl rule_notation::ConceptNames for BranchNames {
    fn reference(
        &mut self,
        descriptor: &ConceptDescriptor,
    ) -> Result<String, rule_notation::RenderRuleError> {
        let form = serde_json::to_value(descriptor)
            .map_err(|e| rule_notation::RenderRuleError::Serialize(e.to_string()))?;
        // Exact serde-form match first.
        if let Some((_, name)) = self
            .entries
            .iter()
            .find(|(candidate, _)| *candidate == form)
        {
            return Ok(name.clone());
        }
        // Analysis narrows an embedded descriptor to the fields
        // the premise actually binds, so a rule that uses a
        // subset of a concept's fields embeds a subset
        // descriptor. Re-analysis re-narrows the same way from
        // the same `where:` keys, so a name resolves when
        // exactly one concept subsumes the narrowed form: every
        // narrowed field present with a structurally identical
        // field descriptor. Descriptions are ignored — an
        // attribute's description is one global claim, so prose
        // captured at rule-authoring time can drift from what the
        // branch now says; refusing such a rule would trade its
        // behavior away to preserve prose. A drift-affected rule
        // re-derives under a new content address with the
        // branch's current metadata.
        // Several concepts can subsume the same narrowed form
        // (`invitation` ⊂ `tonk/agent-invite` share four
        // attributes); the tightest fit — fewest fields, then
        // name for determinism — is the reference the narrowing
        // most plausibly came from.
        let mut matches: Vec<(usize, &String)> = self
            .entries
            .iter()
            .filter(|(candidate, _)| subsumes(candidate, &form))
            .map(|(candidate, name)| {
                let fields = candidate
                    .get("with")
                    .and_then(Json::as_object)
                    .map(|with| with.len())
                    .unwrap_or(usize::MAX);
                (fields, name)
            })
            .collect();
        matches.sort();
        match matches.first() {
            Some((_, name)) => Ok((*name).clone()),
            None => Err(rule_notation::RenderRuleError::UnresolvedConcept {
                concept: form.to_string(),
            }),
        }
    }
}

/// `true` when the `full` concept descriptor (serde form) subsumes
/// the `narrowed` one: every narrowed field present in full with a
/// field descriptor identical up to descriptions.
fn subsumes(full: &Json, narrowed: &Json) -> bool {
    let (Some(Json::Object(full_with)), Some(Json::Object(narrowed_with))) =
        (full.get("with"), narrowed.get("with"))
    else {
        return false;
    };
    !narrowed_with.is_empty()
        && narrowed_with.iter().all(|(field, descriptor)| {
            full_with.get(field).is_some_and(|candidate| {
                sans_description(candidate) == sans_description(descriptor)
            })
        })
}

/// A field descriptor's serde form with the description removed —
/// the structural part that must match exactly for a name
/// reference to be sound.
fn sans_description(descriptor: &Json) -> Json {
    let mut stripped = descriptor.clone();
    if let Some(map) = stripped.as_object_mut() {
        map.remove("description");
    }
    stripped
}

/// Built-in concept names hard-coded in
/// `tonk_schema::builtin::concept_registry`. Documents can't
/// shadow a built-in (the registry wins on lookup), so re-emitting
/// them in `tonk schema` output would be both wasteful and
/// rejected — built-ins carry attributes without descriptions, and
/// the analyzer's `attribute!` validator requires non-empty
/// descriptions.
fn is_builtin_concept(name: &str) -> bool {
    matches!(
        name,
        "attribute"
            | "concept"
            | "name"
            | "rule"
            | "branch"
            | "replica"
            | "remote"
            | "tracking-branch"
    )
}

// ---------------------------------------------------------------- //
// Rendering                                                        //
// ---------------------------------------------------------------- //

fn render_attribute(out: &mut String, attr: &AttributeInfo) {
    let head = match &attr.name {
        Some(name) => format!("attribute!: &{name}"),
        None => "attribute!:".to_string(),
    };
    let _ = writeln!(out, "{head}");
    if !attr.description.is_empty() {
        let _ = writeln!(out, "  description: {}", quote_string(&attr.description));
    }
    if !attr.the.is_empty() {
        let _ = writeln!(out, "  the:         {}", attr.the);
    }
    if !attr.type_name.is_empty() {
        let _ = writeln!(out, "  as:          {}", attr.type_name);
    }
    if !attr.cardinality.is_empty() {
        let _ = writeln!(out, "  cardinality: {}", attr.cardinality);
    }
    out.push('\n');
}

fn render_concept(out: &mut String, concept: &ConceptInfo, uri_to_name: &HashMap<String, String>) {
    // A transient concept is a command; `command!:` derives the
    // same entity as `concept!:` + `transient:`, so the clearer
    // keyword is safe to prefer.
    let keyword = if concept.transient {
        "command!"
    } else {
        "concept!"
    };
    match &concept.name {
        Some(name) => {
            let _ = writeln!(out, "{keyword}: &{name}");
        }
        None => {
            let _ = writeln!(out, "{keyword}:");
        }
    }
    // A stored entity that differs from the descriptor's content
    // address was pinned at authoring time (`this: tonk:view`);
    // re-emit the pin so re-evaluation lands on the same entity.
    if concept.entity != concept.descriptor.this().to_string() {
        let _ = writeln!(out, "  this: {}", concept.entity);
    }
    if let Some(desc) = &concept.description {
        let _ = writeln!(out, "  description: {}", quote_string(desc));
    }
    let (required, optional): (Vec<_>, Vec<_>) = concept
        .descriptor
        .with()
        .iter()
        .partition(|(_, descriptor)| !descriptor.is_optional());
    for (block, fields) in [("with", required), ("maybe", optional)] {
        if fields.is_empty() {
            continue;
        }
        let _ = writeln!(out, "  {block}:");
        for (field, attr_descriptor) in fields {
            render_concept_field(out, field, attr_descriptor, uri_to_name);
        }
    }
    out.push('\n');
}

fn render_concept_field(
    out: &mut String,
    field: &str,
    attr_descriptor: &ConceptFieldDescriptor,
    uri_to_name: &HashMap<String, String>,
) {
    let uri = attr_descriptor.the().to_string();
    match uri_to_name.get(&uri) {
        // Named — use the bare-symbol reference; the
        // analyzer resolves it through the published name
        // table on the branch.
        Some(name) => {
            let _ = writeln!(out, "    {field}: {name}");
        }
        // Anonymous — emit the inline definition so the
        // re-submitted document carries enough information to
        // reconstruct the attribute. Uses `the:` URI plus the
        // type and cardinality.
        None => {
            let _ = writeln!(out, "    {field}:");
            let _ = writeln!(out, "      the:         {uri}");
            if let Some(t) = attr_descriptor.content_type() {
                let _ = writeln!(out, "      as:          {}", type_to_notation(&t));
            }
            let card = match attr_descriptor.cardinality() {
                Cardinality::One => "one",
                Cardinality::Many => "many",
            };
            let _ = writeln!(out, "      cardinality: {card}");
            let desc = attr_descriptor.description();
            if !desc.is_empty() {
                let _ = writeln!(out, "      description: {}", quote_string(desc));
            }
        }
    }
}

/// Map a dialog `Type` onto the string accepted by the analyzer
/// in `as:` slots. The serde rename on `ValueDataType` already
/// produces these strings; serializing through serde_json gives
/// us a quoted form (`"\"Text\""`) so we trim the quotes.
pub(crate) fn type_to_notation(ty: &Type) -> String {
    match serde_json::to_string(ty) {
        Ok(s) => s.trim_matches('"').to_string(),
        Err(_) => format!("{ty:?}"),
    }
}

fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------- //
// Eval-driven query helpers                                        //
// ---------------------------------------------------------------- //

async fn run_query(site: &TonkSite, doc: &str) -> Result<EvaluateResponse> {
    let parsed = parse(doc);
    let syntax = parsed
        .syntax
        .ok_or_else(|| anyhow!("internal tonk-schema query failed to parse: {doc}"))?;
    if !parsed.diagnostics.is_empty() {
        return Err(anyhow!(
            "internal tonk-schema query produced diagnostics: {:?}",
            parsed
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        ));
    }
    let session = site.branch().await?;
    let branch = session.handle();
    let revision = branch.revision();
    let evaluated = syntax
        .evaluate(branch.transaction())
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow!("tonk-schema query failed: {e}"))?;
    // schema-internal docs are pure-query; nothing is committed
    // and the txn is dropped here. before == after.
    Ok(EvaluateResponse {
        revision_before: revision.clone(),
        revision_after: revision,
        matches_before: evaluated.matches.clone(),
        matches_after: evaluated.matches,
        commits: evaluated.commits,
    })
}

/// Extract the matches block whose source-expression head label
/// matches `label`. Schema queries always issue a single named
/// expression, so finding the right block is straightforward.
fn expect_block<'a>(response: &'a EvaluateResponse, label: &str) -> Result<&'a QueryMatchBlock> {
    response
        .matches_after
        .iter()
        .find(|b| b.label == label)
        .ok_or_else(|| anyhow!("expected `{label}` block in matches_after"))
}

fn take_string(fields: &BTreeMap<String, Json>, key: &str) -> String {
    match fields.get(key) {
        Some(Json::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}
