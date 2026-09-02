//! UCAN authorization handler.
//!
//! Handles POST /ucan/ requests by:
//! 1. Reading the CBOR-encoded UCAN container from the request body
//! 2. Passing it to UcanAuthorizer for verification and authorization
//! 3. Returning the serialized AuthorizedRequest as CBOR
//!
//! Served outside the Router, straight from the fetch event: recording
//! an invocation must outlive the response, and only the event's
//! [`Context`] can extend the isolate's life for it.

use crate::error::Refusal;
#[cfg(target_arch = "wasm32")]
use crate::handlers::registration::handle as handle_registration;
#[cfg(target_arch = "wasm32")]
use crate::registration::registration_command;
use dialog_capability::access::AuthorizeError;
use dialog_remote_s3::{Address, S3Error, s3::S3Credential};
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
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type");
    let _ = headers.set("Access-Control-Expose-Headers", "Content-Type");
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

/// POST /ucan/ → Authorize UCAN invocation and return presigned S3
/// request, recording the invocation in ingest under `ctx.wait_until`.
pub async fn serve(mut req: Request, env: Env, ctx: Context) -> Result<Response> {
    // Refused on size alone, before anything is decoded: a body this
    // large is not a UCAN we failed to parse, and running the parser
    // over it is the work the limit exists to avoid.
    if let Some(declared) = declared_length(&req)
        && declared > max_body_bytes(&env)
    {
        return Ok(with_cors_headers(too_large(max_body_bytes(&env))?));
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
    if body_bytes.len() as u64 > max_body_bytes(&env) {
        return Ok(with_cors_headers(too_large(max_body_bytes(&env))?));
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
    if crate::deletion::is_customer_deletion(&body_bytes) {
        return crate::handlers::deletion::handle_customer(&body_bytes, &env)
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

    let (response, metered) = match presign(&body_bytes, &env).await {
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
        record_invocation(&body_bytes, outcome, reason, bytes, &env, &ctx);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (metered, ctx);

    Ok(with_cors_headers(response))
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

/// Authorize the container and answer the presigned request, together
/// with the declared write bytes when the permit carries them.
async fn presign(
    body_bytes: &[u8],
    env: &Env,
) -> std::result::Result<(Response, u64), PresignFailure> {
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

    let authorized_request = authorizer
        .authorize(body_bytes)
        .await
        .map_err(map_access_error)
        .map_err(PresignFailure::authorization)?;

    #[cfg(target_arch = "wasm32")]
    screen_provisioning(body_bytes, env).await?;

    // Write permits carry the declared size as a signed Content-Length,
    // which is the exact byte figure metering records.
    let bytes = authorized_request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);

    let cbor_bytes = serde_ipld_dagcbor::to_vec(&authorized_request)
        .map_err(|e| Refusal::unclassified(format!("failed to serialize response: {e}")))
        .map_err(PresignFailure::authorization)?;
    Response::from_bytes(cbor_bytes)
        .map(|r| {
            let headers = Headers::new();
            let _ = headers.set("Content-Type", "application/cbor");
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
}

/// The UcanAuthorizer for this isolate.
///
/// It is built from deployment configuration — vars and secrets that
/// an isolate cannot see change — so reading the bindings once and
/// reusing the result costs nothing in freshness. A failed build is
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

/// Create UcanAuthorizer from environment configuration.
fn build_authorizer(env: &Env) -> std::result::Result<UcanAuthorizer, Refusal> {
    // Get R2 configuration from environment
    let account_id = env
        .var("R2_ACCOUNT_ID")
        .map_err(|e| Refusal::unclassified(format!("Missing R2_ACCOUNT_ID: {e}")))?
        .to_string();

    let access_key_id = env
        .secret("R2_ACCESS_KEY_ID")
        .map_err(|e| Refusal::unclassified(format!("Missing R2_ACCESS_KEY_ID: {e}")))?
        .to_string();

    let secret_access_key = env
        .secret("R2_SECRET_ACCESS_KEY")
        .map_err(|e| Refusal::unclassified(format!("Missing R2_SECRET_ACCESS_KEY: {e}")))?
        .to_string();

    let bucket = env
        .var("R2_BUCKET_NAME")
        .map_err(|e| Refusal::unclassified(format!("Missing R2_BUCKET_NAME: {e}")))?
        .to_string();

    // Build R2 endpoint URL
    let endpoint = format!("https://{}.r2.cloudflarestorage.com", account_id);

    // Create S3 credentials for R2 (using "auto" region as R2 requires)
    let address = Address::builder(&endpoint)
        .region("auto")
        .bucket(&bucket)
        .build()
        .map_err(|e| Refusal::unclassified(format!("Failed to create address: {e}")))?;

    let credential = S3Credential::new(access_key_id, secret_access_key);

    Ok(UcanAuthorizer::new(address, Some(credential)))
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
