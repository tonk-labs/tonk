//! `POST /api/site` — register/update the requesting client's site.
//!
//! The site is per-tab navigation state, NOT per-query. On first load the
//! navigation predates the service worker (the page is served before the SW
//! exists), so the SW never sees a navigation `FetchEvent` for it — the page
//! must announce itself. Once controlled, the page calls `POST /api/site` with
//! its current path; the SW reads the requesting **client id**, derives the site
//! entity (`site:<client-id>`), asserts a [`Site`] `{path, anchor, replica,
//! route, concept}` on the Level-0-resolved branch's overlay, and returns the
//! site id. The page renders `<tonk-display entity={site} model=tonk:site>`; the
//! `tonk:site` view nests into the matched `{concept}` and renders.
//!
//! The same endpoint handles navigation updates: the page re-calls it on each
//! client-side navigation, and the cardinality-one fields update in place. Read
//! queries never stamp, so a tab's displays re-querying never re-derive or
//! re-poll — the perf cost of stamping is paid once per navigation, not per read.

use ::axum::Json;
use ::axum::extract::{Request, State};
use ::axum::http::HeaderMap;
use axum_wasm_macros::wasm_compat;
use serde::Serialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::{AppState, ClientId};
use crate::TonkWorkerError;

/// `POST /api/site` response: the site entity the client should render against.
#[derive(Debug, Serialize)]
pub struct SiteResponse {
    /// The site entity URI (`site:<client-id>`).
    pub site: String,
}

/// What one SW client has registered with the worker, plus whether we
/// have ever *observed it alive* in `clients.matchAll()`.
///
/// The liveness latch is the load-bearing part. Absence from
/// `matchAll()` proves a client is dead ONLY for a client we have
/// previously seen alive; for a brand-new one it equally means
/// not-born-yet. A navigation's client id is the `FetchEvent`'s
/// `resultingClientId` — the id the *future* document will get — so a
/// booting page is legitimately absent from `matchAll()` (with or
/// without `includeUncontrolled`) for its entire boot, which is exactly
/// when it stamps its site and opens its subscriptions. Sweeping on bare
/// absence therefore deletes the live page's own session out from under
/// it: its `site:` facts vanish, its subscribers are dropped, and its
/// display waits forever on a subscription nobody will ever feed.
///
/// So the sweep only ever reaps **born-then-died**: `seen_live` latched
/// true, and the client has since disappeared.
#[derive(Debug, Default, Clone)]
pub struct ClientState {
    /// Site entities this client has stamped. Tracked per client (not as
    /// a `site → client` map) because the site URI is not a function of
    /// the client: the `/site` endpoints key it `site:<client-id>`, but
    /// the page-minted `tonk:load` command keys it `site:<uuid>`.
    pub sites: std::collections::HashSet<String>,
    /// Latched once this client appeared in `clients.matchAll()`. Until
    /// then the client is presumed to be booting, never dead.
    pub seen_live: bool,
    /// Active-profile generation under which this browser document first
    /// reached a profile-scoped route. Immutable for this Client ID.
    pub context_generation: Option<u64>,
}

/// Bind a browser client to the current profile generation, or reject it when
/// it was already bound before a profile transition.
pub(crate) async fn client_context_is_current(
    tonk: &crate::worker::TonkState,
    client: &ClientId,
) -> bool {
    use std::sync::atomic::Ordering;

    let generation = tonk.context_generation.load(Ordering::Acquire);
    let mut clients = tonk.clients.write().await;
    let client = clients.entry(client.clone()).or_default();
    match client.context_generation {
        Some(bound) => bound == generation,
        None => {
            client.context_generation = Some(generation);
            true
        }
    }
}

/// Shared ledger of SW client → what it registered. The stale-client
/// sweep reconciles this against `clients.matchAll()` and reaps the
/// clients that were born and have since died, dropping their site
/// overlay facts and SSE subscriptions — the GC the `site:<client-id>`
/// keying was designed for but never got.
pub type ClientRegistry =
    std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<super::ClientId, ClientState>>>;

