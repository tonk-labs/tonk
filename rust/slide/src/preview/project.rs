//! Live-data projection for previews. Reuses the *element's own*
//! query builders (`tonk_display::resolve`) so the conclusions a
//! preview renders are byte-identical to what `<tonk-display>`
//! subscribes to: name → referent via `name_query`, concept →
//! descriptor JSON via `phase1_query`, then `entity_query` pinning
//! `this` and projecting every descriptor field.

use anyhow::{Context as _, Result, anyhow};
use dialog_operator::Operator;
use dialog_query::{ConceptQuery, Output as _};
use dialog_repository::Branch;
use dialog_storage::provider::storage::NativeSpace;
use ipld_core::ipld::Ipld;
use tonk_display::resolve;
use tonk_schema::concept::QueryPlan;
use tonk_schema::conclusion::Conclusion;
use tonk_schema::query::Query;

/// What the preview renders against: the projected rows plus the
/// model's full field set (for unbound-field diagnostics even when
/// a field resolved empty).
#[derive(Debug)]
pub struct Projection {
    /// Projected conclusions for `(model, this)` — frame size 0
    /// (entity absent) or 1, exactly like the element's entity
    /// subscription.
    pub conclusions: Vec<Conclusion>,
    /// Field names declared by the model concept's descriptor.
    pub descriptor_fields: Vec<String>,
    /// The resolved subject entity URI.
    pub subject: String,
}

/// Project the live conclusions for `subject` under `model`.
/// `model` and `subject` each accept a bookmark name or an entity
/// URI (anything containing `:` is treated as a URI, matching
/// `<tonk-display>`'s convention).
pub async fn project_entity(
    branch: &Branch,
    operator: &Operator<NativeSpace>,
    model: &str,
    subject: &str,
) -> Result<Projection> {
    let model_uri = resolve_to_uri(branch, operator, model)
        .await?
        .ok_or_else(|| anyhow!("no concept named '{model}' on the branch"))?;
    let descriptor_json = descriptor_for(branch, operator, &model_uri)
        .await?
        .ok_or_else(|| {
            anyhow!("'{model}' resolved to {model_uri} but no concept descriptor was found")
        })?;

    let subject_uri = resolve_to_uri(branch, operator, subject)
        .await?
        .ok_or_else(|| anyhow!("no entity named '{subject}' on the branch"))?;

    let query = resolve::entity_query(&descriptor_json, &subject_uri)
        .context("failed to build entity query from descriptor")?;
    let conclusions = run_query(branch, operator, query).await?;

    Ok(Projection {
        conclusions,
        descriptor_fields: descriptor_fields(&descriptor_json)?,
        subject: subject_uri,
    })
}

/// Resolve a bookmark name to its referent URI via the Name
/// concept; URIs pass through verbatim.
async fn resolve_to_uri(
    branch: &Branch,
    operator: &Operator<NativeSpace>,
    name_or_uri: &str,
) -> Result<Option<String>> {
    if resolve::looks_like_uri(name_or_uri) {
        return Ok(Some(name_or_uri.to_string()));
    }
    let rows = run_query(branch, operator, resolve::name_query(name_or_uri)).await?;
    Ok(rows.first().and_then(|c| string_field(c, "entity")))
}

/// Fetch the concept's descriptor JSON (the `source` field of the
/// concept-of-concept row) for a concept entity URI.
async fn descriptor_for(
    branch: &Branch,
    operator: &Operator<NativeSpace>,
    concept_uri: &str,
) -> Result<Option<String>> {
    let parsed = resolve::parse_source(concept_uri);
    let rows = run_query(branch, operator, resolve::phase1_query(&parsed)).await?;
    Ok(rows.first().and_then(|c| string_field(c, "source")))
}

/// Execute a wire [`Query`] natively and project the rows the same
/// way the worker's query route does (see
/// `rust/tonk-worker/src/router/query.rs`).
async fn run_query(
    branch: &Branch,
    operator: &Operator<NativeSpace>,
    query: Query,
) -> Result<Vec<Conclusion>> {
    let terms = query.terms.clone();
    let plan = QueryPlan::from(ConceptQuery::from(query));
    let raw: Vec<dialog_query::ConceptConclusion> = branch
        .query()
        .select(plan)
        .perform(operator)
        .try_vec()
        .await
        .map_err(|e| anyhow!("preview query failed: {e:?}"))?;
    Ok(raw.iter().map(|c| Conclusion::project(c, &terms)).collect())
}

/// Read a string-valued field off a projected conclusion.
fn string_field(conclusion: &Conclusion, field: &str) -> Option<String> {
    match conclusion.fields.get(field) {
        Some(Ipld::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Enumerate the descriptor's `with:` field names.
fn descriptor_fields(descriptor_json: &str) -> Result<Vec<String>> {
    let predicate: serde_json::Value =
        serde_json::from_str(descriptor_json).context("descriptor JSON did not parse")?;
    Ok(predicate
        .get("with")
        .and_then(|w| w.as_object())
        .map(|with| with.keys().cloned().collect())
        .unwrap_or_default())
}
