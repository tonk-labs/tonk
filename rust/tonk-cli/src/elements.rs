//! `tonk element` — enumerate the custom elements defined on the
//! local branch.
//!
//! One row per entity carrying `method` entries, with its tag
//! recovered from the name registry: an `element!: &<tag>` assertion
//! publishes `db.name/referent` on `id:<tag>` pointing at the entity it
//! minted, and that binding is the only thing that says what a row
//! defines.
//!
//! An entity with methods but no name is a SUPERSEDED definition — the
//! tag was re-authored and now points elsewhere. The loader never sees
//! it (it resolves by name), so the listing marks it rather than
//! hiding it: the facts are still on the branch and still addressable.
//!
//! The methods are a keyed dictionary, so each lands as its own fact
//! under `xyz.tonk.element.method/<key>`. The listing reads that
//! domain directly and recovers each key from the attribute's name
//! half, which is the same place the runtime's method table reads it
//! from.
//!
//! The deprecated `component` concept is listed alongside, tagless,
//! because a branch seeded before `element` existed still loads those
//! rows and an author needs to see them to migrate.

use anyhow::{Context, Result, anyhow};
use std::collections::BTreeMap;

use dialog_artifacts::{Attribute, Entity};
use dialog_query::{AttributeQuery, Output as _, Term, attribute};
use tonk_render::QueryBackend as _;

use crate::site::TonkSite;

/// The attribute the deprecated `component!:` assertion writes.
const COMPONENT_MODULE_ATTRIBUTE: &str = "xyz.tonk.component/module";

/// One row of `tonk element`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementSummary {
    /// The custom element name this row defines, from the tag's
    /// `db.name/referent` binding. `None` when nothing names this
    /// entity — a superseded definition, or a legacy `component` row
    /// that never carried an anchor.
    pub tag: Option<String>,
    /// Entity carrying the module claim.
    pub entity: Entity,
    /// The method keys this element defines, sorted — the lifecycle
    /// hooks and any custom methods. Empty for a legacy `component`
    /// row, which carries one anonymous module instead.
    pub methods: Vec<String>,
    /// Whether this row came from the deprecated `component` concept
    /// rather than `element`.
    pub deprecated: bool,
}

/// Enumerate every element defined on the branch, `element` rows
/// first and legacy `component` rows after, each group ordered by tag
/// then entity so the listing is reproducible.
pub async fn list(site: &TonkSite) -> Result<Vec<ElementSummary>> {
    let mut out: Vec<ElementSummary> = Vec::new();
    // Methods are one fact per key, so an element with three methods
    // is three claims on one entity. Fold them back into a row.
    let names = crate::views::names_by_entity(site).await?;
    for (entity, methods) in method_dictionaries(site).await? {
        out.push(ElementSummary {
            tag: names.get(&entity).cloned(),
            entity,
            methods,
            deprecated: false,
        });
    }
    for claim in claims_for_attribute(site, COMPONENT_MODULE_ATTRIBUTE).await? {
        out.push(ElementSummary {
            tag: names.get(&claim.of).cloned(),
            entity: claim.of,
            methods: Vec::new(),
            deprecated: true,
        });
    }
    out.sort_by(|a, b| {
        a.deprecated
            .cmp(&b.deprecated)
            .then_with(|| a.tag.cmp(&b.tag))
            .then_with(|| a.entity.to_string().cmp(&b.entity.to_string()))
    });
    Ok(out)
}

/// Every claim on the branch carrying `uri`, subject left open.
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

/// The methods currently bound to `tag`, as `(key, source)` pairs in
/// key order. Empty when the tag names nothing.
///
/// Resolves through the name, not through any URI built from the tag:
/// `id:<tag>`'s `db.name/referent` says which entity the tag means
/// right now, and that is the only thing that does. An entity the tag
/// used to point at is a previous value and is deliberately not read.
pub async fn methods_of(site: &TonkSite, tag: &str) -> Result<Vec<(String, String)>> {
    entries_of(
        site,
        tag,
        tonk_template::resolve::element_method_predicate(),
        "method",
    )
    .await
}

