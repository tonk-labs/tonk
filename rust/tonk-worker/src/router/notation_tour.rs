//! Demo (notation tour): a notebook cell written in the Dialog notation,
//! posted to the existing `/evaluate` route with its own content type.
//!
//! No route is added. A ```notation cell in `<tonk-notebook>` posts
//! `{source, context}` as `application/x-dialog-notation+json`; the
//! `/evaluate` handler hands such a body here instead of to the
//! asserted-notation evaluator. The cell is evaluated by
//! `tonk_analyzer::notation::tour` against the branch's own handle, under
//! the branch's writer lock as `/evaluate` commits, and what it committed
//! is announced the way `/evaluate` announces a commit, so every open
//! page sees it.
//!
//! Demo scaffolding: delete this file, its `mod` line in `router.rs` and
//! the branch at the top of `evaluate::evaluate` to remove it.

use ::axum::Json;
use ::axum::http::HeaderMap;
use ::axum::http::header::CONTENT_TYPE as CONTENT_TYPE_HEADER;
use ::axum::response::{IntoResponse as _, Response};
use bytes::Bytes;
use tonk_analyzer::notation::tour::{Cell, Sink, prepare};

use super::AppState;
use crate::TonkWorkerError;
use crate::broadcast::{Notification, broadcast};

/// The content type a notation cell posts.
pub(super) const CONTENT_TYPE: &str = "application/x-dialog-notation+json";

/// Whether a request to `/evaluate` is a notation cell.
pub(super) fn is_cell(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with(CONTENT_TYPE))
}

/// Evaluate one notation cell on `repo`'s `branch`.
pub(super) async fn evaluate(
    state: &AppState,
    repo: &str,
    branch: &str,
    transact: bool,
    body: &Bytes,
) -> Result<Response, TonkWorkerError> {
    let cell: Cell = serde_json::from_slice(body)
        .map_err(|error| TonkWorkerError::Router(format!("a notation cell: {error}")))?;
    let tonk = state.read().await.clone();
    super::adopt::mount_on_first_use(&tonk, state, repo).await;
    let reference = tonk.reactor.repository(repo).branch(branch);
    reference
        .migrate_once(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("migrating the branch: {error}")))?;
    let session = reference
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(error.to_string()))?;
    let started = web_time::Instant::now();
    let (answer, effect) = {
        // The same writer lock `/evaluate` and every reactor commit take,
        // and the same stack: a cell's commit is a commit like any other.
        let _committing = session.transactor().lock().await;
        let handle = session.handle().clone();
        let _ = handle.refresh(&tonk.operator).await;
        let prepared = prepare(&handle, &cell, &tonk.operator).await;
        if transact && prepared.writes() {
            let stack = session.state.stack();
            let committed = match prepared
                .write_into(Stacked(stack.transaction()))
                .0
                .commit()
                .perform(&tonk.operator)
                .await
            {
                Ok(batch) => batch.publish().perform(&tonk.operator).await,
                Err(error) => Err(error),
            };
            match committed {
                Ok(report) => {
                    let effect = report.effect();
                    let _ = handle.refresh(&tonk.operator).await;
                    (
                        prepared.finish(&handle, true, &tonk.operator).await,
                        Some(effect),
                    )
                }
                Err(error) => (prepared.failed(error.to_string()), None),
            }
        } else {
            (prepared.finish(&handle, false, &tonk.operator).await, None)
        }
    };
    tonk_common::log!(
        "notation cell on {repo}/{branch}: {} form(s), wrote {}, {:?}",
        answer.forms.len(),
        answer.wrote,
        started.elapsed()
    );
    if answer.wrote {
        // Announce it as `/evaluate` does, so open subscriptions re-read.
        session.poll(&tonk.operator).await;
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        if let Some(revision) = session.handle().revision() {
            let notification = Notification {
                branch: branch.to_owned(),
                revision,
            };
            broadcast(
                &format!("/api/repository/{repo}/branch/{branch}"),
                &notification,
            );
            broadcast(crate::broadcast::LOCAL_COMMIT_CHANNEL, &notification);
        }
        tonk.sync_queue.mark_dirty(repo, super::sync::now_millis());
        let _ = effect;
    }
    Ok(Json(answer).into_response())
}

/// A stack transaction, as the place a cell's writes go.
struct Stacked<'a>(dialog_repository::StackTransaction<&'a dialog_repository::Branch>);

impl Sink for Stacked<'_> {
    fn assert<C: dialog_corpus::Statement>(self, claim: C) -> Self {
        Stacked(self.0.assert(claim))
    }
    fn retract<C: dialog_corpus::Statement>(self, claim: C) -> Self {
        Stacked(self.0.retract(claim))
    }
    fn dispatch<C: dialog_corpus::Statement>(self, claim: C) -> Self {
        Stacked(self.0.dispatch(claim))
    }
}
