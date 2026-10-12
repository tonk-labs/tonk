//! Execute the original UCAN at the resource boundary.
//!
//! Authorization-header requests perform their verified operation immediately.
//! Legacy CBOR data requests receive actionable upgrade guidance;
//! every supported data request verifies the full chain at execution. No service authority or
//! expiry is minted. Registration/revocation/deletion commands retain their routes.

use crate::error::Refusal;
#[cfg(target_arch = "wasm32")]
use crate::handlers::registration::handle as handle_registration;
use crate::operation::Operation;
#[cfg(target_arch = "wasm32")]
use crate::registration::registration_command;
use dialog_capability::access::AuthorizeError;
#[cfg(target_arch = "wasm32")]
use dialog_remote_s3::Address;
#[cfg(target_arch = "wasm32")]
use dialog_remote_ucan_s3::UcanAuthorizer;
use worker::*;

struct PresignFailure {
    refusal: Refusal,
    operation: crate::observability::AccessOperation,
    failure_kind: crate::observability::AccessFailureKind,
    retryable: bool,
    site: crate::observability::AccessSite,
}

impl PresignFailure {
    fn authorization(refusal: Refusal) -> Self {
        let status = refusal.status();
        Self {
            refusal,
            operation: crate::observability::AccessOperation::Authorization,
            failure_kind: match status {
                401 | 403 => crate::observability::AccessFailureKind::AccessDenied,
                503 => crate::observability::AccessFailureKind::Unavailable,
                500..=599 => crate::observability::AccessFailureKind::Internal,
                _ => crate::observability::AccessFailureKind::Invalid,
            },
            retryable: status >= 500,
            site: crate::observability::AccessSite::Ucan,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn provisioning(
        refusal: Refusal,
        failure_kind: crate::observability::AccessFailureKind,
        retryable: bool,
        site: crate::observability::AccessSite,
    ) -> Self {
        Self {
            refusal,
            operation: crate::observability::AccessOperation::Provisioning,
            failure_kind,
            retryable,
            site,
        }
    }

    fn emit(&self) {
        let status = self.refusal.status();
        crate::observability::AccessFailureLog::new(
            self.operation,
            if status >= 500 {
                crate::observability::AccessOutcome::Unavailable
            } else {
                crate::observability::AccessOutcome::Refused
            },
            self.failure_kind,
            status,
            self.retryable,
            self.site,
        )
        .emit();
    }
}

/// Add CORS headers to a response for WASM compatibility.
fn with_cors_headers(response: Response) -> Response {
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "POST, OPTIONS");
    let _ = headers.set(
        "Access-Control-Allow-Headers",
        "Authorization, Cache-Control, Content-Type, Accept, Range",
    );
    let _ = headers.set(
        "Access-Control-Expose-Headers",
        "Content-Type, Content-Length, Content-Range, ETag, Server-Timing, UCAN-Command, UCAN-Subject, UCAN-Arguments",
    );
    response.with_headers(headers)
}

/// OPTIONS /ucan/ → Handle CORS preflight
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let response = with_cors_headers(Response::empty()?.with_status(204));
    // Only the preflight can be cached, so the lifetime is set here
    // rather than on every response.
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Max-Age", crate::PREFLIGHT_MAX_AGE);
    Ok(response.with_headers(headers))
}

/// The largest `/ucan/` body this service will read.
///
/// Every legitimate request is a container of a few signed tokens: the
/// biggest is an enrollment, carrying an invocation, its delegation
/// chain, and three small blocks — low single-digit KiB. The default
/// leaves an order of magnitude for deep chains while keeping the
/// endpoint from being used as a store.
///
/// `UCAN_MAX_BODY_BYTES` overrides it, so a future command that
/// legitimately carries more does not need a code change to unblock.
const DEFAULT_MAX_BODY_BYTES: u64 = 64 * 1024;

