//! `tonk show --notation` — read every attribute and concept asserted on
//! the branch, emit a notation document that re-submits cleanly.
//!
//! Two query passes:
//!
//! 1. The built-in `attribute` concept enumerates every attribute
//!    (named or not). For each, a separate lookup against
//!    `db.name/referent` recovers the bookmark name where one
//!    exists.
//! 2. The built-in `concept` concept enumerates every concept
//!    *with* a name claim — the concept-of-concept descriptor
//!    requires a `name` field, so anonymous concepts fall through.
//!    Each row's `source` is the JSON-encoded
//!    [`ConceptDescriptor`], deserialized to recover the full
//!    `with:` map.
//!
//! Anonymous attribute and concept emission is intentionally
//! out of scope — the typical workflow names everything via
//! bookmark form, and adding the URI-binding round-trip path
//! would multiply the test surface for a corner case we haven't
//! seen in real schemas yet.
//!
//! The reconstructed `concept:` source may omit its description. A separate
//! batched metadata read recovers stored descriptions without excluding concepts
//! that have none. Attribute descriptions come from the attribute query itself.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use anyhow::{Context, Result, anyhow};
use dialog_artifacts::Entity;
use dialog_query::{
    AttributeQuery, Cardinality, ConceptDescriptor, Output as _, Term, Type, attribute,
};
use serde_json::Value as Json;
use tonk_evaluator::evaluate::{QueryMatchBlock, SyntaxEvaluateExt};
use tonk_notation::parse;

use crate::output::EvaluateResponse;

use crate::site::TonkSite;

/// Slim summary of one named concept on the branch — just enough
/// for `tonk concept` to print. The full descriptor stays internal
/// to [`render`] (which needs it to re-emit the `with:` map as
/// notation).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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

/// Enumerate the concepts this space's author defined, returning a
/// slim per-concept summary. Runtime concepts — analyzer built-ins,
/// the seeded standard library, the CLI's own space-home recipe — are
/// filtered out; see [`is_system_concept`].
///
/// The filter is presentational, not a scoping rule: a system concept
/// is still addressable by name through [`find_concept`], so
/// `tonk query member` keeps working while `tonk concept` stays
/// about the space's own vocabulary.
pub async fn list_concepts(site: &TonkSite) -> Result<Vec<ConceptSummary>> {
    let mut concepts = list_all_concepts(site).await?;
    concepts.retain(|concept| !is_system_concept(&concept.name));
    Ok(concepts)
}

