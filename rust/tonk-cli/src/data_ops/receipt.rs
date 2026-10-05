//! Assert receipts keep commit, local verification, and remote delivery separate.

use serde::Serialize;
use serde_json::Value;

use super::{DataOpError, WriteOptions, query_doc};
use crate::{auto_sync, data, eval, output, site::TonkSite};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Verification {
    status: &'static str,
    scope: &'static str,
    revision: Option<String>,
    rows: Value,
    error: Option<String>,
    #[serde(skip)]
    notation: String,
}

impl Verification {
    fn not_run() -> Self {
        Self {
            status: "not-run",
            scope: "local",
            revision: None,
            rows: Value::Array(vec![]),
            error: None,
            notation: String::new(),
        }
    }

    fn from_read(result: Result<(Option<String>, Value, String), DataOpError>) -> Self {
        match result {
            Ok((revision, rows, notation)) => Self {
                status: if rows.as_array().is_some_and(|rows| !rows.is_empty()) {
                    "verified"
                } else {
                    "not-matched"
                },
                scope: "local",
                revision,
                rows,
                error: None,
                notation,
            },
            Err(error) => Self {
                status: "failed",
                error: Some(error.to_string()),
                ..Self::not_run()
            },
        }
    }
}

/// The full entity and the requested-value constraint are evaluated in the same
/// read transaction. Both must match before returning the full entity projection.
async fn verify(
    site: &TonkSite,
    descriptor: &dialog_query::ConceptDescriptor,
    concept: &str,
    entity: &str,
    pairs: &[(String, String)],
) -> Result<(Option<String>, Value, String), DataOpError> {
    let doc = format!(
        "{}\n{}",
        query_doc(descriptor, concept, Some(entity)),
        data::build_match(descriptor, concept, entity, pairs)?
    );
    let mut read =
        eval::run_against_site(site, eval::Source::Inline(doc), eval::Options::default()).await?;
    let matched = read.response.matches_after.len() == 2
        && read
            .response
            .matches_after
            .iter()
            .all(|block| !block.results.is_empty());
    read.response.matches_after.truncate(1);
    if !matched {
        read.response.matches_after.clear();
    }
    let revision = read
        .response
        .revision_before
        .as_ref()
        .map(|revision| revision.tree.to_string());
    let rows = output::render_results(&read.response, output::Format::Json, concept)
        .map_err(|error| DataOpError::Io(error.to_string()))?;
    let notation = output::render_results(&read.response, output::Format::Notation, concept)
        .map_err(|error| DataOpError::Io(error.to_string()))?;
    Ok((
        revision,
        serde_json::from_str(&rows).map_err(|error| DataOpError::Io(error.to_string()))?,
        notation,
    ))
}

pub(super) struct Request<'a> {
    pub descriptor: &'a dialog_query::ConceptDescriptor,
    pub concept: &'a str,
    pub entity: Option<&'a str>,
    pub pairs: &'a [(String, String)],
    pub write: WriteOptions,
    pub json: bool,
}