/// The configured body limit, or [`DEFAULT_MAX_BODY_BYTES`].
fn max_body_bytes(env: &Env) -> u64 {
    env.var("UCAN_MAX_BODY_BYTES")
        .ok()
        .and_then(|value| value.to_string().parse().ok())
        .unwrap_or(DEFAULT_MAX_BODY_BYTES)
}

/// What the caller said it was sending, when it said.
fn declared_length(req: &Request) -> Option<u64> {
    req.headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok())
}

/// `413`, naming the limit so a caller can act on it.
fn too_large(limit: u64) -> Result<Response> {
    Ok(Response::from_json(&serde_json::json!({
        "error": {
            "code": "PAYLOAD_TOO_LARGE",
            "message": format!("request body exceeds the {limit}-byte limit for /ucan/"),
        }
    }))?
    .with_status(413))
}

/// The invocation the request carries in `Authorization`, when it
/// carries one under the UCAN scheme.
fn credential(req: &Request) -> Option<String> {
    req.headers()
        .get("authorization")
        .ok()
        .flatten()
        .filter(|value| dialog_remote_ucan::is_credential(value))
}

/// POST /ucan/ executes a direct invocation or returns the original-request
/// upgrade-required rejection for legacy data clients, recording execution usage.
pub async fn serve(mut req: Request, env: Env, ctx: Context) -> Result<Response> {
    if let Some(credential) = credential(&req) {
        return serve_invocation(req, &credential, env, ctx)
            .await
            .map(with_cors_headers);
    }

    let limit = max_body_bytes(&env);
    // Refused on size alone, before anything is decoded: a body this
    // large is not a UCAN we failed to parse, and running the parser
    // over it is the work the limit exists to avoid.
    if let Some(declared) = declared_length(&req)
        && declared > limit
    {
        return Ok(with_cors_headers(too_large(limit)?));
    }
    let body_bytes = match req.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            let refusal: Refusal = AuthorizeError::Malformed {
                detail: format!("failed to read request body: {e}"),
            }
            .into();
            return Ok(with_cors_headers(refusal.to_response()?));
        }
    };
    // A request that declared nothing, or lied about it.
    if body_bytes.len() as u64 > limit {
        return Ok(with_cors_headers(too_large(limit)?));
    }

    // Registration commands ride the same endpoint; anything else falls
    // through to the data execution path untouched. Registration is not
    // metered: those invocations are once-per-account ceremonies, not
    // billable operations.
    #[cfg(target_arch = "wasm32")]
    if crate::deletion::is_deletion(&body_bytes) {
        return crate::handlers::deletion::handle(&body_bytes, &env)
            .await
            .map(with_cors_headers);
    }
    #[cfg(target_arch = "wasm32")]
    if crate::deletion::is_purge(&body_bytes) {
        return crate::handlers::deletion::handle_purge(&body_bytes, &env)
            .await
            .map(with_cors_headers);
    }
    // Revocation writes to the index rather than reading it, so it is
    // answered here rather than on the data execution path that consults it.
    #[cfg(target_arch = "wasm32")]
    if crate::revoke::is_revocation(&body_bytes) {
        return crate::handlers::revoke::handle(&body_bytes, &env)
            .await
            .map(with_cors_headers);
    }
    #[cfg(target_arch = "wasm32")]
    if registration_command(&body_bytes).is_some() {
        return handle_registration(&body_bytes, &req, &env)
            .await
            .map(with_cors_headers);
    }

    // Ticket claims remain supported, including claims delegated by a holder.
    #[cfg(target_arch = "wasm32")]
    if tonk_account::ticket::is_claim(&body_bytes) {
        return serve_claim(&body_bytes, &env).await.map(with_cors_headers);
    }

    // Control-plane CBOR commands above remain supported. Data operations
    // must carry original UCAN authority at execution, never redeem a permit.
    Response::from_json(&crate::operation::upgrade_required())
        .map(|r| with_cors_headers(r.with_status(426)))
}