/// Read a header as a `&str`, empty when absent or non-ASCII.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// Register the requesting client's site: assert its [`Site`] and return the
/// site id. The client id (browser-assigned, one per document) keys the site, so
/// it is GC-able (the SW can reconcile against live clients) and needs no minted
/// uuid. Idempotent — re-calling on navigation supersedes the cardinality-one
/// fields in place.
#[wasm_compat]
pub async fn register_site(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> Result<Json<SiteResponse>, TonkWorkerError> {
    let client_id = request
        .extensions()
        .get::<ClientId>()
        .map(|c| c.0.clone())
        .unwrap_or_default();
    if client_id.is_empty() {
        return Err(TonkWorkerError::Router("no client id on /api/site".into()));
    }
    let site = format!("site:{client_id}");

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let path = header(&headers, "x-tonk-path").to_owned();
        let anchor = header(&headers, "x-tonk-hash").to_owned();
        let tonk = state.read().await;
        stamp_site(&tonk, &site, ClientId(client_id), &path, anchor).await;
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = (&state, &headers);
    }

    Ok(Json(SiteResponse { site }))
}

/// Body of a per-branch `POST .../site`: the path to record and match against
/// the branch's route table, plus an optional anchor (URL hash).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(Debug, serde::Deserialize, Default)]
pub struct SiteRequest {
    /// The path to record on the site and match against the branch's `route!`
    /// table. For a per-branch endpoint this is the path the caller wants
    /// routed within that branch (the branch is named in the URL, not parsed
    /// from this path).
    #[serde(default)]
    pub path: String,
    /// The active anchor (URL hash), if any.
    #[serde(default)]
    pub anchor: String,
}

/// `POST /api/repository/{repo}/branch/{branch}/site` — register the requesting
/// client's site on an explicit `(repo, branch)`, matching the body `path`
/// against that branch's route table. Unlike [`register_site`], the branch comes
/// from the request URL (like `/query` and `/transact`), not from parsing the
/// document path — so the SW does no document-path routing here.
#[wasm_compat]
pub async fn register_site_on_repo(
    State(state): State<AppState>,
    ::axum::extract::Path(path): ::axum::extract::Path<crate::router::transact::TransactPath>,
    request: Request,
) -> Result<Json<SiteResponse>, TonkWorkerError> {
    let (site, client) = client_site(&request)?;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let body = read_site_request(request).await?;
        let tonk = state.read().await;
        stamp_site_on(
            &tonk,
            &site,
            client,
            &path.repo,
            &path.branch,
            false,
            &body.path,
            &body.path,
            body.anchor,
        )
        .await;
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = (&state, &path, &request, &client);
    }
    Ok(Json(SiteResponse { site }))
}

/// `POST /api/profile/branch/{branch}/site` — the profile counterpart of
/// [`register_site_on_repo`]. The profile is a singleton repository, so the URL
/// carries only the branch.
#[wasm_compat]
pub async fn register_site_on_profile(
    State(state): State<AppState>,
    ::axum::extract::Path(path): ::axum::extract::Path<
        crate::router::transact::ProfileTransactPath,
    >,
    request: Request,
) -> Result<Json<SiteResponse>, TonkWorkerError> {
    let (site, client) = client_site(&request)?;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let body = read_site_request(request).await?;
        let tonk = state.read().await;
        let repo = tonk.profile_name.clone();
        stamp_site_on(
            &tonk,
            &site,
            client,
            &repo,
            &path.branch,
            true,
            &body.path,
            &body.path,
            body.anchor,
        )
        .await;
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = (&state, &path, &request, &client);
    }
    Ok(Json(SiteResponse { site }))
}

