//! `tonk element` — enumerate the custom elements defined on the
//! local branch.
//!
//! One row per entity carrying `method` entries. An element's entity
//! IS the tag it defines (`element:<tag>`, written by
//! [`crate::authoring::build_element_decl`]), so the listing recovers
//! the tag from the URI rather than from a field — there is only ever
//! one source of truth for what a row defines.
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

use crate::authoring::element_tag;
use crate::site::TonkSite;

/// The domain an `element!:` assertion writes its methods under. Each
/// entry is `<domain>/<key>`.
const ELEMENT_METHOD_DOMAIN: &str = "xyz.tonk.element.method";
/// The attribute the deprecated `component!:` assertion writes.
const COMPONENT_MODULE_ATTRIBUTE: &str = "xyz.tonk.component/module";

/// One row of `tonk element`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementSummary {
    /// The custom element name this row defines, recovered from the
    /// entity URI. `None` for a legacy `component` row, whose entity
    /// is a body digest that names nothing.
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
    for (entity, methods) in method_dictionaries(site).await? {
        out.push(ElementSummary {
            tag: element_tag(&entity.to_string()).map(str::to_owned),
            entity,
            methods,
            deprecated: false,
        });
    }
    for claim in claims_for_attribute(site, COMPONENT_MODULE_ATTRIBUTE).await? {
        out.push(ElementSummary {
            tag: element_tag(&claim.of.to_string()).map(str::to_owned),
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
    let body = serde_json::json!({
        "terms": {
            "this":       { "?": { "name": "this" } },
            "method":     { "?": { "name": "method" } },
            "method/key": { "?": { "name": "method/key" } },
        },
        "predicate": {
            "with": {
                "method": {
                    "the": { "domain": ELEMENT_METHOD_DOMAIN, "keyed": "dictionary" },
                    "as": "Text",
                    "cardinality": "one"
                }
            }
        }
    });
    let query: tonk_schema::query::Query =
        serde_json::from_value(body).context("method query body is well-formed")?;
    let concept_query = query
        .into_concept_query()
        .map_err(|e| anyhow!("method query should lower to a concept query: {e:?}"))?;
    let rows = site
        .query(concept_query)
        .await
        .map_err(|e| anyhow!("method enumeration failed: {e}"))?;
    // One flat row per entry, `method` a one-entry `{key: source}`
    // map; merge rows by entity.
    let mut folded: BTreeMap<Entity, Vec<String>> = BTreeMap::new();
    for row in rows {
        let Ok(entity) = row.this.parse::<Entity>() else {
            continue;
        };
        let Some(ipld_core::ipld::Ipld::Map(entries)) = row.fields.get("method") else {
            continue;
        };
        let keys = folded.entry(entity).or_default();
        for key in entries.keys() {
            keys.push(key.clone());
        }
    }
    Ok(folded
        .into_iter()
        .map(|(entity, mut keys)| {
            keys.sort();
            keys.dedup();
            (entity, keys)
        })
        .collect())
}
