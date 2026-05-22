//! `slide schema` — read every attribute and concept asserted on
//! the branch, emit a notation document that re-submits cleanly.
//!
//! Two query passes:
//!
//! 1. The built-in `attribute` concept enumerates every attribute
//!    (named or not). For each, a separate lookup against
//!    `dialog.name/referent` recovers the bookmark name where one
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
//! Known fidelity gap: `dialog.meta/description` claims on
//! *concepts* are not surfaced. The dialog query engine's
//! anonymous-concept dispatch path (which the `concept:` head
//! lands on) only binds `this`, `name`, and a synthesised
//! `source`; reconstructed descriptors set `description: None`.
//! Concept descriptions are optional in the analyzer, so the
//! emitted schema still re-submits cleanly — only the prose is
//! lost. Attribute descriptions round-trip because the
//! `attribute:` query returns the underlying claim directly.

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

use crate::site::SlideSite;

/// Slim summary of one named concept on the branch — just enough
/// for `slide concepts` to print and for `slide share concept`
/// to verify existence. The full descriptor stays internal to
/// [`render`] (which needs it to re-emit the `with:` map as
/// notation).
#[derive(Debug, Clone)]
pub struct ConceptSummary {
    /// Bookmark name published via `dialog.name/referent` on
    /// the matching `id:<name>` entity.
    pub name: String,
    /// Human description claim (`dialog.meta/description`), if
    /// asserted. Concept descriptions are optional in the
    /// analyzer.
    pub description: Option<String>,
    /// Field names from the concept's `with:` map, in the order
    /// the descriptor yields them.
    pub fields: Vec<String>,
}

/// Enumerate every user-defined concept on the meta branch,
/// returning a slim per-concept summary. Built-in concepts
/// (`attribute`, `concept`, etc.) are filtered out — see
/// [`is_builtin_concept`].
pub async fn list_concepts(site: &SlideSite) -> Result<Vec<ConceptSummary>> {
    let infos = enumerate_concepts(site).await?;
    Ok(infos
        .into_iter()
        .map(|info| ConceptSummary {
            name: info.name,
            description: info.description,
            fields: info
                .descriptor
                .with()
                .iter()
                .map(|(field, _)| field.to_string())
                .collect(),
        })
        .collect())
}

/// Render the site's full schema as a re-submittable notation
/// document. Output is a sequence of `attribute! …:` heads
/// followed by `concept! …:` heads — attributes first so concept
/// `with:` references resolve in document scope.
pub async fn render(site: &SlideSite) -> Result<String> {
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

// ---------------------------------------------------------------- //
// Data shapes                                                      //
// ---------------------------------------------------------------- //

#[derive(Debug)]
struct AttributeInfo {
    /// Bookmark name (the `<n>` from an `id:<n>` `dialog.name/referent`
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
    /// Human description claim (`dialog.meta/description`).
    description: String,
}

#[derive(Debug)]
struct ConceptInfo {
    /// Bookmark name. Required — anonymous concepts aren't yet
    /// surfaced.
    name: String,
    /// `dialog.meta/description` when present.
    description: Option<String>,
    /// Decoded descriptor — the source of truth for the rendered
    /// `with:` map.
    descriptor: ConceptDescriptor,
}

// ---------------------------------------------------------------- //
// Attribute enumeration                                            //
// ---------------------------------------------------------------- //

/// Run the built-in `attribute` query plus a name-claim lookup
/// and merge the two by entity.
async fn enumerate_attributes(site: &SlideSite) -> Result<Vec<AttributeInfo>> {
    const QUERY: &str = "\
attribute:
  this:        ?a
  id:          ?the
  type:        ?type
  cardinality: ?card
  description: ?desc
";
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
/// Names are stored inverted under `dialog.name/referent`: each
/// anchor `&foo` publishes `(dialog.name/referent, id:foo,
/// <target-entity>)`. The *name* lives in the claim's subject as
/// `id:<name>`; the *target* is the value. We invert that mapping
/// here so callers can ask "what's this entity's display name?"
/// in one lookup.
async fn name_claims_by_entity(site: &SlideSite) -> Result<HashMap<Entity, String>> {
    let name_attr: dialog_artifacts::Attribute = "dialog.name/referent"
        .parse()
        .context("dialog.name/referent should be a valid attribute URI")?;
    let the_term: attribute::The = name_attr.into();
    let claims: Vec<dialog_query::Claim> = site
        .branch
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
        .map_err(|e| anyhow!("dialog.name/referent query failed: {e:?}"))?;

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

async fn enumerate_concepts(site: &SlideSite) -> Result<Vec<ConceptInfo>> {
    const QUERY: &str = "\
concept:
  this:        ?c
  concept:     ?cc
  name:        ?name
  description: ?desc
  source:      ?source
";
    let response = run_query(site, QUERY).await?;
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
        let description = match row.fields.get("description") {
            Some(Json::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        out.push(ConceptInfo {
            name,
            description,
            descriptor,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Built-in concept names hard-coded in
/// `tonk_schema::builtin::concept_registry`. Documents can't
/// shadow a built-in (the registry wins on lookup), so re-emitting
/// them in `slide schema` output would be both wasteful and
/// rejected — built-ins carry attributes without descriptions, and
/// the analyzer's `attribute!` validator requires non-empty
/// descriptions.
fn is_builtin_concept(name: &str) -> bool {
    matches!(
        name,
        "attribute" | "concept" | "name" | "branch" | "replica" | "remote" | "tracking-branch"
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
            // type and cardinality.
            None => {
                let _ = writeln!(out, "    {field}:");
                let _ = writeln!(out, "      the:         {uri}");
                if let Some(t) = attr_descriptor.content_type() {
                    let _ = writeln!(out, "      as:          {}", type_to_notation(t));
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
    out.push('\n');
}

/// Map a dialog `Type` onto the string accepted by the analyzer
/// in `as:` slots. The serde rename on `ValueDataType` already
/// produces these strings; serializing through serde_json gives
/// us a quoted form (`"\"Text\""`) so we trim the quotes.
fn type_to_notation(ty: Type) -> String {
    match serde_json::to_string(&ty) {
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

async fn run_query(site: &SlideSite, doc: &str) -> Result<EvaluateResponse> {
    let parsed = parse(doc);
    let syntax = parsed
        .syntax
        .ok_or_else(|| anyhow!("internal slide-schema query failed to parse: {doc}"))?;
    if !parsed.diagnostics.is_empty() {
        return Err(anyhow!(
            "internal slide-schema query produced diagnostics: {:?}",
            parsed
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        ));
    }
    let revision = site.branch.revision();
    let evaluated = syntax
        .evaluate(site.branch.transaction())
        .perform(&site.branch, &site.operator)
        .await
        .map_err(|e| anyhow!("slide-schema query failed: {e}"))?;
    // schema-internal docs are pure-query; the chain didn't
    // commit, so before == after.
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