/// Derive the `site:<client-id>` entity for a request, erroring if the SW set no
/// client id (the per-tab key the site is stamped under).
fn client_site(request: &Request) -> Result<(String, ClientId), TonkWorkerError> {
    let client_id = request
        .extensions()
        .get::<ClientId>()
        .map(|c| c.0.clone())
        .unwrap_or_default();
    if client_id.is_empty() {
        return Err(TonkWorkerError::Router("no client id on /site".into()));
    }
    Ok((format!("site:{client_id}"), ClientId(client_id)))
}

/// Read and decode the [`SiteRequest`] body, defaulting to an empty path when
/// the body is absent or empty.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn read_site_request(request: Request) -> Result<SiteRequest, TonkWorkerError> {
    use ::axum::body::to_bytes;
    let bytes = to_bytes(request.into_body(), usize::MAX)
        .await
        .map_err(|e| TonkWorkerError::Router(format!("failed to read /site body: {e}")))?;
    if bytes.is_empty() {
        return Ok(SiteRequest::default());
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| TonkWorkerError::Router(format!("invalid /site body: {e}")))
}

/// Assert the [`Site`] for `site` (a `site:<client-id>` URI) from `path` on the
/// Level-0-resolved branch's overlay: derive the replica, match the remaining
/// path against the route table, and write `{path, anchor, replica, route,
/// concept}` through the overlay builder (which schedules a poll so subscribers
/// see the change — no inline whole-branch re-poll). Best-effort: a non-space
/// path, an unacquirable branch, an absent replica, or no matched route all skip
/// stamping rather than fail registration.
/// Resolve `path` to its Level-0 branch, then stamp the site there. This is the
/// document-path-driven entry: `resolve_path` decides which repository/branch
/// the path addresses, and [`stamp_site_on`] does the branch-generic work.
///
/// Only spaces route here; the profile (`/`, `/join`) is handled by the
/// per-branch `/site` endpoint, which calls [`stamp_site_on`] directly with the
/// branch named in the request URL.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn stamp_site(
    tonk: &crate::worker::TonkState,
    site: &str,
    client: ClientId,
    path: &str,
    anchor: String,
) {
    use tonk_schema::{RouteTarget, resolve_path};

    let Some(RouteTarget::Space { space, rest }) = resolve_path(path) else {
        return;
    };
    stamp_site_on(
        tonk,
        site,
        client,
        &space.name,
        &space.branch,
        false,
        path,
        &rest,
        anchor,
    )
    .await;
}

