//! `tonk element` — enumerate the custom elements defined on the
//! local branch.
//!
//! One row per `xyz.tonk.element/module` claim. An element's entity IS
//! the tag it defines (`element:<tag>`, written by
//! [`crate::authoring::build_element_decl`]), so the listing recovers
//! the tag from the URI rather than from a field — there is only ever
//! one source of truth for what a row defines.
//!
//! The deprecated `component` concept is listed alongside, tagless,
//! because a branch seeded before `element` existed still loads those
//! rows and an author needs to see them to migrate.

use anyhow::{Result, anyhow};
use dialog_artifacts::{Attribute, Entity, Value};
use dialog_query::{AttributeQuery, Output as _, Term, attribute};

use crate::authoring::element_tag;
use crate::site::TonkSite;

/// The attribute an `element!:` assertion writes.
const ELEMENT_MODULE_ATTRIBUTE: &str = "xyz.tonk.element/module";
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
    /// Byte length of the module source — enough to tell an empty row
    /// from a real one without dumping the JavaScript.
    pub module_bytes: usize,
    /// Whether this row came from the deprecated `component` concept
    /// rather than `element`.
    pub deprecated: bool,
}

/// Enumerate every element defined on the branch, `element` rows
/// first and legacy `component` rows after, each group ordered by tag
/// then entity so the listing is reproducible.
pub async fn list(site: &TonkSite) -> Result<Vec<ElementSummary>> {
    let mut out = Vec::new();
    for (uri, deprecated) in [
        (ELEMENT_MODULE_ATTRIBUTE, false),
        (COMPONENT_MODULE_ATTRIBUTE, true),
    ] {
        for claim in claims_for_attribute(site, uri).await? {
            out.push(ElementSummary {
                tag: element_tag(&claim.of.to_string()).map(str::to_owned),
                entity: claim.of,
                module_bytes: module_byte_len(&claim.is),
                deprecated,
            });
        }
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

fn module_byte_len(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Symbol(s) => s.to_string().len(),
        Value::Bytes(b) => b.len(),
        _ => 0,
    }
}
