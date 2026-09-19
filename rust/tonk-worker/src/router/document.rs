//! Automerge documents: the element route, the command providers and
//! the sync hook.
//!
//! All behaviour lives in the host-neutral `tonk-document` crate; this
//! module only decides WHEN the worker calls it. The CLI calls the same
//! functions from its own write path, so a document command means the
//! same thing in both.
//!
//! The route is data plane, the class the pinned route table allows: an
//! element moves its content and the heads it last saw, with no user
//! intent to record as a fact. It is fixed to the document cell of one
//! entity, so a page can never reach a branch pointer through it, and
//! its path sits under `/api/repository/{repo}/branch/{branch}/…`, which
//! the portal fetch relay already gates by the guest's own reach.
//!
//! Elements hold no automerge. An element sends its content with the
//! heads it last saw; the edit is applied ON TOP OF THOSE HEADS, merged
//! with whatever the branch gained meanwhile, and the reply carries the
//! merged content plus `local` — the heads the element's own content now
//! corresponds to. An element that kept typing during the round trip
//! sends again from `local`, so nothing is applied twice and nothing is
//! lost.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::Entity;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_document::engine::{Content, Edit, Format, Stamp, Table};
use tonk_document::session::{self, Snapshot};

use super::{AppState, CommandEnv};
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// How long after an element asked for a document the sync drain keeps
/// checking its remote cell. Documents nobody has open are not polled.
const REQUEST_WINDOW_SECONDS: f64 = 30.0;

/// Path parameters of the document route.
#[derive(Debug, Deserialize)]
pub struct DocumentPath {
    /// The repository.
    pub repo: String,
    /// The branch whose heads claim selects the version.
    pub branch: String,
    /// The document entity.
    pub entity: String,
}

/// Query parameters of a read.
#[derive(Debug, Default, Deserialize)]
pub struct ReadParams {
    /// The format to create the document with when the entity is not
    /// one yet. An element names its own; without it a missing document
    /// is 404.
    pub format: Option<String>,
}

/// The body of a write.
#[derive(Debug, Deserialize)]
pub struct WriteRequest {
    /// The heads the writer last saw. Absent = the branch's own.
    #[serde(default)]
    pub heads: Option<Vec<String>>,
    /// The edits, applied as one automerge change.
    pub edits: Vec<Edit>,
    /// The format to create with, as in [`ReadParams::format`].
    #[serde(default)]
    pub format: Option<String>,
}

/// A document as one branch sees it, on the wire.
#[derive(Debug, Serialize)]
pub struct DocumentResponse {
    /// `automerge/text@1` or `automerge/table@1`.
    pub format: &'static str,
    /// The heads the branch is at.
    pub heads: Vec<String>,
    /// After a write: the heads the writer's own content corresponds to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<Vec<String>>,
    /// The markdown of a text document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The workbook of a table document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<Table>,
}

impl DocumentResponse {
    fn new(snapshot: Snapshot, local: Option<Vec<String>>) -> Self {
        let (text, table) = match snapshot.content {
            Content::Text(text) => (Some(text), None),
            Content::Table(table) => (None, Some(table)),
        };
        Self {
            format: snapshot.format.name(),
            heads: snapshot.heads,
            local,
            text,
            table,
        }
    }

    fn etag(&self) -> String {
        format!("\"{}\"", self.heads.join("."))
    }
}

type Key = (String, String, String);

#[derive(Default)]
struct Tracker {
    /// When an element last asked for a document, unix seconds.
    requested: HashMap<Key, f64>,
    /// Documents written here since their last successful sync pass.
    dirty: HashSet<Key>,
    /// Branches whose documents were scanned once this worker lifetime:
    /// the dirty set does not survive a restart, the cells do.
    scanned: HashSet<(String, String)>,
}

