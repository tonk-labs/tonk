//! Synced `AGENTS.md` context attached to a space's stable repository subject.
//!
//! The Dialog claim is the source of truth. Agent runtimes that require a
//! filesystem `AGENTS.md` may materialize this value before launch, but local
//! files are projections and must not silently overwrite a newer claim.

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Serialize;

use crate::auto_sync;
use crate::eval::{self, Options, Source};
use crate::schema;
use crate::site::TonkSite;

/// Pinned standard-library concept carrying the Markdown claim.
pub const CONCEPT_NAME: &str = "tonk/agents";

/// Attribute carrying the Markdown on the repository subject.
pub const ATTRIBUTE: &str = "xyz.tonk.repo/agents";

/// Stable source label used by structured output and experiment metadata.
pub const SOURCE: &str = "dialog-claim";

/// Keep the claim bounded so one space cannot consume an agent's entire
/// instruction budget.
pub const MAX_MARKDOWN_BYTES: usize = 32 * 1024;

const SCHEMA_DOCUMENT: &str = r#"attribute!: &tonk/agents-markdown
  the: xyz.tonk.repo/agents
  description: >
    Markdown working context for agents using this repository.
    Keep durable concepts, workflows, decisions, and recurring pitfalls here;
    omit credentials, invite links, transient status, and one-off completions.
  cardinality: one
  as: text

concept!: &tonk/agents
  this: tonk:agents
  description: AGENTS.md context attached to the repository subject DID.
  with:
    markdown: tonk/agents-markdown
"#;

/// Current agent context for one space.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpaceAgents {
    /// Where the agent context came from.
    pub source: &'static str,
    /// Dialog attribute carrying the Markdown.
    pub attribute: &'static str,
    /// Stable repository subject DID the claim maps.
    pub entity: String,
    /// Branch revision observed by the read.
    pub revision: String,
    /// Markdown content suitable for an `AGENTS.md` projection.
    pub markdown: String,
}

/// Whether the `tonk/agents` concept is declared on this branch.
///
/// Asked by name rather than by scanning `list_concepts`: the claim
/// rides a standard-library concept, which that listing deliberately
/// omits.
pub async fn is_declared(site: &TonkSite) -> Result<bool> {
    Ok(schema::find_concept(site, CONCEPT_NAME).await?.is_some())
}

/// Read the current claim, returning `None` for spaces that have never defined
/// or asserted it.
pub async fn get(site: &TonkSite) -> Result<Option<SpaceAgents>> {
    if !is_declared(site).await? {
        return Ok(None);
    }
    get_declared(site).await
}

/// Read the claim when the caller already knows the concept is declared.
pub(crate) async fn get_declared(site: &TonkSite) -> Result<Option<SpaceAgents>> {
    let entity = site.repository.did().to_string();
    let doc = format!("{CONCEPT_NAME}:\n  this: {entity}\n  markdown: ?markdown\n");
    let outcome = eval::run_against_site(site, Source::Inline(doc), Options::default())
        .await
        .context("query space AGENTS.md claim")?;
    let revision = outcome
        .response
        .revision_before
        .as_ref()
        .map(|revision| revision.tree.to_string())
        .ok_or_else(|| anyhow!("space AGENTS.md query returned no branch revision"))?;
    let Some(row) = outcome
        .response
        .matches_after
        .iter()
        .find(|block| block.label == CONCEPT_NAME)
        .and_then(|block| block.results.first())
    else {
        return Ok(None);
    };
    if row.this != entity {
        return Err(anyhow!(
            "space AGENTS.md resolved on {}, expected repository subject {entity}",
            row.this
        ));
    }
    let markdown = row
        .fields
        .get("markdown")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("space AGENTS.md claim has no text `markdown` field"))?
        .to_owned();
    Ok(Some(SpaceAgents {
        source: SOURCE,
        attribute: ATTRIBUTE,
        entity,
        revision,
        markdown,
    }))
}

/// Assert Markdown on the repository subject, defining the schema first so the
/// command also works for spaces created before `tonk/agents` entered the
/// standard library.
///
/// `None` means the write was a dry run: the document was analyzed and
/// dropped, so there is no claim to read back. Reading one anyway would
/// report whatever was already on the branch as though this call had
/// written it.
pub async fn set(
    site: &TonkSite,
    markdown: &str,
    write: crate::data_ops::WriteOptions,
) -> Result<Option<SpaceAgents>> {
    if markdown.trim().is_empty() {
        bail!("AGENTS.md is empty");
    }
    if markdown.len() > MAX_MARKDOWN_BYTES {
        bail!(
            "AGENTS.md is {} bytes; maximum is {MAX_MARKDOWN_BYTES}",
            markdown.len()
        );
    }

    let schema_document = if is_declared(site).await? {
        ""
    } else {
        SCHEMA_DOCUMENT
    };
    let entity = site.repository.did();
    let encoded = serde_json::to_string(markdown).context("encode AGENTS.md Markdown")?;
    let doc = format!(
        r#"{schema_document}
tonk/agents!:
  this: {entity}
  markdown: {encoded}
"#
    );
    let outcome = auto_sync::run_eval(site, Source::Inline(doc), write.eval(), write.sync())
        .await
        .context("assert space AGENTS.md claim")?;
    if !outcome.committed {
        return Ok(None);
    }
    get_declared(site)
        .await?
        .map(Some)
        .ok_or_else(|| anyhow!("AGENTS.md write committed but no claim was readable"))
}
