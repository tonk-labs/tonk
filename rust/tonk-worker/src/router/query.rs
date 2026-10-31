//! `POST /api/repository/{repo}/branch/{branch}/query` —
//! one-shot query or live subscription, depending on the
//! `Accept` header.
//!
//! Body: a serialized [`crate::reactor::Query`] (the on-the-wire
//! shape of a `dialog_query::ConceptQuery`).
//!
//! - **Default** — returns `Vec<Conclusion>` as JSON.
//! - **`Accept: text/event-stream`** — opens an SSE subscription;
//!   the body emits `data: <Vec<Conclusion>>\n\n` events.
//!   First event is the current snapshot; subsequent events fire
//!   whenever a commit/pull/sync changes the result.

use ::axum::Json;
use ::axum::body::{Body, Bytes};
use ::axum::extract::{Path, Query as UrlQuery, Request, State};
use ::axum::http::{HeaderMap, StatusCode, header};
use ::axum::response::{IntoResponse, Response};
use http_body_util::BodyExt as _;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonk_handler_macros::wasm_compat;

use crate::reactor::{Conclusion, Query, ReactorError};
use crate::router::update_pending;
use crate::{TonkWorkerError, router::AppState};

/// Path parameters for `/query`.
#[derive(Debug, Deserialize)]
pub struct QueryPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Path parameters for the profile `/query` (no `repo` segment —
/// the profile lives outside the named-repo namespace).
#[derive(Debug, Deserialize)]
pub struct ProfileQueryPath {
    /// The branch name.
    pub branch: String,
}

/// URL parameters for `/query`.
#[derive(Debug, Default, Deserialize)]
pub struct QueryParams {
    /// Shape this query with the named concept's typed edges. SSE replies
    /// use `graph_snapshot` and `graph_delta` frames when this is present.
    pub graph: Option<String>,
    /// How deeply the subscribing display is nested (0 when absent).
    /// The branch notifies lower levels first, so a display learns of a
    /// change before the displays nested inside it.
    #[serde(default)]
    pub level: u32,
}

/// Explicit envelope accepted only with `?graph=<bookmark>`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphRequest {
    query: Query,
    #[serde(default)]
    constraints: tonk_schema::graph::GraphConstraints,
}

fn decode_query(
    bytes: &[u8],
    graph: bool,
) -> Result<(Query, tonk_schema::graph::GraphConstraints), serde_json::Error> {
    if graph {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        if value.get("query").is_some() {
            let request: GraphRequest = serde_json::from_value(value)?;
            return Ok((request.query, request.constraints));
        }
    }
    Ok((serde_json::from_slice(bytes)?, Default::default()))
}