pub(super) async fn render(
    site: &TonkSite,
    request: Request<'_>,
    outcome: eval::Outcome,
    sync: auto_sync::SyncReport,
) -> Result<String, DataOpError> {
    let Request {
        descriptor,
        concept,
        entity,
        pairs,
        write,
        json,
    } = request;
    // Prefer canonical identity from the evaluation; aliases remain a fallback
    // for updates, never a guessed identity for a new instance.
    let canonical = outcome
        .response
        .matches_after
        .iter()
        .filter(|block| block.label == concept)
        .flat_map(|block| &block.results)
        .map(|row| row.this.as_str())
        .next()
        .or_else(|| {
            outcome
                .response
                .commits
                .entities
                .get("this")
                .map(String::as_str)
        })
        .or(entity);
    let verification = if write.dry_run {
        Verification::not_run()
    } else if let Some(target) = canonical {
        Verification::from_read(verify(site, descriptor, concept, target, pairs).await)
    } else {
        Verification::from_read(Err(DataOpError::Read(
            "could not identify the committed entity".into(),
        )))
    };
    let before = outcome
        .response
        .revision_before
        .as_ref()
        .map(|revision| revision.tree.to_string());
    let after = outcome
        .response
        .revision_after
        .as_ref()
        .map(|revision| revision.tree.to_string());
    let push = sync.receipt();
    sync.warn_write();
    if json {
        return serde_json::to_string_pretty(&serde_json::json!({
            "schemaVersion": "tonk.assert.v1",
            "concept": concept,
            "entity": canonical,
            "committed": outcome.committed,
            "dryRun": write.dry_run,
            "claims": outcome.response.commits.claims,
            "revisionBefore": before,
            "revisionAfter": after,
            "verification": verification,
            "sync": push,
        }))
        .map(|text| format!("{text}\n"))
        .map_err(|error| DataOpError::Io(error.to_string()));
    }
    let mut out = if let Some(target) = entity {
        format!(
            "{}\nclaims: {}\nrevision: {} -> {}\n",
            write.summarize(format_args!("updated {target}")),
            outcome.response.commits.claims,
            before.as_deref().unwrap_or("none"),
            after.as_deref().unwrap_or("none")
        )
    } else {
        format!(
            "{}\n{}",
            write.summarize(format_args!("asserted {concept}")),
            outcome.stdout
        )
    };
    out.push_str(&format!(
        "verification: {} (local requested fields)\npush: {}\n",
        verification.status,
        push["push"].as_str().unwrap_or("unknown")
    ));
    if verification.status == "verified" && !write.quiet {
        out.push_str("current state:\n");
        out.push_str(&verification.notation);
    } else if verification.status != "verified" && verification.status != "not-run" {
        if let Some(error) = &verification.error {
            out.push_str(&format!("the read-back failed: {error}\n"));
        }
        out.push_str(&format!("The local write was saved; do not repeat it to verify. Inspect with: tonk show {concept} {}\n", canonical.unwrap_or("<ENTITY>")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsuccessful_readbacks_never_report_verification() {
        let empty = Verification::from_read(Ok((None, serde_json::json!([]), String::new())));
        assert_eq!(empty.status, "not-matched");
        let failed = Verification::from_read(Err(DataOpError::Read("read unavailable".into())));
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error.as_deref(), Some("read unavailable"));
        assert_eq!(Verification::not_run().status, "not-run");
    }
    #[dialog_common::test]
    async fn verification_queries_reject_wrong_or_missing_local_values() -> anyhow::Result<()> {
        use dialog_effects::storage::Directory;
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().canonicalize()?;
        std::fs::create_dir_all(root.join("profile"))?;
        let config = crate::site::SiteConfig {
            profile_name: "verification-test".into(),
            profile_directory: Directory::At(root.join("profile").to_string_lossy().into_owned()),
            require_account: false,
            provision_account_spaces: false,
            account_store: crate::space::SpaceStore::at(root.join("state")),
        };
        let site = TonkSite::init_with(&root, config).await?;
        super::super::concept_add(
            &site,
            "task",
            &["title:text:one".into(), "done:boolean:one".into()],
            None,
            Default::default(),
        )
        .await?;
        eval::run_against_site(
            &site,
            eval::Source::Inline("task!: &target\n  title: \"Unchanged\"\n  done: false\n".into()),
            Default::default(),
        )
        .await?;
        let info = super::super::require_concept(&site, "task").await?;
        let wrong = Verification::from_read(
            verify(
                &site,
                &info.descriptor,
                "task",
                "target",
                &[("done".into(), "true".into())],
            )
            .await,
        );
        assert_eq!(wrong.status, "not-matched");
        let right = Verification::from_read(
            verify(
                &site,
                &info.descriptor,
                "task",
                "target",
                &[("done".into(), "false".into())],
            )
            .await,
        );
        assert_eq!(right.status, "verified");
        let missing = Verification::from_read(
            verify(
                &site,
                &info.descriptor,
                "task",
                "id:missing",
                &[("done".into(), "false".into())],
            )
            .await,
        );
        assert_eq!(missing.status, "not-matched");
        Ok(())
    }
}