fn tracker() -> &'static Mutex<Tracker> {
    static TRACKER: OnceLock<Mutex<Tracker>> = OnceLock::new();
    TRACKER.get_or_init(Mutex::default)
}

fn now_seconds() -> f64 {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        js_sys::Date::now() / 1000.0
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64())
    }
}

fn key(repo: &str, branch: &str, entity: &Entity) -> Key {
    (repo.to_string(), branch.to_string(), entity.to_string())
}

fn note_requested(repo: &str, branch: &str, entity: &Entity) {
    if let Ok(mut tracker) = tracker().lock() {
        tracker.requested.insert(key(repo, branch, entity), now_seconds());
    }
}

fn note_dirty(repo: &str, branch: &str, entity: &Entity) {
    if let Ok(mut tracker) = tracker().lock() {
        tracker.dirty.insert(key(repo, branch, entity));
    }
}

fn parse_entity(text: &str) -> Result<Entity, TonkWorkerError> {
    text.parse()
        .map_err(|error| TonkWorkerError::Router(format!("Invalid document entity '{text}': {error}")))
}

fn parse_format(name: Option<&str>) -> Result<Option<Format>, TonkWorkerError> {
    match name {
        None => Ok(None),
        Some(name) => Format::parse(name)
            .map(Some)
            .ok_or_else(|| TonkWorkerError::Router(format!("Unknown document format '{name}'"))),
    }
}

fn session_error(error: session::SessionError) -> TonkWorkerError {
    use session::SessionError;
    match error {
        SessionError::NotADocument(_) => TonkWorkerError::NotFound(error.to_string()),
        SessionError::UnknownFormat(..) => TonkWorkerError::Conflict(error.to_string()),
        SessionError::Document(_) => TonkWorkerError::Router(error.to_string()),
        other => TonkWorkerError::Internal(other.to_string()),
    }
}

/// The writer's stamp: this profile, now.
fn stamp(tonk: &TonkState) -> Stamp {
    Stamp {
        author: Some(tonk.profile.did().to_string()),
        time: now_seconds() as i64,
    }
}

/// Acquire the reactor's session for `repo`/`branch`; an empty `repo`
/// is the profile repository, as in [`super::CommandOrigin`].
async fn acquire(
    tonk: &TonkState,
    repo: &str,
    branch: &str,
) -> Result<dialog_reactor::BranchSession, TonkWorkerError> {
    let reference = if repo.is_empty() {
        tonk.reactor.profile_repository().branch(branch)
    } else {
        tonk.reactor.repository(repo).branch(branch)
    };
    reference
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("branch {repo}/{branch}: {error}")))
}

/// Refresh the mirror of `entity` and let subscribers know.
async fn refresh_mirror(tonk: &TonkState, session: &dialog_reactor::BranchSession, entity: &Entity) {
    match session::mirror(session.handle(), entity, &tonk.operator).await {
        Ok(_) => {
            tonk.reactor.schedule_poll(Arc::clone(&session.state));
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        }
        Err(error) => log!("document mirror of {entity} failed: {error}"),
    }
}

/// `GET /api/repository/{repo}/branch/{branch}/document/{entity}`
///
/// The document as this branch sees it. `If-None-Match` with the last
/// ETag answers 304 when the branch's heads did not move; the request
/// never leaves the device, so an element can watch cheaply.
#[wasm_compat]
pub async fn read(
    State(state): State<AppState>,
    Path(path): Path<DocumentPath>,
    Query(params): Query<ReadParams>,
    headers: HeaderMap,
) -> Result<Response, TonkWorkerError> {
    let entity = parse_entity(&path.entity)?;
    let create = parse_format(params.format.as_deref())?;
    let tonk = state.read().await;
    let session = acquire(&tonk, &path.repo, &path.branch).await?;
    note_requested(&path.repo, &path.branch, &entity);

    let before = session.handle().revision();
    let snapshot = session::read(session.handle(), &entity, create, &tonk.operator)
        .await
        .map_err(session_error)?;
    if session.handle().revision() != before {
        // The open declared or converted the document.
        note_dirty(&path.repo, &path.branch, &entity);
        tonk.sync_queue
            .mark_dirty(&path.repo, super::sync::now_millis());
        refresh_mirror(&tonk, &session, &entity).await;
    }

    let body = DocumentResponse::new(snapshot, None);
    let etag = body.etag();
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|seen| seen == etag);
    if unchanged {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }
    Ok(([(header::ETAG, etag)], Json(body)).into_response())
}

