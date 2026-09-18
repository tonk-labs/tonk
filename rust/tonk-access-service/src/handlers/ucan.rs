//! UCAN authorization handler.
//!
//! Handles POST /ucan/ requests. An invocation arrives one of two ways:
//!
//! - Under the UCAN scheme in `Authorization`, the body being the bytes
//!   the operation stores. The invocation is verified and performed in
//!   this request over the bucket ([`crate::objects`]). Off the worker
//!   runtime, where there is no bucket, it is answered with a
//!   service-signed permit against this worker's `/object/` path (see
//!   [`crate::permit`]), as CBOR.
//! - As a CBOR container in the body, the permit flow of clients that
//!   ask for it and of the registration, revocation and deletion
//!   commands, always answered with a permit.
//!
//! Served outside the Router, straight from the fetch event: recording
//! an invocation must outlive the response, and only the event's
//! [`Context`] can extend the isolate's life for it.

use crate::error::Refusal;
#[cfg(target_arch = "wasm32")]
use crate::handlers::registration::handle as handle_registration;
use crate::permit::{Claims, PermitKey};
#[cfg(target_arch = "wasm32")]
use crate::registration::registration_command;
use dialog_capability::access::AuthorizeError;
use dialog_remote_s3::{Address, S3Error};
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

/// The limit for a request that carries its invocation in
/// `Authorization`: its body is the bytes a write stores, so it is
/// bounded by the object size the service accepts rather than by a
/// chain's size.
const DEFAULT_MAX_PAYLOAD_BYTES: u64 = 32 * 1024 * 1024;

fn max_payload_bytes(env: &Env) -> u64 {
    env.var("UCAN_MAX_PAYLOAD_BYTES")
        .ok()
        .and_then(|value| value.to_string().parse().ok())
        .unwrap_or(DEFAULT_MAX_PAYLOAD_BYTES)
        .max(max_body_bytes(env))
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

/// POST /ucan/ → Authorize UCAN invocation and return presigned S3
/// request, recording the invocation in ingest under `ctx.wait_until`.
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
    // through to the presign path untouched. Registration is not
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
    // answered here rather than on the presign path that consults it.
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

    let origin = match origin(&req) {
        Ok(origin) => origin,
        Err(refusal) => return Ok(with_cors_headers(refusal.to_response()?)),
    };
    let served = presign(&body_bytes, &origin, &env).await;
    answer(served, &body_bytes, &env, &ctx).map(with_cors_headers)
}

/// Serve an invocation that arrived in `Authorization`: verify it and
/// perform it in this request when this runtime has the bucket, else
/// answer with a permit for it.
async fn serve_invocation(
    mut req: Request,
    credential: &str,
    env: Env,
    ctx: Context,
) -> Result<Response> {
    let limit = max_payload_bytes(&env);
    if let Some(declared) = declared_length(&req)
        && declared > limit
    {
        return too_large(limit);
    }
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
    let origin = match origin(&req) {
        Ok(origin) => origin,
        Err(refusal) => return refusal.to_response(),
    };

    let served = match perform(container, &container_bytes, &mut req, &env, &ctx).await {
        Ok(Some(answer)) => Ok(answer),
        Ok(None) => presign(&container_bytes, &origin, &env).await,
        Err(failure) => Err(failure),
    };
    answer(served, &container_bytes, &env, &ctx)
}