/// Answer a `/use/get/ticket/claim` with the ticket the space it names
/// keeps for its subject (see [`tonk_account::ticket`]), or 404 when it
/// keeps none. The claim is verified as any invocation is, chain and
/// revocations included, so the holder's delegates claim for it; the
/// space, not the holder, is screened, since that is what is served.
#[cfg(target_arch = "wasm32")]
async fn serve_claim(body_bytes: &[u8], env: &Env) -> Result<Response> {
    use crate::revocation::{checker::IndexedRevocations, index::kv::KvRevocationIndex};

    let container = match dialog_ucan_core::Container::from_bytes(body_bytes) {
        Ok(container) => container,
        Err(error) => {
            return Refusal::Authorization(AuthorizeError::Malformed {
                detail: error.to_string(),
            })
            .to_response();
        }
    };
    let revocations = match env.kv("REVOCATIONS_KV") {
        Ok(store) => IndexedRevocations(KvRevocationIndex::new(store)),
        Err(_) => return unavailable().to_response(),
    };
    let claim = match dialog_remote_ucan_s3::verify_invocation(
        container,
        shared_resolver().as_ref(),
        &revocations,
    )
    .await
    .and_then(|chain| tonk_account::ticket::claimed(&chain))
    {
        Ok(claim) => claim,
        Err(reason) => return Refusal::Authorization(reason).to_response(),
    };
    if let Err(failure) = screen_subject(claim.space.as_ref(), env).await {
        failure.emit();
        return failure.refusal.to_response();
    }
    let bucket = match env.bucket("BUCKET") {
        Ok(bucket) => bucket,
        Err(error) => {
            return Refusal::unclassified(format!("Missing BUCKET: {error}")).to_response();
        }
    };
    match crate::objects::read(&bucket, &claim.object_key()).await {
        Ok(Some((bytes, _))) => Response::from_bytes(bytes),
        Ok(None) => Ok(Response::empty()?.with_status(404)),
        Err(error) => Refusal::unclassified(error).to_response(),
    }
}

/// Serve an invocation that arrived in `Authorization`: verify it and
/// perform it in this request.
async fn serve_invocation(
    mut req: Request,
    credential: &str,
    env: Env,
    ctx: Context,
) -> Result<Response> {
    let container = match dialog_remote_ucan::credential_container(credential) {
        Ok(container) => container,
        Err(error) => {
            let refusal: Refusal = AuthorizeError::Malformed {
                detail: format!("the credential does not carry a container: {error}"),
            }
            .into();
            return refusal.to_response();
        }
    };
    // The container's own bytes, for the screens and the ledger that
    // read a container as the body used to carry it.
    let container_bytes = match container.to_bytes() {
        Ok(bytes) => bytes,
        Err(error) => {
            let refusal = Refusal::unclassified(format!("container: {error}"));
            return refusal.to_response();
        }
    };
    let served = perform(container, &container_bytes, &mut req, &env, &ctx).await;
    answer(served, &container_bytes, &env, &ctx)
}

