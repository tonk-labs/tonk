//! UCAN authorization handler.
//!
//! Handles POST /ucan/ requests by:
//! 1. Reading the CBOR-encoded UCAN container from the request body
//! 2. Passing it to UcanAuthorizer for verification and authorization
//! 3. Returning the serialized AuthorizedRequest as CBOR

use crate::error::Refusal;
use dialog_capability::access::AuthorizeError;
use dialog_remote_s3::{Address, S3Error, s3::S3Credential};
use dialog_remote_ucan_s3::UcanAuthorizer;
use worker::*;

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

/// POST /ucan/ → Authorize UCAN invocation and return presigned S3 request
pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, Refusal> {
    // 1. Read the request body as bytes
    let body_bytes = req.bytes().await.map_err(|e| AuthorizeError::Malformed {
        detail: format!("failed to read request body: {e}"),
    })?;

    // 2. Create the UcanAuthorizer from environment config
    let authorizer = create_authorizer(ctx)?;

    // 3. Authorize the UCAN container
    let authorized_request = authorizer
        .authorize(&body_bytes)
        .await
        .map_err(map_access_error)?;

    // 3b. Screen the presented credentials: the window they claim, and
    // whether any of them belongs to a revoked device. Runs only after
    // cryptographic authorization succeeded, and fails closed: a
    // presign the screen cannot clear is refused.
    #[cfg(target_arch = "wasm32")]
    screen_credentials(&body_bytes, ctx).await?;

    // 4. Serialize the response as CBOR
    let cbor_bytes = serde_ipld_dagcbor::to_vec(&authorized_request)
        .map_err(|e| Refusal::unclassified(format!("failed to serialize response: {e}")))?;

    // 5. Return CBOR response
    Response::from_bytes(cbor_bytes)
        .map(|r| {
            let headers = Headers::new();
            let _ = headers.set("Content-Type", "application/cbor");
            r.with_headers(headers)
        })
        .map_err(|e| Refusal::unclassified(format!("response error: {e}")))
}

#[cfg(target_arch = "wasm32")]
async fn screen_credentials(
    body_bytes: &[u8],
    ctx: &RouteContext<()>,
) -> std::result::Result<(), Refusal> {
    use crate::expiry::{WindowVerdict, check_window};
    use crate::revocation::{self, SetVerdict, r2::R2RevocationSource};

    let presented = match revocation::collect_presented(body_bytes) {
        Ok(presented) => presented,
        Err(err) => {
            // The authorizer already accepted this container, so a parse
            // failure here is shape drift between two parsers of the same
            // bytes. There is no key set to screen and no cached verdict
            // to fall back on, so the request cannot be cleared.
            console_error!("revocation screen unavailable, container unparseable: {err}");
            return Err(unavailable());
        }
    };

    // The window screen needs no registry, so it runs first: an expired
    // chain is refused without spending an R2 listing on it.
    let now_ms = Date::now().as_millis();
    match check_window(&presented, now_ms / 1_000) {
        WindowVerdict::Valid => {}
        WindowVerdict::Expired => {
            worker::console_log!("presign rejected: presented chain has expired");
            return Err(AuthorizeError::Expired {
                expiration: presented.expires_at.unwrap_or_default(),
                at: now_ms / 1_000,
            }
            .into());
        }
        WindowVerdict::NotYetValid => {
            worker::console_log!("presign rejected: presented chain is not yet valid");
            return Err(AuthorizeError::NotValidBefore {
                not_before: presented.not_before.unwrap_or_default(),
                at: now_ms / 1_000,
            }
            .into());
        }
    }

    let registry = match ctx.bucket("REVOCATIONS") {
        Ok(bucket) => R2RevocationSource::new(bucket),
        Err(err) => {
            console_error!("revocation screen unavailable, no REVOCATIONS binding: {err}");
            return Err(unavailable());
        }
    };
    match revocation::assess(&registry, &presented, now_ms).await {
        SetVerdict::Allowed => Ok(()),
        SetVerdict::AllowedStale(reason) => {
            console_error!("revocation registry unreachable, serving cached verdicts: {reason}");
            Ok(())
        }
        SetVerdict::Revoked => {
            worker::console_log!("presign rejected: revoked credential present");
            Err(AuthorizeError::Revoked {
                subject: presented.subject.clone(),
            }
            .into())
        }
        SetVerdict::Unavailable(reason) => {
            console_error!("presign refused, revocation registry unreachable: {reason}");
            Err(unavailable())
        }
    }
}

/// The client-facing 503. The reason stays in the logs: it names
/// internal infrastructure and the caller can do nothing with it but
/// retry.
#[cfg(target_arch = "wasm32")]
fn unavailable() -> Refusal {
    AuthorizeError::Unavailable {
        detail: "revocation registry unavailable, retry shortly".to_string(),
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
fn create_authorizer(ctx: &RouteContext<()>) -> std::result::Result<UcanAuthorizer, Refusal> {
    AUTHORIZER.with(|cached| {
        if let Some(authorizer) = cached.get() {
            return Ok(authorizer.clone());
        }
        let authorizer = build_authorizer(ctx)?;
        let _ = cached.set(authorizer.clone());
        Ok(authorizer)
    })
}

/// Create UcanAuthorizer from environment configuration.
fn build_authorizer(ctx: &RouteContext<()>) -> std::result::Result<UcanAuthorizer, Refusal> {
    // Get R2 configuration from environment
    let account_id = ctx
        .var("R2_ACCOUNT_ID")
        .map_err(|e| Refusal::unclassified(format!("Missing R2_ACCOUNT_ID: {e}")))?
        .to_string();

    let access_key_id = ctx
        .secret("R2_ACCESS_KEY_ID")
        .map_err(|e| Refusal::unclassified(format!("Missing R2_ACCESS_KEY_ID: {e}")))?
        .to_string();

    let secret_access_key = ctx
        .secret("R2_SECRET_ACCESS_KEY")
        .map_err(|e| Refusal::unclassified(format!("Missing R2_SECRET_ACCESS_KEY: {e}")))?
        .to_string();

    let bucket = ctx
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