/// Permits are redeemed where they were issued: the origin the client
/// reached this service at is the one its `/object/` URLs name, so a
/// preview alias and a custom domain each answer for themselves.
fn origin(req: &Request) -> std::result::Result<String, Refusal> {
    req.url()
        .map(|url| url.origin().ascii_serialization())
        .map_err(|error| Refusal::unclassified(format!("request url: {error}")))
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
        Ok((response, bytes)) => (response, Some(("ok", None, bytes))),
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

/// Authorize the container and answer the signed permit, together
/// with the declared write bytes when the permit carries them.
/// When the stages of verifying an invocation finished, in
/// milliseconds since the epoch, for `Server-Timing`.
struct Stages {
    started: u64,
    authorized: u64,
    screened: u64,
}

/// Verify the invocation's chain, its revocations and the subject's
/// provisioning, and read the operation it authorizes: the same three
/// steps whether the answer is a permit or the operation's outcome.
async fn authorize(
    body_bytes: &[u8],
    env: &Env,
) -> std::result::Result<(dialog_remote_s3::Permit, Stages), PresignFailure> {
    let started = Date::now().as_millis();
    let authorizer = create_authorizer(env).map_err(PresignFailure::authorization)?;

    // Revocation is checked inside the chain walk rather than after it,
    // so every proof is measured against the principals entitled to
    // revoke that particular link. The index is bound per request: it
    // wraps a KV handle taken from this request's `Env`, which is not
    // ours to keep, unlike the deployment config the authorizer caches.
    #[cfg(target_arch = "wasm32")]
    let authorizer = {
        use crate::revocation::{checker::IndexedRevocations, index::kv::KvRevocationIndex};

        let store = env
            .kv("REVOCATIONS_KV")
            .map_err(|_| PresignFailure::authorization(unavailable()))?;
        authorizer.with_revocations(IndexedRevocations(KvRevocationIndex::new(store)))
    };

    let (authorized_request, expires) = authorizer
        .authorize_with_expiration(body_bytes)
        .await
        .map_err(map_access_error)
        .map_err(PresignFailure::authorization)?;
    let authorized = Date::now().as_millis();

    #[cfg(target_arch = "wasm32")]
    screen_provisioning(body_bytes, env).await?;
    let screened = Date::now().as_millis();

    Ok((
        authorized_request,
        Stages {
            started,
            authorized,
            screened,
        },
    ))
}

/// Carry the operation out in the request that proved it, through the
/// access layer over this service's bucket: the layer verifies the invocation and checks a
/// write's body against what the invocation bound, feeding a blob's
/// bytes to the bucket as they arrive; this service screens the
/// subject's provisioning between the two, as it does before issuing a
/// permit. `None` when the layer does not perform the operation, which
/// the caller answers with a permit instead. Never answers with a
/// permit's media type, which is how the client tells the two answers
/// apart.
#[cfg(target_arch = "wasm32")]
async fn perform(
    container: dialog_ucan_core::Container,
    container_bytes: &[u8],
    req: &mut Request,
    env: &Env,
    ctx: &Context,
) -> std::result::Result<Option<(Response, u64)>, PresignFailure> {
    use crate::cached::{Cached, worker::WorkerCache};
    use crate::revocation::{checker::IndexedRevocations, index::kv::KvRevocationIndex};
    use dialog_remote_ucan::{Access, Answer, Content, Payload};

    let started = Date::now().as_millis();
    let bucket = env.bucket("BUCKET").map_err(|e| {
        PresignFailure::authorization(Refusal::unclassified(format!("Missing BUCKET: {e}")))
    })?;
    let revocations = env
        .kv("REVOCATIONS_KV")
        .map_err(|_| PresignFailure::authorization(unavailable()))?;
    // Content-addressed objects are served from the data center's cache
    // when it holds them, and every one read or written fills it.
    let objects = Cached::new(crate::objects::Objects::new(bucket), WorkerCache::default())
        .with_mode(cache_mode(req));
    let access = Access::with_shared_resolver(objects, shared_resolver())
        .with_revocations(IndexedRevocations(KvRevocationIndex::new(revocations)));

    let verified = access.verify(container).await.map_err(|refusal| {
        PresignFailure::authorization(Refusal::Authorization(refusal.reason().clone()))
    })?;
    let authorized = Date::now().as_millis();
    screen_provisioning(container_bytes, env).await?;
    let screened = Date::now().as_millis();
    let described = crate::describe::describe(verified.chain());

    // A write's bytes are the body, metered as declared; the layer
    // reads them as they arrive.
    let declared = declared_length(req).unwrap_or(0);
    let payload: dialog_effects::blob::BlobReader = Box::new(Incoming {
        stream: req
            .stream()
            .map_err(|e| Refusal::unclassified(format!("request body: {e}")))
            .map_err(PresignFailure::authorization)?,
    });
    let answer = match access.perform(verified, Payload::Stream(payload)).await {
        Answer::Unsupported => return Ok(None),
        Answer::Refused(refusal) => {
            return Err(PresignFailure::authorization(Refusal::Authorization(
                refusal.reason().clone(),
            )));
        }
        Answer::Performed(answer) => answer,
    };
    let stored = Date::now().as_millis();
    let cache = access.provider().outcome();
    let fills = access.provider().take_fills();
    if !fills.is_empty() {
        ctx.wait_until(async move {
            for fill in fills {
                fill.await;
            }
        });
    }

    let bytes = match answer.length {
        Some(length) if length > 0 => length,
        _ => declared,
    };
    let headers = Headers::new();
    let _ = headers.set("Content-Type", answer.content_type);
    if let Some(version) = &answer.version {
        let _ = headers.set("ETag", &format!("\"{version}\""));
    }
    for (name, value) in &described {
        let _ = headers.set(name, value);
    }
    let _ = headers.set(
        "Server-Timing",
        &format!(
            "authorize;dur={}, screen;dur={}, store;dur={}, total;dur={}, cache;desc={}",
            authorized.saturating_sub(started),
            screened.saturating_sub(authorized),
            stored.saturating_sub(screened),
            Date::now().as_millis().saturating_sub(started),
            cache.as_str()
        ),
    );
    let response = match answer.body {
        Content::Bytes(body) => Response::from_bytes(body),
        Content::Stream(source) => {
            if let Some(length) = answer.length {
                let _ = headers.set("Content-Length", &length.to_string());
            }
            Response::from_stream(futures_util::stream::unfold(
                source,
                |mut source| async move {
                    match source.next().await {
                        Ok(Some(chunk)) => Some((Ok(chunk), source)),
                        Ok(None) => None,
                        Err(error) => {
                            Some((Err(worker::Error::RustError(error.to_string())), source))
                        }
                    }
                },
            ))
        }
    };
    let response = response
        .map_err(|e| Refusal::unclassified(format!("response error: {e}")))
        .map_err(PresignFailure::authorization)?
        .with_status(answer.status)
        .with_headers(headers);
    Ok(Some((response, bytes)))
}

/// The request body as the layer reads it: chunk by chunk, as it
/// arrives.
#[cfg(target_arch = "wasm32")]
struct Incoming {
    stream: ByteStream,
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl dialog_effects::blob::BlobSource for Incoming {
    async fn next(
        &mut self,
    ) -> std::result::Result<Option<Vec<u8>>, dialog_effects::blob::BlobError> {
        use futures_util::StreamExt as _;
        self.stream
            .next()
            .await
            .transpose()
            .map_err(|error| dialog_effects::blob::BlobError::Storage(error.to_string()))
    }
}

/// Off the worker runtime there is no bucket to perform against; every
/// request is answered with a permit.
#[cfg(not(target_arch = "wasm32"))]
async fn perform(
    _container: dialog_ucan_core::Container,
    _container_bytes: &[u8],
    _req: &mut Request,
    _env: &Env,
    _ctx: &Context,
) -> std::result::Result<Option<(Response, u64)>, PresignFailure> {
    Ok(None)
}

/// Whether the request asks to read past the cache: `Cache-Control:
/// no-cache` or `no-store`, as HTTP says it, or `cache=bypass` in the
/// query, which a deployment config can carry on the endpoint URL. The
/// cache is still filled either way.
#[cfg(target_arch = "wasm32")]
fn cache_mode(req: &Request) -> crate::cached::Mode {
    let by_header = req
        .headers()
        .get("cache-control")
        .ok()
        .flatten()
        .is_some_and(|value| value.contains("no-cache") || value.contains("no-store"));
    let by_query = req.url().is_ok_and(|url| {
        url.query_pairs()
            .any(|(name, value)| name == "cache" && value == "bypass")
    });
    if by_header || by_query {
        crate::cached::Mode::Bypass
    } else {
        crate::cached::Mode::ReadThrough
    }
}

async fn presign(
    body_bytes: &[u8],
    origin: &str,
    env: &Env,
) -> std::result::Result<(Response, u64), PresignFailure> {
    let (authorized_request, stages) = authorize(body_bytes, env).await?;
    let started = stages.started;
    let authorized = stages.authorized;
    let screened = stages.screened;
    let permit_key = permit_key(env).map_err(PresignFailure::authorization)?;

    // Write permits carry the declared size as a signed Content-Length,
    // which is the exact byte figure metering records.
    let bytes = authorized_request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);

    // The authorizer described the operation against its placeholder
    // address; what the client gets is that operation signed for this
    // service's own `/object/` path.
    let permit = Claims::lift(&authorized_request, authorizer_address(), expires)
        .and_then(|claims| permit_key.issue(origin, &claims))
        .map_err(Refusal::unclassified)
        .map_err(PresignFailure::authorization)?;

    let cbor_bytes = serde_ipld_dagcbor::to_vec(&permit)
        .map_err(|e| Refusal::unclassified(format!("failed to serialize response: {e}")))
        .map_err(PresignFailure::authorization)?;
    Response::from_bytes(cbor_bytes)
        .map(|r| {
            let headers = Headers::new();
            let _ = headers.set("Content-Type", "application/cbor");
            // Where the redeem's time went: the chain verify with its
            // revocation lookups, the servability screen, and the whole.
            let _ = headers.set(
                "Server-Timing",
                &format!(
                    "authorize;dur={}, screen;dur={}, total;dur={}",
                    authorized.saturating_sub(started),
                    screened.saturating_sub(authorized),
                    Date::now().as_millis().saturating_sub(started)
                ),
            );
            (r.with_headers(headers), bytes)
        })
        .map_err(|e| Refusal::unclassified(format!("response error: {e}")))
        .map_err(PresignFailure::authorization)
}

/// Screen the subject against the provisioning gate: a space is served
/// only while an active customer pays for it. Registration commands
/// never reach here — `serve` answers them before the presign path —
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
    use crate::provisioning::cache::{self, CachedVerdict};
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
/// the presign path calls it on a cache miss, the registration and
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
#[cfg(target_arch = "wasm32")]
fn unavailable() -> Refusal {
    AuthorizeError::Unavailable {
        detail: "access service unavailable, retry shortly".to_string(),
    }
    .into()
}

thread_local! {
    /// The authorizer built by this isolate, if it has built one.
    static AUTHORIZER: std::cell::OnceCell<UcanAuthorizer> =
        const { std::cell::OnceCell::new() };
    /// The permit key derived by this isolate, if it has derived one.
    static PERMIT_KEY: std::cell::OnceCell<PermitKey> = const { std::cell::OnceCell::new() };
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
static ADDRESS: std::sync::LazyLock<Address> = std::sync::LazyLock::new(|| {
    Address::builder("https://object.invalid")
        .region("auto")
        .bucket("objects")
        .build()
        .expect("the placeholder address is well-formed")
});

/// The key permits are signed and verified with, derived once per
/// isolate from the service seed. A failed derivation is not cached.
pub(crate) fn permit_key(env: &Env) -> std::result::Result<PermitKey, Refusal> {
    PERMIT_KEY.with(|cached| {
        if let Some(key) = cached.get() {
            return Ok(key.clone());
        }
        let seed = env
            .secret("SERVICE_SECRET_KEY")
            .map_err(|e| Refusal::unclassified(format!("Missing SERVICE_SECRET_KEY: {e}")))?
            .to_string();
        let key = PermitKey::derive(&seed).map_err(Refusal::unclassified)?;
        let _ = cached.set(key.clone());
        Ok(key)
    })
}

/// The address the authorizer describes requests against.
///
/// It is a placeholder. The authorizer needs an S3 address to turn a
/// verified invocation into a request, but nothing here talks S3 any
/// more: the request is read back off the permit ([`Claims::lift`])
/// and performed over the R2 binding, or reissued for the client to
/// present at `/object/`. The host does not resolve on purpose, so a
/// permit that escaped this translation fails loudly instead of
/// reaching a bucket.
pub(crate) fn authorizer_address() -> &'static Address {
    &ADDRESS
}

/// The UcanAuthorizer for this isolate.
///
/// Built once and reused: what it carries across requests is its
/// `did:web` resolution cache, which is what makes a chain issued by a
/// web identity cheap to verify the second time. A failed build is
/// not cached: the next request tries again.
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
fn build_authorizer(_env: &Env) -> std::result::Result<UcanAuthorizer, Refusal> {
    Ok(UcanAuthorizer::new(authorizer_address().clone(), None))
}

/// The typed refusal for an authorization failure: the reason itself
/// where the authorizer produced one, `Unclassified` for anything
/// that is not an access decision.
fn map_access_error(err: S3Error) -> Refusal {
    match err {
        S3Error::Authorization(reason) => Refusal::Authorization(reason),
        S3Error::Rejected(rejection) => Refusal::Rejection(rejection),
        other => Refusal::unclassified(other.to_string()),
    }
}