/// `POST /api/repository/{repo}/branch/{branch}/document/{entity}`
///
/// Apply edits on top of the heads the writer last saw.
#[wasm_compat]
pub async fn write(
    State(state): State<AppState>,
    Path(path): Path<DocumentPath>,
    body: Bytes,
) -> Result<Response, TonkWorkerError> {
    let entity = parse_entity(&path.entity)?;
    let request: WriteRequest = serde_json::from_slice(&body)
        .map_err(|error| TonkWorkerError::Router(format!("invalid document write: {error}")))?;
    let create = parse_format(request.format.as_deref())?;
    let tonk = state.read().await;
    let session = acquire(&tonk, &path.repo, &path.branch).await?;
    note_requested(&path.repo, &path.branch, &entity);

    let written = session::write(
        session.handle(),
        &entity,
        create,
        request.heads.as_deref(),
        &request.edits,
        &stamp(&tonk),
        &tonk.operator,
    )
    .await
    .map_err(session_error)?;

    note_dirty(&path.repo, &path.branch, &entity);
    tonk.sync_queue
        .mark_dirty(&path.repo, super::sync::now_millis());
    refresh_mirror(&tonk, &session, &entity).await;

    let body = DocumentResponse::new(written.snapshot, Some(written.local));
    let etag = body.etag();
    Ok(([(header::ETAG, etag)], Json(body)).into_response())
}

/// Run one document command on the branch it fired in. The behaviour
/// is `tonk_document::command::run`, the same function the CLI calls;
/// this wrapper adds what only the worker has — the sync queue, the
/// mirror and the subscription poll. A refused edit (no match, two
/// matches, a bad path) changes nothing and is reported as an overlay
/// fact on the command's own entity, so the page that asked can read
/// why.
async fn run(env: &CommandEnv, request: tonk_document::command::Request) {
    let origin = env.origin().clone();
    let document = request.document.clone();
    let tonk = env.state().read().await;
    let session = match acquire(&tonk, &origin.repo, &origin.branch).await {
        Ok(session) => session,
        Err(error) => return log!("document command on {document}: {error}"),
    };
    let result =
        tonk_document::command::run(session.handle(), &tonk.operator, &stamp(&tonk), &request).await;
    match result {
        Ok(_) => {
            note_dirty(&origin.repo, &origin.branch, &document);
            if !origin.repo.is_empty() {
                tonk.sync_queue
                    .mark_dirty(&origin.repo, super::sync::now_millis());
            }
            refresh_mirror(&tonk, &session, &document).await;
        }
        Err(error) => {
            log!("document command on {document} refused: {error}");
            session.state.assert_overlay(session::failure(
                &request.command,
                &document,
                &error.to_string(),
            ));
            tonk.reactor.schedule_poll(Arc::clone(&session.state));
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        }
    }
}

macro_rules! document_provider {
    ($command:ty) => {
        #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
        #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
        impl dialog_capability::Provider<$command> for CommandEnv {
            async fn execute(&self, command: $command) {
                run(self, command.into()).await;
            }
        }
    };
}

document_provider!(tonk_schema::command::DocumentReplace);
document_provider!(tonk_schema::command::DocumentInsert);
document_provider!(tonk_schema::command::DocumentSplice);
document_provider!(tonk_schema::command::DocumentPut);
document_provider!(tonk_schema::command::DocumentRemove);
document_provider!(tonk_schema::command::DocumentRestore);

