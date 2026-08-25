//! `tonk view ls` — enumerate every entity on the local branch that
//! carries a renderable claim.
//!
//! Claim-driven, not concept-driven: we don't care whether an entity
//! is a member of the `view` concept or picked its claim up some
//! other way. What counts is the attribute the renderer selects on,
//! and there are five of them:
//!
//! - the four template attributes the display stack resolves —
//!   `xyz.tonk.view/display` (what `tonk view add` writes), plus the
//!   `/directory`, `/label`, and `/title` view kinds;
//! - `text/html`, which the host route at
//!   `/api/repository/{repo}/branch/{branch}/host/{host}/{entity}`
//!   selects on for a one-off page.
//!
//! Listing only `text/html` — as this module once did — meant
//! `tonk view ls` came back empty right after `tonk view add`
//! succeeded, because the two were looking at different claims.
//!
//! The standard library seeds twenty-five views of its own. Those are
//! filtered out for the same reason `tonk concept ls` drops the
//! runtime vocabulary: they resolve everywhere, and listing them
//! buries the view the author just wrote.
//!
//! Used by `tonk view ls`, the only caller.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use anyhow::{Context, Result, anyhow};
use dialog_artifacts::{Attribute, Entity, Value};
use dialog_query::{AttributeQuery, Output as _, Term, attribute};

use crate::site::TonkSite;

/// One row of `tonk view ls`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSummary {
    /// `db.meta/name` claim on the entity, when one is
    /// asserted. The same name might be reused across the
    /// branch's history; we surface the current binding only.
    pub name: Option<String>,
    /// Entity URI carrying the renderable claim — what a route's
    /// trailing `{entity}` segment takes.
    pub entity: Entity,
    /// What the view renders: the published name of the concept its
    /// `xyz.tonk.view/model` points at, or that entity's URI when the
    /// model publishes no name. `None` for a bare `text/html` page,
    /// which binds no model.
    pub model: Option<String>,
    /// Byte length of the body claim. Lets the listing show a
    /// rough "is this empty / huge?" without dumping the HTML.
    /// When an entity holds more than one renderable claim, this is
    /// the longest one — the most likely candidate for "the current
    /// view body."
    pub body_bytes: usize,
}

/// The attributes a renderable claim can land under. The four view
/// kinds the display stack resolves, then the host route's raw page
/// body. Kept in one place so the listing and its documentation
/// cannot drift apart.
const RENDERABLE_ATTRIBUTES: &[&str] = &[
    "xyz.tonk.view/display",
    "xyz.tonk.view/directory",
    "xyz.tonk.view/label",
    "xyz.tonk.view/title",
    "text/html",
];

/// The attribute binding a view to the concept it renders.
const MODEL_ATTRIBUTE: &str = "xyz.tonk.view/model";

/// Enumerate every distinct entity on the branch holding a renderable
/// claim, minus the ones the standard library seeded.
///
/// One row per entity. When an entity carries several renderable
/// claims — a model with both a detail and a directory template, say —
/// the [`ViewSummary::body_bytes`] field records the longest; the name
/// and model lookups are unaffected (each entity has at most one of
/// each).
pub async fn list(site: &TonkSite) -> Result<Vec<ViewSummary>> {
    let entities_with_lengths = enumerate_view_claims(site).await?;
    if entities_with_lengths.is_empty() {
        return Ok(Vec::new());
    }
    let names = name_claims_by_entity(site).await?;
    let models = model_claims_by_entity(site).await?;
    let mut out: Vec<ViewSummary> = entities_with_lengths
        .into_iter()
        .map(|(entity, body_bytes)| ViewSummary {
            name: names.get(&entity).cloned(),
            // A model is an entity reference; show the concept name it
            // publishes, since that is the name `tonk view add` took
            // and `tonk query` will take back.
            model: models.get(&entity).map(|model| {
                names
                    .get(model)
                    .cloned()
                    .unwrap_or_else(|| model.to_string())
            }),
            entity,
            body_bytes,
        })
        .collect();
    out.sort_by(|a, b| match (&a.name, &b.name) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.entity.to_string().cmp(&b.entity.to_string()),
    });
    Ok(out)
}

