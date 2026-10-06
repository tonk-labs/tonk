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
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::{AppState, ClientId};
use crate::TonkWorkerError;

/// The repo-token prefix naming the profile-as-repository endpoint, mirroring
/// `tonk_host::location`'s `PROFILE_PREFIX`. A site's stamped `repo` is a
/// location token a view interpolates into `with="{branch}@{repo}"`, so the
/// profile's must carry this prefix or the location parses as a named
/// repository. Duplicated rather than imported: `tonk-host` is guest-side and
/// the worker does not depend on it.
const PROFILE_LOCATION_PREFIX: &str = "profile:";

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
    /// The stamps this client holds, by site entity. Tracked per client
    /// (not as a `site → client` map) because the site URI is not a
    /// function of the client: the `/site` endpoints key it
    /// `site:<client-id>`, but the page-minted `tonk:load` command keys it
    /// `site:<uuid>`.
    pub sites: std::collections::HashMap<String, Stamp>,
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

/// What a site stamp was made from. The stamp itself (route, concept,
/// captured params) is derived from these against the branch, so keeping
/// the inputs is enough to make it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stamp {
    /// The site entity the stamp lands on.
    pub site: String,
    /// The repository recorded on the site.
    pub repo: String,
    /// The branch the site routes against.
    pub branch: String,
    /// Whether `branch` is the profile's rather than `repo`'s.
    pub profile: bool,
    /// The path recorded on the site.
    pub path: String,
    /// The part of `path` matched against the branch's route table.
    pub rest: String,
    /// The active anchor (URL hash).
    pub anchor: String,
}

/// The version of [`Saved`]. A worker that finds another version starts
/// without the saved stamps, and pages claim their sites again.
const SAVED_VERSION: u32 = 1;

/// The stamps a worker holds, in the form it saves them in so that the next
/// instance (after the browser stops this one, or after an update) can make
/// them again instead of waiting for every page to claim its site anew.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    version: u32,
    /// Stamps by the client they serve.
    clients: std::collections::BTreeMap<String, Vec<Stamp>>,
}

impl Saved {
    /// The stamps held by `clients`, sorted so that equal state saves the
    /// same bytes.
    pub async fn of(clients: &ClientRegistry) -> Self {
        let clients = clients.read().await;
        let clients = clients
            .iter()
            .filter(|(_, state)| !state.sites.is_empty())
            .map(|(client, state)| {
                let mut stamps: Vec<Stamp> = state.sites.values().cloned().collect();
                stamps.sort_by(|a, b| a.site.cmp(&b.site));
                (client.0.clone(), stamps)
            })
            .collect();
        Self {
            version: SAVED_VERSION,
            clients,
        }
    }

    /// Read saved stamps, keeping only those of the clients `live` accepts:
    /// a client that closed while no worker ran has nothing to restore.
    /// `None` for a version this worker does not read.
    pub fn read(bytes: &[u8], live: impl Fn(&str) -> bool) -> Option<Self> {
        let mut saved: Self = serde_json::from_slice(bytes).ok()?;
        if saved.version != SAVED_VERSION {
            return None;
        }
        saved.clients.retain(|client, _| live(client));
        Some(saved)
    }

    /// Make every saved stamp again. The clients were alive when the stamps
    /// were saved and still are, so they are recorded as seen alive: once
    /// one closes, the sweep reaps its stamps.
    pub async fn restore(self, tonk: &crate::worker::TonkState) {
        for (client, stamps) in self.clients {
            let client = ClientId(client);
            for stamp in stamps {
                stamp_site_on(tonk, client.clone(), stamp).await;
            }
            if let Some(state) = tonk.clients.write().await.get_mut(&client) {
                state.seen_live = true;
            }
        }
    }
}

/// Read a header as a `&str`, empty when absent or non-ASCII.
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

    let path = header(&headers, "x-tonk-path").to_owned();
    let anchor = header(&headers, "x-tonk-hash").to_owned();
    let tonk = state.read().await;
    stamp_site(&tonk, site.clone(), ClientId(client_id), path, anchor).await;

    Ok(Json(SiteResponse { site }))
}