/// Stamp a site on an explicit `(repo, branch)` — the branch-generic core,
/// independent of any document-path parsing. The caller supplies the branch
/// coordinates (from `resolve_path` for the legacy document path, or from the
/// request URL for the per-branch `/site` endpoint), the full `path` to record,
/// and the `rest` to match against that branch's `route!` table.
///
/// Acquires the branch, derives the replica (the branch's origin), matches the
/// route, and writes `{path, anchor, repo, branch, replica, route, concept}`
/// plus the captured route params into the session overlay. Best-effort: an
/// unacquirable branch, an absent replica, or no matched route skip stamping.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(clippy::too_many_arguments)]
async fn stamp_site_on(
    tonk: &crate::worker::TonkState,
    site: &str,
    client: ClientId,
    repo: &str,
    branch_name: &str,
    profile: bool,
    path: &str,
    rest: &str,
    anchor: String,
) {
    use tonk_schema::Site;

    let Ok(entity): Result<dialog_artifacts::Entity, _> = site.parse() else {
        return;
    };

    // The profile lives outside the named-repo namespace, so it is acquired
    // through `profile_repository()`, not `repository(name)`. The `repo` string
    // is still recorded on the site (the `space` field), but it does not select
    // the branch in profile mode.
    let branch = if profile {
        tonk.reactor.profile_repository().branch(branch_name)
    } else {
        tonk.reactor.repository(repo).branch(branch_name)
    };
    let state = match branch.acquire(&tonk.operator).await {
        Ok(session) => session,
        Err(e) => {
            tonk_common::log!("register_site: failed to acquire branch for {path}: {e}");
            return;
        }
    };

    let Some(replica) = origin_entity(tonk, &state).await else {
        tonk_common::log!(
            "[stamp] {site} SKIPPED: no origin_entity (repo={repo} branch={branch_name} rest={rest})"
        );
        return;
    };
    let Some(matched) = match_route(tonk, &state, rest).await else {
        tonk_common::log!("[stamp] {site} SKIPPED: no route match for rest={rest:?}");
        return;
    };

    // A re-stamp REPLACES this site's overlay facts, not merges into them:
    // params the new route does not capture must not survive the previous
    // navigation as stale cardinality-one values. The bare space route
    // (`/space/{id}`) captures no `{rest}`, so after visiting
    // `/space/{id}/inspector` a merge would leave `rest="inspector"` on the
    // site and the nested `<tonk-site path={rest}>` would keep routing the
    // old sub-path. Pruned only once the route matched (above), so a
    // no-match navigation still keeps the previous stamp; the write below
    // schedules the poll that lets subscribers observe the swap atomically.
    state
        .state
        .retain_overlay_entities(|overlaid| overlaid.as_str() != site);

    // Write through the overlay builder: it asserts into the session overlay and
    // schedules a poll so subscribers are notified — the request dispatcher
    // drains the poll once. Cardinality-one fields supersede in place, so a
    // navigation re-call just updates this site's path/route/concept.
    //
    // The fixed `Site` stamp carries path/anchor/space/branch/replica/route/
    // concept; the route's captured params (`{model}`, `{entity}`, `{view}`, …)
    // are stamped alongside as `xyz.tonk.site/{name}` facts so each route model
    // picks the ones it declares — the same per-field pickup `tonk:space/route`
    // uses for `replica`. Params are variable per route, so they ride raw claims
    // rather than the fixed `Site` struct.
    let stamp = Site::new(
        entity.clone(),
        path.to_owned(),
        anchor,
        repo.to_owned(),
        branch_name.to_owned(),
        replica,
        matched.route,
        matched.concept,
    );
    let mut overlay = branch.overlay().assert(stamp);
    for (name, value) in matched.params.iter() {
        // Decode captured params so both URL spellings of a value stamp the same
        // fact — a raw `/space/did:key:z…` and its `encodeURIComponent`'d
        // `/space/did%3Akey%3Az…` are equivalent per URL semantics, but the route
        // matcher captures the segment verbatim. Without this, an encoded link
        // stamps a `:`-less string that fails entity-URI validation downstream.
        let value = percent_decode(value);
        match site_param_claim(&entity, name, &value) {
            Some(claim) => overlay = overlay.assert(claim),
            None => tonk_common::log!("register_site: bad site param attribute for {name}"),
        }
    }
    // Stamp the Level-0-resolved repository + branch on the site too, so a route
    // view can give its content a `<tonk-repository>`/`<tonk-branch>` context.
    // Repository-context elements (`<tonk-tree>`, `<tonk-inspector>`) resolve
    // repo/branch by walking DOM ancestors, which the sealed guest otherwise
    // lacks. These are `as: text` site fields, hence string-typed raw claims.
    for (name, value) in [("repo", repo), ("branch", branch_name)] {
        match site_param_claim(&entity, name, value) {
            Some(claim) => overlay = overlay.assert(claim),
            None => tonk_common::log!("register_site: bad site {name} attribute"),
        }
    }
    if let Err(e) = overlay.write().perform(&tonk.operator).await {
        tonk_common::log!("register_site: overlay write failed for {path}: {e}");
    } else {
        // Record the site against its client so the stale-client sweep can
        // drop these overlay facts once the client is provably gone. The
        // entry is created NOT-seen-live: this stamp lands while the page is
        // still booting (its client is not yet in `clients.matchAll()`), and
        // registering it as live-then-absent would let the very next sweep
        // reap the page that just announced itself.
        if !client.0.is_empty() {
            tonk.clients
                .write()
                .await
                .entry(client)
                .or_default()
                .sites
                .insert(site.to_owned());
        }
        tonk_common::log!("[stamp] {site} WROTE path={path}");
    }
}

