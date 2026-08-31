//! `tonk view` — enumerate every entity on the local branch that
//! carries a renderable claim.
//!
//! Two sources:
//!
//! - the `show` dictionary the display stack resolves — every
//!   `xyz.tonk.view/<facet>` entry on a model entity (what
//!   `tonk view add` writes);
//! - `text/html`, which the host route at
//!   `/api/repository/{repo}/branch/{branch}/host/{host}/{entity}`
//!   selects on for a one-off page.
//!
//! Listing only `text/html` — as this module once did — meant
//! `tonk view` came back empty right after `tonk view add`
//! succeeded, because the two were looking at different claims.
//!
//! The standard library seeds views of its own. Those are filtered
//! out for the same reason `tonk concept` drops the runtime
//! vocabulary: they resolve everywhere, and listing them buries the
//! view the author just wrote.
//!
//! Used by bare `tonk view`.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use anyhow::{Context, Result, anyhow};
use dialog_artifacts::{Attribute, Entity, Value};
use dialog_query::{AttributeQuery, Output as _, Term, attribute};
use tonk_render::QueryBackend as _;

use crate::site::TonkSite;

/// One row of `tonk view`.
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
    /// What the view renders. The view instance IS the model entity,
    /// so this is the entity's own published name (or its URI when
    /// unnamed). `None` for a bare `text/html` page, which binds no
    /// model.
    pub model: Option<String>,
    /// Byte length of the body claim. Lets the listing show a
    /// rough "is this empty / huge?" without dumping the HTML.
    /// When an entity holds more than one renderable claim, this is
    /// the longest one — the most likely candidate for "the current
    /// view body."
    pub body_bytes: usize,
}

/// The authored parts of one view, for `tonk show <view>`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewDescription {
    /// Published anchor name, falling back to the entity URI.
    pub anchor: String,
    /// Entity carrying the view facts.
    pub entity: Entity,
    /// Published concept name, or its entity URI when unnamed.
    pub model: Option<String>,
    /// Renderable template body.
    pub template: String,
}

/// The host route's raw page attribute — the one renderable claim
/// that is not a `show` entry.
const TEXT_HTML_ATTRIBUTE: &str = "text/html";