/// Handler. Reads body, decides one-shot vs subscription based on
/// `Accept`, dispatches to the reactor.
#[wasm_compat]
pub async fn query(
    State(state): State<AppState>,
    Path(path): Path<QueryPath>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, TonkWorkerError> {
    if super::names_profile(&state, &path.repo).await {
        let path = ProfileQueryPath {
            branch: path.branch,
        };
        return query_profile(State(state), Path(path), headers, request).await;
    }
    let client = request_client(&request);
    let tonk = state.read().await;
    // First use of a directory-listed space this device has not
    // replicated mounts it on demand — the lazy half of
    // directory-driven adoption. A no-op for mounted repos.
    super::adopt::mount_on_first_use(&tonk, &state, &path.repo).await;
    let target = QueryTarget::Space {
        repo: path.repo.clone(),
        branch: path.branch.clone(),
    };
    query_on_branch(&state, &tonk, target, headers, request, client).await
}

/// [`query`] for the profile's own repository, which [`query`] hands a
/// request naming it. Same body / `Accept` / response contract; there is
/// no space to mount. Lets a `<tonk-display>` read the profile's meta
/// branch (e.g. the Hub's list of spaces) the same way it reads any
/// repository branch.
#[wasm_compat]
async fn query_profile(
    State(state): State<AppState>,
    Path(path): Path<ProfileQueryPath>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, TonkWorkerError> {
    let client = request_client(&request);
    let tonk = state.read().await;
    let target = QueryTarget::Profile {
        branch: path.branch.clone(),
    };
    query_on_branch(&state, &tonk, target, headers, request, client).await
}

/// The requesting SW client's id, when the fetch handler stamped one.
/// Tags SSE subscribers so the stale-client sweep can prune the ones
/// whose page is gone (a dead client's stream may never cancel, so
/// send-failure pruning alone can't be relied on).
fn request_client(request: &Request) -> Option<String> {
    request
        .extensions()
        .get::<crate::router::ClientId>()
        .map(|c| c.0.clone())
        .filter(|id| !id.is_empty())
}

/// Move what an earlier release stored on `branch` to the shape this one
/// reads and writes, the first time this worker opens the branch (see
/// [`crate::reactor::BranchReference::migrate_once`]): the storage rewrite
/// under the number band, attribute and concept definitions recorded the
/// earlier way, and rules stored under an identity an earlier dialog
/// release gave them. What moved is logged; a failure is logged and
/// returned, so a stream can tell its page, and the read proceeds on what
/// the earlier release left.
pub(super) async fn migrate_while_opening(
    tonk: &crate::worker::TonkState,
    branch: crate::reactor::BranchReference<'_>,
) -> Result<(), ReactorError> {
    match branch.migrate_once(&tonk.operator).await {
        Ok(Some(upgraded)) => {
            if upgraded.storage.is_some() {
                tonk_common::log!(
                    "rewrote the storage of '{}' under one number band",
                    branch.repository.name()
                );
            }
            if !upgraded.definitions.is_empty() {
                tonk_common::log!(
                    "moved {} definition(s) on '{}' to their current identity",
                    upgraded.definitions.moves.len() + upgraded.definitions.rewritten.len(),
                    branch.repository.name()
                );
            }
            if !upgraded.rules.reinstalled.is_empty() {
                tonk_common::log!(
                    "re-installed {} rule(s) on '{}' under their current identity",
                    upgraded.rules.reinstalled.len(),
                    branch.repository.name()
                );
            }
            if !upgraded.rules.rewritten.is_empty() {
                tonk_common::log!(
                    "rewrote {} rule body(ies) on '{}' in the current format",
                    upgraded.rules.rewritten.len(),
                    branch.repository.name()
                );
            }
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(error) if is_absence(&error) => Ok(()),
        Err(error) => {
            tonk_common::log!("upgrade on '{}' failed: {error}", branch.repository.name());
            Err(error)
        }
    }
}

/// The control frame a stream opens with while its branch is being moved
/// to this release's storage format. The host hands it to the page as a
/// notice; the rows follow when the move is done.
const UPGRADING_FRAME: &[u8] = br#"{"control":"upgrading"}"#;

/// Where a query's branch lives, by name, so a stream that outlives the
/// request that opened it finds the branch again.
#[derive(Clone)]
enum QueryTarget {
    /// A space, by repository name.
    Space { repo: String, branch: String },
    /// The profile's own repository.
    Profile { branch: String },
}

impl QueryTarget {
    fn branch<'a>(
        &'a self,
        tonk: &'a crate::worker::TonkState,
    ) -> crate::reactor::BranchReference<'a> {
        match self {
            QueryTarget::Space { repo, branch } => tonk.reactor.repository(repo).branch(branch),
            QueryTarget::Profile { branch } => tonk.reactor.profile_repository().branch(branch),
        }
    }
}

/// Run `future` detached from the request: on the service worker as a
/// local task, natively on the runtime.
fn spawn_detached<F>(future: F)
where
    F: std::future::Future<Output = ()> + dialog_common::ConditionalSend + 'static,
{
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_futures::spawn_local(future);
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    drop(tokio::spawn(future));
}

/// The stream a page gets when the branch it subscribes to is about to be
/// moved to this release's storage format: it opens with
/// [`UPGRADING_FRAME`] at once, so the page says an upgrade is taking
/// place rather than loading; the move runs detached from this request;
/// then the subscription opens and its frames follow on the same stream.
/// A failed move sends `{"control":"upgrade-failed","reason":…}` and the
/// subscription still opens, on what the earlier release left.
fn upgrading_response(
    state: AppState,
    target: QueryTarget,
    query: dialog_query::ConceptQuery,
    level: u32,
    client: Option<String>,
) -> Response {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
    let _ = sender.send(Bytes::from_static(UPGRADING_FRAME));
    spawn_detached(async move {
        // The guard is held for the move and the subscribe only: the
        // frames are forwarded below without it, as a stream the request
        // returned directly would be.
        let subscribed = {
            let tonk = state.read().await;
            let branch = target.branch(&tonk);
            if let Err(error) = migrate_while_opening(&tonk, branch).await {
                let failed = serde_json::json!({
                    "control": "upgrade-failed",
                    "reason": error.to_string(),
                });
                let _ = sender.send(Bytes::from(failed.to_string()));
            }
            let mut subscribe = branch.subscribe(query.clone()).level(level);
            if let Some(client) = client.clone() {
                subscribe = subscribe.client(client);
            }
            match subscribe.perform(&tonk.operator).await {
                Ok(subscriber) => Ok(subscriber.receiver),
                Err(error) if is_absence(&error) => {
                    let _ = sender.send(Bytes::from_static(b"[]"));
                    Err(())
                }
                Err(error) => {
                    tonk_common::log!("subscribe after upgrade failed: {error}");
                    Err(())
                }
            }
        };
        let Ok(mut frames) = subscribed else {
            return;
        };
        while let Some(frame) = frames.recv().await {
            if sender.send(frame).is_err() {
                break;
            }
        }
    });
    sse_response(receiver)
}

/// Shared body for [`query`] and [`query_profile`]. Takes a
/// [`crate::reactor::BranchReference`] so the URL extraction is the
/// only difference between the two routes.
async fn query_on_branch(
    state: &AppState,
    tonk: &crate::worker::TonkState,
    target: QueryTarget,
    headers: HeaderMap,
    request: Request,
    client: Option<String>,
) -> Result<Response, TonkWorkerError> {
    let branch = target.branch(tonk);
    let UrlQuery(params) = UrlQuery::<QueryParams>::try_from_uri(request.uri())
        .map_err(|e| TonkWorkerError::Router(format!("invalid query parameters: {e}")))?;
    let bytes = request
        .into_body()
        .collect()
        .await
        .map_err(|e| TonkWorkerError::Router(format!("failed to read body: {e}")))?
        .to_bytes();
    let (wire, constraints) = decode_query(&bytes, params.graph.is_some())
        .map_err(|e| TonkWorkerError::Router(format!("invalid query body: {e}")))?;

    let want_stream = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("text/event-stream"));

    // A formula query (string predicate) is resolved by the worker
    // rather than dialog's planner — the `tree/*` introspection
    // family. One-shot only for now.
    let query = match wire.into_concept_query() {
        Ok(query) => query,
        Err(wire) => {
            if params.graph.is_some() {
                return Err(TonkWorkerError::Router(
                    "graph queries require a concept predicate".into(),
                ));
            }
            if want_stream {
                return Err(TonkWorkerError::Router(
                    "formula queries do not support subscriptions yet".into(),
                ));
            }
            let session = branch
                .acquire(&tonk.operator)
                .await
                .map_err(reactor_to_error)?;
            let conclusions =
                crate::reactor::resolve_formula(session.handle(), &tonk.operator, &wire)
                    .await
                    .map_err(|e| TonkWorkerError::Router(e.to_string()))?;
            return Ok(Json(conclusions).into_response());
        }
    };

    if let Some(name) = params.graph {
        if want_stream && (update_pending() || tonk.is_retiring()) {
            return Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::RETRY_AFTER, "1")],
                "worker is retiring",
            )
                .into_response());
        }
        migrate_while_opening(tonk, branch)
            .await
            .map_err(reactor_to_error)?;
        if want_stream {
            let mut subscribe = branch
                .subscribe(query)
                .graph(name)
                .graph_constraints(constraints)
                .level(params.level);
            if let Some(client) = client {
                subscribe = subscribe.client(client);
            }
            let subscriber = subscribe
                .perform(&tonk.operator)
                .await
                .map_err(reactor_to_error)?;
            return Ok(sse_response(subscriber.receiver));
        }
        let session = branch
            .acquire(&tonk.operator)
            .await
            .map_err(reactor_to_error)?;
        let mut graph = tonk_schema::graph::resolve_graph(
            query,
            &name,
            &session.handle().into(),
            &tonk.operator,
        )
        .await
        .map_err(|error| TonkWorkerError::Router(error.to_string()))?
        .ok_or_else(|| TonkWorkerError::Router(format!("concept {name:?} has no typed edges")))?;
        constraints
            .apply(&mut graph)
            .map_err(|error| TonkWorkerError::Router(error.to_string()))?;
        let result = session
            .handle()
            .query()
            .graph(graph)
            .perform(&tonk.operator)
            .await
            .map_err(|error| TonkWorkerError::Router(error.to_string()))?;
        return Ok(Json(result).into_response());
    }

    if want_stream {
        // Once this worker starts retiring, refuse to open a long-lived
        // stream: an SSE response is a fetch event that never settles, so one
        // reopened stream would keep the outgoing instance serving stale
        // clients after replacement. A live `waiting` check closes the race
        // before the Rust retirement
        // hook runs; the worker-owned latch keeps the refusal terminal after
        // the successor activates and `registration.waiting` becomes empty.
        if update_pending() || tonk.is_retiring() {
            let snapshot = match branch.query(query.clone()).perform(&tonk.operator).await {
                Ok(rows) => rows,
                Err(error) if is_absence(&error) => Vec::new(),
                Err(error) => return Err(reactor_to_error(error)),
            };
            return Ok(retry_later(snapshot));
        }

        // A branch an earlier release wrote is moved to this release's
        // format before it is read. The page hears so first: the stream
        // opens with a notice, and the rows follow the move.
        if branch
            .migration_pending(&tonk.operator)
            .await
            .unwrap_or(false)
        {
            return Ok(upgrading_response(
                state.clone(),
                target,
                query,
                params.level,
                client,
            ));
        }
        let _ = migrate_while_opening(tonk, branch).await;

        let mut subscribe = branch.subscribe(query.clone()).level(params.level);
        if let Some(client) = client.clone() {
            subscribe = subscribe.client(client);
        }
        let subscriber = match subscribe.perform(&tonk.operator).await {
            Ok(subscriber) => subscriber,
            Err(ReactorError::Shutdown) => {
                let snapshot = match branch.query(query).perform(&tonk.operator).await {
                    Ok(rows) => rows,
                    Err(error) if is_absence(&error) => Vec::new(),
                    Err(error) => return Err(reactor_to_error(error)),
                };
                return Ok(retry_later(snapshot));
            }
            // The branch is not here YET. Absence is a result, not a
            // failure, so answer with an open stream carrying the empty
            // set and keep watching: when the repo mounts (a join in
            // another tab, a directory row syncing in), the standing
            // subscription attaches and delivers a real frame into this
            // same stream. Erroring here instead closed the stream, and
            // the page had nothing left to hear the answer on.
            // The branch is not here YET. Register in the reactor's
            // waiting room and answer with an OPEN stream carrying the
            // empty set: nothing matched, which is exactly what a
            // present-but-empty branch answers. When the repo mounts or
            // the branch is created, `acquire` moves this subscriber
            // onto the real branch and the next poll delivers rows into
            // this same stream. No polling, no reconnect.
            Err(error) if is_absence(&error) => {
                let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
                // The honest current answer, delivered immediately so the
                // consumer leaves `loading` and renders its empty state.
                let _ = sender.send(Bytes::from_static(b"[]"));
                if tonk
                    .reactor
                    .register_pending(
                        branch.repository.name(),
                        branch.name,
                        dialog_reactor::PendingSubscription {
                            query,
                            client,
                            level: params.level,
                            sender,
                        },
                    )
                    .is_err()
                {
                    return Ok(retry_later(Vec::new()));
                }
                return Ok(sse_response(receiver));
            }
            Err(error) => return Err(reactor_to_error(error)),
        };

        Ok(sse_response(subscriber.receiver))
    } else {
        let _ = migrate_while_opening(tonk, branch).await;
        // An absent repo/branch answers with the EMPTY SET, not a 404 —
        // "nothing matched" is a result, and it is the same result a
        // present-but-empty branch gives. See [`is_absence`].
        let wire: Vec<Conclusion> = match branch.query(query).perform(&tonk.operator).await {
            Ok(rows) => rows,
            Err(error) if is_absence(&error) => Vec::new(),
            Err(error) => return Err(reactor_to_error(error)),
        };
        Ok(Json(wire).into_response())
    }
}

