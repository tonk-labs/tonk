//! Retain complete signed heads for native historical checkout.
//!
//! Dialog's in-tree log contains ancestry and claims, but not the old signed
//! tree roots. Keep heads observed by this CLI in repository-local memory,
//! outside application history. These receipts are local, not replicated.

use dialog_capability::Subject;
use dialog_effects::MethodExt as _;
use dialog_effects::memory::MemoryError;
use dialog_effects::memory::prelude::*;
use dialog_peer::{Peer, Session};
use dialog_repository::Revision;
use dialog_storage::provider::storage::NativeSpace;

const SPACE: &str = "tonk/revisions";

/// Retain a signed head without changing any branch or application fact.
pub(crate) async fn remember(
    subject: Subject,
    bytes: &[u8],
    env: &Peer<NativeSpace, Session>,
) -> Result<(), MemoryError> {
    let revision: Revision = serde_ipld_dagcbor::from_slice(bytes)
        .map_err(|error| MemoryError::Storage(error.to_string()))?;
    revision
        .verify()
        .map_err(|error| MemoryError::Storage(error.to_string()))?;
    for key in [
        format!("version/{}", revision.version()),
        format!("tree/{}", revision.tree.to_string().trim_start_matches('#')),
    ] {
        subject
            .clone()
            .writer()
            .memory()
            .space(SPACE)
            .cell(key)
            .publish(bytes.to_vec(), None)
            .perform(env)
            .await?;
    }
    Ok(())
}

/// Validate a log version or a tree hash and return its receipt key.
pub fn key(reference: &str) -> Result<String, crate::log::LogError> {
    if reference.contains('@') {
        return crate::log::parse_version(reference).map(|version| format!("version/{version}"));
    }
    use base58::FromBase58 as _;
    let hash = reference.strip_prefix('#').unwrap_or(reference);
    let bytes = hash.from_base58().unwrap_or_default();
    if bytes.len() != 32 {
        return Err(crate::log::LogError::InvalidVersion(reference.to_owned()));
    }
    Ok(format!("tree/{hash}"))
}

/// Resolve a retained, verified head on this repository.
pub async fn resolve(site: &crate::site::TonkSite, key: &str) -> anyhow::Result<Option<Revision>> {
    let saved = site
        .repository
        .subject()
        .reader()
        .memory()
        .space(SPACE)
        .cell(key)
        .resolve()
        .perform(site.operator.inner())
        .await?;
    let Some(saved) = saved else { return Ok(None) };
    let revision: Revision = serde_ipld_dagcbor::from_slice(&saved.content)?;
    revision.verify()?;
    anyhow::ensure!(
        key == format!("version/{}", revision.version())
            || key == format!("tree/{}", revision.tree.to_string().trim_start_matches('#')),
        "saved revision does not match {key}"
    );
    Ok(Some(revision))
}