/// Enumerate every distinct entity on the branch holding a renderable
/// claim, minus the ones the standard library seeded.
///
/// One row per entity. When an entity carries several renderable
/// claims — a model with both a `ui` and a `directory` template, say —
/// the [`ViewSummary::body_bytes`] field records the longest.
pub async fn list(site: &TonkSite) -> Result<Vec<ViewSummary>> {
    let names = name_claims_by_entity(site).await?;
    let shows = show_dictionaries(site).await?;
    let mut with_show: HashMap<Entity, usize> = HashMap::new();
    for (entity, entries) in shows {
        // The library's views key off its own model entities: pinned
        // URIs, or — for library concepts without a pinned `this:`
        // (`board`, `prose`, `table`) — entities whose published name
        // is a system concept.
        if crate::site::standard_library_pins_entity(&entity.to_string())
            || names
                .get(&entity)
                .is_some_and(|name| crate::schema::is_system_concept(name))
        {
            continue;
        }
        let longest = entries.values().map(String::len).max().unwrap_or(0);
        with_show.insert(entity, longest);
    }
    let mut by_entity = with_show;
    for row in claims_for_attribute(site, TEXT_HTML_ATTRIBUTE).await? {
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
    if by_entity.is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<ViewSummary> = by_entity
        .into_iter()
        .map(|(entity, body_bytes)| ViewSummary {
            name: names.get(&entity).cloned(),
            // The view instance IS the model: its own published name.
            model: names.get(&entity).cloned(),
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

/// Query every `show` entry on the branch and fold them into one
/// facet dictionary per entity. The concept query is the same one
/// the display stack runs (`resolve.rs view_predicate`), with `this`
/// left as a variable so it matches every model that declares views.
async fn show_dictionaries(site: &TonkSite) -> Result<Vec<(Entity, BTreeMap<String, String>)>> {
    let body = serde_json::json!({
        "terms": {
            "this":     { "?": { "name": "this" } },
            "show":     { "?": { "name": "show" } },
            "show/key": { "?": { "name": "show/key" } },
        },
        "predicate": {
            "with": {
                "show": {
                    "the": { "domain": "xyz.tonk.view", "keyed": "dictionary" },
                    "as": "Text",
                    "cardinality": "one"
                }
            }
        }
    });
    let query: tonk_schema::query::Query =
        serde_json::from_value(body).context("show query body is well-formed")?;
    let concept_query = query
        .into_concept_query()
        .map_err(|e| anyhow!("show query should lower to a concept query: {e:?}"))?;
    let rows = site
        .query(concept_query)
        .await
        .map_err(|e| anyhow!("show enumeration failed: {e}"))?;
    // One flat row per entry, `show` a one-entry `{facet: template}`
    // map; merge rows by entity.
    let mut order: Vec<Entity> = Vec::new();
    let mut folded: HashMap<Entity, BTreeMap<String, String>> = HashMap::new();
    for row in rows {
        let Ok(entity) = row.this.parse::<Entity>() else {
            continue;
        };
        let Some(ipld_core::ipld::Ipld::Map(entries)) = row.fields.get("show") else {
            continue;
        };
        let dict = match folded.entry(entity.clone()) {
            Entry::Occupied(held) => held.into_mut(),
            Entry::Vacant(slot) => {
                order.push(entity);
                slot.insert(BTreeMap::new())
            }
        };
        for (facet, value) in entries {
            if let ipld_core::ipld::Ipld::String(template) = value {
                dict.insert(facet.clone(), template.clone());
            }
        }
    }
    Ok(order
        .into_iter()
        .filter_map(|entity| {
            let dict = folded.remove(&entity)?;
            Some((entity, dict))
        })
        .collect())
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

/// Resolve a view name or entity and return its model, anchor, and template.
pub async fn describe(site: &TonkSite, reference: &str) -> Result<Option<ViewDescription>> {
    let entity = match reference.parse::<Entity>() {
        Ok(entity) => Some(entity),
        Err(_) => entity_for_name(site, reference).await?,
    };
    let Some(entity) = entity else {
        return Ok(None);
    };

    // The `ui` facet is the canonical body; otherwise the first entry
    // of the dictionary, else a raw `text/html` page claim.
    let mut template = show_dictionaries(site)
        .await?
        .into_iter()
        .find(|(this, _)| *this == entity)
        .and_then(|(_, mut dict)| dict.remove("ui").or_else(|| dict.into_values().next()));
    if template.is_none()
        && let Some(claim) = claims_for_attribute(site, TEXT_HTML_ATTRIBUTE)
            .await?
            .into_iter()
            .find(|claim| claim.of == entity)
    {
        template = Some(match claim.is {
            Value::String(value) => value.to_string(),
            Value::Symbol(value) => value.to_string(),
            Value::Bytes(value) => String::from_utf8_lossy(&value).into_owned(),
            value => format!("{value:?}"),
        });
    }
    let Some(template) = template else {
        return Ok(None);
    };

    let names = name_claims_by_entity(site).await?;
    let model = names.get(&entity).cloned();
    let anchor = names
        .get(&entity)
        .cloned()
        .unwrap_or_else(|| entity.to_string());
    Ok(Some(ViewDescription {
        anchor,
        entity,
        model,
        template,
    }))
}

/// One current fact carried by an entity, for `tonk show <entity>`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityFact {
    /// Attribute URI.
    pub attribute: String,
    /// Debug-stable rendering of the typed value.
    pub value: String,
}

/// Resolve an entity URI or bookmark and enumerate every current fact on it.
pub async fn facts_for_entity(
    site: &TonkSite,
    reference: &str,
) -> Result<Option<(Entity, Vec<EntityFact>)>> {
    let entity = match reference.parse::<Entity>() {
        Ok(entity) => Some(entity),
        Err(_) => entity_for_name(site, reference).await?,
    };
    let Some(entity) = entity else {
        return Ok(None);
    };
    let session = site.branch().await?;
    let claims: Vec<dialog_query::Claim> = session
        .handle()
        .query()
        .select(AttributeQuery::new(
            Term::<attribute::The>::var("the"),
            Term::from(entity.clone()),
            Term::<dialog_query::Any>::var("is"),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|e| anyhow!("fact lookup for entity {entity} failed: {e:?}"))?;
    let mut facts: Vec<_> = claims
        .into_iter()
        .map(|claim| EntityFact {
            attribute: claim.the.to_string(),
            value: format!("{:?}", claim.is),
        })
        .collect();
    facts.sort_by(|a, b| a.attribute.cmp(&b.attribute).then(a.value.cmp(&b.value)));
    Ok(Some((entity, facts)))
}

fn text_html_attribute() -> Result<Attribute> {
    "text/html"
        .parse()
        .map_err(|e| anyhow!("text/html should be a valid attribute URI: {e:?}"))
}