/// Tell a query consumer to hold its reconnect for `controllerchange`, then
/// close immediately so this response cannot pin the retiring worker.
fn retry_later(conclusions: Vec<Conclusion>) -> Response {
    let snapshot = serde_json::to_string(&tonk_worker_api::Frame::Snapshot { conclusions })
        .expect("snapshot frame serialization failed");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(format!(
            "data: {snapshot}\n\ndata: {{\"control\":\"update-pending\"}}\n\n"
        )))
        .expect("response builder failed")
}

fn reactor_to_error(err: ReactorError) -> TonkWorkerError {
    match err {
        ReactorError::Shutdown => TonkWorkerError::Internal(err.to_string()),
        ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. } => {
            TonkWorkerError::NotFound(err.to_string())
        }
        ReactorError::Graph(_)
        | ReactorError::QueryFailed(_)
        | ReactorError::Commit(_)
        | ReactorError::Pull(_)
        | ReactorError::Download(_)
        | ReactorError::Push(_)
        | ReactorError::Upgrade { .. } => TonkWorkerError::Internal(err.to_string()),
    }
}

/// Frame an subscriber's receiver as an SSE response.
///
/// Shared by the ordinary subscribe path and the waiting-room path so a
/// pending subscription is indistinguishable on the wire from a live
/// one — same content type, same framing, same open stream.
fn sse_response(receiver: tokio::sync::mpsc::UnboundedReceiver<Bytes>) -> Response {
    let body_stream = UnboundedReceiverStream::new(receiver).map(|bytes: Bytes| {
        let mut framed = Vec::with_capacity(bytes.len() + 8);
        framed.extend_from_slice(b"data: ");
        framed.extend_from_slice(&bytes);
        framed.extend_from_slice(b"\n\n");
        Ok::<_, std::io::Error>(Bytes::from(framed))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::from_stream(body_stream))
        .expect("response builder failed")
}

/// Whether this failure means "there is nothing here" rather than
/// "something went wrong".
///
/// A repository or branch this device has not replicated is an ABSENCE,
/// and absence is the same answer a query gets when a branch exists but
/// holds no matching rows: the empty set. Reporting it as `404` made it
/// a transport failure instead of a result, which every consumer then
/// had to special-case — and, because a failed subscription has no
/// frames, a page that asked before the repo arrived could never be told
/// when it did. A space joined in another tab left the first tab showing
/// "not here" forever.
///
/// Absence is not permanent: the subscription stays open against the
/// named branch, so the moment the repo mounts, the standing query
/// delivers a real frame.
fn is_absence(err: &ReactorError) -> bool {
    matches!(
        err,
        ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// A repository or branch this device has not replicated is an
    /// ABSENCE, not a failure. Classifying it as one is what lets the
    /// query paths answer with the empty set and park the subscription
    /// in the reactor's waiting room, instead of returning `404` — which
    /// closed the stream and left a page with nothing to hear the answer
    /// on when the repo finally arrived.
    #[dialog_common::test]
    fn it_reads_a_missing_repository_or_branch_as_absence() {
        assert!(is_absence(&ReactorError::RepositoryNotFound {
            repo: "did:key:zAbsent".into(),
            reason: "not replicated here".into(),
        }));
        assert!(is_absence(&ReactorError::BranchNotFound {
            repo: "did:key:zPresent".into(),
            branch: "main".into(),
            reason: "branch not created yet".into(),
        }));
    }

    /// A genuine fault is still a fault: answering it with the empty set
    /// would report "nothing matched" for a broken query or a failed
    /// commit, hiding the error behind a plausible-looking result. The
    /// match is exhaustive, so a new variant has to be classified here
    /// rather than silently defaulting to one side.
    #[dialog_common::test]
    fn it_classifies_every_variant_deliberately() {
        fn absent(error: &ReactorError) -> bool {
            match error {
                ReactorError::Shutdown => false,
                ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. } => {
                    true
                }
                ReactorError::Graph(_)
                | ReactorError::QueryFailed(_)
                | ReactorError::Commit(_)
                | ReactorError::Pull(_)
                | ReactorError::Download(_)
                | ReactorError::Push(_)
                | ReactorError::Upgrade { .. } => false,
            }
        }
        // `is_absence` must agree with that intent for the cases we can
        // construct without fabricating upstream error types.
        let missing_repo = ReactorError::RepositoryNotFound {
            repo: "did:key:zAbsent".into(),
            reason: "not replicated here".into(),
        };
        assert_eq!(is_absence(&missing_repo), absent(&missing_repo));
    }

    /// A rule an earlier dialog release stored under an entity that is not
    /// its identity is inert: no read finds it and no commit fires it. The
    /// first read of a branch laid out the way that release wrote moves
    /// the storage to the band and the rule under its identity; a second
    /// read finds the branch migrated and reads nothing.
    ///
    /// Native only: it needs a whole worker state, which the native test
    /// fixture builds. The upgrade itself is target-agnostic dialog code,
    /// covered on wasm by dialog-repository's `transaction::migration` tests.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[dialog_common::test]
    async fn the_first_read_of_a_branch_re_installs_rules_stored_the_earlier_way() {
        use dialog_artifacts::{ArtifactSelector, Entity, Pick, Value};
        use dialog_query::rule::DeductiveRuleDescriptor;
        use dialog_query::rule::statement::source_attr;
        use dialog_query::{AttributeStatement, Cardinality};
        use futures_util::TryStreamExt as _;

        let state = crate::router::command::tests::native::test_state().await;
        let tonk = state.read().await;
        let branch = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch);
        let descriptor: DeductiveRuleDescriptor = serde_json::from_value(serde_json::json!({
            "deduce": { "with": { "label": { "the": "test.upgrade/label", "as": "Text" } } },
            "when": [{
                "assert": { "with": { "name": { "the": "test.upgrade/name", "as": "Text" } } },
                "where": { "this": { "?": { "name": "this" } }, "name": { "?": { "name": "label" } } }
            }]
        }))
        .expect("a rule descriptor");
        let rule = descriptor.compile().expect("the rule compiles");
        let legacy: Entity = "rule:stored-the-earlier-way".parse().expect("an entity");
        branch
            .transaction()
            .assert(AttributeStatement {
                the: source_attr().into(),
                of: legacy.clone(),
                is: Value::Bytes(rule.encode()),
                cause: None,
                cardinality: Some(Cardinality::Many),
                pick: Some(Pick::All),
            })
            .commit()
            .perform(&tonk.operator)
            .await
            .expect("the earlier release's rule is stored");
        // Laid out the way the earlier release wrote, which is what gates
        // the migration.
        let handle = branch
            .acquire(&tonk.operator)
            .await
            .expect("the branch opens")
            .handle()
            .clone();
        handle
            .rekey_storage(0)
            .perform(&tonk.operator)
            .await
            .expect("the branch is laid out the earlier way");
        assert!(
            branch
                .migration_pending(&tonk.operator)
                .await
                .expect("the layout is read"),
            "a branch the earlier release laid out needs migrating"
        );

        let _ = migrate_while_opening(&tonk, branch).await;
        assert!(
            !branch
                .migration_pending(&tonk.operator)
                .await
                .expect("the layout is read")
        );

        let session = branch
            .acquire(&tonk.operator)
            .await
            .expect("the branch opens");
        let stored_under = |entity: Entity| {
            let handle = session.handle().clone();
            let operator = &tonk.operator;
            async move {
                handle
                    .claims()
                    .select(ArtifactSelector::new().the(source_attr()).of(entity))
                    .perform(operator)
                    .await
                    .expect("the source selects")
                    .try_collect::<Vec<_>>()
                    .await
                    .expect("the source reads")
                    .len()
            }
        };
        assert_eq!(
            stored_under(rule.this()).await,
            1,
            "the rule is under its identity"
        );
        assert_eq!(
            stored_under(legacy).await,
            0,
            "and no longer under the earlier entity"
        );
        assert!(
            matches!(branch.migrate_once(&tonk.operator).await, Ok(None)),
            "a second read does not upgrade again"
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod graph_tests {
    use super::*;
    use tonk_evaluator::evaluate::SyntaxEvaluateExt;

    #[dialog_common::test]
    async fn graph_http_envelope_narrows_json_and_sse_and_emits_membership_changes()
    -> anyhow::Result<()> {
        let state = crate::router::command::tests::native::test_state().await;
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await?;
        let source = r#"concept!: &person
  with:
    name: { the: example.person/name, as: text, description: "Name" }
concept!: &issue
  with:
    title: { the: example.issue/title, as: text, description: "Title" }
    owner: { the: example.issue/owner, as: person, description: "Owner" }
person!:
  this: id:alice
  name: "Alice"
person!:
  this: id:bob
  name: "Bob"
issue!:
  this: id:first
  title: "First"
  owner: id:alice
issue!:
  this: id:second
  title: "Second"
  owner: id:bob
"#;
        tonk_notation::parse(source)
            .syntax
            .unwrap()
            .evaluate(session.handle().transaction())
            .perform(&tonk.operator)
            .await?
            .commit()
            .publish()
            .perform(&tonk.operator)
            .await?;
        let definition = tonk_schema::resolution::ConceptReference::from(
            tonk_schema::resolution::NamedReference("issue".into()),
        )
        .resolve(session.handle())
        .perform(&tonk.operator)
        .await?
        .unwrap();
        let query = dialog_query::ConceptQuery {
            predicate: definition.descriptor.concept().clone(),
            terms: {
                let mut terms = dialog_query::Parameters::new();
                terms.insert("this".into(), dialog_query::Term::var("this"));
                terms
            },
        };
        let body = serde_json::to_vec(
            &serde_json::json!({ "query": Query::from(&query), "constraints": { "matching": { "owner": { "name": "Alice" } }, "shaping": { "owner": { "name": "Alice" } } } }),
        )?;
        // The envelope is unambiguous, enabled only by the graph URL parameter.
        assert!(decode_query(&body, false).is_err());
        assert!(decode_query(br#"{"query":{},"constraints":{"unknown":{}}}"#, true).is_err());
        let target = QueryTarget::Profile {
            branch: tonk.active_branch.clone(),
        };
        let request = || {
            Request::builder()
                .uri("/query?graph=issue")
                .body(Body::from(body.clone()))
                .unwrap()
        };
        let json = query_on_branch(
            &state,
            &tonk,
            target.clone(),
            HeaderMap::new(),
            request(),
            None,
        )
        .await?;
        assert_eq!(json.status(), StatusCode::OK);
        let graph: serde_json::Value =
            serde_json::from_slice(&json.into_body().collect().await?.to_bytes())?;
        assert_eq!(graph["roots"], serde_json::json!(["id:first"]));
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, "text/event-stream".parse()?);
        let response = query_on_branch(&state, &tonk, target, headers, request(), None).await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        let mut body = response.into_body();
        async fn frame(body: &mut Body) -> anyhow::Result<serde_json::Value> {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
                .await?
                .unwrap()?
                .into_data()
                .unwrap();
            let text = std::str::from_utf8(&frame)?;
            Ok(serde_json::from_str(
                text.strip_prefix("data: ").unwrap().trim(),
            )?)
        }
        let initial = frame(&mut body).await?;
        assert_eq!(initial["kind"], "graph_snapshot");
        assert_eq!(initial["graph"], graph);
        tonk_notation::parse("person!:\n  this: id:bob\n  name: \"Alice\"\n")
            .syntax
            .unwrap()
            .evaluate(session.handle().transaction())
            .perform(&tonk.operator)
            .await?
            .commit()
            .publish()
            .perform(&tonk.operator)
            .await?;
        session.poll(&tonk.operator).await;
        let delta = frame(&mut body).await?;
        assert_eq!(delta["kind"], "graph_delta");
        assert_eq!(
            delta["delta"]["roots_added"],
            serde_json::json!(["id:second"])
        );
        Ok(())
    }
}
