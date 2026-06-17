//! `slide views` — enumerate every entity holding a `text/html`
//! claim on the local branch.
//!
//! Claim-driven, not concept-driven: we don't care whether an
//! entity is a member of the `view` concept or got its
//! `text/html` claim some other way. The host route at
//! `/api/repository/{repo}/branch/{branch}/host/{host}/{entity}`
//! ultimately selects on `(the=text/html, of=<entity>)`, so
//! anything that satisfies that selector is a candidate view
//! and should surface here.
//!
//! Used by `slide views` (the listing command) and by
//! `slide share view` (which calls back into it to confirm the
//! resolved entity actually has a `text/html` claim before
//! minting a launcher URL).

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use dialog_artifacts::{Attribute, Entity, Value};
use dialog_query::{AttributeQuery, Output as _, Term, attribute};

use crate::site::SlideSite;

/// One row of `slide views`.
#[derive(Debug, Clone)]
pub struct ViewSummary {
    /// `dialog.meta/name` claim on the entity, when one is
    /// asserted. The same name might be reused across the
    /// branch's history; we surface the current binding only.
    pub name: Option<String>,
    /// Entity URI carrying the `text/html` claim — what the
    /// host route's `view/{entity}` segment takes.
    pub entity: Entity,
    /// Byte length of the body claim. Lets the listing show a
    /// rough "is this empty / huge?" without dumping the HTML.
    /// When an entity holds more than one `text/html` claim
    /// (which the seed schema's cardinality-many allows in
    /// theory but git-tag semantics make uncommon), this is the
    /// longest one — the most likely candidate for "the current
    /// view body."
    pub body_bytes: usize,
}

/// Enumerate every distinct entity on the branch holding a
/// `text/html` claim.
///
/// One row per entity. When an entity carries multiple
/// `text/html` claims, the [`ViewSummary::body_bytes`] field
/// records the longest; the name lookup is unaffected (each
/// entity has at most one `dialog.meta/name` claim).
pub async fn list(site: &SlideSite) -> Result<Vec<ViewSummary>> {
    let entities_with_lengths = enumerate_view_claims(site).await?;
    if entities_with_lengths.is_empty() {
        return Ok(Vec::new());
    }
    let names = name_claims_by_entity(site).await?;
    let mut out: Vec<ViewSummary> = entities_with_lengths
        .into_iter()
        .map(|(entity, body_bytes)| ViewSummary {
            name: names.get(&entity).cloned(),
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
/// claim. Used by `slide share view` to refuse minting a
/// launcher URL for an entity the host route would 404 on.
pub async fn entity_has_text_html(site: &SlideSite, entity: &Entity) -> Result<bool> {
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

/// Run the `(text/html, ?of, ?is)` query and reduce each entity
/// to (entity, max body length). String, symbol, and bytes
/// payloads are all counted by their on-disk byte length; other
/// value flavours surface as zero — they shouldn't appear under
/// `text/html` in practice, but ignoring them keeps the listing
/// from panicking if something weird sneaks in.
async fn enumerate_view_claims(site: &SlideSite) -> Result<Vec<(Entity, usize)>> {
    let the = text_html_attribute()?;
    let the_term: attribute::The = the.into();
    let session = site.branch().await?;
    let rows: Vec<dialog_query::Claim> = session
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
        .map_err(|e| anyhow!("text/html enumeration failed: {e:?}"))?;

    let mut by_entity: HashMap<Entity, usize> = HashMap::new();
    for row in rows {
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
    Ok(by_entity.into_iter().collect())
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
/// Names are stored inverted under the `dialog.name/referent`
/// relation: each anchor `&foo` publishes
/// `(dialog.name/referent, id:foo, <target-entity>)`. The *name*
/// lives in the claim's subject as `id:<name>`; the *target* is
/// the value. We invert that mapping here so callers can ask
/// "what's this entity's display name?" with one lookup.
async fn name_claims_by_entity(site: &SlideSite) -> Result<HashMap<Entity, String>> {
    let name_attr: Attribute = "dialog.name/referent"
        .parse()
        .context("dialog.name/referent should be a valid attribute URI")?;
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
        .map_err(|e| anyhow!("dialog.name/referent query failed: {e:?}"))?;
    let mut out = HashMap::with_capacity(claims.len());
    for claim in claims {
        let Some(name) = name_from_id_entity(&claim.of) else {
            continue;
        };
        if let Value::Entity(target) = claim.is {
            out.insert(target, name);
        }
    }
    Ok(out)
}

/// Strip the `id:` scheme prefix from a name-publishing entity.
/// `id:foo` → `Some("foo")`; anything else → `None`.
fn name_from_id_entity(entity: &Entity) -> Option<String> {
    entity.to_string().strip_prefix("id:").map(str::to_owned)
}

/// Look up the entity bound to a `dialog.meta/name` bookmark on
/// the local branch. `Ok(None)` when nothing matches. Used by
/// `slide share view` to resolve a positional name argument
/// into an entity URI for the launcher path. Delegates to
/// `tonk_schema::concept::lookup_named_entity`, the canonical
/// name→entity helper.
pub async fn entity_for_name(site: &SlideSite, name: &str) -> Result<Option<Entity>> {
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
