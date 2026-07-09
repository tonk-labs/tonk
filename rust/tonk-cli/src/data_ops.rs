//! Library handlers for the data verbs — the testable core the thin
//! `bin/tonk.rs` handlers call. Each returns rendered stdout; the
//! binary maps errors to exit codes.

use anyhow::Result;

use crate::schema::{self, type_to_notation};
use crate::site::TonkSite;

/// Render a concept's schema as a human-readable field list.
pub async fn describe(site: &TonkSite, concept: &str) -> Result<String> {
    let info = schema::find_concept(site, concept)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no concept named '{concept}'"))?;
    let mut out = String::new();
    if let Some(desc) = info.descriptor.description() {
        out.push_str(desc);
        out.push_str("\n\n");
    }
    for (field, fd) in info.descriptor.with().iter() {
        let ty = fd
            .content_type()
            .map(|t| type_to_notation(&t))
            .unwrap_or_else(|| "any".into());
        let card = format!("{:?}", fd.cardinality()).to_lowercase();
        let req = if fd.is_optional() { "" } else { " (required)" };
        out.push_str(&format!(
            "  --{field} <{ty}> [{card}]{req}  {}\n",
            fd.description()
        ));
    }
    Ok(out)
}