/// Every concept on the branch bar the analyzer's built-ins: the
/// author's own, plus the runtime vocabulary the standard library
/// seeds. `command` is the one built-in kept — see
/// [`is_builtin_concept`] for why the rest are dropped here.
///
/// For callers that need to answer a question about a system concept
/// and list the author's in the same pass — `tonk status` reads the
/// `tonk/agents` claim and prints the author's concepts, and would
/// otherwise enumerate the branch twice.
pub async fn list_all_concepts(site: &TonkSite) -> Result<Vec<ConceptSummary>> {
    let infos = enumerate_concepts(site).await?;
    Ok(infos
        .into_iter()
        .map(|info| ConceptSummary {
            name: info.name,
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

/// Compact schema orientation, without reading any concept's instances.
/// Full field descriptions and attribute identities belong to named `show`
/// and `show --notation`; `--all` expands the overview's concepts and fields.
pub fn render_overview(space: &str, concepts: &[ConceptSummary], all: bool) -> String {
    const CONCEPT_LIMIT: usize = 20;
    const FIELD_LIMIT: usize = 8;
    const DESCRIPTION_LIMIT: usize = 80;

    let mut visible: Vec<_> = concepts
        .iter()
        .filter(|concept| all || !is_system_concept(&concept.name))
        .collect();
    visible.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = format!("Space: {space}\n\nConcepts ({})\n", visible.len());
    let concept_limit = if all { usize::MAX } else { CONCEPT_LIMIT };
    let field_limit = if all { usize::MAX } else { FIELD_LIMIT };
    for concept in visible.iter().take(concept_limit) {
        let _ = write!(out, "  {}  ", concept.name);
        for (index, field) in concept.field_specs.iter().take(field_limit).enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            let many = if field.cardinality == "many" {
                "[]"
            } else {
                ""
            };
            let optional = if field.required { "" } else { "?" };
            let _ = write!(out, "{}: {}{many}{optional}", field.name, field.value_type);
        }
        if concept.field_specs.is_empty() {
            out.push_str("(no fields)");
        }
        if concept.field_specs.len() > field_limit {
            let _ = write!(
                out,
                ", ... {} more fields (tonk show {})",
                concept.field_specs.len() - field_limit,
                concept.name
            );
        }
        if let Some(description) = &concept.description {
            let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
            if !description.is_empty() {
                out.push_str(" — ");
                out.extend(description.chars().take(DESCRIPTION_LIMIT));
                if description.chars().count() > DESCRIPTION_LIMIT {
                    out.push_str("...");
                }
            }
        }
        out.push('\n');
    }
    if visible.is_empty() {
        out.push_str("  No application concepts. Define one with tonk concept add.\n");
    }
    if visible.len() > concept_limit {
        let _ = writeln!(
            out,
            "  {} more concepts; use tonk show --all to see them.",
            visible.len() - concept_limit
        );
    }
    out.push_str("\n? = optional; [] = many\n\nInspect instances:  tonk query <concept> [--where FIELD=VALUE]\nInspect schema:     tonk show <concept>\nUpdate an instance: tonk assert <concept> <entity> --<field> <value>\nVerify:             check the write receipt; verified means a fresh local read matched.\nIf not verified:    tonk show <concept> <entity>\n");
    if !all && visible.len() < concepts.len() {
        out.push_str("\nRuntime concepts omitted; use tonk show --all to include them.\n");
    }
    out.push_str("Full schema export: tonk show --notation\n");
    out
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

/// Render one named concept's schema subset — the `attribute!:`
/// declarations it references followed by its `concept!:` block —
/// in the same re-submittable notation as [`render`]. Returns
/// `Ok(None)` when no user concept has that name.
pub async fn render_one(site: &TonkSite, name: &str) -> Result<Option<String>> {
    let attrs = enumerate_attributes(site).await?;
    let concepts = enumerate_concepts(site).await?;
    let Some(concept) = concepts.iter().find(|c| c.name == name) else {
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

/// A single named concept's schema — fields, types, cardinalities,
/// and description — as read off the branch.
#[derive(Debug)]
pub struct ConceptInfo {
    /// Bookmark name. Required — anonymous concepts aren't yet
    /// surfaced.
    pub name: String,
    /// Entity identifier of the concept descriptor.
    pub entity: String,
    /// `db.meta/description` when present.
    pub description: Option<String>,
    /// Decoded descriptor — the source of truth for the rendered
    /// `with:` map.
    pub descriptor: ConceptDescriptor,
}

/// Find a single user-defined concept by its bookmark name,
/// returning its full descriptor (fields, types, cardinalities,
/// descriptions) or `None`. Built-in concepts are excluded, matching
/// `enumerate_concepts`.
pub async fn find_concept(site: &TonkSite, name: &str) -> Result<Option<ConceptInfo>> {
    Ok(enumerate_concepts(site)
        .await?
        .into_iter()
        .find(|c| c.name == name))
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
  as:          ?type
  pick:        ?pick
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
        out.push(AttributeInfo {
            name: names.get(&entity).cloned(),
            the: take_string(&row.fields, "id"),
            type_name: {
                let wire = take_string(&row.fields, "as");
                tonk_notation::ValueType::from_wire(&wire)
                    .map(|kind| kind.anchor().to_owned())
                    .unwrap_or(wire)
            },
            cardinality: take_string(&row.fields, "pick"),
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
    let name_attr: dialog_artifacts::Relation = "db.name/referent"
        .parse()
        .context("db.name/referent should be a valid attribute URI")?;
    let the_term: attribute::Relation = name_attr.into();
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
  this:        ?c
  concept:     ?cc
  name:        ?name
  description: ?desc
  source:      ?source
"#;
    let response = run_query(site, QUERY).await?;
    let descriptions = descriptions_by_entity(site).await?;
    let block = expect_block(&response, "concept")?;
    let mut out: Vec<ConceptInfo> = Vec::with_capacity(block.results.len());
    for row in &block.results {
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
            Some(Json::String(s)) => s.clone(),
            _ => continue,
        };
        // Built-in concepts are baked into the analyzer's
        // registry — every branch already resolves them without
        // needing them in the document. Skipping them keeps the
        // emitted schema portable to a fresh branch (no
        // duplicate-shadow attempts) and short.
        if is_builtin_concept(&name) {
            continue;
        }
        let description =
            descriptions
                .get(&row.this)
                .cloned()
                .or_else(|| match row.fields.get("description") {
                    Some(Json::String(s)) if !s.is_empty() => Some(s.clone()),
                    _ => None,
                });
        out.push(ConceptInfo {
            name,
            entity: row.this.to_string(),
            description,
            descriptor,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Read optional descriptions once rather than adding a required description
/// join (which would hide undescribed concepts) or a per-concept query.
async fn descriptions_by_entity(site: &TonkSite) -> Result<HashMap<String, String>> {
    let attribute: dialog_artifacts::Relation = "db.meta/description".parse()?;
    let session = site.branch().await?;
    let claims: Vec<dialog_query::Claim> = session
        .handle()
        .query()
        .select(AttributeQuery::new(
            Term::from(attribute::Relation::from(attribute)),
            Term::<Entity>::var("of"),
            Term::<dialog_query::Any>::var("is"),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|error| anyhow!("concept descriptions query failed: {error:?}"))?;
    Ok(claims
        .into_iter()
        .filter_map(|claim| {
            let description = String::try_from(claim.is).ok()?;
            (!description.is_empty()).then(|| (claim.of.to_string(), description))
        })
        .collect())
}

/// Built-in concept names hard-coded in
/// `tonk_schema::builtin::concept_registry`. Documents can't
/// shadow a built-in (the registry wins on lookup), so re-emitting
/// them in `tonk show --notation` output would be both wasteful and
/// rejected — built-ins carry attributes without descriptions, and
/// the analyzer's `attribute!` validator requires non-empty
/// descriptions.
///
/// Deliberately not the whole registry: `command` is a queryable view
/// over the branch's transient concepts, and dropping it here would
/// also drop it from [`find_concept`], breaking `tonk query command`.
/// [`is_system_concept`] reads the registry instead, because hiding a
/// name from a listing costs nothing.
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

/// Name of the concept `tonk space home` authors to key the space home.
/// Machinery for the home recipe, not vocabulary the author asserts
/// against, so it is filtered out of agent-facing listings alongside
/// the standard library.
pub const SPACE_HOME_CONCEPT: &str = "space-home";

/// Whether `name` belongs to the runtime rather than to this space's
/// author: an analyzer built-in, a standard-library concept or
/// command, or the CLI's own space-home recipe.
///
/// A fresh space carries forty-one of these. Listing them alongside
/// the one concept an agent just defined — and pasting the same list
/// into every "no concept named X" error — buries the answer, so
/// every agent-facing surface filters on this.
pub fn is_system_concept(name: &str) -> bool {
    tonk_schema::builtin::concept_registry()
        .iter()
        .any(|(builtin, _)| *builtin == name)
        || name == SPACE_HOME_CONCEPT
        || crate::site::standard_library_declares_concept(name)
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
    if !attr.cardinality.is_empty() && attr.cardinality != "last" {
        let _ = writeln!(out, "  pick:        {}", attr.cardinality);
    }
    out.push('\n');
}

fn render_concept(out: &mut String, concept: &ConceptInfo, uri_to_name: &HashMap<String, String>) {
    let _ = writeln!(out, "concept!: &{name}", name = concept.name);
    if let Some(desc) = &concept.description {
        let _ = writeln!(out, "  description: {}", quote_string(desc));
    }
    out.push_str("  with:\n");
    for (field, attr_descriptor) in concept.descriptor.with().iter() {
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
            // type, or the values it lists, and the pick.
            None => {
                let _ = writeln!(out, "    {field}:");
                let _ = writeln!(out, "      the:         {uri}");
                let among = attr_descriptor.descriptor().among();
                if !among.is_empty() {
                    let _ = writeln!(out, "      as:");
                    for value in among {
                        let _ = writeln!(out, "        - {}", value_to_notation(value));
                    }
                } else if let Some(t) = attr_descriptor.content_type() {
                    let _ = writeln!(out, "      as:          {}", type_to_notation(&t));
                }
                let pick = attr_descriptor.descriptor().pick();
                let implied = if among.is_empty() { "last" } else { "top" };
                if pick.name() != implied {
                    let _ = writeln!(out, "      pick:        {}", pick.name());
                }
                let desc = attr_descriptor.description();
                if !desc.is_empty() {
                    let _ = writeln!(out, "      description: {}", quote_string(desc));
                }
            }
        }
    }
    out.push('\n');
}

/// A listed value as the notation writes it: an entity as its URI, text
/// quoted, anything else as dialog spells it.
fn value_to_notation(value: &dialog_artifacts::Value) -> String {
    match value {
        dialog_artifacts::Value::Entity(entity) => entity.to_string(),
        dialog_artifacts::Value::String(text) => quote_string(text),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Map a dialog `Type` onto the built-in type anchor the analyzer
/// accepts in `as:` slots (`text`, `natural`, ...).
pub(crate) fn type_to_notation(ty: &Type) -> String {
    match tonk_notation::ValueType::from_wire(ty.uri()) {
        Some(kind) => kind.anchor().to_string(),
        None => format!("{ty:?}"),
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