/// The document half of a repository's sync sweep: run the sync pass
/// for every document of `branch` that is dirty or that an element
/// asked for lately. Called after the branch itself reconciled, so the
/// heads claims a pull brought in can be shown as soon as their bytes
/// land.
pub(crate) async fn sync_documents(state: &AppState, repo: &str, branch: &str) -> Result<(), String> {
    let tonk = state.read().await;
    let session = acquire(&tonk, repo, branch)
        .await
        .map_err(|error| error.to_string())?;
    let handle = session.handle();

    let first_scan = tracker()
        .lock()
        .map(|mut tracker| tracker.scanned.insert((repo.to_string(), branch.to_string())))
        .unwrap_or(false);

    let mut candidates: HashSet<String> = HashSet::new();
    if let Ok(tracker) = tracker().lock() {
        let now = now_seconds();
        for ((r, b, entity), at) in &tracker.requested {
            if r == repo && b == branch && now - at <= REQUEST_WINDOW_SECONDS {
                candidates.insert(entity.clone());
            }
        }
        for (r, b, entity) in &tracker.dirty {
            if r == repo && b == branch {
                candidates.insert(entity.clone());
            }
        }
    }
    if first_scan {
        // Claims-era prose bodies and workbooks convert once: a
        // document-mode view matches on the format claim, so until then
        // an old entity renders nothing.
        match session::adopt_legacy(handle, &tonk.operator).await {
            Ok(0) => {}
            Ok(converted) => {
                log!("converted {converted} claims-era documents in {repo}/{branch}");
                tonk.sync_queue.mark_dirty(repo, super::sync::now_millis());
            }
            Err(error) => log!("document conversion in {repo}/{branch} failed: {error}"),
        }
        // After a restart the dirty set is gone but the cells are not:
        // find unpublished work once, and fill the mirror while here.
        match session::documents(handle, &tonk.operator).await {
            Ok(documents) => {
                for (entity, _) in documents {
                    if session::is_dirty(handle, &entity, &tonk.operator)
                        .await
                        .unwrap_or(false)
                    {
                        candidates.insert(entity.to_string());
                    }
                    refresh_mirror(&tonk, &session, &entity).await;
                }
            }
            Err(error) => log!("document scan of {repo}/{branch} failed: {error}"),
        }
    }

    let mut failed = false;
    for text in candidates {
        let Ok(entity) = text.parse::<Entity>() else {
            continue;
        };
        match session::sync(handle, &entity, &tonk.operator).await {
            Ok(outcome) => {
                if let Ok(mut tracker) = tracker().lock() {
                    tracker.dirty.remove(&key(repo, branch, &entity));
                }
                if outcome.pulled {
                    refresh_mirror(&tonk, &session, &entity).await;
                }
            }
            Err(error) => {
                log!("document sync of {entity} in {repo}/{branch} failed: {error}");
                failed = true;
            }
        }
    }
    if failed {
        Err(format!("documents of '{repo}/{branch}' did not fully sync"))
    } else {
        Ok(())
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use super::*;
    use crate::router::command::tests::native::test_state;
    use crate::router::{CommandOrigin, dispatch};
    use ::axum::body::to_bytes;
    use dialog_artifacts::{Changes, Statement as _};
    use dialog_query::{Output as _, Query as ConceptQuery, Term, the};
    use tonk_schema::Replica;
    use tonk_schema::prelude::DidExt as _;

    /// Create a space through the real command and return its repo key.
    async fn space(state: &AppState) -> String {
        let mut changes = Changes::new();
        the!("xyz.tonk.command.create-space/name")
            .of("cmd:create".parse::<Entity>().unwrap())
            .is("Documents".to_string())
            .assert(&mut changes);
        dispatch(state, CommandOrigin::default(), changes).await;

        let tonk = state.read().await;
        let meta = tonk
            .reactor
            .profile_repository()
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let rows: Vec<Replica> = meta
            .handle()
            .query()
            .select(ConceptQuery::<Replica> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                profile: Term::var("profile"),
                kind: Term::var("kind"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        let subject: dialog_varsig::Did = rows
            .into_iter()
            .find(|replica| replica.kind == Replica::repository_kind())
            .and_then(|replica| replica.subject.0.to_string().parse().ok())
            .expect("the space was created");
        subject.repo_key().to_owned()
    }

    fn path(repo: &str) -> Path<DocumentPath> {
        Path(DocumentPath {
            repo: repo.to_string(),
            branch: "main".to_string(),
            entity: "id:prose/doc".to_string(),
        })
    }

    async fn json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn post(state: &AppState, repo: &str, body: serde_json::Value) -> serde_json::Value {
        let response = write(State(state.clone()), path(repo), Bytes::from(body.to_string()))
            .await
            .expect("the write is accepted");
        json(response).await
    }

    fn heads(value: &serde_json::Value, field: &str) -> Vec<String> {
        serde_json::from_value(value[field].clone()).unwrap()
    }

    #[dialog_common::test]
    async fn it_serves_a_document_to_an_element_and_runs_a_command_on_it() {
        let state = test_state().await;
        let repo = space(&state).await;

        // A missing document is 404 until a caller names a format.
        let missing = read(State(state.clone()), path(&repo), Query(ReadParams::default()), HeaderMap::new()).await;
        assert!(matches!(missing, Err(TonkWorkerError::NotFound(_))));

        // The element's first write creates it.
        let first = post(
            &state,
            &repo,
            serde_json::json!({ "format": "automerge/text@1", "edits": [{ "edit": "set-text", "text": "hello world" }] }),
        )
        .await;
        assert_eq!(first["text"], "hello world");
        assert_eq!(first["format"], "automerge/text@1");

        // A read returns the same heads; the ETag answers 304 after.
        let response = read(State(state.clone()), path(&repo), Query(ReadParams::default()), HeaderMap::new())
            .await
            .unwrap();
        let etag = response.headers().get(header::ETAG).unwrap().clone();
        let body = json(response).await;
        assert_eq!(heads(&body, "heads"), heads(&first, "heads"));
        let mut seen = HeaderMap::new();
        seen.insert(header::IF_NONE_MATCH, etag);
        let again = read(State(state.clone()), path(&repo), Query(ReadParams::default()), seen)
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        // An agent asserts `document/replace`: the same command a page
        // sends through /transact, dispatched on the space's own branch.
        let mut command = Changes::new();
        let this: Entity = "cmd:replace".parse().unwrap();
        the!("xyz.tonk.document.replace/document")
            .of(this.clone())
            .is("id:prose/doc".parse::<Entity>().unwrap())
            .assert(&mut command);
        the!("xyz.tonk.document.replace/find")
            .of(this.clone())
            .is("world".to_string())
            .assert(&mut command);
        the!("xyz.tonk.document.replace/with")
            .of(this)
            .is("there".to_string())
            .assert(&mut command);
        let origin = CommandOrigin {
            repo: repo.clone(),
            branch: "main".to_string(),
            client: None,
        };
        dispatch(&state, origin, command).await;

        let after = json(
            read(State(state.clone()), path(&repo), Query(ReadParams::default()), HeaderMap::new())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(after["text"], "hello there", "the command edited the document");

        // The element, which still holds the OLD heads, sends its own
        // edit: both survive.
        let merged = post(
            &state,
            &repo,
            serde_json::json!({ "heads": heads(&first, "heads"), "edits": [{ "edit": "set-text", "text": "oh hello world" }] }),
        )
        .await;
        assert_eq!(merged["text"], "oh hello there");
        assert_ne!(heads(&merged, "local"), heads(&merged, "heads"));
    }
}