/// Body of a per-branch `POST .../site`: the path to record and match against
/// the branch's route table, plus an optional anchor (URL hash).
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
    if super::names_profile(&state, &path.repo).await {
        let path = crate::router::transact::ProfileTransactPath {
            branch: path.branch,
        };
        return register_site_on_profile(State(state), ::axum::extract::Path(path), request).await;
    }
    let (site, client) = client_site(&request)?;
    let body = read_site_request(request).await?;
    let tonk = state.read().await;
    // The stamp acquires the branch; a directory-listed space this
    // device has not pulled yet is mounted first, as on every other
    // route that addresses a space by key.
    match super::adopt::ensure_space_mounted(&tonk, &path.repo).await {
        Ok(true) => {
            super::adopt::schedule_seed_upgrade(&tonk, state.clone(), &path.repo).await;
        }
        Ok(false) => {}
        Err(error) => {
            tonk_common::log!("on-demand mount of '{}' failed: {error}", path.repo);
        }
    }
    stamp_site_on(
        &tonk,
        client,
        Stamp {
            site: site.clone(),
            repo: path.repo,
            branch: path.branch,
            profile: false,
            rest: body.path.clone(),
            path: body.path,
            anchor: body.anchor,
        },
    )
    .await;
    Ok(Json(SiteResponse { site }))
}