/// Build a raw claim stamping a captured route param as a `xyz.tonk.site/{name}`
/// fact on the site entity, in the value type the route model's field expects so
/// the field's typed query resolves it. Returns `None` only if the attribute name
/// is malformed.
///
/// The value type is keyed by param name to match the route models in the
/// standard library: `entity` is an `as: entity` field (stored [`Value::Entity`]);
/// `model` and `view` are `as: text` fields (stored [`Value::String`]). This is
/// the interim before descriptor-driven typing — once `match_route` resolves each
/// route model's field descriptors (and threads the `as:` types through
/// `tonk_router::Route::with_types`), the value type comes from the field itself
/// and this name table goes away. An unknown param name defaults to string.
/// Percent-decode a captured route param using the browser's own
/// `decodeURIComponent`, so URL-encoded links round-trip to the same value the
/// raw form would (`did%3Akey%3Az…` → `did:key:z…`). On a malformed escape the
/// raw value is returned unchanged.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn percent_decode(value: &str) -> String {
    js_sys::decode_uri_component(value)
        .ok()
        .and_then(|decoded| decoded.as_string())
        .unwrap_or_else(|| value.to_owned())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn site_param_claim(
    site: &dialog_artifacts::Entity,
    name: &str,
    value: &str,
) -> Option<crate::router::claim::RawClaim> {
    use dialog_artifacts::{Entity, Value};

    let attribute = format!("xyz.tonk.site/{name}").parse().ok()?;
    let is = match name {
        // Entity-typed route-model fields.
        "entity" => Value::Entity(value.parse::<Entity>().ok()?),
        // Text-typed route-model fields (model name, view name) and anything else.
        _ => Value::String(value.to_owned()),
    };
    // Cardinality-one: a navigation must SUPERSEDE the prior value, not pile up
    // a new fact per visited route (else a stale `model`/`entity`/`view` lingers).
    Some(crate::router::claim::RawClaim {
        the: attribute,
        of: site.clone(),
        is,
        unique: true,
    })
}

/// Post-commit handler for the [`Load`](tonk_schema::command::Load) command —
/// the transact-driven replacement for the `POST /api/.../site` endpoint.
///
/// A `<tonk-site>` asserts a transient `tonk:load { this: site:<uuid>, path }`
/// through the regular transact API; its ancestor `<tonk-repository>` /
/// `<tonk-branch>` annotate the origin repo/branch, so the commit lands on the
/// branch the tab routes against. This handler reads `this`/`path` from the
/// command and `repo`/`branch` from [`CommandEnv::origin`](crate::router::CommandEnv::origin),
/// then runs [`stamp_site_on`] — matching `path` against that branch's `route!`
/// table and stamping the `tonk:site` (+ captured params) onto `this` in the
/// branch overlay. A profile-branch commit carries an empty `origin.repo` (see
/// `transact_profile`), which is exactly the `profile` flag `stamp_site_on` wants.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct LoadHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl LoadHandler {
    /// Cache `Load`'s trigger attributes (its `path` field) so the registry
    /// indexes this handler under them.
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::Load::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for LoadHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::Load::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;

        // Decode synchronously (the caller still holds the lock); carry owned
        // values into the `'static` future. `this` is the site entity to stamp,
        // `path` the route-relative path to match + record.
        let decoded = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::Load::decode(entity, facts))
            .map(|command| (command.this.to_string(), command.path.0));
        let env = env.clone();

        Box::pin(async move {
            let Some((site, path)) = decoded else {
                return;
            };
            // An empty `origin.repo` means the commit landed on the profile
            // branch (the profile is outside the named-repo namespace).
            let repo = env.origin().repo.clone();
            let branch = env.origin().branch.clone();
            let profile = repo.is_empty();
            // The site entity here is page-minted (`site:<uuid>`), so the
            // commit's origin is what names the client the stamp serves.
            // An absent client leaves the site unregistered — its facts then
            // outlive the client (the pre-sweep behaviour) rather than being
            // attributed to the wrong one.
            let client = env
                .origin()
                .client
                .clone()
                .unwrap_or(crate::router::ClientId(String::new()));
            dialog_common::log!(
                "command Load site={} path={} repo={} branch={} profile={}",
                site,
                path,
                repo,
                branch,
                profile
            );

            let tonk = env.state().read().await;
            // The command's `path` is already the route-relative path the tab
            // routes (a nested `<tonk-site path={rest}>`), so it is both the
            // recorded `path` and the `rest` matched against the route table.
            stamp_site_on(
                &tonk,
                &site,
                client,
                &repo,
                &branch,
                profile,
                &path,
                &path,
                String::new(),
            )
            .await;
        })
    }
}