/// Check whether `entity` carries at least one `text/html`
/// claim — the host route 404s on anything that doesn't.
pub async fn entity_has_text_html(site: &TonkSite, entity: &Entity) -> Result<bool> {
    let the = text_html_attribute()?;
    let the_term: attribute::The = the.into();
    let session = site.branch().await?;
    let rows: Vec<dialog_query::Claim> = session
        .handle()
        .query()
        .select(AttributeQuery::new(
            Term::from(the_term),
            Term::from(entity.clone()),
            Term::<dialog_query::Any>::var("is"),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|e| anyhow!("text/html lookup for entity {entity} failed: {e:?}"))?;
    Ok(!rows.is_empty())
}

/// Run one `(<attribute>, ?of, ?is)` query per entry of
/// [`RENDERABLE_ATTRIBUTES`] and reduce each entity to (entity, max
/// body length), dropping the entities the standard library pinned.
/// String, symbol, and bytes payloads are all counted by their
/// on-disk byte length; other value flavours surface as zero — they
/// shouldn't appear under a template attribute in practice, but
/// ignoring them keeps the listing from panicking if something weird
/// sneaks in.
async fn enumerate_view_claims(site: &TonkSite) -> Result<Vec<(Entity, usize)>> {
    let mut by_entity: HashMap<Entity, usize> = HashMap::new();
    for uri in RENDERABLE_ATTRIBUTES {
        for row in claims_for_attribute(site, uri).await? {
            if crate::site::standard_library_pins_entity(&row.of.to_string()) {
                continue;
            }
            let len = body_byte_len(&row.is);
            by_entity
                .entry(row.of)
                .and_modify(|current| {
                    if len > *current {
                        *current = len;
                    }
                })
                .or_insert(len);
        }
    }
    Ok(by_entity.into_iter().collect())
}

/// Pull every `xyz.tonk.view/model` claim and return a
/// `view entity → model entity` map. One branch query, for the same
/// reason [`name_claims_by_entity`] is one.
async fn model_claims_by_entity(site: &TonkSite) -> Result<HashMap<Entity, Entity>> {
    let mut out = HashMap::new();
    for claim in claims_for_attribute(site, MODEL_ATTRIBUTE).await? {
        if let Value::Entity(model) = claim.is {
            out.insert(claim.of, model);
        }
    }
    Ok(out)
}

/// Select every current claim under one attribute URI.
async fn claims_for_attribute(site: &TonkSite, uri: &str) -> Result<Vec<dialog_query::Claim>> {
    let the: Attribute = uri
        .parse()
        .map_err(|e| anyhow!("{uri} should be a valid attribute URI: {e:?}"))?;
    let the_term: attribute::The = the.into();
    let session = site.branch().await?;
    session
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
        .map_err(|e| anyhow!("{uri} enumeration failed: {e:?}"))
}

fn body_byte_len(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Symbol(s) => s.to_string().len(),
        Value::Bytes(b) => b.len(),
        Value::Record(r) => r.len(),
        _ => 0,
    }
}

/// Pull every name-publication claim and return a `target →
/// name` map. Done as one branch query — bulk faster than
/// per-entity lookups and avoids N+1 round trips against a large
/// branch.
///
/// Names are stored inverted under the `db.name/referent`
/// relation: each anchor `&foo` publishes
/// `(db.name/referent, id:foo, <target-entity>)`. The *name*
/// lives in the claim's subject as `id:<name>`; the *target* is
/// the value. We invert that mapping here so callers can ask
/// "what's this entity's display name?" with one lookup.
async fn name_claims_by_entity(site: &TonkSite) -> Result<HashMap<Entity, String>> {
    let name_attr: Attribute = "db.name/referent"
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
        let Some(name) = name_from_id_entity(&claim.of) else {
            continue;
        };
        if let Value::Entity(target) = claim.is {
            // The relation is many-to-one — `space:home`, for one,
            // answers to both `space-home` and the `tonk/space` alias —
            // so inverting it has to pick. Take the lexicographically
            // first, which at least makes the listing reproducible
            // instead of leaving it to claim order.
            match out.entry(target) {
                Entry::Occupied(mut held) if name < *held.get() => {
                    held.insert(name);
                }
                Entry::Occupied(_) => {}
                Entry::Vacant(slot) => {
                    slot.insert(name);
                }
            }
        }
    }
    Ok(out)
}

/// Strip the `id:` scheme prefix from a name-publishing entity.
/// `id:foo` → `Some("foo")`; anything else → `None`.
fn name_from_id_entity(entity: &Entity) -> Option<String> {
    entity.to_string().strip_prefix("id:").map(str::to_owned)
}

/// Look up the entity bound to a `db.meta/name` bookmark on
/// the local branch. `Ok(None)` when nothing matches. Resolves a
/// positional name argument into an entity URI. Delegates to
/// `tonk_schema::concept::lookup_named_entity`, the canonical
/// name→entity helper.
pub async fn entity_for_name(site: &TonkSite, name: &str) -> Result<Option<Entity>> {
    let session = site.branch().await?;
    tonk_schema::concept::lookup_named_entity(name, session.handle(), &site.operator)
        .await
        .map_err(|e| anyhow!("name lookup failed for {name}: {e:?}"))
}

fn text_html_attribute() -> Result<Attribute> {
    "text/html"
        .parse()
        .map_err(|e| anyhow!("text/html should be a valid attribute URI: {e:?}"))
}
