//! `change/undo`: undo one change to a space, named by the revision that
//! made it.
//!
//! The inverse and the rule for a change a later one touched are
//! [`tonk_evaluator::undo`]'s; this commits what it stages on the space's
//! branch and records the outcome there, as a `tonk:undo`. Two callers:
//! the command, asserted on the space's own branch (a view, the CLI), and
//! the hosted writer, which runs it between its pull and its push so the
//! undo lands before the tool that asked for it answers.

use serde::{Deserialize, Serialize};
use tonk_common::log;
use tonk_evaluator::undo::{UndoOutcome, stage_undo};

use crate::{TonkState, TonkWorkerError};

/// What an undo did, as a caller reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoReport {
    /// The revision whose change was asked to be undone.
    pub revision: String,
    /// `undone` or `refused`.
    pub status: String,
    /// Why it was refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The revision this call committed, when it committed one: the
    /// inverse with its record when `undone` (undoing that revision redoes
    /// the change), the record alone when `refused`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed: Option<String>,
}

/// How many times an undo is staged again after a sync raced its publish.
const RETRY_LIMIT: usize = 3;

/// Undo the change `revision` made on `branch` of the space at `repo`,
/// recording the outcome on that branch.
pub(crate) async fn undo_change(
    tonk: &TonkState,
    repo: &str,
    branch: &str,
    revision: &str,
) -> Result<UndoReport, TonkWorkerError> {
    let session = tonk
        .reactor
        .repository(repo)
        .branch(branch)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("open '{repo}' '{branch}': {error}")))?;
    let (outcome, committed) = {
        // Serialized with every other committer of the branch. A sync can
        // still advance the head between the staging and the publish; then
        // the undo is staged again on the new head, as an evaluate is.
        let _committing = session.transactor().lock().await;
        let mut attempt = 0;
        loop {
            let staged = stage_undo(session.handle(), revision, &tonk.operator)
                .await
                .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
            let outcome = staged.outcome.clone();
            if !staged.writes() {
                break (outcome, None);
            }
            let published = match staged.transaction.commit().perform(&tonk.operator).await {
                Ok(batch) => batch.publish().perform(&tonk.operator).await,
                Err(error) => Err(error),
            };
            match published {
                Ok(revision) => {
                    break (
                        outcome,
                        Some(tonk_schema::encode_revision(&revision.version())),
                    );
                }
                Err(error)
                    if attempt < RETRY_LIMIT && error.to_string().contains("Version mismatch") =>
                {
                    attempt += 1;
                    session
                        .handle()
                        .refresh(&tonk.operator)
                        .await
                        .map_err(|error| TonkWorkerError::Internal(format!("refresh: {error}")))?;
                }
                Err(error) => {
                    return Err(TonkWorkerError::Internal(format!(
                        "commit the undo: {error}"
                    )));
                }
            }
        }
    };
    if committed.is_some() {
        session.poll(&tonk.operator).await;
        tonk.sync_queue.mark_dirty(repo, super::sync::now_millis());
    }
    Ok(match outcome {
        UndoOutcome::Undone { .. } => UndoReport {
            revision: revision.to_owned(),
            status: tonk_schema::Undo::UNDONE.to_owned(),
            reason: None,
            committed,
        },
        UndoOutcome::Refused(refusal) => UndoReport {
            revision: revision.to_owned(),
            status: tonk_schema::Undo::REFUSED.to_owned(),
            reason: Some(refusal.to_string()),
            committed,
        },
    })
}

/// Run `change/undo` on the space branch it was asserted on. The change
/// must be one of that branch's own: the origin is the target, so a space
/// can only undo its own history.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::UndoChange> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::UndoChange) {
        let (repo, branch) = (self.origin().repo.clone(), self.origin().branch.clone());
        let revision = command.revision.0;
        let tonk = self.state().read().await.clone();
        match undo_change(&tonk, &repo, &branch, &revision).await {
            Ok(report) => log!(
                "change/undo: {revision} on {repo} {}{}",
                report.status,
                report
                    .reason
                    .map(|reason| format!(": {reason}"))
                    .unwrap_or_default()
            ),
            Err(error) => log!("change/undo: {revision} on {repo} failed: {error}"),
        }
    }
}