/// The existing dialog [`Replica`](dialog_repository::schema::Replica) entity for
/// this device's `(profile, subject)` on the branch — the entity `tonk/replica`
/// and `tonk:binder` live on. Queried (not derived) so it stays correct even if
/// tonk's and dialog's hashing drift. `None` if no replica is on the branch yet.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn origin_entity(
    tonk: &crate::worker::TonkState,
    state: &dialog_reactor::BranchSession,
) -> Option<dialog_artifacts::Entity> {
    use dialog_query::{Output as _, Query, Term};
    use dialog_repository::schema::replica::{Profile, Subject};
    use dialog_repository::schema::{DidExt as _, Replica};

    let subject = state.handle().of().this();
    let profile = tonk.profile.did().this();

    let replicas: Vec<Replica> = state
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::from(Subject(subject)),
            profile: Term::from(Profile(profile)),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    replicas.into_iter().next().map(|replica| replica.this)
}

/// A matched route: the route-table entry, the model the shell mounts, and the
/// params captured from the path (`{model}`, `{entity}`, `{view}`, …).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
struct MatchedRoute {
    /// The route-table entry's entity.
    route: dialog_artifacts::Entity,
    /// The route model to mount.
    concept: dialog_artifacts::Entity,
    /// The captured path params, by name.
    params: tonk_router::Params,
}

/// Match `rest` (the Level 1 remaining path) against the branch's durable
/// `tonk:route` table.
///
/// Builds a fresh [`tonk_router::Router`] per call from the queried routes:
/// each route's `path` pattern compiles via [`Route::parse_pattern`], paired with
/// its `(route entity, model)`. [`recognize`](tonk_router::Router::recognize)
/// matches most-specific-first (static > param > catch-all) and returns the
/// captured params. Routes are inserted in stable entity-URI order so equal-
/// specificity ties resolve deterministically. Returns `None` when nothing
/// matches or a pattern fails to compile.
///
/// [`recognize`]: tonk_router::Router::recognize
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn match_route(
    tonk: &crate::worker::TonkState,
    state: &dialog_reactor::BranchSession,
    rest: &str,
) -> Option<MatchedRoute> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_router::Route as RoutePattern;
    use tonk_schema::Route;

    let mut routes: Vec<Route> = state
        .handle()
        .query()
        .select(Query::<Route> {
            this: Term::var("this"),
            path: Term::var("path"),
            concept: Term::var("concept"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    // Stable order by entity URI so equal-specificity ties resolve
    // deterministically (the table preserves insertion order among equal scores).
    routes.sort_by(|a, b| a.this.to_string().cmp(&b.this.to_string()));

    let mut router = tonk_router::Router::new();
    for route in &routes {
        match RoutePattern::parse_pattern(&route.path.0) {
            Ok(pattern) => {
                router.insert(pattern, (route.this.clone(), route.concept.0.clone()));
            }
            Err(e) => {
                tonk_common::log!(
                    "match_route: skipping invalid route {}: {e:?}",
                    route.path.0
                );
            }
        }
    }

    match router.recognize(rest) {
        Ok(matched) => {
            let (route, concept) = matched.value.clone();
            Some(MatchedRoute {
                route,
                concept,
                params: matched.params,
            })
        }
        Err(_) => None,
    }
}