/// [`register_site_on_repo`] for the profile's own repository, which that
/// route hands a request naming it: the site is stamped as the profile's,
/// under the profile's name.
#[wasm_compat]
async fn register_site_on_profile(
    State(state): State<AppState>,
    ::axum::extract::Path(path): ::axum::extract::Path<
        crate::router::transact::ProfileTransactPath,
    >,
    request: Request,
) -> Result<Json<SiteResponse>, TonkWorkerError> {
    let (site, client) = client_site(&request)?;
    let body = read_site_request(request).await?;
    let tonk = state.read().await;
    stamp_site_on(
        &tonk,
        client,
        Stamp {
            site: site.clone(),
            repo: tonk.profile_name.clone(),
            branch: path.branch,
            profile: true,
            rest: body.path.clone(),
            path: body.path,
            anchor: body.anchor,
        },
    )
    .await;
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
async fn stamp_site(
    tonk: &crate::worker::TonkState,
    site: String,
    client: ClientId,
    path: String,
    anchor: String,
) {
    use tonk_schema::{RouteTarget, resolve_path};

    let Some(RouteTarget::Space { space, rest }) = resolve_path(&path) else {
        return;
    };
    stamp_site_on(
        tonk,
        client,
        Stamp {
            site,
            repo: space.name,
            branch: space.branch,
            profile: false,
            path,
            rest,
            anchor,
        },
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
async fn stamp_site_on(tonk: &crate::worker::TonkState, client: ClientId, stamp: Stamp) {
    use tonk_schema::Site;

    let site = stamp.site.as_str();
    let repo = stamp.repo.as_str();
    let branch_name = stamp.branch.as_str();
    let profile = stamp.profile;
    let path = stamp.path.as_str();
    let rest = stamp.rest.as_str();

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
    // A page is looking at this space: say who its session acts for, where
    // the space's roster is, so a view can tell which member that is. Here
    // because a stamp is made whenever a page loads a space and again when
    // a restarted worker restores it, and the overlay lasts no longer than
    // either.
    if !profile
        && let Err(error) = super::sync::publish_session_account(tonk, repo, branch_name).await
    {
        tonk_common::log!("register_site: {error}");
    }

    let Some(replica) = origin_entity(tonk, &state).await else {
        tonk_common::log!(
            "[stamp] {site} SKIPPED: no origin_entity (repo={repo} branch={branch_name} rest={rest})"
        );
        return;
    };
    let matched = match match_route(tonk, &state, rest).await {
        Some(Matched::View(matched)) => matched,
        // A path answered with content is not a page to show in place: the
        // worker answers a request for it, and there is no model to mount.
        Some(Matched::Http(_)) => {
            tonk_common::log!("[stamp] {site} SKIPPED: rest={rest:?} answers with content");
            return;
        }
        None => {
            tonk_common::log!("[stamp] {site} SKIPPED: no route match for rest={rest:?}");
            return;
        }
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
    let fact = Site::new(
        entity.clone(),
        path.to_owned(),
        stamp.anchor.clone(),
        repo.to_owned(),
        branch_name.to_owned(),
        replica,
        matched.route,
        matched.concept,
        tonk.active_branch.clone(),
    );
    let mut overlay = branch.overlay().assert(fact);
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
    //
    // `repo` is a LOCATION TOKEN, not a bare name: it is what a view
    // interpolates into `with="{branch}@{repo}"`, and `tonk_host::Location`
    // reads a bare token as a NAMED repository. The profile lives outside the
    // named-repo namespace, so stamping its name alone sent every query and
    // transact a profile-side view built to `/api/repository/<profile>/…` — a
    // repository that does not exist. Prefixing it here is what lets one view
    // work in both contexts without knowing which it is in.
    let repo_token = if profile {
        format!("{PROFILE_LOCATION_PREFIX}{repo}")
    } else {
        repo.to_owned()
    };
    for (name, value) in [("repo", repo_token.as_str()), ("branch", branch_name)] {
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
                .insert(site.to_owned(), stamp.clone());
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

/// Percent-decode a captured route param — the native sibling of the
/// `decodeURIComponent` arm above, with the same contract: any malformed
/// escape (truncated, non-hex, or invalid UTF-8 once decoded) returns the
/// raw value unchanged, exactly as the browser arm does when
/// `decodeURIComponent` throws.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let decoded = bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            let Some(byte) = decoded else {
                return value.to_owned();
            };
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_owned())
}

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

/// Run the [`Load`](tonk_schema::command::Load) command — the
/// transact-driven replacement for the `POST /api/.../site` endpoint.
///
/// A `<tonk-site>` asserts a transient `tonk:load { this: site:<uuid>, path }`
/// through the regular transact API; its ancestor `<tonk-repository>` /
/// `<tonk-branch>` annotate the origin repo/branch, so the commit lands on the
/// branch the tab routes against. This provider reads `this`/`path` from the
/// command and `repo`/`branch` from [`CommandEnv::origin`](crate::router::CommandEnv::origin),
/// then runs [`stamp_site_on`] — matching `path` against that branch's `route!`
/// table and stamping the `tonk:site` (+ captured params) onto `this` in the
/// branch overlay. A profile-branch commit carries an empty `origin.repo` (see
/// `transact_profile`), which is exactly the `profile` flag `stamp_site_on` wants.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::Load> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::Load) {
        // `this` is the site entity to stamp, `path` the route-relative
        // path to match + record.
        let site = command.this.to_string();
        let path = command.path.0;
        // An empty `origin.repo` means the commit landed on the profile
        // branch (the profile is outside the named-repo namespace).
        let repo = self.origin().repo.clone();
        let branch = self.origin().branch.clone();
        let profile = repo.is_empty();
        // The site entity here is page-minted (`site:<uuid>`), so the
        // commit's origin is what names the client the stamp serves.
        // An absent client leaves the site unregistered — its facts then
        // outlive the client (the pre-sweep behaviour) rather than being
        // attributed to the wrong one.
        let client = self
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

        let tonk = self.state().read().await;
        // In profile mode the origin carries no repo at all, but the stamp
        // records a `profile:<name>` location token — and a nameless
        // `profile:` is not a location. Fill the name from the worker's own
        // profile, which is what the per-branch endpoint already stamps.
        let repo = if profile {
            tonk.profile_name.clone()
        } else {
            repo
        };
        // The command's `path` is already the route-relative path the tab
        // routes (a nested `<tonk-site path={rest}>`), so it is both the
        // recorded `path` and the `rest` matched against the route table.
        stamp_site_on(
            &tonk,
            client,
            Stamp {
                site,
                repo,
                branch,
                profile,
                rest: path.clone(),
                path,
                anchor: String::new(),
            },
        )
        .await;
    }
}

/// The existing dialog [`Replica`](dialog_repository::schema::Replica) entity for
/// this device's `(profile, subject)` on the branch — the entity `tonk/replica`
/// and `tonk:binder` live on. Queried (not derived) so it stays correct even if
/// tonk's and dialog's hashing drift. `None` if no replica is on the branch yet.
async fn origin_entity(
    tonk: &crate::worker::TonkState,
    state: &dialog_reactor::BranchSession,
) -> Option<dialog_artifacts::Entity> {
    use dialog_query::{Output as _, Query, Term};
    use dialog_repository::schema::replica::{Peer, Subject};
    use dialog_repository::schema::{DidExt as _, Replica};

    let subject = state.handle().of().this();
    let profile = tonk.profile.did().this();

    let replicas: Vec<Replica> = state
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::from(Subject(subject)),
            peer: Term::from(Peer(profile)),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    replicas.into_iter().next().map(|replica| replica.this)
}

/// A matched route: the route-table entry, the model the shell mounts, and the
/// params captured from the path (`{model}`, `{entity}`, `{view}`, …).
pub(super) struct MatchedRoute {
    /// The route-table entry's entity.
    route: dialog_artifacts::Entity,
    /// The route model to mount.
    concept: dialog_artifacts::Entity,
    /// The captured path params, by name.
    params: tonk_router::Params,
}

/// A matched `route/http`: the entry and the body it answers with.
pub(super) struct MatchedHttp {
    /// The route-table entry's entity, which its headers are read from.
    pub(super) route: dialog_artifacts::Entity,
    /// The response's body.
    pub(super) body: String,
}

/// What a path matched: a route a page shows, or one the worker answers.
pub(super) enum Matched {
    /// A `route`: the page mounts its model.
    View(MatchedRoute),
    /// A `route/http`: a request is answered with its content.
    Http(MatchedHttp),
}

/// Order routes for insertion into the router: the space's own routes first,
/// then the ones a library pinned, each group by entity URI.
///
/// The router preserves insertion order among routes of equal specificity, so
/// this ordering is what settles those ties. Libraries pinned their routes to
/// fixed entities before they shipped them as commands, and a space keeps
/// them until its upgrade withdraws them; a route the space wrote for the
/// same path wins meanwhile. A library's commands write routes only where no
/// route claims the path, so the routes they write tie with nothing until
/// the space writes its own, and then only until the next upgrade. The URI
/// tiebreak keeps the result deterministic within a group.
///
/// Split out of [`match_route`] so it is testable off-target — `match_route`
/// itself needs a branch session and so is wasm-only.
fn route_order(routes: &mut [tonk_schema::Route], pinned: &std::collections::HashSet<String>) {
    let library = |route: &tonk_schema::Route| pinned.contains(&route.this.to_string());
    routes.sort_by(|a, b| {
        library(a)
            .cmp(&library(b))
            .then_with(|| a.this.to_string().cmp(&b.this.to_string()))
    });
}

/// Match `rest` (the Level 1 remaining path) against the branch's durable
/// route tables: `tonk:route`, whose match a page shows, and
/// `tonk:route/http`, whose match the worker answers with.
///
/// Builds a fresh [`tonk_router::Router`] per call from the queried routes:
/// each route's `path` pattern compiles via [`Route::parse_pattern`], paired
/// with what it matches to. [`recognize`](tonk_router::Router::recognize)
/// matches most-specific-first (static > param > catch-all) across both
/// kinds and returns the captured params. Routes are inserted in
/// [`route_order`] so equal-specificity ties resolve deterministically, the
/// ones answered with content ahead of the ones a page shows. Returns `None`
/// when nothing matches.
///
/// [`recognize`]: tonk_router::Router::recognize
pub(super) async fn match_route(
    tonk: &crate::worker::TonkState,
    state: &dialog_reactor::BranchSession,
    rest: &str,
) -> Option<Matched> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_router::Route as RoutePattern;
    use tonk_schema::{HttpRoute, Route};

    /// What a pattern in the router stands for.
    #[derive(Clone)]
    enum Target {
        View(dialog_artifacts::Entity, dialog_artifacts::Entity),
        Http(dialog_artifacts::Entity, String),
    }

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

    // The routes libraries pinned before they shipped them as commands, which
    // a space keeps until the upgrade that withdraws them.
    let pinned = super::repository::LEGACY_ROUTES
        .iter()
        .map(|route| (*route).to_owned())
        .collect();
    route_order(&mut routes, &pinned);

    let mut answered: Vec<HttpRoute> = state
        .handle()
        .query()
        .select(Query::<HttpRoute> {
            this: Term::var("this"),
            path: Term::var("path"),
            body: Term::var("body"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();
    answered.sort_by_key(|route| route.this.to_string());

    let patterns = answered
        .into_iter()
        .map(|route| (route.path.0, Target::Http(route.this, route.body.0)))
        .chain(
            routes
                .into_iter()
                .map(|route| (route.path.0, Target::View(route.this, route.concept.0))),
        );

    let mut router = tonk_router::Router::new();
    for (path, target) in patterns {
        match RoutePattern::parse_pattern(&path) {
            Ok(pattern) => {
                router.insert(pattern, target);
            }
            Err(e) => {
                tonk_common::log!("match_route: skipping invalid route {path}: {e:?}");
            }
        }
    }

    let matched = router.recognize(rest).ok()?;
    Some(match matched.value.clone() {
        Target::View(route, concept) => Matched::View(MatchedRoute {
            route,
            concept,
            params: matched.params,
        }),
        Target::Http(route, body) => Matched::Http(MatchedHttp { route, body }),
    })
}

/// The route entity [`match_route`] picks for `rest`, for tests outside this
/// module that need the router's real answer.
#[cfg(test)]
pub(super) async fn matched_route(
    tonk: &crate::worker::TonkState,
    state: &dialog_reactor::BranchSession,
    rest: &str,
) -> Option<dialog_artifacts::Entity> {
    match match_route(tonk, state, rest).await? {
        Matched::View(matched) => Some(matched.route),
        Matched::Http(matched) => Some(matched.route),
    }
}

/// End-to-end: the route table a branch actually holds, resolved through
/// `match_route`. Complements `route_order_tests`, which pins the ordering
/// alone — these prove the router wiring agrees with it.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod match_route_tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// Install core plus `library` the way creation does, author `authored`
    /// on top if given, and resolve `path` through the real router to the
    /// model it mounts.
    async fn resolve(library: &str, authored: Option<&str>, path: &str) -> Option<String> {
        const LIBRARY: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "route-e2e").await;
        let tonk = state.read().await;

        crate::router::repository::install_fresh_seed(
            &tonk,
            &key,
            "main",
            &format!("{LIBRARY}\n{library}"),
            &[],
        )
        .await
        .expect("the library installs");

        if let Some(authored) = authored {
            crate::router::evaluate::evaluate_body(&tonk, &key, "main", authored.to_owned(), true)
                .await
                .expect("the space's own route commits");
        }

        let session = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .expect("main acquires");
        match super::match_route(&tonk, &session, path).await? {
            super::Matched::View(matched) => Some(matched.concept.to_string()),
            super::Matched::Http(matched) => Some(matched.body),
        }
    }

    /// A route a library ships as a command resolves: the rule wrote it.
    #[dialog_common::test]
    async fn it_resolves_a_route_the_library_ships() {
        let shipped = "seed/route!:\n  path: \"/probe\"\n  concept: probe:library\n";

        assert_eq!(
            resolve(shipped, None, "/probe").await.as_deref(),
            Some("probe:library")
        );
    }

    /// A route a library pinned before it shipped routes as commands loses
    /// to the space's own route for the same path, though its entity sorts
    /// first: resolved through the real router, not just the sort.
    #[dialog_common::test]
    async fn it_prefers_a_space_route_over_one_a_library_pinned() {
        let routes = "route!:\n  this: id:tonk:route/space\n  path: \"/probe\"\n  concept: probe:pinned\n\nroute!:\n  this: id:zzz/space-probe\n  path: \"/probe\"\n  concept: probe:space\n";

        assert_eq!(
            resolve("", Some(routes), "/probe").await.as_deref(),
            Some("probe:space"),
            "the space's own route must win the tie against the pinned one"
        );
    }
}

#[cfg(test)]
mod saved_tests {
    use super::{ClientRegistry, ClientState, SAVED_VERSION, Saved, Stamp};
    use crate::router::ClientId;

    fn stamp(site: &str) -> Stamp {
        Stamp {
            site: site.to_owned(),
            repo: "did:key:z6Mkspace".to_owned(),
            branch: "main".to_owned(),
            profile: false,
            path: "/notes".to_owned(),
            rest: "/notes".to_owned(),
            anchor: String::new(),
        }
    }

    async fn registry(clients: &[(&str, &[&str])]) -> ClientRegistry {
        let registry = ClientRegistry::default();
        {
            let mut ledger = registry.write().await;
            for (client, sites) in clients {
                let state = ledger.entry(ClientId((*client).to_owned())).or_default();
                for site in *sites {
                    state.sites.insert((*site).to_owned(), stamp(site));
                }
            }
            ledger.insert(ClientId("bare".to_owned()), ClientState::default());
        }
        registry
    }

    #[dialog_common::test]
    async fn it_reads_back_what_it_saved() {
        let clients = registry(&[("a", &["site:1", "site:2"]), ("b", &["site:3"])]).await;
        let saved = Saved::of(&clients).await;
        let bytes = serde_json::to_vec(&saved).expect("saves");
        let read = Saved::read(&bytes, |_| true).expect("reads");
        assert_eq!(read, saved);
        assert_eq!(
            read.clients.len(),
            2,
            "a client with no stamps is not saved"
        );
        assert_eq!(read.clients["a"], vec![stamp("site:1"), stamp("site:2")]);
    }

    #[dialog_common::test]
    async fn it_saves_equal_state_as_equal_bytes() {
        let first = registry(&[("a", &["site:1", "site:2", "site:3"])]).await;
        let second = registry(&[("a", &["site:3", "site:1", "site:2"])]).await;
        let first = serde_json::to_vec(&Saved::of(&first).await).expect("saves");
        let second = serde_json::to_vec(&Saved::of(&second).await).expect("saves");
        assert_eq!(first, second);
    }

    #[dialog_common::test]
    async fn it_drops_the_stamps_of_clients_that_closed() {
        let clients = registry(&[("a", &["site:1"]), ("b", &["site:2"])]).await;
        let bytes = serde_json::to_vec(&Saved::of(&clients).await).expect("saves");
        let read = Saved::read(&bytes, |client| client == "b").expect("reads");
        assert_eq!(read.clients.keys().collect::<Vec<_>>(), vec!["b"]);
    }

    #[dialog_common::test]
    async fn it_ignores_a_version_it_does_not_read() {
        let clients = registry(&[("a", &["site:1"])]).await;
        let mut saved = serde_json::to_value(Saved::of(&clients).await).expect("saves");
        saved["version"] = (SAVED_VERSION + 1).into();
        let bytes = serde_json::to_vec(&saved).expect("encodes");
        assert!(Saved::read(&bytes, |_| true).is_none());
        assert!(Saved::read(b"not json", |_| true).is_none());
    }
}

#[cfg(test)]
mod route_order_tests {
    use std::collections::HashSet;

    use tonk_schema::Route;
    use tonk_schema::domain::route::{Concept as RoutePathConcept, Path as RouteTablePath};

    /// A route row at `uri` matching `path`. The concept is irrelevant to
    /// ordering, so every row shares one.
    fn route(uri: &str, path: &str) -> Route {
        let entity: dialog_artifacts::Entity = uri.parse().expect("route uri parses");
        let concept: dialog_artifacts::Entity = "tonk:model".parse().expect("concept uri parses");
        Route {
            this: entity,
            path: RouteTablePath(path.to_string()),
            concept: RoutePathConcept(concept),
        }
    }

    fn ordered(routes: &[Route]) -> Vec<String> {
        routes.iter().map(|route| route.this.to_string()).collect()
    }

    /// The tie this exists to settle: a space's own route and one a library
    /// pinned on the same path. The space's wins regardless of how the URIs
    /// sort — which is what the old entity-URI-only order got wrong.
    #[test]
    fn it_orders_a_space_route_before_a_pinned_one() {
        let mut routes = vec![route("id:aaa/pinned", "/"), route("id:zzz/space", "/")];
        let pinned = HashSet::from(["id:aaa/pinned".to_string()]);

        super::route_order(&mut routes, &pinned);

        assert_eq!(
            ordered(&routes),
            vec!["id:zzz/space", "id:aaa/pinned"],
            "the space's own route must precede the pinned one"
        );
    }

    /// Within one group the order is by entity URI, so the table a router is
    /// built from is the same on every device.
    #[test]
    fn it_breaks_ties_within_a_group_by_entity_uri() {
        let mut routes = vec![route("id:zzz", "/"), route("id:aaa", "/")];

        super::route_order(&mut routes, &HashSet::new());

        assert_eq!(ordered(&routes), vec!["id:aaa", "id:zzz"]);
    }

    /// Two pinned routes on one path still order deterministically between
    /// themselves, so every device builds the same router.
    #[test]
    fn it_orders_two_pinned_routes_by_entity_uri() {
        let mut routes = vec![route("id:zzz/second", "/"), route("id:aaa/first", "/")];
        let pinned = HashSet::from(["id:zzz/second".to_string(), "id:aaa/first".to_string()]);

        super::route_order(&mut routes, &pinned);

        assert_eq!(ordered(&routes), vec!["id:aaa/first", "id:zzz/second"]);
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use ::axum::body::Body;
    use ::axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    use crate::router::tests::test_state;
    use crate::router::{ClientId, api_router_with_state};

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// The `xyz.tonk.site/{field}` value stamped for `client`, read back
    /// through `endpoint`'s query route.
    async fn stamped_field(
        app: &::axum::Router,
        client: &str,
        field: &str,
        endpoint: &str,
    ) -> Option<String> {
        // `concept` is an entity-typed site field; the rest read as text.
        let as_type = if field == "concept" { "Entity" } else { "Text" };
        let query = format!(
            r#"{{"predicate":{{"with":{{"{field}":{{"the":"xyz.tonk.site/{field}","as":"{as_type}","cardinality":"one"}}}}}},"terms":{{"this":"site:{client}","{field}":{{"?":{{"name":"{field}"}}}}}}}}"#
        );
        let mut request = Request::builder()
            .method("POST")
            .uri(endpoint.to_owned())
            .header("content-type", "application/json")
            .body(Body::from(query))
            .unwrap();
        request.extensions_mut().insert(ClientId(client.to_owned()));
        let response = app.clone().oneshot(request).await.unwrap();
        let body = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let rows: serde_json::Value = serde_json::from_slice(&body).ok()?;
        rows.as_array()?
            .first()?
            .get("fields")?
            .get(field)?
            .as_str()
            .map(str::to_owned)
    }

    /// The site's stamped `repo` — the location token a view interpolates.
    async fn stamped_repo(app: &::axum::Router, client: &str, endpoint: &str) -> Option<String> {
        stamped_field(app, client, "repo", endpoint).await
    }

    /// The profile library, embedded at compile time — seeded so the profile
    /// branch has a `route!` table for `/` to match against. Without it the
    /// stamp is skipped entirely and nothing is written.
    const PROFILE_LIBRARY: &str = include_str!("../../../tonk-core/assets/library/profile.yaml");

    /// The standard library, embedded at compile time — the space-branch
    /// counterpart of [`PROFILE_LIBRARY`], seeded so `/` matches a route.
    const CORE_LIBRARY: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

    /// The notebook library, embedded at compile time.
    const NOTEBOOK_LIBRARY: &str = include_str!("../../../tonk-core/assets/library/notebook.yaml");

    #[dialog_common::test]
    async fn it_stamps_a_resolvable_repo_token_on_the_profile_branch() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        {
            let tonk = state.read().await;
            crate::router::evaluate::evaluate_profile_body(
                &tonk,
                "main",
                PROFILE_LIBRARY.to_owned(),
                true,
            )
            .await
            .expect("the profile library seeds");
        }

        let mut request = Request::builder()
            .method("POST")
            .uri("/api/repository/profile:tonk/branch/main/site")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"path":"/"}"#))
            .unwrap();
        request
            .extensions_mut()
            .insert(ClientId("probe".to_owned()));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let repo = stamped_repo(
            &app,
            "probe",
            "/api/repository/profile:tonk/branch/main/query",
        )
        .await
        .expect("the profile site stamps a repo field");
        assert!(
            repo.starts_with("profile:"),
            "stamped repo {repo:?} must be a `profile:<name>` location token, not a bare name: \
             a `with=\"{{branch}}@{{repo}}\"` template built from it otherwise addresses a named \
             repository that does not exist"
        );
    }

    /// A site stamped on a NAMED space records the bare repository key — the
    /// same location token it always did. The profile prefix must not leak
    /// into the space path, or every space view's `with="{branch}@{repo}"`
    /// would address the profile endpoint instead of its own repository.
    #[dialog_common::test]
    async fn it_stamps_a_bare_repo_token_on_a_named_space() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        // A branchless `{}` create: the worker seeds nothing, so this test
        // drives the seed itself and the repo exists to seed onto.
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/repository/space")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = ::axum::body::to_bytes(created.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let key = info["name"]
            .as_str()
            .expect("the create returns a key")
            .to_owned();
        {
            let tonk = state.read().await;
            crate::router::evaluate::evaluate_body(
                &tonk,
                &key,
                "main",
                CORE_LIBRARY.to_owned(),
                true,
            )
            .await
            .expect("the core library seeds");
        }

        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/api/repository/{key}/branch/main/site"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"path":"/"}"#))
            .unwrap();
        request
            .extensions_mut()
            .insert(ClientId("space-probe".to_owned()));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let repo = stamped_repo(
            &app,
            "space-probe",
            &format!("/api/repository/{key}/branch/main/query"),
        )
        .await
        .expect("the space site stamps a repo field");
        assert_eq!(
            repo, key,
            "a named space stamps its bare repository key, unprefixed"
        );
    }

    /// A notebook installed on a PROFILE resolves its route and hands its view
    /// a location that addresses the profile endpoint.
    ///
    /// This is the whole point: `notebook.yaml` is written once, and its route
    /// view mounts `<tonk-display with="{branch}@{repo}">`. The guest does not
    /// know or care whether it is in a profile or a named space — it only
    /// interpolates the site's fields. So the `repo` the site stamps has to be
    /// a location token that reaches the branch the notebook actually lives on.
    #[dialog_common::test]
    async fn it_resolves_a_notebook_route_installed_on_a_profile() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        {
            let tonk = state.read().await;
            // The profile library first (it declares the shared `view` /
            // `route` concepts), then the notebook on top — the install a
            // author performs to get a notebook onto their profile.
            for library in [PROFILE_LIBRARY, NOTEBOOK_LIBRARY] {
                crate::router::evaluate::evaluate_profile_body(
                    &tonk,
                    "main",
                    library.to_owned(),
                    true,
                )
                .await
                .expect("the library installs on the profile");
            }
        }

        let mut request = Request::builder()
            .method("POST")
            .uri("/api/repository/profile:tonk/branch/main/site")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"path":"/notebook"}"#))
            .unwrap();
        request.extensions_mut().insert(ClientId("nb".to_owned()));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // The notebook index route matched, so the notebook's own model is
        // what the shell mounts rather than the profile's `/{*rest}`
        // fallback. Compared against the NOT-FOUND model rather than a
        // literal: the index concept is anchor-named, so its entity derives
        // from its body and no URI is stable to assert.
        let concept = stamped_field(
            &app,
            "nb",
            "concept",
            "/api/repository/profile:tonk/branch/main/query",
        )
        .await
        .expect("the notebook route stamps a concept");
        // What `/` resolves to is the profile's Hub; `/notebook` must NOT be
        // that, and must not be the not-found fallback either.
        let home = {
            let mut request = Request::builder()
                .method("POST")
                .uri("/api/repository/profile:tonk/branch/main/site")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"path":"/"}"#))
                .unwrap();
            request
                .extensions_mut()
                .insert(ClientId("nb-home".to_owned()));
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            stamped_field(
                &app,
                "nb-home",
                "concept",
                "/api/repository/profile:tonk/branch/main/query",
            )
            .await
            .expect("the home route stamps a concept")
        };
        assert_eq!(home, "tonk:hub", "the profile's `/` is the Hub");
        assert_ne!(
            concept, home,
            "/notebook on a profile must match the notebook index route, not the Hub"
        );
        assert_ne!(
            concept, "tonk:not-found",
            "/notebook on a profile must not fall through to not-found"
        );

        // And the location its view builds reaches the profile endpoint.
        let repo = stamped_repo(&app, "nb", "/api/repository/profile:tonk/branch/main/query")
            .await
            .expect("the notebook route stamps a repo");
        let branch = stamped_field(
            &app,
            "nb",
            "branch",
            "/api/repository/profile:tonk/branch/main/query",
        )
        .await
        .expect("the notebook route stamps a branch");
        let with = format!("{branch}@{repo}");
        assert!(
            with.starts_with("main@profile:") && with.len() > "main@profile:".len(),
            "the notebook view's `with` ({with:?}) must address the profile endpoint \
             with a named profile, not a repository"
        );
    }
}