/// The attribute defaults currently bound to `tag`, as `(name, value)`
/// pairs in name order. Empty when the tag names nothing, and equally
/// when it names an element that declares no defaults — which is most
/// of them, and is not an error.
pub async fn attributes_of(site: &TonkSite, tag: &str) -> Result<Vec<(String, String)>> {
    entries_of(
        site,
        tag,
        tonk_template::resolve::element_attribute_predicate(),
        "attribute",
    )
    .await
}

/// One dictionary of the element `tag` names, read through `predicate`
/// and folded out of `field`.
///
/// Resolves through the name, not through any URI built from the tag:
/// `id:<tag>`'s `db.name/referent` says which entity the tag means
/// right now, and that is the only thing that does. An entity the tag
/// used to point at is a previous value and is deliberately not read.
async fn entries_of(
    site: &TonkSite,
    tag: &str,
    predicate: serde_json::Value,
    field: &str,
) -> Result<Vec<(String, String)>> {
    let Some(entity) = crate::views::entity_for_name(site, tag).await? else {
        return Ok(Vec::new());
    };
    let mut entries: Vec<(String, String)> = dictionaries(site, predicate, field)
        .await?
        .into_iter()
        .find(|(candidate, _)| *candidate == entity)
        .map(|(_, sources)| sources.into_iter().collect())
        .unwrap_or_default();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

/// Every element's method dictionary, folded to one entry per entity.
///
/// The same wire query the display stack runs for a view's `show`
/// (`resolve.rs view_predicate`), pointed at the method domain with
/// `this` left as a variable so it matches every element on the
/// branch. A keyed collection binds two terms — the field and its key
/// operand — because an entry is a `(key, value)` pair; requesting
/// only the field leaves the key unbound and every entry reads empty.
///
/// A raw `AttributeQuery` cannot do this job: a dictionary's key set
/// is open, so the attribute would have to be a variable too, and a
/// selector with nothing constrained is refused as a full scan.
async fn method_dictionaries(site: &TonkSite) -> Result<Vec<(Entity, Vec<String>)>> {
    Ok(source_dictionaries(site)
        .await?
        .into_iter()
        .map(|(entity, sources)| (entity, sources.into_keys().collect()))
        .collect())
}

/// [`method_dictionaries`], keeping each method's source.
async fn source_dictionaries(site: &TonkSite) -> Result<Vec<(Entity, BTreeMap<String, String>)>> {
    dictionaries(
        site,
        tonk_template::resolve::element_method_predicate(),
        "method",
    )
    .await
}

/// Every element's `field` dictionary, folded to one entry per entity.
async fn dictionaries(
    site: &TonkSite,
    predicate: serde_json::Value,
    field: &str,
) -> Result<Vec<(Entity, BTreeMap<String, String>)>> {
    // `this` left as a variable so this matches every element on the
    // branch; the predicate is the shared one, so the listing and the
    // browser registry cannot drift about what a dictionary looks like
    // on the wire.
    let body = serde_json::json!({
        "terms": {
            "this":                    { "?": { "name": "this" } },
            field:                     { "?": { "name": field } },
            format!("{field}/key"):    { "?": { "name": format!("{field}/key") } },
        },
        "predicate": predicate,
    });
    let query: tonk_schema::query::Query =
        serde_json::from_value(body).context("dictionary query body is well-formed")?;
    let concept_query = query
        .into_concept_query()
        .map_err(|e| anyhow!("method query should lower to a concept query: {e:?}"))?;
    let rows = site
        .query(concept_query)
        .await
        .map_err(|e| anyhow!("{field} enumeration failed: {e}"))?;
    // One flat row per entry, `field` a one-entry `{key: value}` map;
    // merge rows by entity.
    let mut folded: BTreeMap<Entity, BTreeMap<String, String>> = BTreeMap::new();
    for row in rows {
        let Ok(entity) = row.this.parse::<Entity>() else {
            continue;
        };
        let Some(ipld_core::ipld::Ipld::Map(entries)) = row.fields.get(field) else {
            continue;
        };
        let slot = folded.entry(entity).or_default();
        for (key, value) in entries {
            if let ipld_core::ipld::Ipld::String(source) = value {
                slot.insert(key.clone(), source.clone());
            }
        }
    }
    Ok(folded.into_iter().collect())
}