/// The response for how an invocation was served, its record queued
/// behind it.
fn answer(
    served: std::result::Result<(Response, u64), PresignFailure>,
    container_bytes: &[u8],
    env: &Env,
    ctx: &Context,
) -> Result<Response> {
    let (response, metered) = match served {
        Ok((response, bytes)) => {
            let status = response.status_code();
            let metered = if (200..300).contains(&status) {
                Some(("ok", None, bytes))
            } else if matches!(status, 401 | 403) {
                Some(("denied", Some("execution refused".into()), 0))
            } else {
                None
            };
            (response, metered)
        }
        Err(failure) => {
            failure.emit();
            let refusal = failure.refusal;
            // Denials are recorded — a client retrying against a blocked
            // consumer still costs invocations — but only attributable
            // ones: infra failures and malformed containers are the
            // service's cost, not the consumer's.
            let metered =
                matches!(refusal.status(), 401 | 403).then(|| ("denied", Some(refusal.kind()), 0));
            (refusal.to_response()?, metered)
        }
    };

    #[cfg(target_arch = "wasm32")]
    if let Some((outcome, reason, bytes)) = metered {
        record_invocation(container_bytes, outcome, reason, bytes, env, ctx);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (metered, container_bytes, env, ctx);

    Ok(response)
}

/// Queue the invocation record behind the response. Failures are logged,
/// never surfaced: metering loss errs in the customer's favour and must
/// not cost them the permit they already hold.
#[cfg(target_arch = "wasm32")]
fn record_invocation(
    body_bytes: &[u8],
    outcome: &'static str,
    reason: Option<String>,
    bytes: u64,
    env: &Env,
    ctx: &Context,
) {
    use crate::store::ingest::{D1Ingest, IngestStore};

    let now = Date::now().as_millis() / 1_000;
    let Some(record) = crate::metering::collect(body_bytes, outcome, reason, bytes, now) else {
        return;
    };
    match env.d1("INGEST") {
        Ok(database) => ctx.wait_until(async move {
            if let Err(err) = D1Ingest::new(database).record(&record).await {
                console_error!("metering write failed: {err}");
            }
        }),
        Err(err) => console_error!("metering skipped, no INGEST binding: {err}"),
    }
}

/// When the stages of verifying an invocation finished, in
/// milliseconds since the epoch, for `Server-Timing`.
struct Stages {
    started: u64,
    authorized: u64,
    screened: u64,
}

/// Verify the invocation's chain, its revocations and the subject's
/// provisioning, and read the operation it authorizes: the same three
/// steps for the operation's outcome.
async fn authorize(
    body_bytes: &[u8],
    env: &Env,
) -> std::result::Result<(dialog_remote_ucan::Verified, Stages), PresignFailure> {
    let started = Date::now().as_millis();
    let container = dialog_ucan_core::Container::from_bytes(body_bytes).map_err(|e| {
        PresignFailure::authorization(
            AuthorizeError::Malformed {
                detail: e.to_string(),
            }
            .into(),
        )
    })?;
    #[cfg(target_arch = "wasm32")]
    let access = {
        use crate::revocation::{checker::IndexedRevocations, index::r2::R2RevocationIndex};
        let index = R2RevocationIndex::from_env(env)
            .map_err(|_| PresignFailure::authorization(unavailable()))?;
        dialog_remote_ucan::Access::with_shared_resolver((), shared_resolver())
            .with_revocations(IndexedRevocations(index))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let access = dialog_remote_ucan::Access::new(());
    let verified = access
        .verify(container)
        .await
        .map_err(|e| PresignFailure::authorization(Refusal::Authorization(e.reason().clone())))?;
    let authorized = Date::now().as_millis();
    #[cfg(target_arch = "wasm32")]
    screen_provisioning(body_bytes, env).await?;
    #[cfg(not(target_arch = "wasm32"))]
    let _ = env;
    Ok((
        verified,
        Stages {
            started,
            authorized,
            screened: Date::now().as_millis(),
        },
    ))
}

async fn perform(
    _container: dialog_ucan_core::Container,
    container_bytes: &[u8],
    req: &mut Request,
    env: &Env,
    ctx: &Context,
) -> std::result::Result<(Response, u64), PresignFailure> {
    let (verified, stages) = authorize(container_bytes, env).await?;
    let operation = Operation::from_verified(&verified).map_err(|detail| {
        PresignFailure::authorization(AuthorizeError::Malformed { detail }.into())
    })?;
    let url = req
        .url()
        .map_err(|e| PresignFailure::authorization(Refusal::unclassified(e.to_string())))?;
    let bucket = env
        .bucket("BUCKET")
        .map_err(|_| PresignFailure::authorization(unavailable()))?;
    // Cache only immutable content, after the complete chain and provisioning
    // check. A cache hit is data, never an authorization result.
    let cache_url = format!(
        "https://objects.tonk.invalid/v2/{}",
        hex::encode(operation.key.as_bytes())
    );
    let bypass = req
        .headers()
        .get("cache-control")
        .ok()
        .flatten()
        .is_some_and(|v| v.contains("no-cache") || v.contains("no-store"))
        || url
            .query_pairs()
            .any(|(k, v)| k == "cache" && v == "bypass");
    let mut cached = None;
    if operation.immutable && operation.method == crate::operation::Method::Get && !bypass {
        let headers = Headers::new();
        if let Some(range) = operation.range_header() {
            let _ = headers.set("Range", &range);
        }
        let mut init = RequestInit::new();
        init.with_headers(headers);
        if let Ok(request) = Request::new_with_init(&cache_url, &init) {
            cached = Cache::default()
                .get(&request, false)
                .await
                .ok()
                .flatten()
                .filter(|r| r.status_code() == if operation.range.is_some() { 206 } else { 200 });
        }
    }
    let hit = cached.is_some();
    let mut bytes = 0;
    let response = if let Some(response) = cached {
        response
    } else {
        match super::object::execute(req, &bucket, &operation, &mut bytes).await {
            Ok(response) => response,
            Err(failure) => failure
                .into_response()
                .map_err(|e| PresignFailure::authorization(Refusal::unclassified(e.to_string())))?,
        }
    };
    if !hit && operation.immutable && operation.range.is_none() && response.status_code() == 200 {
        let key = operation.key.clone();
        // A separate R2 stream avoids teeing an unbounded slow-client queue or
        // collecting the upload/read in Rust memory. No partial responses cache.
        ctx.wait_until(async move {
            if let Ok(response) = super::object::get(&bucket, &key, None).await
                && response.status_code() == 200
            {
                let headers = response.headers().clone();
                let _ = headers.set("Cache-Control", "public, max-age=31536000, immutable");
                let _ = Cache::default()
                    .put(cache_url, response.with_headers(headers))
                    .await;
            }
        });
    }
    let headers = response.headers().clone();
    for (name, value) in crate::describe::describe(verified.chain()) {
        let _ = headers.set(name, &value);
    }
    let _ = headers.set(
        "Server-Timing",
        &format!(
            "authorize;dur={}, screen;dur={}, total;dur={}, cache;desc={}",
            stages.authorized.saturating_sub(stages.started),
            stages.screened.saturating_sub(stages.authorized),
            Date::now().as_millis().saturating_sub(stages.started),
            if hit {
                "hit"
            } else if bypass {
                "bypass"
            } else {
                "miss"
            }
        ),
    );
    Ok((response.with_headers(headers), bytes))
}

/// Screen the subject against the provisioning gate: a space is served
/// only while an active customer pays for it. Registration commands
/// never reach here — `serve` answers them before the data execution path —
/// so enrolling and activating stay possible while the gate denies the
/// data plane.
///
/// The verdict resolves per plan/Access metering.md §11.3: isolate
/// cache, then KV, then control D1 on a miss, writing the derived
/// verdict back. D1 is the authority; the caches only remember its
/// answers until their `not_after`.
#[cfg(target_arch = "wasm32")]
async fn screen_provisioning(
    body_bytes: &[u8],
    env: &Env,
) -> std::result::Result<(), PresignFailure> {
    use crate::provisioning::container_subject;

    let Some(subject) = container_subject(body_bytes) else {
        // The authorizer accepted these bytes, so a subject we cannot
        // read is shape drift between two parsers rather than a caller
        // error. There is nothing to screen against, so it cannot clear.
        return Err(PresignFailure::provisioning(
            provisioning_unavailable(),
            crate::observability::AccessFailureKind::Internal,
            false,
            crate::observability::AccessSite::Provisioning,
        ));
    };
    screen_subject(&subject, env).await
}

/// Screen `subject` against the control store, through the isolate's
/// cache of recent verdicts.
#[cfg(target_arch = "wasm32")]
async fn screen_subject(subject: &str, env: &Env) -> std::result::Result<(), PresignFailure> {
    use crate::provisioning::cache::{self, CachedVerdict};

    let subject = subject.to_owned();
    let now = Date::now().as_millis() / 1_000;

    if let Some(cached) = cache::isolate_lookup(&subject, now) {
        return cached.verdict().map_err(|reason| {
            PresignFailure::provisioning(
                reason.into(),
                crate::observability::AccessFailureKind::NotProvisioned,
                false,
                crate::observability::AccessSite::Provisioning,
            )
        });
    }

    let kv = servability_kv(env);
    if let Some(kv) = &kv {
        match kv.get(&cache::key(&subject)).text().await {
            Ok(Some(text)) => {
                if let Some(cached) = CachedVerdict::decode(&text).filter(|c| c.fresh(now)) {
                    cache::isolate_store(&subject, cached.clone(), now);
                    return cached.verdict().map_err(|reason| {
                        PresignFailure::provisioning(
                            reason.into(),
                            crate::observability::AccessFailureKind::NotProvisioned,
                            false,
                            crate::observability::AccessSite::Provisioning,
                        )
                    });
                }
            }
            // A miss is not an answer — KV is eventually consistent —
            // so it falls through to authoritative D1.
            Ok(None) => {}
            Err(_) => {
                // A KV read error is served, per the plan: the gate
                // exists for billing, and its cache being unreachable
                // is the service's own trouble, not the caller's.
                console_error!("servability cache unreadable; serving request unscreened");
                return Ok(());
            }
        }
    }

    let outcome = derive_verdict(&subject, now, env, kv.as_ref())
        .await
        .map_err(|_| {
            PresignFailure::provisioning(
                provisioning_unavailable(),
                crate::observability::AccessFailureKind::Unavailable,
                true,
                crate::observability::AccessSite::ControlStore,
            )
        })?;
    match outcome {
        Ok(()) => Ok(()),
        Err(reason) => Err(PresignFailure::provisioning(
            reason.into(),
            crate::observability::AccessFailureKind::NotProvisioned,
            false,
            crate::observability::AccessSite::Provisioning,
        )),
    }
}

/// The verdict-cache KV namespace, if this deployment has one. Serving
/// degrades to per-request D1 reads without it rather than refusing.
#[cfg(target_arch = "wasm32")]
pub(crate) fn servability_kv(env: &Env) -> Option<worker::kv::KvStore> {
    use crate::provisioning::cache;

    match env.kv(cache::BINDING) {
        Ok(kv) => Some(kv),
        Err(_) => {
            console_error!("servability cache binding is absent");
            None
        }
    }
}

/// Derive `subject`'s verdict from control D1 and remember it in the
/// isolate cache and KV. The single write path for cached verdicts:
/// the data execution path calls it on a cache miss, the registration and
/// deletion handlers after a state change, so a change propagates as
/// itself rather than waiting out a stale entry's validity.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn derive_verdict(
    subject: &str,
    now: u64,
    env: &Env,
    kv: Option<&worker::kv::KvStore>,
) -> std::result::Result<std::result::Result<(), AuthorizeError>, Refusal> {
    use crate::provisioning::cache::{self, CachedVerdict};
    use crate::provisioning::screen;
    use crate::store::d1::D1Store;

    let store = D1Store::new(env.d1("CONTROL").map_err(|_| provisioning_unavailable())?);
    match screen(&store, subject, now).await {
        Ok(outcome) => {
            let cached = CachedVerdict::record(&outcome, subject, now);
            cache::isolate_store(subject, cached.clone(), now);
            if let Some(kv) = kv {
                let write = kv
                    .put(&cache::key(subject), cached.encode())
                    .map(|put| put.expiration_ttl(cached.retention(now)));
                let written = match write {
                    Ok(put) => put.execute().await.map_err(|err| err.to_string()),
                    Err(err) => Err(err.to_string()),
                };
                if written.is_err() {
                    console_error!("servability verdict was not cached");
                }
            }
            Ok(outcome)
        }
        Err(_) => {
            // The gate fails closed, but a store failure is the
            // service's own unavailability, not a denial to bill.
            Err(provisioning_unavailable())
        }
    }
}

/// The 503 for a gate that could not reach a verdict.
#[cfg(target_arch = "wasm32")]
fn provisioning_unavailable() -> Refusal {
    AuthorizeError::Unavailable {
        detail: "provisioning registry unavailable, retry shortly".to_string(),
    }
    .into()
}

/// The client-facing 503. The reason stays in the logs: it names
/// internal infrastructure and the caller can do nothing with it but
/// retry.
fn unavailable() -> Refusal {
    AuthorizeError::Unavailable {
        detail: "access service unavailable, retry shortly".to_string(),
    }
    .into()
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// The authorizer built by this isolate, if it has built one.
    static AUTHORIZER: std::cell::OnceCell<UcanAuthorizer> =
        const { std::cell::OnceCell::new() };
        /// The issuer resolver the access layer verifies with, one per
    /// isolate so its cache of resolved `did:web` documents outlives a
    /// request.
    static RESOLVER: std::cell::OnceCell<std::sync::Arc<dialog_remote_ucan_s3::DefaultResolver>> =
        const { std::cell::OnceCell::new() };
}

#[cfg(target_arch = "wasm32")]
fn shared_resolver() -> std::sync::Arc<dialog_remote_ucan_s3::DefaultResolver> {
    RESOLVER.with(|cached| {
        cached
            .get_or_init(|| {
                std::sync::Arc::new(dialog_did_web::CachingResolver::new(
                    dialog_did_web::WebResolver::new(),
                ))
            })
            .clone()
    })
}

/// The address every authorizer describes requests against. See
/// [`authorizer_address`].
#[cfg(target_arch = "wasm32")]
static ADDRESS: std::sync::LazyLock<Address> = std::sync::LazyLock::new(|| {
    Address::builder("https://object.invalid")
        .region("auto")
        .bucket("objects")
        .build()
        .expect("the placeholder address is well-formed")
});

/// The address the authorizer describes requests against.
///
/// It is a placeholder. The authorizer needs an S3 address to turn a
/// verified invocation into a request, but nothing here talks S3 any
/// more: the request is read back off the permit ([`Operation::lift`])
/// for enrollment custody storage only. The host does not resolve on purpose, so a
/// permit that escaped this translation fails loudly instead of
/// reaching a bucket.
#[cfg(target_arch = "wasm32")]
pub(crate) fn authorizer_address() -> &'static Address {
    &ADDRESS
}

/// The UcanAuthorizer for this isolate.
///
/// Built once and reused: what it carries across requests is its
/// `did:web` resolution cache, which is what makes a chain issued by a
/// web identity cheap to verify the second time. A failed build is
/// not cached: the next request tries again.
#[cfg(target_arch = "wasm32")]
pub(crate) fn create_authorizer(env: &Env) -> std::result::Result<UcanAuthorizer, Refusal> {
    AUTHORIZER.with(|cached| {
        if let Some(authorizer) = cached.get() {
            return Ok(authorizer.clone());
        }
        let authorizer = build_authorizer(env)?;
        let _ = cached.set(authorizer.clone());
        Ok(authorizer)
    })
}

/// Create the authorizer: verification only, describing requests
/// against the placeholder [`authorizer_address`] with no credential,
/// since nothing it produces is ever presigned.
#[cfg(target_arch = "wasm32")]
fn build_authorizer(_env: &Env) -> std::result::Result<UcanAuthorizer, Refusal> {
    Ok(UcanAuthorizer::new(authorizer_address().clone(), None))
}
